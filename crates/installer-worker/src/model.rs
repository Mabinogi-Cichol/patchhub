//! Platform-independent data model shared by the launcher, the probe runner and the
//! probe worker.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A structured failure from the isolation launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolationError {
    /// Coarse stage name, e.g. "profile", "acl", "job", "attributes", "create-process".
    pub stage: String,
    /// Raw Win32 error code when available (as reported by `GetLastError`).
    pub code: Option<u32>,
    /// Human readable message.
    pub message: String,
}

impl IsolationError {
    pub fn new(stage: impl Into<String>, code: Option<u32>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for IsolationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.code {
            Some(code) => write!(
                f,
                "[{}] {} (win32 error {})",
                self.stage, self.message, code
            ),
            None => write!(f, "[{}] {}", self.stage, self.message),
        }
    }
}

impl std::error::Error for IsolationError {}

/// Limits applied to the worker Job Object. `None` means "do not configure".
#[derive(Debug, Clone, Default)]
pub struct SandboxLimits {
    /// `JOB_OBJECT_LIMIT_PROCESS_MEMORY` value in bytes.
    pub process_memory_bytes: Option<u64>,
    /// `JOB_OBJECT_LIMIT_JOB_MEMORY` value in bytes.
    pub job_memory_bytes: Option<u64>,
    /// `JOB_OBJECT_LIMIT_ACTIVE_PROCESS` value.
    pub active_process_limit: Option<u32>,
    /// `JOB_OBJECT_LIMIT_JOB_TIME` value in 100ns units.
    pub per_job_user_time_100ns: Option<i64>,
    /// CPU hard cap as a percentage in 1..=100 (uses `JobObjectCpuRateControlInformation`).
    pub cpu_rate_percent: Option<u32>,
    /// Hard wall-clock timeout; on expiry the job is terminated.
    pub timeout: Option<Duration>,
    /// After this delay, terminate the job via `TerminateJobObject`.
    pub terminate_after: Option<Duration>,
    /// After this delay, close the job handle (KILL_ON_JOB_CLOSE terminates members).
    pub close_job_after: Option<Duration>,
    /// After this delay, capture one job accounting/memory snapshot.
    pub snapshot_after: Option<Duration>,

    // ------------------------------------------------------------------
    // Linux-only resource limits (`setrlimit` applied in the child before
    // `exec`). Ignored by the Windows launcher. `None` selects the platform
    // default documented on each field.
    // ------------------------------------------------------------------
    /// `RLIMIT_AS` address-space cap in bytes. Linux default 3 GiB.
    pub address_space_bytes: Option<u64>,
    /// `RLIMIT_CPU` per-process CPU seconds. Linux default 300.
    pub cpu_seconds: Option<u64>,
    /// `RLIMIT_CORE` core-dump size in bytes. Linux default 0 (no core dump).
    pub core_bytes: Option<u64>,
    /// `RLIMIT_FSIZE` maximum file size in bytes. Linux default: unset (inherit).
    pub file_size_bytes: Option<u64>,
}

