//! Ordinary child-process I/O and lifetime management; no access isolation.
use crate::production::{cancelled, err, ioerr};
use crate::WorkerError;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::time::Duration;

pub(crate) struct Outcome {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

// std::process::Child does not kill or reap on Drop. Keep that guarantee on
// every early return, including failures while setting up or reading pipes.
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(crate) fn run(
    worker: &Path,
    archive: &Path,
    descriptor: &[u8],
    cancel: &AtomicBool,
    stream: Option<SyncSender<Vec<u8>>>,
) -> Result<Outcome, WorkerError> {
    cancelled(cancel)?;
    let mut command = Command::new(worker);
    command
        .arg(archive)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_lifetime(&mut command)?;
    let mut child = ChildGuard(command.spawn().map_err(|e| ioerr("spawn", e))?);
    #[cfg(windows)]
    let _job = windows_lifetime::Job::attach(&child.0)?;
    let mut stdin = child.0.stdin.take().unwrap();
    let mut stdout = child.0.stdout.take().unwrap();
    let mut stderr = child.0.stderr.take().unwrap();
    let io_failed = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let failed = &io_failed;
        let input = scope.spawn(move || {
            let result = stdin.write_all(descriptor).map_err(|e| ioerr("stdin", e));
            drop(stdin);
            if result.is_err() {
                failed.store(true, Ordering::Release);
            }
            result
        });
        let output = scope.spawn(move || {
            let result = (|| {
                let mut captured = Vec::new();
                let mut buffer = vec![0; 64 * 1024];
                loop {
                    let n = stdout.read(&mut buffer).map_err(|e| ioerr("stdout", e))?;
                    if n == 0 {
                        break;
                    }
                    if let Some(tx) = &stream {
                        tx.send(buffer[..n].to_vec())
                            .map_err(|_| err("stdout", "output consumer closed"))?;
                    } else {
                        captured.extend_from_slice(&buffer[..n]);
                    }
                }
                Ok(captured)
            })();
            if result.is_err() {
                failed.store(true, Ordering::Release);
            }
            result
        });
        let errors = scope.spawn(move || {
            let mut bytes = Vec::new();
            let result = stderr
                .read_to_end(&mut bytes)
                .map(|_| bytes)
                .map_err(|e| ioerr("stderr", e));
            if result.is_err() {
                failed.store(true, Ordering::Release);
            }
            result
        });
        let status = (|| loop {
            if let Some(status) = child.0.try_wait().map_err(|e| ioerr("wait", e))? {
                break Ok(status);
            }
            if cancel.load(Ordering::Acquire) || io_failed.load(Ordering::Acquire) {
                child.0.kill().map_err(|e| ioerr("terminate", e))?;
                break child.0.wait().map_err(|e| ioerr("wait", e));
            }
            // Polling checks cancellation; there is no execution timeout.
            std::thread::sleep(Duration::from_millis(20));
        })();
        if status.is_err() {
            // Close child pipe endpoints before joining the I/O threads.
            let _ = child.0.kill();
            let _ = child.0.wait();
        }
        let input_result = input
            .join()
            .map_err(|_| err("stdin", "input writer panicked"))?;
        let stdout_result = output
            .join()
            .map_err(|_| err("stdout", "output reader panicked"))?;
        let stderr_result = errors
            .join()
            .map_err(|_| err("stderr", "error reader panicked"))?;
        cancelled(cancel)?;
        input_result?;
        Ok(Outcome {
            status: status?,
            stdout: stdout_result?,
            stderr: stderr_result?,
        })
    })
}

#[cfg(windows)]
fn configure_lifetime(command: &mut Command) -> Result<(), WorkerError> {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    Ok(())
}

