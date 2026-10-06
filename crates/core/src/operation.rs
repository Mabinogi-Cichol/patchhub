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
        })
    });
    let _reset = Reset;
    work()
}
pub fn check() -> io::Result<()> {
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