/// Everything needed to launch one sandboxed worker process.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Absolute path to the worker executable.
    pub worker_exe: PathBuf,
    /// Directory that must be granted to the AppContainer package SID with
    /// read+execute (the worker binary and its private DLLs).
    pub worker_dir: PathBuf,
    /// Optional input directory granted read-only.
    pub input_dir: Option<PathBuf>,
    /// Optional output directory granted read/write/delete.
    pub output_dir: Option<PathBuf>,
    /// Working directory for the worker.
    pub working_dir: Option<PathBuf>,
    /// Command-line arguments (excluding argv[0]).
    pub args: Vec<String>,
    /// Bytes written to the worker's stdin, then stdin is closed.
    pub stdin: Vec<u8>,
    /// Job Object and timeout limits.
    pub limits: SandboxLimits,
    /// AppContainer profile name; the caller is responsible for creating it.
    pub profile_name: String,
    /// When `Some`, this exact environment block is passed to the worker. When `None`
    /// the parent environment is inherited.
    pub environment: Option<Vec<(String, String)>>,
    /// When true, only the stdio handles (plus `extra_inheritable_handles`) are inherited
    /// via `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`. When false the attribute is omitted.
    pub handle_whitelist: bool,
    /// Extra raw `HANDLE` values (as `usize`) that should be inheritable and, when
    /// `handle_whitelist` is true, listed in the handle whitelist.
    pub extra_inheritable_handles: Vec<usize>,
    /// Add `PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY=OPT_OUT` (LPAC).
    pub less_privileged: bool,
    /// Number of extra `child-sleep` worker processes to place in the same Job,
    /// simulating a worker process tree (the sandboxed worker itself cannot spawn
    /// children; see the probe evidence).
    pub tree_children: usize,
    /// Sleep duration for `tree_children` processes.
    pub tree_sleep_ms: u64,
    /// Linux-only: when the launcher runs with `euid == 0`, drop to this
    /// `(uid, gid)` in `pre_exec` before `exec`. `None` falls back to the
    /// `SUDO_UID`/`SUDO_GID` environment variables, then to no drop. Ignored on
    /// Windows.
    pub run_as: Option<(u32, u32)>,
    /// Production-only strict identity and single-process seccomp constraint.
    pub production_strict: bool,
}

impl SandboxConfig {
    pub fn new(
        worker_exe: impl Into<PathBuf>,
        worker_dir: impl Into<PathBuf>,
        profile: &str,
    ) -> Self {
        Self {
            worker_exe: worker_exe.into(),
            worker_dir: worker_dir.into(),
            input_dir: None,
            output_dir: None,
            working_dir: None,
            args: Vec::new(),
            stdin: Vec::new(),
            limits: SandboxLimits::default(),
            profile_name: profile.to_string(),
            environment: None,
            handle_whitelist: true,
            extra_inheritable_handles: Vec::new(),
            less_privileged: true,
            tree_children: 0,
            tree_sleep_ms: 15_000,
            run_as: None,
            production_strict: false,
        }
    }
}

/// Why the worker process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminationReason {
    /// The worker exited on its own before any control action.
    Completed,
    /// Cancellation requested while the worker was running.
    Cancelled,
    /// The caller-configured wall-clock timeout fired.
    Timeout,
    /// `TerminateJobObject` was invoked by `terminate_after`.
    TerminatedByHost,
    /// The job handle was closed early, triggering `KILL_ON_JOB_CLOSE`.
    KilledOnJobClose,
}

/// One accounting snapshot from the job object.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobSnapshot {
    pub active_processes: u32,
    pub total_processes: u32,
    pub total_user_time_100ns: u64,
    pub total_kernel_time_100ns: u64,
    pub peak_process_memory_bytes: u64,
    pub peak_job_memory_bytes: u64,
}

/// Result of one sandboxed launch.
#[derive(Debug, Clone)]
pub struct LaunchOutcome {
    pub pid: u32,
    pub exit_code: u32,
    pub termination: TerminationReason,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub wall_time: Duration,
    pub snapshot: Option<JobSnapshot>,
    /// Which creation API succeeded: "CreateProcessAsUserW" or "CreateProcessW".
    pub create_api: String,
    /// PIDs of extra tree processes placed in the same job (see `SandboxConfig::tree_children`).
    pub tree_pids: Vec<u32>,
    /// CPU rate control read back from the job: `(ControlFlags, CpuRate)`.
    pub cpu_rate_control: Option<(u32, u32)>,
}

/// Request sent to the probe worker over stdin as one JSON line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRequest {
    pub scenario: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_exe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_ms: Option<u64>,
}

impl WorkerRequest {
    pub fn new(scenario: &str) -> Self {
        Self {
            scenario: scenario.to_string(),
            path: None,
            text: None,
            ms: None,
            mb: None,
            host: None,
            port: None,
            key: None,
            value: None,
            child_exe: None,
            child_ms: None,
        }
    }

