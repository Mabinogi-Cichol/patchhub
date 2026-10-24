//! Fixed cache lock: shared for installer lifetimes, exclusive for cache deletion.
//! Lock acquisition never initializes databases or republishes an interrupted store.
use super::{service::ValidatedHostContext, InstallerPaths};
use crate::projection::DomainError;
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Component, Path, PathBuf},
};

type Result<T> = std::result::Result<T, DomainError>;

/// No caller-supplied lock path is accepted. The file persists after release so
/// cooperating processes always contend on the same filesystem object.
#[derive(Debug)]
pub(crate) struct CacheLock {
    file: File,
    anchors: Vec<DirectoryAnchor>,
    path: PathBuf,
}

impl CacheLock {
    pub(crate) fn shared(host: &ValidatedHostContext) -> Result<Self> {
        Self::acquire(host, false)
    }

    pub(crate) fn exclusive(host: &ValidatedHostContext) -> Result<Self> {
        Self::acquire(host, true)
    }

    /// Legacy transaction stores share their fixed journal.lock with older writers.
    /// Create it even for orphan stores so a later writer cannot bypass the sweep.
    pub(crate) fn legacy_journal(root: &Path) -> Result<Self> {
        let anchors = directories(root)?;
        let path = root.join("journal.lock");
        let file = open_lock(&anchors.last().ok_or_else(unsafe_state)?.file, &path)?;
        validate_file(&file)?;
        let guard = Self {
            file,
            anchors,
            path,
        };
        FileExt::try_lock_exclusive(&guard.file).map_err(lock_error)?;
        guard.validate()?;
        Ok(guard)
    }

    fn acquire(host: &ValidatedHostContext, exclusive: bool) -> Result<Self> {
        // Both modes may create the fixed lock, hence both require write identity
        // and fresh state/game boundary validation before any filesystem mutation.
        host.refresh()?;
        Self::acquire_paths(host.installer_paths(), exclusive)
    }

    fn acquire_paths(paths: &InstallerPaths, exclusive: bool) -> Result<Self> {
        let anchors = directories(paths.root())?;
        let path = paths.cache_lock();
        let root = anchors.last().ok_or_else(unsafe_state)?;
        let file = open_lock(&root.file, &path)?;
        validate_file(&file)?;
        let guard = Self {
            file,
            anchors,
            path,
        };
        guard.validate()?;
        // Unix flock on the existing store directory also prevents inode splitting
        // if a non-cooperating process replaces only cache.lock while a writer runs.
        #[cfg(unix)]
        {
            let root = &guard.anchors.last().ok_or_else(unsafe_state)?.file;
            let locked = if exclusive {
                FileExt::try_lock_exclusive(root)
            } else {
                FileExt::try_lock_shared(root)
            };
            locked.map_err(lock_error)?;
        }
        let locked = if exclusive {
            FileExt::try_lock_exclusive(&guard.file)
        } else {
            FileExt::try_lock_shared(&guard.file)
        };
        locked.map_err(lock_error)?;
        // A path or directory exchanged during open/lock cannot grant authority.
        guard.validate()?;
        Ok(guard)
    }

    /// Recheck before an operation if it has yielded to other work since acquire.
    pub(crate) fn validate(&self) -> Result<()> {
        for anchor in &self.anchors {
            let current = open_directory(&anchor.path)?;
            if file_identity(&current)? != file_identity(&anchor.file)? {
                return Err(unsafe_state());
            }
        }
        validate_file(&self.file)?;
        let current = open_existing_lock(&self.path)?;
        validate_file(&current)?;
        if file_identity(&current)? != file_identity(&self.file)? {
            return Err(unsafe_state());
        }
        Ok(())
    }
}

impl Drop for CacheLock {
    fn drop(&mut self) {
        // Explicit unlock before directory handles drop. Never unlink cache.lock.
        let _ = FileExt::unlock(&self.file);
        #[cfg(unix)]
        if let Some(root) = self.anchors.last() {
            let _ = FileExt::unlock(&root.file);
        }
    }
}

#[derive(Debug)]
struct DirectoryAnchor {
    path: PathBuf,
    file: File,
}

