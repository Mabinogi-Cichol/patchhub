//! Read-only OS probes used by the isolated worker (token, console, handles, memory)
//! and by the runner (process liveness, current-process job membership).

use core::ffi::c_void;
use core::mem::size_of;

use serde::{Deserialize, Serialize};
use windows::core::BOOL;
use windows::Win32::Foundation::{DuplicateHandle, GetLastError, DUPLICATE_SAME_ACCESS, HANDLE};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{
    GetTokenInformation, TokenAppContainerSid, TokenCapabilities, TokenElevation,
    TokenIsAppContainer, PSID, TOKEN_QUERY,
};
use windows::Win32::System::Console::{
    GetConsoleMode, GetConsoleWindow, GetStdHandle, CONSOLE_MODE, STD_OUTPUT_HANDLE,
};
use windows::Win32::System::JobObjects::IsProcessInJob;
use windows::Win32::System::Memory::{
    VirtualAlloc, VirtualFree, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::util::{close_handle, last_error, local_free};

/// Token facts reported by the isolated worker.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenInfo {
    pub is_app_container: bool,
    pub app_container_sid: Option<String>,
    pub capability_count: Option<u32>,
    pub is_elevated: Option<bool>,
    pub error_code: Option<u32>,
}

/// Console facts reported by the isolated worker.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsoleInfo {
    /// `GetConsoleWindow()` value as an integer (0 when there is no console window).
    pub console_window: u64,
    /// Whether the std output handle is an attached console.
    pub std_output_is_console: bool,
}

/// Query `TokenIsAppContainer`, `TokenAppContainerSid`, `TokenCapabilities` and
/// `TokenElevation` for the current process.
pub fn token_info() -> TokenInfo {
    let mut info = TokenInfo::default();
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            info.error_code = Some(last_error());
            return info;
        }

        let mut is_ac: u32 = 0;
        let mut ret: u32 = 0;
        if GetTokenInformation(
            token,
            TokenIsAppContainer,
            Some(&mut is_ac as *mut u32 as *mut c_void),
            size_of::<u32>() as u32,
            &mut ret,
        )
        .is_ok()
        {
            info.is_app_container = is_ac != 0;
        }

        if let Some(buf) = query_token(token, TokenCapabilities) {
            if buf.len() >= 4 {
                info.capability_count = Some(u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]));
            }
        }

        if let Some(buf) = query_token(token, TokenAppContainerSid) {
            if buf.len() >= size_of::<PSID>() {
                let sid = *(buf.as_ptr() as *const PSID);
                if !sid.0.is_null() {
                    info.app_container_sid = sid_to_string(sid);
                }
            }
        }

        let mut elevated: u32 = 0;
        if GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevated as *mut u32 as *mut c_void),
            size_of::<u32>() as u32,
            &mut ret,
        )
        .is_ok()
        {
            info.is_elevated = Some(elevated != 0);
        }

        close_handle(token);
    }
    info
}

unsafe fn query_token(
    token: HANDLE,
    class: windows::Win32::Security::TOKEN_INFORMATION_CLASS,
) -> Option<Vec<u8>> {
    let mut needed: u32 = 0;
    let _ = GetTokenInformation(token, class, None, 0, &mut needed);
    if needed == 0 {
        return None;
    }
    let mut buf = vec![0u8; needed as usize];
    let mut ret: u32 = 0;
    if GetTokenInformation(
        token,
        class,
        Some(buf.as_mut_ptr() as *mut c_void),
        needed,
        &mut ret,
    )
    .is_err()
    {
        return None;
    }
    Some(buf)
}

fn sid_to_string(sid: PSID) -> Option<String> {
    unsafe {
        let mut raw = windows::core::PWSTR::null();
        if ConvertSidToStringSidW(sid, &mut raw).is_err() {
            return None;
        }
        let out = raw.to_string().ok();
        local_free(raw.0 as *mut c_void);
        out
    }
}

/// Console probe: no console window under `CREATE_NO_WINDOW` + redirected stdio.
pub fn console_info() -> ConsoleInfo {
    let mut info = ConsoleInfo::default();
    unsafe {
        info.console_window = GetConsoleWindow().0 as u64;
        if let Ok(handle) = GetStdHandle(STD_OUTPUT_HANDLE) {
            let mut mode = CONSOLE_MODE::default();
            info.std_output_is_console = GetConsoleMode(handle, &mut mode).is_ok();
        }
    }
    info
}

/// Test whether `raw` names a handle present in this process by attempting to
/// duplicate it.
///
/// Avoids probing arbitrary low handle values with `GetHandleInformation`/`GetFileType`:
/// in this hardened probe environment those calls terminated the worker with an
/// unhandled `STATUS_INVALID_HANDLE` (0xC0000008) as soon as an invalid value was hit.
pub fn try_duplicate(raw: usize) -> bool {
    let source = HANDLE(raw as *mut c_void);
    let mut out = HANDLE::default();
    let res = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            source,
            GetCurrentProcess(),
            &mut out,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if res.is_ok() {
        unsafe { close_handle(out) };
        true
    } else {
        false
    }
}

/// Attempt to commit `mb` MiB; returns the Win32 error on failure.
pub fn try_commit(mb: u64) -> Result<(), u32> {
    let size = (mb as usize).saturating_mul(1024 * 1024);
    let ptr = unsafe { VirtualAlloc(None, size, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
    if ptr.is_null() {
        Err(unsafe { GetLastError().0 })
    } else {
        unsafe {
            let _ = VirtualFree(ptr, 0, MEM_RELEASE);
        }
        Ok(())
    }
}

/// Whether a process with `pid` is still running.
///
/// `OpenProcess` can briefly succeed for a terminated process whose kernel object is
/// still referenced, so liveness is derived from `GetExitCodeProcess != STILL_ACTIVE`.
pub fn process_alive(pid: u32) -> bool {
    const STILL_ACTIVE: u32 = 259;
    match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(handle) => {
            let mut code: u32 = 0;
            let alive =
                unsafe { GetExitCodeProcess(handle, &mut code) }.is_ok() && code == STILL_ACTIVE;
            unsafe { close_handle(handle) };
            alive
        }
        Err(_) => false,
    }
}

/// Whether the current (runner) process is already inside a Job Object.
pub fn self_in_job() -> bool {
    let mut in_job = BOOL(0);
    unsafe { IsProcessInJob(GetCurrentProcess(), None, &mut in_job) }.is_ok() && in_job.as_bool()
}
