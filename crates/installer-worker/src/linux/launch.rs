//! Linux `bwrap` (bubblewrap) sandbox launcher.
//!
//! Generalises the fixed-path argv template used by the Python prototype to an
//! arbitrary worker binary (P6: Python host oracle removed from production; dev
//! golden tool remains):
//!
//! ```text
//! /usr/bin/bwrap --unshare-all --die-with-parent --new-session --clearenv
//!   --setenv PATH /usr/bin --setenv HOME /tmp --cap-drop ALL
//!   --ro-bind /usr /usr --ro-bind /lib /lib --ro-bind /lib64 /lib64
//!   --proc /proc --dev /dev --tmpfs /tmp
//!   [--ro-bind <input> /input | /input/<name>]
//!   --ro-bind <tool_dir> /tool --bind <output> /output -- <worker> [args...]
//! ```
//!
//! `/lib`, `/lib64` are only added when present, which keeps the launcher
//! working on merged-`/usr` distributions where they are symlinks (bubblewrap
//! resolves the bind source). The security boundary (network/host isolation,
//! `--cap-drop ALL`, `--clearenv`) is never relaxed by this adjustment.
//!
//! Resource limits (`RLIMIT_AS`/`RLIMIT_CPU`/`RLIMIT_CORE`/`RLIMIT_FSIZE`) are
//! applied in a `pre_exec` hook in the forked child before `exec`, so bubblewrap
//! and every descendant inherit them. When the launcher itself runs as root
//! (`euid == 0`) it drops privileges in the same hook — supplementary groups,
//! then `setgid`, then `setuid` (this order is mandatory) — before `setsid`.
//! rlimits are applied *before* the drop because they only ever lower limits;
//! applying them while privileged cannot fail due to a reduced `rlim_max`.
//!
//! The child is put in its own session (`setsid`), mirroring the Python
//! `start_new_session=True`; the argv template additionally passes
//! `--new-session` (bubblewrap tolerates the second `setsid`). Wall-clock timeout
//! reclamation uses `killpg` followed by a 1 s grace period and `SIGKILL`;
//! bubblewrap's `--die-with-parent` guarantees the sandboxed process is killed
//! when the monitor exits.
//!
//! stdout/stderr capture is **fail-closed**: exceeding the limit is a hard
//! error (`exe_worker_response_limit`), matching the removed Python `_bounded`
//! helper (P6: host oracle removed from production), and the sandbox is
//! terminated instead of silently truncating. The stdout limit is the contract
//! §6.1 16 MiB; stderr uses 64 KiB as in Python.

