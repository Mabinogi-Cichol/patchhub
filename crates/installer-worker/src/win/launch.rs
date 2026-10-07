//! Process launch with LPAC attributes, stdio pipes, Job Object assignment and
//! bounded resource reclamation.

use core::ffi::c_void;
use core::mem::size_of;
use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    SetHandleInformation, HANDLE, HANDLE_FLAGS, HANDLE_FLAG_INHERIT, WAIT_OBJECT_0,
};
use windows::Win32::Security::{PSID, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessAsUserW, CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, ResumeThread, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW,
};
use windows::Win32::System::WindowsProgramming::PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT;

use super::appcontainer::ensure_profile;
use super::job::Job;
use super::util::{close_handle, env_block, hresult_to_win32, hstring, last_error, wide};
use crate::model::{IsolationError, LaunchOutcome, SandboxConfig, TerminationReason};

struct PipeHandles {
    read: HANDLE,
    write: HANDLE,
}

fn inheritable_security_attributes() -> SECURITY_ATTRIBUTES {
    SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: windows::core::BOOL(1),
    }
}

fn create_inheritable_pipe() -> Result<PipeHandles, IsolationError> {
    let sa = inheritable_security_attributes();
    let mut read = HANDLE::default();
    let mut write = HANDLE::default();
    unsafe { CreatePipe(&mut read, &mut write, Some(&sa), 0) }.map_err(|err| {
        IsolationError::new(
            "pipe",
            Some(hresult_to_win32(&err)),
            format!("CreatePipe failed: {err}"),
        )
    })?;
    Ok(PipeHandles { read, write })
}

fn set_inherit(handle: HANDLE, inherit: bool) -> Result<(), IsolationError> {
    let flags = if inherit {
        HANDLE_FLAG_INHERIT
    } else {
        HANDLE_FLAGS(0)
    };
    unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, flags) }.map_err(|err| {
        IsolationError::new(
            "pipe",
            Some(hresult_to_win32(&err)),
            format!("SetHandleInformation failed: {err}"),
        )
    })
}

