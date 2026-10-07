//! Windows LPAC + Job Object sandbox: profile/ACL, job limits, process launch and
//! read-only OS probes.

pub mod appcontainer;
pub mod job;
pub mod launch;
pub mod probe_os;
pub mod util;

use core::ffi::c_void;

pub use appcontainer::{delete_profile, ensure_profile, grant_path_access, AccessKind, Profile};
pub use job::Job;
pub use launch::{run_sandboxed, run_worker};
pub use probe_os::{console_info, process_alive, self_in_job, token_info, try_duplicate};

/// Change the inherit flag of a raw handle value (used to set up the handle-whitelist
/// differential probe).
pub fn set_raw_handle_inherit(
    raw: usize,
    inherit: bool,
) -> Result<(), crate::model::IsolationError> {
    use windows::Win32::Foundation::{
        SetHandleInformation, HANDLE, HANDLE_FLAGS, HANDLE_FLAG_INHERIT,
    };
    let flags = if inherit {
        HANDLE_FLAG_INHERIT
    } else {
        HANDLE_FLAGS(0)
    };
    unsafe { SetHandleInformation(HANDLE(raw as *mut c_void), HANDLE_FLAG_INHERIT.0, flags) }
        .map_err(|err| {
            crate::model::IsolationError::new(
                "handle",
                Some(util::hresult_to_win32(&err)),
                format!("SetHandleInformation failed: {err}"),
            )
        })
}
