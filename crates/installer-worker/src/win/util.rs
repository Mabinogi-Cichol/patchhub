//! Small Win32 helpers shared by the launcher modules.

use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, GetLastError, LocalFree, HANDLE, HLOCAL};

/// Encode a Rust string as a null-terminated UTF-16 buffer.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `HSTRING` for APIs that accept `Param<PCWSTR>`.
pub fn hstring(s: &str) -> HSTRING {
    HSTRING::from(s)
}

/// Raw `GetLastError` value.
pub fn last_error() -> u32 {
    unsafe { GetLastError().0 }
}

/// Convert a `windows_core::Error` into its underlying Win32 code when it is a
/// `HRESULT_FROM_WIN32` wrapper.
pub fn hresult_to_win32(err: &windows::core::Error) -> u32 {
    let hr = err.code().0 as u32;
    if hr & 0xFFFF_0000 == 0x8007_0000 {
        hr & 0xFFFF
    } else {
        hr
    }
}

/// Build a UTF-16 environment block (`KEY=VALUE\0...\0`) from key/value pairs.
pub fn env_block(vars: &[(String, String)]) -> Vec<u16> {
    let mut out = Vec::new();
    for (k, v) in vars {
        out.extend(k.encode_utf16());
        out.push(u16::from(b'='));
        out.extend(v.encode_utf16());
        out.push(0);
    }
    out.push(0);
    out
}

/// Close a handle, ignoring the result. Used for best-effort cleanup (including the
/// deliberate `KILL_ON_JOB_CLOSE` test).
///
/// # Safety
/// `handle` must be a valid handle owned by the caller, or null/invalid (which is
/// ignored). The handle must not be used after this call.
pub unsafe fn close_handle(handle: HANDLE) {
    if handle.0.is_null() || handle.0 as isize == -1 {
        return;
    }
    let _ = CloseHandle(handle);
}

/// Free a buffer allocated by `SetEntriesInAclW`/`GetNamedSecurityInfoW`.
///
/// # Safety
/// `ptr` must be a pointer previously returned by an API that allocates with
/// `LocalAlloc` (e.g. `SetEntriesInAclW`), and must not be freed twice.
pub unsafe fn local_free(ptr: *mut core::ffi::c_void) {
    if ptr.is_null() {
        return;
    }
    LocalFree(Some(HLOCAL(ptr)));
}