/// Launch the worker under LPAC + Job Object and collect its output.
pub fn run_sandboxed(cfg: &SandboxConfig) -> Result<LaunchOutcome, IsolationError> {
    let profile = ensure_profile(&cfg.profile_name)?;

    let mut job = Job::create()?;
    job.set_limits(&cfg.limits)?;
    let cpu_rate_control = job.cpu_rate_control().ok();

    let stdin_pipe = create_inheritable_pipe()?;
    let stdout_pipe = create_inheritable_pipe()?;
    let stderr_pipe = create_inheritable_pipe()?;

    // Parent-only ends must not be inherited by the child.
    set_inherit(stdin_pipe.write, false)?;
    set_inherit(stdout_pipe.read, false)?;
    set_inherit(stderr_pipe.read, false)?;

    // Handles offered to the child.
    let mut child_handles: Vec<HANDLE> =
        vec![stdin_pipe.read, stdout_pipe.write, stderr_pipe.write];
    for raw in &cfg.extra_inheritable_handles {
        child_handles.push(HANDLE(*raw as *mut c_void));
    }

    let attribute_count = if cfg.handle_whitelist { 4 } else { 3 };
    let mut attr_size: usize = 0;
    let _ =
        unsafe { InitializeProcThreadAttributeList(None, attribute_count, None, &mut attr_size) };
    if attr_size == 0 {
        cleanup_pipes(&stdin_pipe, &stdout_pipe, &stderr_pipe);
        return Err(IsolationError::new(
            "attributes",
            Some(last_error()),
            "InitializeProcThreadAttributeList size query failed",
        ));
    }
    let mut attr_buf: Vec<usize> = vec![0; attr_size.div_ceil(size_of::<usize>())];
    let attr_list = LPPROC_THREAD_ATTRIBUTE_LIST(attr_buf.as_mut_ptr() as *mut c_void);
    unsafe {
        InitializeProcThreadAttributeList(Some(attr_list), attribute_count, None, &mut attr_size)
    }
    .map_err(|err| {
        cleanup_pipes(&stdin_pipe, &stdout_pipe, &stderr_pipe);
        IsolationError::new(
            "attributes",
            Some(hresult_to_win32(&err)),
            format!("InitializeProcThreadAttributeList failed: {err}"),
        )
    })?;

    // The values pointed to by the attribute list must outlive the CreateProcess call.
    let capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: profile.sid,
        Capabilities: std::ptr::null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let opt_out: u32 = PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT;
    let job_handle = job.handle();

    let set_attr =
        |attribute: u32, value: *const c_void, size: usize| -> Result<(), IsolationError> {
            unsafe {
                UpdateProcThreadAttribute(
                    attr_list,
                    0,
                    attribute as usize,
                    Some(value),
                    size,
                    None,
                    None,
                )
            }
            .map_err(|err| {
                IsolationError::new(
                    "attributes",
                    Some(hresult_to_win32(&err)),
                    format!("UpdateProcThreadAttribute(0x{attribute:04X}) failed: {err}"),
                )
            })
        };

    let attr_result = (|| -> Result<(), IsolationError> {
        set_attr(
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
            &capabilities as *const _ as *const c_void,
            size_of::<SECURITY_CAPABILITIES>(),
        )?;
        if cfg.less_privileged {
            set_attr(
                PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
                &opt_out as *const _ as *const c_void,
                size_of::<u32>(),
            )?;
        }
        set_attr(
            PROC_THREAD_ATTRIBUTE_JOB_LIST,
            &job_handle as *const _ as *const c_void,
            size_of::<HANDLE>(),
        )?;
        if cfg.handle_whitelist {
            set_attr(
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                child_handles.as_ptr() as *const c_void,
                child_handles.len() * size_of::<HANDLE>(),
            )?;
        }
        Ok(())
    })();
    if let Err(err) = attr_result {
        unsafe { DeleteProcThreadAttributeList(attr_list) };
        cleanup_pipes(&stdin_pipe, &stdout_pipe, &stderr_pipe);
        return Err(err);
    }

    let working_dir: PathBuf = cfg
        .working_dir
        .clone()
        .unwrap_or_else(|| cfg.worker_dir.clone());
    let working_dir_s = working_dir.to_string_lossy().to_string();
    let exe_s = cfg.worker_exe.to_string_lossy().to_string();

    let mut command_line = format!("\"{exe_s}\"");
    for arg in &cfg.args {
        command_line.push(' ');
        command_line.push_str(arg);
    }
    let mut command_line_w = wide(&command_line);

    let env_block_w = cfg.environment.as_ref().map(|vars| env_block(vars));
    let env_ptr: *const c_void = match &env_block_w {
        Some(block) => block.as_ptr() as *const c_void,
        None => std::ptr::null(),
    };

    let si_ex = STARTUPINFOEXW {
        StartupInfo: STARTUPINFOW {
            cb: size_of::<STARTUPINFOEXW>() as u32,
            dwFlags: STARTF_USESTDHANDLES,
            hStdInput: stdin_pipe.read,
            hStdOutput: stdout_pipe.write,
            hStdError: stderr_pipe.write,
            ..Default::default()
        },
        lpAttributeList: attr_list,
    };

    let flags = EXTENDED_STARTUPINFO_PRESENT
        | CREATE_UNICODE_ENVIRONMENT
        | CREATE_NO_WINDOW
        | CREATE_SUSPENDED;

    let app = hstring(&exe_s);
    let cwd = hstring(&working_dir_s);

    let mut pi = PROCESS_INFORMATION::default();
    let mut create_api = "CreateProcessAsUserW";
    let first = unsafe {
        CreateProcessAsUserW(
            None,
            &app,
            Some(PWSTR::from_raw(command_line_w.as_mut_ptr())),
            None,
            None,
            true,
            flags,
            Some(env_ptr),
            &cwd,
            &si_ex.StartupInfo,
            &mut pi,
        )
    };
    if let Err(err) = first {
        let code = hresult_to_win32(&err);
        // A NULL token makes CreateProcessAsUser equivalent to CreateProcess, but some
        // hardened/EDR environments reject it. Try CreateProcess as the documented
        // AppContainer activation API and record which one succeeded.
        let second = unsafe {
            CreateProcessW(
                &app,
                Some(PWSTR::from_raw(command_line_w.as_mut_ptr())),
                None,
                None,
                true,
                flags,
                Some(env_ptr),
                &cwd,
                &si_ex.StartupInfo,
                &mut pi,
            )
        };
        match second {
            Ok(()) => {
                create_api = "CreateProcessW";
            }
            Err(err2) => {
                unsafe { DeleteProcThreadAttributeList(attr_list) };
                cleanup_pipes(&stdin_pipe, &stdout_pipe, &stderr_pipe);
                return Err(IsolationError::new(
                    "create-process",
                    Some(hresult_to_win32(&err2)),
                    format!(
                        "CreateProcessAsUserW failed ({code}: {err}); CreateProcessW also failed: {err2}"
                    ),
                ));
            }
        }
    }

    unsafe { DeleteProcThreadAttributeList(attr_list) };

    // Close the child's ends in the parent.
    unsafe {
        close_handle(stdin_pipe.read);
        close_handle(stdout_pipe.write);
        close_handle(stderr_pipe.write);
    }

    let pid = pi.dwProcessId;
    let process = pi.hProcess;
    let thread_handle = pi.hThread;

    // Reader/writer threads. `File` takes ownership of the parent pipe handles.
    let mut stdin_file = unsafe { File::from_raw_handle(stdin_pipe.write.0 as RawHandle) };
    let stdin_data = cfg.stdin.clone();
    let stdin_join = thread::spawn(move || {
        let _ = stdin_file.write_all(&stdin_data);
        let _ = stdin_file.flush();
        drop(stdin_file);
    });

    let mut stdout_file = unsafe { File::from_raw_handle(stdout_pipe.read.0 as RawHandle) };
    let stdout_join = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_file.read_to_end(&mut buf);
        buf
    });

    let mut stderr_file = unsafe { File::from_raw_handle(stderr_pipe.read.0 as RawHandle) };
    let stderr_join = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_file.read_to_end(&mut buf);
        buf
    });

    let previous = unsafe { ResumeThread(thread_handle) };
    if previous == u32::MAX {
        let _ = job.terminate(1);
        unsafe {
            close_handle(thread_handle);
            close_handle(process);
        }
        let _ = stdin_join.join();
        let _ = stdout_join.join();
        let _ = stderr_join.join();
        return Err(IsolationError::new(
            "create-process",
            Some(last_error()),
            "ResumeThread failed",
        ));
    }

    let start = Instant::now();
    let mut termination = TerminationReason::Completed;
    let mut snapshot = None;
    let mut job_closed = false;
    let mut terminated = false;
    let mut timed_out = false;

    // Optional extra tree processes in the same job. Used to exercise process-tree
    // reclamation; the sandboxed worker itself is denied child creation.
    let mut tree_pids = Vec::new();
    for _ in 0..cfg.tree_children {
        match spawn_tree_child(cfg, profile.sid, &job, cfg.tree_sleep_ms) {
            Ok(pid) => tree_pids.push(pid),
            Err(err) => {
                let _ = job.terminate(4);
                unsafe {
                    close_handle(thread_handle);
                    close_handle(process);
                }
                let _ = stdin_join.join();
                let _ = stdout_join.join();
                let _ = stderr_join.join();
                return Err(err);
            }
        }
    }

    loop {
        let elapsed = start.elapsed();

        if !job_closed {
            if let Some(delay) = cfg.limits.close_job_after {
                if elapsed >= delay && !terminated {
                    job.close();
                    job_closed = true;
                    termination = TerminationReason::KilledOnJobClose;
                }
            }
        }
        if !job_closed && !terminated {
            if let Some(delay) = cfg.limits.terminate_after {
                if elapsed >= delay {
                    let _ = job.terminate(2);
                    terminated = true;
                    termination = TerminationReason::TerminatedByHost;
                }
            }
        }
        if snapshot.is_none() {
            if let Some(delay) = cfg.limits.snapshot_after {
                if elapsed >= delay {
                    snapshot = job.snapshot().ok();
                }
            }
        }
        if let Some(limit) = cfg.limits.timeout {
            if elapsed >= limit && !terminated && !job_closed {
                let _ = job.terminate(3);
                terminated = true;
                timed_out = true;
            }
        }

        let wr = unsafe { WaitForSingleObject(process, 50) };
        if wr == WAIT_OBJECT_0 {
            break;
        }
        if job_closed && elapsed > Duration::from_secs(300) {
            // Safety valve: a KILL_ON_JOB_CLOSE member that refuses to die.
            break;
        }
    }

    if timed_out {
        termination = TerminationReason::Timeout;
    }

    let mut exit_code: u32 = 0;
    unsafe { GetExitCodeProcess(process, &mut exit_code) }.ok();

    unsafe {
        close_handle(thread_handle);
        close_handle(process);
    }

    let _ = stdin_join.join();
    let stdout = stdout_join.join().unwrap_or_default();
    let stderr = stderr_join.join().unwrap_or_default();

    Ok(LaunchOutcome {
        pid,
        exit_code,
        termination,
        stdout,
        stderr,
        wall_time: start.elapsed(),
        snapshot,
        create_api: create_api.to_string(),
        tree_pids,
        cpu_rate_control,
    })
}