fn unsafe_state() -> DomainError {
    DomainError::code("linked_installer_state")
}
fn io_error(error: io::Error) -> DomainError {
    DomainError::new("io_error", error.to_string())
}
fn open_error(error: io::Error) -> DomainError {
    #[cfg(unix)]
    if matches!(
        error.raw_os_error(),
        Some(libc::ELOOP | libc::ENOTDIR | libc::EISDIR)
    ) {
        return unsafe_state();
    }
    io_error(error)
}
fn lock_error(error: io::Error) -> DomainError {
    if error.kind() == io::ErrorKind::WouldBlock
        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
    {
        DomainError::code("installer_busy")
    } else {
        io_error(error)
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

/// Open every ancestor before creating children. Unix child creation is anchored
/// to parent descriptors; Windows handles deny directory replacement via DELETE.
fn directories(root: &Path) -> Result<Vec<DirectoryAnchor>> {
    if !root.is_absolute() {
        return Err(unsafe_state());
    }
    let mut path = PathBuf::new();
    let mut anchors: Vec<DirectoryAnchor> = Vec::new();
    for component in root.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(unsafe_state());
        }
        path.push(component.as_os_str());
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let file = match component {
            Component::Normal(name) => {
                let parent = anchors.last().ok_or_else(unsafe_state)?;
                open_child_directory(&parent.file, name, &path)?
            }
            _ => open_directory(&path)?,
        };
        let metadata = file.metadata().map_err(io_error)?;
        if !metadata.is_dir() || linked(&metadata) {
            return Err(unsafe_state());
        }
        anchors.push(DirectoryAnchor {
            path: path.clone(),
            file,
        });
    }
    Ok(anchors)
}

#[cfg(unix)]
fn open_directory(path: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(path)
        .map_err(open_error)?;
    Ok(file)
}
#[cfg(windows)]
fn open_directory(path: &Path) -> Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    let file = OpenOptions::new()
        .read(true)
        .share_mode(3)
        .custom_flags(0x0200_0000 | 0x0020_0000)
        .open(path)
        .map_err(open_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_dir() || linked(&metadata) {
        return Err(unsafe_state());
    }
    Ok(file)
}

#[cfg(unix)]
fn open_child_directory(parent: &File, name: &std::ffi::OsStr, _path: &Path) -> Result<File> {
    use std::{
        ffi::CString,
        os::unix::{
            ffi::OsStrExt,
            io::{AsRawFd, FromRawFd},
        },
    };
    let name = CString::new(name.as_bytes()).map_err(|_| unsafe_state())?;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let mut fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 && io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
        let made = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
        if made < 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
            return Err(io_error(io::Error::last_os_error()));
        }
        fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    }
    if fd < 0 {
        return Err(open_error(io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
#[cfg(windows)]
fn open_child_directory(_parent: &File, _name: &std::ffi::OsStr, path: &Path) -> Result<File> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if linked(&metadata) || !metadata.is_dir() => return Err(unsafe_state()),
        Ok(_) => (),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::create_dir(path) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(io_error(error)),
        },
        Err(error) => return Err(io_error(error)),
    }
    open_directory(path)
}