#[cfg(target_os = "linux")]
fn configure_lifetime(command: &mut Command) -> Result<(), WorkerError> {
    use std::os::unix::process::CommandExt;
    // PR_SET_PDEATHSIG only manages lifetime; credentials, environment,
    // namespaces and resource limits remain those of the ordinary child.
    unsafe extern "C" {
        fn getppid() -> i32;
        fn prctl(option: i32, arg2: usize, arg3: usize, arg4: usize, arg5: usize) -> i32;
    }
    let parent = std::process::id() as i32;
    unsafe {
        command.pre_exec(move || {
            if prctl(1, 9, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if getppid() != parent {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "worker host exited",
                ));
            }
            Ok(())
        });
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn configure_lifetime(_command: &mut Command) -> Result<(), WorkerError> {
    Err(err("spawn", "worker platform unsupported"))
}

#[cfg(windows)]
mod windows_lifetime {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub(super) struct Job(HANDLE);
    impl Job {
        pub(super) fn attach(child: &Child) -> Result<Self, WorkerError> {
            let raw = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
                .map_err(|e| err("job", e.to_string()))?;
            let job = Self(raw);
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of_val(&info) as u32,
                )
                .map_err(|e| err("job", e.to_string()))?;
                AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle()))
                    .map_err(|e| err("job", e.to_string()))?;
            }
            Ok(job)
        }
    }
    impl Drop for Job {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Instant;

    // The Rust test binary doubles as a disposable worker fixture when the
    // child is launched with this test's name as its only filter argument.
    #[test]
    fn worker_process_fixture() {
        if !std::env::args().any(|arg| arg == "process::tests::worker_process_fixture") {
            return;
        }
        let mut descriptor = String::new();
        std::io::stdin().read_to_string(&mut descriptor).unwrap();
        let descriptor: serde_json::Value = serde_json::from_str(&descriptor).unwrap();
        fs::write(
            descriptor["pid_path"].as_str().unwrap(),
            std::process::id().to_string(),
        )
        .unwrap();
        loop {
            if descriptor["stream"] == true {
                std::io::stdout().write_all(&[b'x'; 64 * 1024]).unwrap();
                std::io::stdout().flush().unwrap();
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    #[test]
    fn cancelling_a_running_silent_worker_kills_and_reaps_it() {
        let root = tempfile::tempdir().unwrap();
        let pid_path = root.path().join("pid");
        let descriptor =
            serde_json::to_vec(&serde_json::json!({"pid_path":pid_path,"stream":false})).unwrap();
        let cancel = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let launched = scope.spawn(|| {
                run(
                    &std::env::current_exe().unwrap(),
                    Path::new("process::tests::worker_process_fixture"),
                    &descriptor,
                    &cancel,
                    None,
                )
            });
            let start = Instant::now();
            while !pid_path.exists() && start.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(10));
            }
            cancel.store(true, Ordering::Release);
            let error = launched.join().unwrap().err().unwrap();
            assert_eq!(error.stage, "cancelled");
            let pid = fs::read_to_string(&pid_path)
                .expect("fixture started before cancellation")
                .parse()
                .unwrap();
            assert!(!process_alive(pid), "cancelled child remained alive");
        });
    }

    #[test]
    fn closing_stream_consumer_kills_and_reaps_the_worker() {
        let root = tempfile::tempdir().unwrap();
        let pid_path = root.path().join("pid");
        let descriptor =
            serde_json::to_vec(&serde_json::json!({"pid_path":pid_path,"stream":true})).unwrap();
        let cancel = AtomicBool::new(false);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::scope(|scope| {
            let launched = scope.spawn(|| {
                run(
                    &std::env::current_exe().unwrap(),
                    Path::new("process::tests::worker_process_fixture"),
                    &descriptor,
                    &cancel,
                    Some(tx),
                )
            });
            let first = rx.recv_timeout(Duration::from_secs(10));
            drop(rx);
            if first.is_err() {
                cancel.store(true, Ordering::Release);
            }
            let error = launched.join().unwrap().err().unwrap();
            assert!(first.is_ok(), "fixture produced no stream");
            assert_eq!(error.stage, "stdout");
            let pid = fs::read_to_string(pid_path).unwrap().parse().unwrap();
            assert!(
                !process_alive(pid),
                "worker remained alive after consumer failure"
            );
        });
    }

    #[cfg(windows)]
    fn process_alive(pid: u32) -> bool {
        use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return false;
            };
            let mut code = 0;
            let alive =
                GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
            let _ = CloseHandle(handle);
            alive
        }
    }
    #[cfg(target_os = "linux")]
    fn process_alive(pid: u32) -> bool {
        unsafe extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        unsafe { kill(pid as i32, 0) == 0 }
    }
}