/// Launch an extra worker process directly into `job` (LPAC + JOB_LIST), used to build
/// a process tree for reclamation tests. No stdio redirection is needed.
fn spawn_tree_child(
    cfg: &SandboxConfig,
    profile_sid: PSID,
    job: &Job,
    sleep_ms: u64,
) -> Result<u32, IsolationError> {
    let attribute_count = 3u32;
    let mut attr_size: usize = 0;
    let _ =
        unsafe { InitializeProcThreadAttributeList(None, attribute_count, None, &mut attr_size) };
    if attr_size == 0 {
        return Err(IsolationError::new(
            "attributes",
            Some(last_error()),
            "spawn_tree_child: attribute size query failed",
        ));
    }
    let mut attr_buf: Vec<usize> = vec![0; attr_size.div_ceil(size_of::<usize>())];
    let attr_list = LPPROC_THREAD_ATTRIBUTE_LIST(attr_buf.as_mut_ptr() as *mut c_void);
    unsafe {
        InitializeProcThreadAttributeList(Some(attr_list), attribute_count, None, &mut attr_size)
    }
    .map_err(|err| {
        IsolationError::new(
            "attributes",
            Some(hresult_to_win32(&err)),
            format!("spawn_tree_child: InitializeProcThreadAttributeList failed: {err}"),
        )
    })?;

    let capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: profile_sid,
        Capabilities: std::ptr::null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let opt_out: u32 = PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT;
    let job_handle = job.handle();

    let attr_result = (|| -> Result<(), IsolationError> {
        let set = |attribute: u32, value: *const c_void, size: usize| {
            unsafe {
                UpdateProcThreadAttribute(
                    attr_list,
                    0,
                    attribute as usize,
                    Some(value),
                    size,
                    None,
                    None,
                )
            }
            .map_err(|err| {
                IsolationError::new(
                    "attributes",
                    Some(hresult_to_win32(&err)),
                    format!("spawn_tree_child: UpdateProcThreadAttribute failed: {err}"),
                )
            })
        };
        set(
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
            &capabilities as *const _ as *const c_void,
            size_of::<SECURITY_CAPABILITIES>(),
        )?;
        if cfg.less_privileged {
            set(
                PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
                &opt_out as *const _ as *const c_void,
                size_of::<u32>(),
            )?;
        }
        set(
            PROC_THREAD_ATTRIBUTE_JOB_LIST,
            &job_handle as *const _ as *const c_void,
            size_of::<HANDLE>(),
        )?;
        Ok(())
    })();
    if let Err(err) = attr_result {
        unsafe { DeleteProcThreadAttributeList(attr_list) };
        return Err(err);
    }

    let exe_s = cfg.worker_exe.to_string_lossy().to_string();
    let mut command_line = wide(&format!("\"{exe_s}\" child-sleep {sleep_ms}"));
    let working_dir_s = cfg
        .working_dir
        .clone()
        .unwrap_or_else(|| cfg.worker_dir.clone())
        .to_string_lossy()
        .to_string();
    let env_block_w = cfg.environment.as_ref().map(|vars| env_block(vars));
    let env_ptr: *const c_void = match &env_block_w {
        Some(block) => block.as_ptr() as *const c_void,
        None => std::ptr::null(),
    };
    let si_ex = STARTUPINFOEXW {
        StartupInfo: STARTUPINFOW {
            cb: size_of::<STARTUPINFOEXW>() as u32,
            ..Default::default()
        },
        lpAttributeList: attr_list,
    };
    let app = hstring(&exe_s);
    let cwd = hstring(&working_dir_s);
    let flags = EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW;
    let mut pi = PROCESS_INFORMATION::default();
    let res = unsafe {
        CreateProcessW(
            &app,
            Some(PWSTR::from_raw(command_line.as_mut_ptr())),
            None,
            None,
            false,
            flags,
            Some(env_ptr),
            &cwd,
            &si_ex.StartupInfo,
            &mut pi,
        )
    };
    unsafe { DeleteProcThreadAttributeList(attr_list) };
    match res {
        Ok(()) => {
            let pid = pi.dwProcessId;
            unsafe {
                close_handle(pi.hThread);
                close_handle(pi.hProcess);
            }
            Ok(pid)
        }
        Err(err) => Err(IsolationError::new(
            "create-tree-process",
            Some(hresult_to_win32(&err)),
            format!("spawn_tree_child: CreateProcessW failed: {err}"),
        )),
    }
}

