//! Minimal raw `libc` bindings used by the Linux launcher and worker.
//!
//! The prototype deliberately avoids adding the `libc` crate as a direct
//! dependency (which would require updating the workspace `Cargo.lock`, outside
//! this task's allowed file set). Only the handful of symbols needed for
//! `setrlimit`/`setsid`/`killpg`/privilege-drop are declared here.
//!
//! Target assumption: 64-bit Linux (`rlim_t == u64`), which covers SteamOS,
//! Steam Deck and the WSL2 amd64 host used for the P0-D probe.

#[repr(C)]
pub struct RLimit {
    pub rlim_cur: u64,
    pub rlim_max: u64,
}

/// `RLIM_INFINITY` on 64-bit Linux.
pub const RLIM_INFINITY: u64 = u64::MAX;

// linux/resource.h values (identical on all Linux architectures).
pub const RLIMIT_CPU: i32 = 0;
pub const RLIMIT_FSIZE: i32 = 1;
pub const RLIMIT_CORE: i32 = 4;
pub const RLIMIT_AS: i32 = 9;

pub const SIGTERM: i32 = 15;
pub const SIGKILL: i32 = 9;

extern "C" {
    pub fn setrlimit(resource: i32, rlim: *const RLimit) -> i32;
    pub fn getrlimit(resource: i32, rlim: *mut RLimit) -> i32;
    pub fn setsid() -> i32;
    pub fn killpg(pgrp: i32, sig: i32) -> i32;
    pub fn geteuid() -> u32;
    /// `int setgroups(size_t size, const gid_t *list)` (`gid_t == u32`).
    pub fn setgroups(size: usize, list: *const u32) -> i32;
    pub fn setgid(gid: u32) -> i32;
    pub fn setuid(uid: u32) -> i32;
}