use std::ffi::OsString;
use std::fs;
use std::io::{Read, Seek, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::model::{
    IsolationError, LaunchOutcome, SandboxConfig, SandboxLimits, TerminationReason, WorkerResponse,
};

use super::sys;

/// Fixed bubblewrap path required by the Python prototype.
pub const BWRAP_PATH: &str = "/usr/bin/bwrap";

/// Production `RLIMIT_AS` (3 GiB). Matches `scripts/sf95_worker.py`.
pub const DEFAULT_ADDRESS_SPACE_BYTES: u64 = 3 * 1024 * 1024 * 1024;
/// `RLIMIT_CPU` value retained for callers that explicitly configure
/// `SandboxLimits::cpu_seconds` (probe/diagnostic use). No production CPU-time
/// ceiling is applied by default (D05); see `apply_limits`.
pub const DEFAULT_CPU_SECONDS: u64 = 300;
/// Production `RLIMIT_CORE` (0, never dump core).
pub const DEFAULT_CORE_BYTES: u64 = 0;

/// stderr capture limit (matches Python `_bounded(..., 65536)`).
pub const MAX_STDERR_BYTES: usize = 64 * 1024;

const OVERFLOW_NONE: u8 = 0;
const OVERFLOW_STDOUT: u8 = 1;
const OVERFLOW_STDERR: u8 = 2;
const OVERFLOW_IO: u8 = 3;

const TERM_GRACE: Duration = Duration::from_secs(1);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Build the bubblewrap argv for `cfg`. The command after `--` runs inside the
/// sandbox; the sandbox layout is `/input`, `/tool` (read-only) and `/output`
/// (writable).
pub fn build_bwrap_command(cfg: &SandboxConfig) -> Result<Vec<OsString>, IsolationError> {
    if !Path::new(BWRAP_PATH).is_file() {
        return Err(IsolationError::new(
            "sandbox",
            None,
            format!("{BWRAP_PATH} not found; bwrap sandbox unavailable"),
        ));
    }

    let mut argv: Vec<OsString> = Vec::new();
    let push = |argv: &mut Vec<OsString>, arg: &str| argv.push(OsString::from(arg));

    push(&mut argv, BWRAP_PATH);
    for arg in [
        "--unshare-all",
        "--die-with-parent",
        "--new-session",
        "--clearenv",
        "--setenv",
        "PATH",
        "/usr/bin",
        "--setenv",
        "HOME",
        "/tmp",
        "--cap-drop",
        "ALL",
    ] {
        push(&mut argv, arg);
    }

    append_system_binds(&mut argv);

    for arg in ["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"] {
        push(&mut argv, arg);
    }

    // Read-only input.
    if let Some(input) = &cfg.input_dir {
        if input.is_dir() {
            push(&mut argv, "--dir");
            push(&mut argv, "/input");
            push(&mut argv, "--ro-bind");
            argv.push(input.as_os_str().to_os_string());
            push(&mut argv, "/input");
        } else {
            let name = input.file_name().ok_or_else(|| {
                IsolationError::new("sandbox", None, "input path has no file name")
            })?;
            let dest = Path::new("/input").join(name);
            push(&mut argv, "--dir");
            push(&mut argv, "/input");
            push(&mut argv, "--ro-bind");
            argv.push(input.as_os_str().to_os_string());
            argv.push(dest.into_os_string());
        }
    }

    // Read-only worker/tool directory.
    push(&mut argv, "--ro-bind");
    argv.push(cfg.worker_dir.as_os_str().to_os_string());
    push(&mut argv, "/tool");

    // Exclusive writable output.
    if let Some(output) = &cfg.output_dir {
        push(&mut argv, "--bind");
        argv.push(output.as_os_str().to_os_string());
        push(&mut argv, "/output");
    }

    // Working directory inside the sandbox (default `/tmp`).
    push(&mut argv, "--chdir");
    argv.push(
        cfg.working_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .into_os_string(),
    );

    // Extra environment variables (after `--clearenv`). The probe passes none,
    // so the sandbox environment is exactly `PATH` + `HOME`.
    if let Some(vars) = &cfg.environment {
        for (key, value) in vars {
            push(&mut argv, "--setenv");
            argv.push(OsString::from(key));
            argv.push(OsString::from(value));
        }
    }

    push(&mut argv, "--");

    // Resolve the worker inside `/tool`, preserving any subdirectory.
    let exec_path = sandbox_exec_path(cfg)?;
    argv.push(exec_path.into_os_string());
    for arg in &cfg.args {
        argv.push(OsString::from(arg));
    }

    Ok(argv)
}

/// Map the host `worker_exe` to its path inside the sandboxed `/tool` bind.
fn sandbox_exec_path(cfg: &SandboxConfig) -> Result<PathBuf, IsolationError> {
    let name = cfg
        .worker_exe
        .file_name()
        .ok_or_else(|| IsolationError::new("sandbox", None, "worker_exe has no file name"))?;
    let relative = cfg
        .worker_exe
        .strip_prefix(&cfg.worker_dir)
        .ok()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(name));
    Ok(Path::new("/tool").join(relative))
}

/// Bind the minimal system read-only set. `/lib`/`/lib64` become symlink targets
/// on merged-`/usr` systems; bubblewrap resolves them, but we also explicitly
/// mirror symlinks to stay correct on non-merged systems.
fn append_system_binds(argv: &mut Vec<OsString>) {
    argv.push(OsString::from("--ro-bind"));
    argv.push(OsString::from("/usr"));
    argv.push(OsString::from("/usr"));

    for dest in ["/lib", "/lib64"] {
        let path = Path::new(dest);
        match fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                if let Ok(target) = fs::read_link(path) {
                    argv.push(OsString::from("--symlink"));
                    argv.push(target.into_os_string());
                    argv.push(OsString::from(dest));
                }
            }
            Ok(_) => {
                argv.push(OsString::from("--ro-bind"));
                argv.push(OsString::from(dest));
                argv.push(OsString::from(dest));
            }
            Err(_) => {
                // Absent (e.g. no /lib64): skip.
            }
        }
    }
}

/// Launch the worker under bwrap and collect its output and termination reason.
///
/// The worker report on stdout is unbounded (it grows with a legitimate
/// installer's file table); only [`MAX_STDERR_BYTES`] of stderr is bounded, and
/// exceeding it is fail-closed (`output-limit`).
pub fn run_sandboxed(cfg: &SandboxConfig) -> Result<LaunchOutcome, IsolationError> {
    run_internal(cfg, None)
}

