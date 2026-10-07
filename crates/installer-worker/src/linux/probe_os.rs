//! Read-only Linux OS probes shared by the sandboxed worker and the host runner.

use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use super::launch::BWRAP_PATH;
use super::sys;

/// Current effective uid (inside the sandbox this reflects the namespace mapping).
pub fn euid() -> u32 {
    unsafe { sys::geteuid() }
}

/// Whether the fixed bubblewrap binary exists.
pub fn bwrap_available() -> bool {
    Path::new(BWRAP_PATH).is_file()
}

/// `bwrap --version` output, when available.
pub fn bwrap_version() -> Option<String> {
    let output = Command::new(BWRAP_PATH).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Resource limits observed by the current process (the sandboxed worker).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RlimitInfo {
    /// `RLIMIT_AS` soft limit in bytes (`None` = unlimited / unavailable).
    pub address_space: Option<u64>,
    /// `RLIMIT_CPU` soft limit in seconds (`None` = unlimited / unavailable).
    pub cpu_seconds: Option<u64>,
    /// `RLIMIT_CORE` soft limit in bytes.
    pub core_bytes: Option<u64>,
    /// `RLIMIT_FSIZE` soft limit in bytes.
    pub file_size: Option<u64>,
}

/// Read the current process's soft rlimits.
pub fn rlimits() -> RlimitInfo {
    RlimitInfo {
        address_space: read_limit(sys::RLIMIT_AS),
        cpu_seconds: read_limit(sys::RLIMIT_CPU),
        core_bytes: read_limit(sys::RLIMIT_CORE),
        file_size: read_limit(sys::RLIMIT_FSIZE),
    }
}

fn read_limit(resource: i32) -> Option<u64> {
    unsafe {
        let mut current = sys::RLimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if sys::getrlimit(resource, &mut current) != 0 {
            return None;
        }
        if current.rlim_cur == sys::RLIM_INFINITY {
            None
        } else {
            Some(current.rlim_cur)
        }
    }
}

/// Whether a process with `pid` (host namespace) still exists.
pub fn process_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// Host-visible PIDs whose `comm` exactly equals `name`. Used to check that a
/// terminated sandbox left no residue.
pub fn find_processes(name: &str) -> Vec<u32> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return found;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(pid) = file_name.to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) else {
            continue;
        };
        if comm.trim() == name {
            found.push(pid);
        }
    }
    found
}

/// Kernel identity collected without assuming any extra crate.
pub fn uname_info() -> serde_json::Value {
    let field = |arg: &str, sysctl: &str| -> String {
        uname_field(arg)
            .or_else(|| read_trim(sysctl))
            .unwrap_or_default()
    };
    serde_json::json!({
        "sysname": field("-s", "/proc/sys/kernel/ostype"),
        "release": field("-r", "/proc/sys/kernel/osrelease"),
        "version": field("-v", "/proc/sys/kernel/version"),
        "machine": field("-m", ""),
    })
}

fn read_trim(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
}

fn uname_field(arg: &str) -> Option<String> {
    let output = Command::new("uname").arg(arg).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
