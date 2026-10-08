//! Platform sandbox prototypes and the P2 fixed SF worker for the
//! cross-platform installer migration.
//!
//! - Windows: LPAC (Less Privileged AppContainer) + Job Object under [`win`].
//! - Linux: bubblewrap (`bwrap`) under [`linux`].
//! - Other platforms: a stub so the workspace still compiles and tests.
//!
//! SF parsing/conversion lives in [`sf`]; core authorization and production launcher
//! integration are separate from the worker. Probe binaries remain development tools.

pub mod model;
pub mod sf;

#[cfg(windows)]
pub mod win;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(not(any(windows, target_os = "linux")))]
pub mod stub;

#[cfg(windows)]
pub use win::run_sandboxed;

#[cfg(target_os = "linux")]
pub use linux::run_sandboxed;

#[cfg(not(any(windows, target_os = "linux")))]
pub use stub::run_sandboxed;

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