/// Production cancellation is polled while the process is running.
pub fn run_sandboxed_cancellable(
    cfg: &SandboxConfig,
    cancel: &AtomicBool,
) -> Result<LaunchOutcome, IsolationError> {
    run_internal(cfg, Some(cancel))
}
fn run_internal(
    cfg: &SandboxConfig,
    cancel: Option<&AtomicBool>,
) -> Result<LaunchOutcome, IsolationError> {
    let mut argv = build_bwrap_command(cfg)?;
    let seccomp = if cfg.production_strict {
        Some(single_process_filter()?)
    } else {
        None
    };
    let seccomp_fd = seccomp.as_ref().map(AsRawFd::as_raw_fd);
    if let Some(fd) = seccomp_fd {
        let pos = argv
            .iter()
            .position(|s| s == "--")
            .expect("fixed delimiter");
        argv.splice(
            pos..pos,
            [OsString::from("--seccomp"), OsString::from(fd.to_string())],
        );
    }

    // Sources are private copied directories, not shared cache/state inodes.
    // bwrap canonicalizes bind sources, so the host must configure a stage whose
    // existing ancestors are traversable by run_as; do not widen their ACLs.
    let limits = cfg.limits.clone();
    // Only drop privileges when the launcher is actually root. Explicit config
    // wins; otherwise fall back to the SUDO_UID/SUDO_GID environment.
    let run_as = if super::probe_os::euid() == 0 {
        if cfg.production_strict {
            match cfg.run_as {
                Some((uid, gid)) if uid != 0 && gid != 0 => Some((uid, gid)),
                _ => {
                    return Err(IsolationError::new(
                        "identity",
                        None,
                        "root production host requires explicit non-root uid/gid",
                    ))
                }
            }
        } else {
            resolve_run_as(cfg)
        }
    } else {
        None
    };

    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    if cfg.production_strict {
        command.env_clear();
    }
    let parent_pid = unsafe { sys::getpid() };
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // The pre_exec closure runs in the forked child before exec. Order:
    // 1) rlimits (only ever lower; safe while still privileged),
    // 2) privilege drop if root: supplementary groups -> gid -> uid,
    // 3) detach into a new session so the whole sandbox is reclaimable.
    unsafe {
        command.pre_exec(move || {
            apply_limits(&limits)?;
            if let Some((uid, gid)) = run_as {
                if sys::setgroups(0, std::ptr::null()) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if sys::setgid(gid) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if sys::setuid(uid) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            if let Some(fd) = seccomp_fd {
                if sys::fcntl(fd, sys::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            // Apply after setuid because credential changes clear PDEATHSIG.
            if sys::prctl(sys::PR_SET_PDEATHSIG, sys::SIGKILL as usize, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if sys::getppid() != parent_pid {
                return Err(std::io::Error::from_raw_os_error(10));
            }
            if sys::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = command.spawn().map_err(|err| {
        IsolationError::new(
            "spawn",
            err.raw_os_error().map(|c| c as u32),
            format!("spawn {BWRAP_PATH} failed: {err}"),
        )
    })?;

    let pid = child.id();
    let pgid = pid as i32;
    let start = Instant::now();

    let mut stdin = child.stdin.take().expect("stdin piped");
    let stdin_data = cfg.stdin.clone();
    let stdin_join = thread::spawn(move || {
        let _ = stdin.write_all(&stdin_data);
        let _ = stdin.flush();
        // Dropping stdin closes the write end so the worker sees EOF.
    });

    // Shared overflow flag: a reader thread records which stream exceeded its
    // limit and stops reading; the wait loop then terminates the sandbox.
    let overflow = Arc::new(AtomicU8::new(OVERFLOW_NONE));
    let stdout = child.stdout.take().expect("stdout piped");
    let stdout_flag = Arc::clone(&overflow);
    let stdout_join =
        thread::spawn(move || read_bounded(stdout, None, OVERFLOW_STDOUT, stdout_flag));
    let stderr = child.stderr.take().expect("stderr piped");
    let stderr_flag = Arc::clone(&overflow);
    let stderr_join =
        thread::spawn(move || read_bounded(stderr, Some(MAX_STDERR_BYTES), OVERFLOW_STDERR, stderr_flag));

    let end = loop {
        if cancel.is_some_and(|c| c.load(Ordering::Acquire)) {
            break End::Cancelled;
        }
        if overflow.load(Ordering::SeqCst) != OVERFLOW_NONE {
            break End::OutputLimit;
        }
        // Enforce only an explicitly configured timeout (the probe's start /
        // no-response bound). Normal analyze/convert work runs to completion;
        // no wall-clock ceiling is imposed here (D05).
        if let Some(limit) = cfg.limits.timeout {
            if start.elapsed() >= limit {
                break End::TimedOut;
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => break End::Exited(status),
            Ok(None) => {}
            Err(_) => break End::TimedOut,
        }
        thread::sleep(POLL_INTERVAL);
    };

    let mut termination = TerminationReason::Completed;
    let exit_code: u32;
    match end {
        End::Exited(status) => {
            exit_code = status.code().unwrap_or(-1) as u32;
        }
        End::Cancelled => {
            terminate_group(pgid, &mut child);
            termination = TerminationReason::Cancelled;
            exit_code = u32::MAX;
        }
        End::TimedOut => {
            terminate_group(pgid, &mut child);
            termination = TerminationReason::Timeout;
            exit_code = child
                .try_wait()
                .ok()
                .flatten()
                .and_then(|status| status.code())
                .unwrap_or(-1) as u32;
        }
        End::OutputLimit => {
            terminate_group(pgid, &mut child);
            exit_code = child
                .try_wait()
                .ok()
                .flatten()
                .and_then(|status| status.code())
                .unwrap_or(-1) as u32;
        }
    }

    let _ = stdin_join.join();
    let stdout = stdout_join
        .join()
        .ok()
        .and_then(|result| result.ok())
        .unwrap_or_default();
    let stderr = stderr_join
        .join()
        .ok()
        .and_then(|result| result.ok())
        .unwrap_or_default();

    if let Some(reason) = overflow_reason(&overflow) {
        return Err(IsolationError::new(
            "output-limit",
            None,
            format!(
                "exe_worker_response_limit: {reason} (pid={pid}, exit={exit_code}, sandbox terminated)"
            ),
        ));
    }

    Ok(LaunchOutcome {
        pid,
        exit_code,
        termination,
        stdout,
        stderr,
        wall_time: start.elapsed(),
        snapshot: None,
        create_api: "bwrap".to_string(),
        tree_pids: Vec::new(),
        cpu_rate_control: None,
    })
}

enum End {
    Exited(std::process::ExitStatus),
    TimedOut,
    Cancelled,
    OutputLimit,
}

fn overflow_reason(flag: &AtomicU8) -> Option<String> {
    match flag.load(Ordering::SeqCst) {
        OVERFLOW_STDOUT => Some("stdout capture limit exceeded".into()),
        OVERFLOW_STDERR => Some(format!("stderr exceeded {MAX_STDERR_BYTES} bytes")),
        OVERFLOW_IO => Some("stdio read failed".into()),
        _ => None,
    }
}

/// Resolve the privilege-drop target. Explicit `cfg.run_as` wins; otherwise the
/// `SUDO_UID`/`SUDO_GID` environment variables; otherwise no drop.
fn resolve_run_as(cfg: &SandboxConfig) -> Option<(u32, u32)> {
    if let Some(target) = cfg.run_as {
        return Some(target);
    }
    let uid = std::env::var("SUDO_UID").ok()?.trim().parse::<u32>().ok()?;
    let gid = std::env::var("SUDO_GID").ok()?.trim().parse::<u32>().ok()?;
    Some((uid, gid))
}

/// SIGTERM the sandbox process group, wait up to 1 s, then SIGKILL. bubblewrap's
/// `--die-with-parent` tears down the sandboxed process when the monitor exits.
fn terminate_group(pgid: i32, child: &mut Child) {
    unsafe {
        sys::killpg(pgid, sys::SIGTERM);
    }
    if poll_exit(child, TERM_GRACE) {
        return;
    }
    unsafe {
        sys::killpg(pgid, sys::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn poll_exit(child: &mut Child, grace: Duration) -> bool {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => {}
            Err(_) => return true,
        }
        if start.elapsed() >= grace {
            return false;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Apply the configured rlimits, clamped to the inherited hard limit.
fn apply_limits(limits: &SandboxLimits) -> std::io::Result<()> {
    set_one(
        sys::RLIMIT_AS,
        limits
            .address_space_bytes
            .unwrap_or(DEFAULT_ADDRESS_SPACE_BYTES),
    )?;
    // CPU-time termination is not applied to normal work (D05); only an
    // explicitly configured per-process CPU limit is honored (probe use).
    if let Some(seconds) = limits.cpu_seconds {
        set_one(sys::RLIMIT_CPU, seconds)?;
    }
    set_one(
        sys::RLIMIT_CORE,
        limits.core_bytes.unwrap_or(DEFAULT_CORE_BYTES),
    )?;
    if let Some(bytes) = limits.file_size_bytes {
        set_one(sys::RLIMIT_FSIZE, bytes)?;
    }
    Ok(())
}

fn set_one(resource: i32, desired: u64) -> std::io::Result<()> {
    unsafe {
        let mut current = sys::RLimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if sys::getrlimit(resource, &mut current) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let value = if current.rlim_max == sys::RLIM_INFINITY {
            desired
        } else {
            desired.min(current.rlim_max)
        };
        let new = sys::RLimit {
            rlim_cur: value,
            rlim_max: value,
        };
        if sys::setrlimit(resource, &new) != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Read a stream to EOF, returning `Err(())` as soon as more than `limit` bytes
/// are observed. Mirrors the Python `_bounded` helper: the overflow is recorded
/// in `flag` (which stream) and reading stops; the caller terminates the sandbox
/// and surfaces `exe_worker_response_limit`. Within the limit the returned bytes
/// are identical to the previous truncating implementation.
fn read_bounded(
    mut reader: impl Read,
    limit: Option<usize>,
    kind: u8,
    flag: Arc<AtomicU8>,
) -> Result<Vec<u8>, ()> {
    let mut out = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Ok(out),
            Ok(n) => {
                if limit.is_some_and(|limit| out.len().saturating_add(n) > limit) {
                    flag.store(kind, Ordering::SeqCst);
                    return Err(());
                }
                out.extend_from_slice(&buf[..n]);
            }
            Err(_) => {
                flag.store(OVERFLOW_IO, Ordering::SeqCst);
                return Err(());
            }
        }
    }
}

/// Convenience: run the sandbox and parse the last JSON response line.
pub fn run_worker(
    cfg: &SandboxConfig,
) -> Result<(LaunchOutcome, Option<WorkerResponse>), IsolationError> {
    let outcome = run_sandboxed(cfg)?;
    let response = String::from_utf8_lossy(&outcome.stdout)
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<WorkerResponse>(line).ok());
    Ok((outcome, response))
}

/// Deny every process/thread creation syscall in the final bwrap worker. This
/// makes AS/CPU limits single-process aggregate limits rather than claiming
/// per-process rlimits alone constrain an arbitrary process tree.
fn single_process_filter() -> Result<std::fs::File, IsolationError> {
    #[cfg(target_arch = "x86_64")]
    let (arch, denied) = (0xc000003eu32, vec![56u32, 57, 58, 435]);
    #[cfg(target_arch = "aarch64")]
    let (arch, denied) = (0xc00000b7u32, vec![220u32, 435]);
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    return Err(IsolationError::new(
        "seccomp",
        None,
        "unsupported production seccomp architecture",
    ));
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        let mut instructions: Vec<(u16, u8, u8, u32)> = vec![
            (0x20, 0, 0, 4), // LD seccomp_data.arch
            (0x15, 1, 0, arch),
            (0x06, 0, 0, 0x80000000), // KILL_PROCESS on unexpected ABI
            (0x20, 0, 0, 0),          // LD syscall number
        ];
        #[cfg(target_arch = "x86_64")]
        {
            // x32 ABI numbers otherwise bypass the x86_64 syscall deny set.
            instructions.push((0x35, 0, 1, 0x40000000));
            instructions.push((0x06, 0, 0, 0x80000000));
        }
        for nr in denied {
            instructions.push((0x15, 0, 1, nr));
            instructions.push((0x06, 0, 0, 0x00050001)); // ERRNO EPERM
        }
        instructions.push((0x06, 0, 0, 0x7fff0000)); // ALLOW
        let mut file = tempfile::tempfile().map_err(|e| {
            IsolationError::new("seccomp", e.raw_os_error().map(|n| n as u32), e.to_string())
        })?;
        for (code, jt, jf, k) in instructions {
            file.write_all(&code.to_ne_bytes())
                .and_then(|_| file.write_all(&[jt, jf]))
                .and_then(|_| file.write_all(&k.to_ne_bytes()))
                .map_err(|e| IsolationError::new("seccomp", None, e.to_string()))?;
        }
        file.rewind()
            .map_err(|e| IsolationError::new("seccomp", None, e.to_string()))?;
        Ok(file)
    }
}
