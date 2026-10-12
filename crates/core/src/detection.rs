//! Shared, read-only evidence for catalog and dynamic projections.
//! Missing is authoritative only after checking both the game and library boundary.
use crate::{catalog::relative_path, steam::GameInstall};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::SystemTime,
};

pub const DEFAULT_MAX_BYTES: u64 = crate::installer::space::MAX_ORIGINAL_BYTES;

#[derive(Debug, Clone)]
pub struct RelativeGamePath(String);
impl RelativeGamePath {
    pub fn parse(value: &str) -> io::Result<Self> {
        relative_path(value)?;
        Ok(Self(value.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Debug, Clone)]
pub struct Sha256Digest(String);
impl Sha256Digest {
    pub fn parse(value: &str) -> io::Result<Self> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "evidence_invalid_hash",
            ));
        }
        Ok(Self(value.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Match,
    Mismatch,
    Missing,
    UnreadableOrUnsafe,
}
impl EvidenceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Match => "match",
            Self::Mismatch => "mismatch",
            Self::Missing => "missing",
            Self::UnreadableOrUnsafe => "unreadable_or_unsafe",
        }
    }
}
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct EvidenceDiagnostic {
    pub code: String,
    pub detail: Option<String>,
}
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct FileEvidence {
    pub path: String,
    pub status: EvidenceStatus,
    pub actual_sha256: Option<String>,
    pub diagnostic: Option<EvidenceDiagnostic>,
}
pub struct EvidenceRequest<'a> {
    pub game: &'a GameInstall,
    pub relative_path: &'a RelativeGamePath,
    pub expected_sha256: &'a Sha256Digest,
    pub max_bytes: u64,
    pub cancel: &'a AtomicBool,
}
fn fail(code: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, code)
}
fn cancelled(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(fail("evidence_cancelled"))
    } else {
        Ok(())
    }
}
fn unknown(path: &str, code: &str) -> FileEvidence {
    FileEvidence {
        path: path.into(),
        status: EvidenceStatus::UnreadableOrUnsafe,
        actual_sha256: None,
        diagnostic: Some(EvidenceDiagnostic {
            code: code.into(),
            detail: None,
        }),
    }
}
/// String adapter also validates inputs supplied directly by library callers.
pub fn inspect_file(
    game: &GameInstall,
    relative: &str,
    expected: &str,
    max_bytes: u64,
    cancel: &AtomicBool,
) -> FileEvidence {
    let Ok(path) = RelativeGamePath::parse(relative) else {
        return unknown("", "evidence_invalid_path");
    };
    let Ok(hash) = Sha256Digest::parse(expected) else {
        return unknown(relative, "evidence_invalid_hash");
    };
    inspect(EvidenceRequest {
        game,
        relative_path: &path,
        expected_sha256: &hash,
        max_bytes,
        cancel,
    })
}
pub fn inspect(request: EvidenceRequest<'_>) -> FileEvidence {
    let path = request.relative_path.as_str();
    match read_digest(&request) {
        Ok(actual) => FileEvidence {
            path: path.into(),
            status: match &actual {
                None => EvidenceStatus::Missing,
                Some(hash) if hash == request.expected_sha256.as_str() => EvidenceStatus::Match,
                Some(_) => EvidenceStatus::Mismatch,
            },
            actual_sha256: actual,
            diagnostic: None,
        },
        Err(error) => {
            let text = error.to_string();
            let code = if text.starts_with("evidence_") {
                text.as_str()
            } else {
                "evidence_read_failed"
            };
            unknown(path, code)
        }
    }
}
fn linked(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
fn absolute(path: &Path) -> io::Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(fail("evidence_boundary_violation"));
    }
    Ok(())
}
#[cfg(windows)]
fn open_component(
    path: &Path,
    _parent: Option<&File>,
    _name: &std::ffi::OsStr,
    directory: bool,
) -> io::Result<File> {
    use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
    // OPEN_REPARSE_POINT never follows the final reparse point. Holding each
    // ancestor without FILE_SHARE_DELETE pins its path while later components open.
    OpenOptions::new()
        .read(true)
        .share_mode(1 | 2)
        .custom_flags(0x00200000 | if directory { 0x02000000 } else { 0 })
        .open(path)
}
#[cfg(unix)]
fn open_component(
    path: &Path,
    parent: Option<&File>,
    name: &std::ffi::OsStr,
    directory: bool,
) -> io::Result<File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
    };
    let name = CString::new(if parent.is_some() {
        name.as_bytes()
    } else {
        path.as_os_str().as_bytes()
    })
    .map_err(|_| fail("evidence_invalid_path"))?;
    let flags = libc::O_RDONLY
        | libc::O_CLOEXEC
        | libc::O_NOFOLLOW
        | libc::O_NONBLOCK
        | if directory { libc::O_DIRECTORY } else { 0 };
    let fd = unsafe {
        libc::openat(
            parent.map_or(libc::AT_FDCWD, AsRawFd::as_raw_fd),
            name.as_ptr(),
            flags,
        )
    };
    if fd < 0 {
        let error = io::Error::last_os_error();
        return Err(if error.raw_os_error() == Some(libc::ELOOP) {
            fail("evidence_linked_path")
        } else {
            error
        });
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
#[cfg(not(any(unix, windows)))]
fn open_component(
    _path: &Path,
    _parent: Option<&File>,
    _name: &std::ffi::OsStr,
    _directory: bool,
) -> io::Result<File> {
    Err(fail("evidence_boundary_violation"))
}

#[derive(Debug, PartialEq, Eq)]
struct Identity(u64, u64);
#[cfg(unix)]
fn identity(file: &File) -> io::Result<Identity> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata()?;
    Ok(Identity(m.dev(), m.ino()))
}
#[cfg(windows)]
fn identity(file: &File) -> io::Result<Identity> {
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
        return Err(io::Error::last_os_error());
    }
    Ok(Identity(
        info.volume as u64,
        ((info.index_high as u64) << 32) | info.index_low as u64,
    ))
}
#[cfg(not(any(unix, windows)))]
fn identity(_file: &File) -> io::Result<Identity> {
    Err(fail("evidence_boundary_violation"))
}
struct HeldPath {
    path: PathBuf,
    file: File,
    id: Identity,
    directory: bool,
}
/// Open each component with nofollow and retain ancestors for the entire read.
fn walk(
    path: &Path,
    final_directory: bool,
    allow_missing: bool,
    held: &mut Vec<HeldPath>,
) -> io::Result<Option<usize>> {
    absolute(path)?;
    let components: Vec<_> = path.components().collect();
    let mut current = PathBuf::new();
    let mut parent = None;
    for (index, component) in components.iter().enumerate() {
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let directory = index + 1 != components.len() || final_directory;
        // Library, game and target walks share ancestors inside one read. Reuse
        // only these held handles; every path is still revalidated after reading.
        // No evidence or handle survives this inspection or a file change.
        if let Some(held_index) = held.iter().position(|entry| entry.path == current) {
            if held[held_index].directory != directory {
                return Err(fail("evidence_not_regular_file"));
            }
            parent = Some(held_index);
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(m) if linked(&m) => return Err(fail("evidence_linked_path")),
            Ok(m) if directory && !m.is_dir() => return Err(fail("evidence_not_regular_file")),
            Ok(m) if !directory && !m.is_file() => return Err(fail("evidence_not_regular_file")),
            Ok(_) => (),
            Err(e) if allow_missing && e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        }
        let file = open_component(
            &current,
            parent.map(|i: usize| &held[i].file),
            component.as_os_str(),
            directory,
        )?;
        let metadata = file.metadata()?;
        if linked(&metadata) {
            return Err(fail("evidence_linked_path"));
        }
        if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
            return Err(fail("evidence_not_regular_file"));
        }
        let id = identity(&file)?;
        held.push(HeldPath {
            path: current.clone(),
            file,
            id,
            directory,
        });
        parent = Some(held.len() - 1);
    }
    Ok(parent)
}
fn verify_paths(held: &[HeldPath]) -> io::Result<()> {
    for entry in held {
        let m = fs::symlink_metadata(&entry.path).map_err(|_| fail("evidence_changed"))?;
        if linked(&m) {
            return Err(fail("evidence_linked_path"));
        }
        let current = open_component(&entry.path, None, entry.path.as_os_str(), entry.directory)
            .map_err(|_| fail("evidence_changed"))?;
        if identity(&current)? != entry.id {
            return Err(fail("evidence_changed"));
        }
    }
    Ok(())
}
/// Bounded immutable source JSON read using the same nofollow/identity guard
/// as game evidence. Captured bytes never cross a linked ancestor boundary.
pub(crate) fn read_bounded(path: &Path, max: u64) -> io::Result<Option<Vec<u8>>> {
    let path = std::path::absolute(path)?;
    let mut held = Vec::new();
    let index = walk(&path, false, true, &mut held)?;
    verify_paths(&held)?;
    let Some(index) = index else {
        return Ok(None);
    };
    let before = snapshot(&held[index].file)?;
    if before.size > max {
        return Err(fail("evidence_size_limit"));
    }
    let mut bytes = Vec::new();
    (&held[index].file)
        .take(before.size.saturating_add(1))
        .read_to_end(&mut bytes)?;
    verify_paths(&held)?;
    let after = snapshot(&held[index].file)?;
    if bytes.len() as u64 != before.size || !unchanged(&before, &after) {
        return Err(fail("evidence_changed"));
    }
    Ok(Some(bytes))
}