#[cfg(unix)]
fn open_lock(root: &File, path: &Path) -> Result<File> {
    use std::os::unix::io::{AsRawFd, FromRawFd};
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let name = CString::new(path.file_name().ok_or_else(unsafe_state)?.as_bytes())
        .map_err(|_| unsafe_state())?;
    let flags = libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
    let mut fd = unsafe {
        libc::openat(
            root.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )
    };
    if fd < 0 && io::Error::last_os_error().kind() == io::ErrorKind::AlreadyExists {
        fd = unsafe { libc::openat(root.as_raw_fd(), name.as_ptr(), flags) };
    }
    if fd < 0 {
        return Err(open_error(io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
#[cfg(windows)]
fn lock_options() -> OpenOptions {
    use std::os::windows::fs::OpenOptionsExt;
    let mut options = OpenOptions::new();
    // Allow cooperating read/write opens, deny DELETE/rename while held.
    options
        .read(true)
        .write(true)
        .share_mode(3)
        .custom_flags(0x0020_0000);
    options
}
#[cfg(windows)]
fn open_lock(_root: &File, path: &Path) -> Result<File> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if linked(&metadata) || !metadata.is_file() => return Err(unsafe_state()),
        Ok(_) => (),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(io_error(error)),
    }
    match lock_options().create_new(true).open(path) {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => open_existing_lock(path),
        Err(error) => Err(open_error(error)),
    }
}
fn open_existing_lock(path: &Path) -> Result<File> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if linked(&metadata) || !metadata.is_file() {
        return Err(unsafe_state());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path)
            .map_err(open_error)
    }
    #[cfg(windows)]
    {
        lock_options().open(path).map_err(open_error)
    }
}
fn validate_file(file: &File) -> Result<()> {
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() || linked(&metadata) {
        return Err(unsafe_state());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(unsafe_state());
        }
    }
    #[cfg(windows)]
    if information(file)?.links != 1 {
        return Err(unsafe_state());
    }
    Ok(())
}
#[cfg(unix)]
fn file_identity(file: &File) -> Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(io_error)?;
    Ok((metadata.dev(), metadata.ino()))
}
#[cfg(windows)]
fn file_identity(file: &File) -> Result<(u64, u64)> {
    let info = information(file)?;
    Ok((
        u64::from(info.volume),
        u64::from(info.index_high) << 32 | u64::from(info.index_low),
    ))
}
#[cfg(windows)]
#[repr(C)]
struct FileInformation {
    attributes: u32,
    times: [u32; 6],
    volume: u32,
    size_high: u32,
    size_low: u32,
    links: u32,
    index_high: u32,
    index_low: u32,
}
#[cfg(windows)]
fn information(file: &File) -> Result<FileInformation> {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(
            handle: *mut std::ffi::c_void,
            info: *mut FileInformation,
        ) -> i32;
    }
    let mut info = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io_error(io::Error::last_os_error()));
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_child() {
        let Some(state) = std::env::var_os("PATCHHUB_CACHE_LOCK_CHILD_STATE") else {
            return;
        };
        let mode = std::env::var("PATCHHUB_CACHE_LOCK_CHILD_MODE").unwrap();
        let expected = std::env::var("PATCHHUB_CACHE_LOCK_CHILD_EXPECT").unwrap();
        let paths = InstallerPaths::new(Path::new(&state));
        let result = if mode == "legacy" {
            CacheLock::legacy_journal(paths.root())
        } else {
            CacheLock::acquire_paths(&paths, mode == "exclusive")
        };
        if expected == "ok" {
            result.unwrap();
        } else {
            assert_eq!(result.unwrap_err().code, expected);
        }
    }

    fn child(state: &Path, mode: &str, expected: &str) {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "installer::cache_lock::tests::lock_child",
                "--nocapture",
            ])
            .env("PATCHHUB_CACHE_LOCK_CHILD_STATE", state)
            .env("PATCHHUB_CACHE_LOCK_CHILD_MODE", mode)
            .env("PATCHHUB_CACHE_LOCK_CHILD_EXPECT", expected)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "child test did not execute"
        );
    }

    #[test]
    fn cross_process_shared_exclusive_and_release() {
        let temp = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(temp.path());
        let reader = CacheLock::acquire_paths(&paths, false).unwrap();
        child(temp.path(), "shared", "ok");
        child(temp.path(), "exclusive", "installer_busy");
        drop(reader);
        let writer = CacheLock::acquire_paths(&paths, true).unwrap();
        child(temp.path(), "shared", "installer_busy");
        child(temp.path(), "exclusive", "installer_busy");
        drop(writer);
        child(temp.path(), "exclusive", "ok");
    }

    #[test]
    fn missing_legacy_journal_lock_is_created_and_excludes_a_later_process() {
        let temp = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(temp.path());
        let guard = CacheLock::legacy_journal(paths.root()).unwrap();
        assert!(paths.journal_lock().is_file());
        child(temp.path(), "legacy", "installer_busy");
        drop(guard);
        child(temp.path(), "legacy", "ok");
    }

    #[test]
    fn shared_readers_coexist_and_release_for_writer() {
        let temp = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(temp.path());
        let first = CacheLock::acquire_paths(&paths, false).unwrap();
        let second = CacheLock::acquire_paths(&paths, false).unwrap();
        assert_eq!(
            CacheLock::acquire_paths(&paths, true).unwrap_err().code,
            "installer_busy"
        );
        drop(first);
        assert_eq!(
            CacheLock::acquire_paths(&paths, true).unwrap_err().code,
            "installer_busy"
        );
        drop(second);
        let writer = CacheLock::acquire_paths(&paths, true).unwrap();
        drop(writer);
        assert!(paths.cache_lock().is_file());
        assert!(!paths.schema().exists());
        assert!(!paths.journal().exists());
        assert!(!paths.requests().exists());
        CacheLock::acquire_paths(&paths, false).unwrap();
    }

    #[test]
    fn writer_excludes_readers_and_other_writers() {
        let temp = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(temp.path());
        let writer = CacheLock::acquire_paths(&paths, true).unwrap();
        assert_eq!(
            CacheLock::acquire_paths(&paths, false).unwrap_err().code,
            "installer_busy"
        );
        assert_eq!(
            CacheLock::acquire_paths(&paths, true).unwrap_err().code,
            "installer_busy"
        );
        drop(writer);
        CacheLock::acquire_paths(&paths, true).unwrap();
    }

    #[test]
    fn hard_link_lock_is_rejected_without_touching_external_file() {
        let temp = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(temp.path());
        fs::create_dir_all(paths.root()).unwrap();
        let external = temp.path().join("external");
        fs::write(&external, b"leave intact").unwrap();
        fs::hard_link(&external, paths.cache_lock()).unwrap();
        assert_eq!(
            CacheLock::acquire_paths(&paths, true).unwrap_err().code,
            "linked_installer_state"
        );
        assert_eq!(fs::read(external).unwrap(), b"leave intact");
    }

    #[cfg(unix)]
    #[test]
    fn linked_lock_and_ancestor_are_rejected_without_external_mutation() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(temp.path());
        fs::create_dir_all(paths.root()).unwrap();
        let external = temp.path().join("external");
        fs::write(&external, b"leave intact").unwrap();
        symlink(&external, paths.cache_lock()).unwrap();
        assert_eq!(
            CacheLock::acquire_paths(&paths, true).unwrap_err().code,
            "linked_installer_state"
        );
        assert_eq!(fs::read(&external).unwrap(), b"leave intact");
        let linked_state = temp.path().join("linked-state");
        symlink(temp.path(), &linked_state).unwrap();
        assert_eq!(
            CacheLock::acquire_paths(&InstallerPaths::new(&linked_state), true)
                .unwrap_err()
                .code,
            "linked_installer_state"
        );
    }

    #[cfg(unix)]
    #[test]
    fn replaced_lock_or_directory_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(temp.path());
        let lock = CacheLock::acquire_paths(&paths, false).unwrap();
        fs::rename(paths.cache_lock(), paths.root().join("old.lock")).unwrap();
        fs::write(paths.cache_lock(), b"").unwrap();
        assert_eq!(lock.validate().unwrap_err().code, "linked_installer_state");
        assert_eq!(
            CacheLock::acquire_paths(&paths, true).unwrap_err().code,
            "installer_busy"
        );
        drop(lock);
        let lock = CacheLock::acquire_paths(&paths, false).unwrap();
        fs::rename(paths.root(), paths.root().with_file_name("old-root")).unwrap();
        fs::create_dir(paths.root()).unwrap();
        assert_eq!(lock.validate().unwrap_err().code, "linked_installer_state");
    }

    #[cfg(windows)]
    #[test]
    fn held_lock_and_directory_cannot_be_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(temp.path());
        let lock = CacheLock::acquire_paths(&paths, false).unwrap();
        assert!(fs::rename(paths.cache_lock(), paths.root().join("old.lock")).is_err());
        assert!(fs::remove_file(paths.cache_lock()).is_err());
        assert!(fs::rename(paths.root(), paths.root().with_file_name("old-root")).is_err());
        lock.validate().unwrap();
    }

    #[test]
    fn lock_survives_partial_store_initialization() {
        let temp = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(temp.path());
        let lock = CacheLock::acquire_paths(&paths, false).unwrap();
        super::super::init_atomic(&paths).unwrap();
        lock.validate().unwrap();
        assert_eq!(
            CacheLock::acquire_paths(&paths, true).unwrap_err().code,
            "installer_busy"
        );
        assert!(paths.is_protected(&paths.cache_lock()));
    }
}
