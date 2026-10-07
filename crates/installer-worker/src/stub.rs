//! Non-Windows stub. The sandbox is only implemented on Windows; this keeps the
//! workspace compiling (and unit tests running) on Linux/macOS.

use crate::model::{IsolationError, SandboxConfig};

/// Always fails with `Unsupported` on non-Windows platforms.
pub fn run_sandboxed(_cfg: &SandboxConfig) -> Result<crate::model::LaunchOutcome, IsolationError> {
    Err(IsolationError::new(
        "unsupported",
        None,
        "LPAC/Job Object sandbox is only implemented on Windows",
    ))
}

/// Always fails with `Unsupported` on non-Windows platforms.
pub fn ensure_profile(_name: &str) -> Result<(), IsolationError> {
    Err(IsolationError::new(
        "unsupported",
        None,
        "AppContainer profiles are only available on Windows",
    ))
}