struct Snapshot {
    identity: Identity,
    size: u64,
    modified: SystemTime,
}
fn snapshot(file: &File) -> io::Result<Snapshot> {
    let m = file.metadata()?;
    Ok(Snapshot {
        identity: identity(file)?,
        size: m.len(),
        modified: m.modified()?,
    })
}
fn unchanged(before: &Snapshot, after: &Snapshot) -> bool {
    before.identity == after.identity
        && before.size == after.size
        && before.modified == after.modified
}
// The same no-follow walks and handle identity checks serve file evidence and
// snapshot action facts; action projection never hashes a synthetic file.
fn hold_game_boundary(game: &GameInstall, held: &mut Vec<HeldPath>) -> io::Result<()> {
    walk(&game.library_path, true, false, held)?;
    walk(&game.install_path, true, false, held)?;
    let library = fs::canonicalize(&game.library_path)?;
    let install = fs::canonicalize(&game.install_path)?;
    if !install.starts_with(&library) {
        return Err(fail("evidence_boundary_violation"));
    }
    Ok(())
}
pub(crate) fn validate_game_boundary(game: &GameInstall, cancel: &AtomicBool) -> io::Result<()> {
    cancelled(cancel)?;
    let mut held = Vec::new();
    hold_game_boundary(game, &mut held)?;
    let common = game.library_path.join("steamapps/common");
    walk(&common, true, false, &mut held)?;
    let common = fs::canonicalize(common)?;
    let install = fs::canonicalize(&game.install_path)?;
    if install == common || !install.starts_with(&common) {
        return Err(fail("evidence_boundary_violation"));
    }
    verify_paths(&held)?;
    cancelled(cancel)
}
fn read_digest(request: &EvidenceRequest<'_>) -> io::Result<Option<String>> {
    read_digest_with(request, |_| {})
}
fn read_digest_with(
    request: &EvidenceRequest<'_>,
    mut after_chunk: impl FnMut(u64),
) -> io::Result<Option<String>> {
    cancelled(request.cancel)?;
    let mut held = Vec::new();
    hold_game_boundary(request.game, &mut held)?;
    let target = request
        .game
        .install_path
        .join(relative_path(request.relative_path.as_str())?);
    let index = walk(&target, false, true, &mut held)?;
    verify_paths(&held)?;
    cancelled(request.cancel)?;
    let Some(index) = index else {
        return Ok(None);
    };
    let before = snapshot(&held[index].file)?;
    if before.size > request.max_bytes.min(DEFAULT_MAX_BYTES) {
        return Err(fail("evidence_size_limit"));
    }
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        cancelled(request.cancel)?;
        // Read at most one sentinel byte beyond the pre-read size/limit, so
        // growing files cannot make this readonly operation unbounded.
        let remaining = before
            .size
            .min(request.max_bytes.min(DEFAULT_MAX_BYTES))
            .saturating_sub(total);
        let capacity = remaining.saturating_add(1).min(buffer.len() as u64) as usize;
        let count = (&held[index].file).read(&mut buffer[..capacity])?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or_else(|| fail("evidence_size_overflow"))?;
        if total > request.max_bytes.min(DEFAULT_MAX_BYTES) {
            return Err(fail("evidence_size_limit"));
        }
        if total > before.size {
            return Err(fail("evidence_changed"));
        }
        hasher.update(&buffer[..count]);
        after_chunk(total);
    }
    cancelled(request.cancel)?;
    verify_paths(&held)?;
    let after = snapshot(&held[index].file)?;
    cancelled(request.cancel)?;
    if total != before.size || !unchanged(&before, &after) {
        return Err(fail("evidence_changed"));
    }
    Ok(Some(format!("{:x}", hasher.finalize())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, time::Duration};
    fn game(root: &Path) -> GameInstall {
        GameInstall {
            instance_id: "test".into(),
            appid: 1,
            name: "test".into(),
            localized_name: None,
            name_language: None,
            library_path: root.into(),
            install_path: root.into(),
            buildid: None,
            availability: "installed".into(),
            kind: "unknown".into(),
        }
    }
    #[test]
    fn same_size_mutation_during_stream_discards_digest() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("patch.dat");
        fs::write(&target, vec![1_u8; 131072]).unwrap();
        let initial = fs::metadata(&target).unwrap().modified().unwrap();
        let game = game(temp.path());
        let path = RelativeGamePath::parse("patch.dat").unwrap();
        let hash = Sha256Digest::parse(&"0".repeat(64)).unwrap();
        let cancel = AtomicBool::new(false);
        let request = EvidenceRequest {
            game: &game,
            relative_path: &path,
            expected_sha256: &hash,
            max_bytes: DEFAULT_MAX_BYTES,
            cancel: &cancel,
        };
        let error = read_digest_with(&request, |total| {
            if total == 65536 {
                let mut writer = fs::OpenOptions::new().write(true).open(&target).unwrap();
                writer.write_all(&vec![2_u8; 65536]).unwrap();
                writer
                    .set_times(fs::FileTimes::new().set_modified(initial + Duration::from_secs(10)))
                    .unwrap();
            }
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "evidence_changed");
    }
    #[cfg(unix)]
    #[test]
    fn replaced_file_during_stream_discards_digest() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("patch.dat");
        fs::write(&target, vec![1_u8; 131072]).unwrap();
        let game = game(temp.path());
        let path = RelativeGamePath::parse("patch.dat").unwrap();
        let hash = Sha256Digest::parse(&"0".repeat(64)).unwrap();
        let cancel = AtomicBool::new(false);
        let request = EvidenceRequest {
            game: &game,
            relative_path: &path,
            expected_sha256: &hash,
            max_bytes: DEFAULT_MAX_BYTES,
            cancel: &cancel,
        };
        let error = read_digest_with(&request, |total| {
            if total == 65536 {
                fs::rename(&target, temp.path().join("old.dat")).unwrap();
                fs::write(&target, vec![1_u8; 131072]).unwrap();
            }
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "evidence_changed");
    }
    #[test]
    fn cancellation_during_stream_discards_digest() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("patch.dat"), vec![1_u8; 131072]).unwrap();
        let game = game(temp.path());
        let path = RelativeGamePath::parse("patch.dat").unwrap();
        let hash = Sha256Digest::parse(&"0".repeat(64)).unwrap();
        let cancel = AtomicBool::new(false);
        let request = EvidenceRequest {
            game: &game,
            relative_path: &path,
            expected_sha256: &hash,
            max_bytes: DEFAULT_MAX_BYTES,
            cancel: &cancel,
        };
        let error =
            read_digest_with(&request, |_| cancel.store(true, Ordering::Relaxed)).unwrap_err();
        assert_eq!(error.to_string(), "evidence_cancelled");
    }
}