fn cleanup_pipes(stdin_pipe: &PipeHandles, stdout_pipe: &PipeHandles, stderr_pipe: &PipeHandles) {
    unsafe {
        close_handle(stdin_pipe.read);
        close_handle(stdin_pipe.write);
        close_handle(stdout_pipe.read);
        close_handle(stdout_pipe.write);
        close_handle(stderr_pipe.read);
        close_handle(stderr_pipe.write);
    }
}

/// Convenience: spin up the sandbox and return the parsed worker response.
pub fn run_worker(
    cfg: &SandboxConfig,
) -> Result<(LaunchOutcome, Option<crate::model::WorkerResponse>), IsolationError> {
    let outcome = run_sandboxed(cfg)?;
    let response = String::from_utf8_lossy(&outcome.stdout)
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<crate::model::WorkerResponse>(line).ok());
    Ok((outcome, response))
}

/// Turn a `CreateProcessAsUserW`/`CreateProcessW` `PWSTR` argument buffer into a stable
/// raw pointer helper (kept for readability at call sites).
#[allow(dead_code)]
fn command_line_ptr(buf: &mut [u16]) -> PWSTR {
    PWSTR::from_raw(buf.as_mut_ptr())
}

/// Expose the null `PCWSTR` for callers that need an absent path.
#[allow(dead_code)]
fn null_pcwstr() -> PCWSTR {
    PCWSTR::null()
}
