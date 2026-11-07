//! Input file digests with cancellation and file-identity verification.
use super::zip_recipe::no_links;
use super::{invalid, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
const CHUNK_BYTES: usize = 1024 * 1024;

/// SHA-256 and size of one regular input file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileDigest {
    pub sha256: String,
    pub size: u64,
}

pub fn digest_file(path: &Path, cancel: &AtomicBool) -> Result<FileDigest> {
    if !no_links(path) {
        return Err(invalid("exe_linked_game_file"));
    }
    let mut file = open_nofollow(path)?;
    let before = file.metadata()?;
    #[cfg(windows)]
    let identity_before = windows_handle_identity(&file)?;
    if !before.is_file() {
        return Err(invalid("exe_game_file_limit"));
    }

    let mut hasher = Sha256::new();
    let mut size: u64 = 0;
    let mut buffer = vec![0u8; CHUNK_BYTES];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("download_cancelled"));
        }
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| invalid("exe_game_file_limit"))?;
        hasher.update(&buffer[..count]);
    }

    let after = file.metadata()?;
    // Re-stat through the path after the read. `no_links` is re-checked because
    // the final component could have been replaced mid-read.
    if !no_links(path) {
        return Err(invalid("exe_game_file_changed"));
    }
    #[cfg(not(windows))]
    let current = fs::metadata(path)?;

    #[cfg(not(windows))]
    let (identity_before, identity_after, identity_current) = (
        file_identity(&before),
        file_identity(&after),
        file_identity(&current),
    );
    #[cfg(windows)]
    let (identity_after, identity_current) = (
        windows_handle_identity(&file)?,
        windows_handle_identity(&open_nofollow(path)?)?,
    );
    let changed_before = changed_time(&before);
    let changed_after = changed_time(&after);
    #[cfg(unix)]
    let changed_matches = changed_after == changed_time(&current);
    // Windows `fstat` legacy ctime differs from `path.stat` creation time, so
    // the cross-handle change check is platform-specific (exe_facts.py comment).
    #[cfg(windows)]
    let changed_matches = true;
    if identity_before != identity_after
        || changed_before != changed_after
        || identity_after != identity_current
        || !changed_matches
    {
        return Err(invalid("exe_game_file_changed"));
    }

    Ok(FileDigest {
        sha256: format!("{:x}", hasher.finalize()),
        size,
    })
}

// ---- file identity --------------------------------------------------------

/// Comparable identity fields (`st_dev`/`st_ino`/size/mtime on unix; volume
/// serial/file index/size/last-write on Windows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    volume: u64,
    index: u64,
    size: u64,
    modified_secs: i64,
    modified_nanos: i64,
}

#[cfg(unix)]
fn file_identity(metadata: &fs::Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;
    FileIdentity {
        volume: metadata.dev(),
        index: metadata.ino(),
        size: metadata.len(),
        modified_secs: metadata.mtime(),
        modified_nanos: metadata.mtime_nsec(),
    }
}

#[cfg(not(any(unix, windows)))]
fn file_identity(metadata: &fs::Metadata) -> FileIdentity {
    FileIdentity {
        volume: 0,
        index: 0,
        size: metadata.len(),
        modified_secs: 0,
        modified_nanos: 0,
    }
}

/// Change timestamp used only for the descriptor-vs-descriptor comparison and
/// (on unix) descriptor-vs-path comparison.
#[cfg(unix)]
fn changed_time(metadata: &fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    metadata.ctime()
}

#[cfg(windows)]
fn changed_time(metadata: &fs::Metadata) -> i64 {
    use std::os::windows::fs::MetadataExt;
    metadata.creation_time() as i64
}

#[cfg(not(any(unix, windows)))]
fn changed_time(_metadata: &fs::Metadata) -> i64 {
    0
}

#[cfg(unix)]
pub(crate) fn open_nofollow(path: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

#[cfg(windows)]
pub(crate) fn open_nofollow(path: &Path) -> Result<File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(0x0020_0000)
        .share_mode(1)
        .open(path)?;
    if file.metadata()?.file_attributes() & 0x400 != 0 {
        return Err(invalid("exe_linked_game_file"));
    }
    Ok(file)
}
#[cfg(not(any(unix, windows)))]
pub(crate) fn open_nofollow(path: &Path) -> Result<File> {
    File::open(path)
}
#[cfg(windows)]
fn windows_handle_identity(file: &File) -> Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    struct Information {
        attributes: u32,
        times: [u32; 6],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut Information)
            -> i32;
    }
    let mut info: Information = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    if info.attributes & 0x400 != 0 {
        return Err(invalid("exe_linked_game_file"));
    }
    Ok(FileIdentity {
        volume: info.volume as u64,
        index: ((info.index_high as u64) << 32) | info.index_low as u64,
        size: ((info.size_high as u64) << 32) | info.size_low as u64,
        modified_secs: (((info.times[5] as u64) << 32) | info.times[4] as u64) as i64,
        modified_nanos: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn digest_records_bytes_and_honors_cancellation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("input");
        fs::write(&path, b"123456789").unwrap();
        let digest = digest_file(&path, &AtomicBool::new(false)).unwrap();
        assert_eq!(digest.size, 9);
        assert_eq!(digest.sha256, format!("{:x}", Sha256::digest(b"123456789")));
        assert!(digest_file(&path, &AtomicBool::new(true)).is_err());
        assert!(digest_file(temp.path(), &AtomicBool::new(false)).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn digest_does_not_follow_links() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("input");
        fs::write(&path, b"bytes").unwrap();
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(digest_file(&link, &AtomicBool::new(false)).is_err());
    }
}
