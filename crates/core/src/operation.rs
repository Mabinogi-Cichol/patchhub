//! Optional, thread-scoped task telemetry. Existing library callers remain synchronous.
//! Cancellation is checked at chunk boundaries; rollback deliberately ignores it.
use serde::Serialize;
use std::{
    cell::RefCell,
    io::{self, Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Default, Serialize)]
pub struct Progress {
    pub stage: String,
    pub bytes_done: u64,
    pub total_bytes: Option<u64>,
    pub files_done: usize,
    pub total_files: Option<usize>,
    pub current_file: Option<String>,
    pub archive_bytes: Option<u64>,
}
struct Control {
    cancel: Arc<AtomicBool>,
    report: Box<dyn FnMut(&Progress)>,
    progress: Progress,
    last: Instant,
    restoring: bool,
    gate: Option<Box<dyn Fn() -> io::Result<()>>>,
    safety: Option<Box<dyn Fn() -> io::Result<()>>>,
}
thread_local! { static CONTROL: RefCell<Option<Control>> = const { RefCell::new(None) }; }

pub fn run<T>(
    cancel: Arc<AtomicBool>,
    report: impl FnMut(&Progress) + 'static,
    work: impl FnOnce() -> T,
) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            CONTROL.with(|c| *c.borrow_mut() = None);
        }
    }
    CONTROL.with(|c| {
        *c.borrow_mut() = Some(Control {
            cancel,
            report: Box::new(report),
            progress: Progress::default(),
            last: Instant::now(),
            restoring: false,
            gate: None,
            safety: None,
        })
    });
    let _reset = Reset;
    work()
}
/// Replace the thread's raw request flag with a durably revoked effective flag.
/// Telemetry and successful-commit precedence remain intact.
pub fn with_cancel<T>(cancel: Arc<AtomicBool>, work: impl FnOnce() -> T) -> T {
    if CONTROL.with(|c| c.borrow().is_none()) {
        return run(cancel, |_| {}, work);
    }
    struct Restore(Option<Arc<AtomicBool>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(old) = self.0.take() {
                CONTROL.with(|c| {
                    if let Some(c) = c.borrow_mut().as_mut() {
                        c.cancel = old;
                    }
                });
            }
        }
    }
    let old = CONTROL.with(|c| {
        c.borrow_mut()
            .as_mut()
            .map(|c| std::mem::replace(&mut c.cancel, cancel))
    });
    let _restore = Restore(old);
    work()
}
pub fn cancel_flag() -> Option<Arc<AtomicBool>> {
    CONTROL.with(|c| c.borrow().as_ref().map(|c| c.cancel.clone()))
}

/// Poll only to observe cancellation; there is no total-operation deadline.
/// Dropping the pending network future aborts its IO on the task's runtime.
pub(crate) async fn cancellable<T>(
    future: impl std::future::Future<Output = io::Result<T>>,
) -> io::Result<T> {
    use std::{
        future::{poll_fn, Future},
        task::Poll,
    };
    let mut future = std::pin::pin!(future);
    let mut wake = std::pin::pin!(tokio::time::sleep(Duration::from_millis(25)));
    poll_fn(|context| {
        if let Err(error) = check() {
            return Poll::Ready(Err(error));
        }
        if let Poll::Ready(result) = future.as_mut().poll(context) {
            return Poll::Ready(result);
        }
        if wake.as_mut().poll(context).is_ready() {
            wake.as_mut()
                .reset(tokio::time::Instant::now() + Duration::from_millis(25));
            context.waker().wake_by_ref();
        }
        Poll::Pending
    })
    .await
}
/// Host-owned durable gate runs synchronously at every copy/hash/write checkpoint.
pub fn with_gate<T>(gate: impl Fn() -> io::Result<()> + 'static, work: impl FnOnce() -> T) -> T {
    struct Reset(Option<Box<dyn Fn() -> io::Result<()>>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            CONTROL.with(|c| {
                if let Some(c) = c.borrow_mut().as_mut() {
                    c.gate = self.0.take();
                }
            });
        }
    }
    let old = CONTROL.with(|c| {
        c.borrow_mut()
            .as_mut()
            .and_then(|c| c.gate.replace(Box::new(gate)))
    });
    let _reset = Reset(old);
    work()
}
/// Installer safety checks are independent from the task lease gate, including rollback.
pub(crate) fn with_safety<T>(
    gate: impl Fn() -> io::Result<()> + 'static,
    work: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<Box<dyn Fn() -> io::Result<()>>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            CONTROL.with(|c| {
                if let Some(c) = c.borrow_mut().as_mut() {
                    c.safety = self.0.take();
                }
            });
        }
    }
    let old = CONTROL.with(|c| {
        c.borrow_mut()
            .as_mut()
            .and_then(|c| c.safety.replace(Box::new(gate)))
    });
    let _reset = Reset(old);
    work()
}
pub fn check() -> io::Result<()> {
    CONTROL.with(|c| {
        if let Some(c) = c.borrow().as_ref() {
            if let Some(safety) = &c.safety {
                safety()?;
            }
            if !c.restoring {
                if let Some(gate) = &c.gate {
                    gate()?;
                }
            }
        }
        Ok::<_, io::Error>(())
    })?;
    let cancelled = CONTROL.with(|c| {
        c.borrow()
            .as_ref()
            .is_some_and(|c| !c.restoring && c.cancel.load(Ordering::Relaxed))
    });
    if cancelled {
        Err(io::Error::other("task_cancelled"))
    } else {
        Ok(())
    }
}
fn update(force: bool, edit: impl FnOnce(&mut Progress)) {
    CONTROL.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut() {
            edit(&mut c.progress);
            if force || c.last.elapsed() >= Duration::from_millis(100) {
                (c.report)(&c.progress);
                c.last = Instant::now();
            }
        }
    });
}
pub fn stage(name: &str, total: Option<u64>, files: Option<usize>) {
    update(true, |p| {
        p.stage = name.into();
        p.bytes_done = 0;
        p.total_bytes = total;
        p.files_done = 0;
        p.total_files = files;
        p.current_file = None;
    });
}
pub fn archive(bytes: u64) {
    update(true, |p| p.archive_bytes = Some(bytes));
}
pub fn file(done: usize, path: &str) {
    update(true, |p| {
        p.files_done = done;
        p.current_file = Some(path.into());
    });
}
pub fn advance(bytes: u64) {
    let force = CONTROL.with(|c| {
        c.borrow().as_ref().is_some_and(|c| {
            c.progress.bytes_done == 0
                || c.progress
                    .total_bytes
                    .is_some_and(|total| c.progress.bytes_done.saturating_add(bytes) >= total)
        })
    });
    update(force, |p| p.bytes_done += bytes);
}
pub fn finish_file(done: usize) {
    update(true, |p| p.files_done = done);
}
/// A durable successful commit wins over a cancellation received afterwards.
pub(crate) fn committed() {
    CONTROL.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut() {
            c.restoring = true;
        }
    });
}

pub fn restoring() {
    CONTROL.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut() {
            c.restoring = true;
        }
    });
    stage("rolling_back", None, None);
}

pub fn copy(reader: &mut impl Read, writer: &mut impl Write) -> io::Result<u64> {
    let mut buffer = [0u8; 65536];
    let mut total = 0;
    loop {
        check()?;
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        writer.write_all(&buffer[..count])?;
        total += count as u64;
        advance(count as u64);
    }
    Ok(total)
}