    pub fn path(mut self, p: impl Into<String>) -> Self {
        self.path = Some(p.into());
        self
    }

    pub fn text(mut self, t: impl Into<String>) -> Self {
        self.text = Some(t.into());
        self
    }

    pub fn ms(mut self, ms: u64) -> Self {
        self.ms = Some(ms);
        self
    }

    pub fn mb(mut self, mb: u64) -> Self {
        self.mb = Some(mb);
        self
    }

    pub fn host(mut self, h: impl Into<String>, port: u16) -> Self {
        self.host = Some(h.into());
        self.port = Some(port);
        self
    }

    pub fn key(mut self, k: impl Into<String>) -> Self {
        self.key = Some(k.into());
        self
    }

    pub fn value(mut self, v: impl Into<String>) -> Self {
        self.value = Some(v.into());
        self
    }

    pub fn child(mut self, exe: impl Into<String>, ms: u64) -> Self {
        self.child_exe = Some(exe.into());
        self.child_ms = Some(ms);
        self
    }

    pub fn to_line(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(self).unwrap_or_default();
        bytes.push(b'\n');
        bytes
    }
}

/// Response written by the worker to stdout as one JSON line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerResponse {
    pub scenario: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default)]
    pub data: serde_json::Value,
}

impl WorkerResponse {
    pub fn new(scenario: &str, ok: bool) -> Self {
        Self {
            scenario: scenario.to_string(),
            ok,
            error: None,
            error_code: None,
            detail: None,
            data: serde_json::Value::Null,
        }
    }

    pub fn fail(scenario: &str, code: Option<u32>, error: impl Into<String>) -> Self {
        Self {
            scenario: scenario.to_string(),
            ok: false,
            error: Some(error.into()),
            error_code: code,
            detail: None,
            data: serde_json::Value::Null,
        }
    }

    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = data;
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

/// Status of one probe check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    Unsupported,
    NotCovered,
    Info,
}

/// One row of the probe evidence table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub id: String,
    pub title: String,
    pub expected: String,
    pub actual: String,
    pub status: CheckStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl CheckResult {
    pub fn new(id: &str, title: &str) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            expected: String::new(),
            actual: String::new(),
            status: CheckStatus::Info,
            error_code: None,
            detail: None,
        }
    }

    pub fn expected(mut self, e: impl Into<String>) -> Self {
        self.expected = e.into();
        self
    }

    pub fn actual(mut self, a: impl Into<String>) -> Self {
        self.actual = a.into();
        self
    }

    pub fn status(mut self, s: CheckStatus) -> Self {
        self.status = s;
        self
    }

    pub fn code(mut self, c: Option<u32>) -> Self {
        self.error_code = c;
        self
    }

    pub fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_request_round_trips() {
        let req = WorkerRequest::new("read_file")
            .path("C:\\中文\\file.txt")
            .text("data");
        let json = serde_json::to_string(&req).unwrap();
        let back: WorkerRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.scenario, "read_file");
        assert_eq!(back.path.as_deref(), Some("C:\\中文\\file.txt"));
        assert_eq!(back.text.as_deref(), Some("data"));
    }

    #[test]
    fn worker_request_line_is_terminated() {
        let line = WorkerRequest::new("selfcheck").to_line();
        assert_eq!(line.last(), Some(&b'\n'));
    }

    #[test]
    fn check_status_serializes_snake_case() {
        let json = serde_json::to_string(&CheckStatus::NotCovered).unwrap();
        assert_eq!(json, "\"not_covered\"");
    }

    #[test]
    fn sandbox_defaults_are_lpac_and_whitelisted() {
        let cfg = SandboxConfig::new("worker.exe", "dir", "profile");
        assert!(cfg.less_privileged);
        assert!(cfg.handle_whitelist);
        assert_eq!(cfg.tree_children, 0);
    }
}
