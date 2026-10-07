//! P0 prototype: Windows LPAC (Less Privileged AppContainer) + Job Object launcher
//! and probe worker for the cross-platform installer migration.
//!
//! Windows-only functionality lives under [`win`]; other platforms get a stub so the
//! workspace still compiles and tests on Linux. The probe binaries are only meaningful
//! on Windows and print an "unsupported" message elsewhere.

pub mod model;

#[cfg(windows)]
pub mod win;

#[cfg(not(windows))]
pub mod stub;

pub use model::{
    CheckResult, CheckStatus, IsolationError, JobSnapshot, LaunchOutcome, SandboxConfig,
    SandboxLimits, TerminationReason, WorkerRequest, WorkerResponse,
};

/// Schema tag written into the probe evidence JSON.
pub const PROBE_SCHEMA: &str = "steam-patch/p0-sandbox-probe/v1";

/// Default AppContainer profile name used by the probe binaries.
pub const DEFAULT_PROFILE_NAME: &str = "steam-patch-probe";

/// Windows build (as `CurrentBuildNumber`) from which LPAC is available (Win10 1703).
pub const LPAC_MIN_BUILD: u32 = 15063;
