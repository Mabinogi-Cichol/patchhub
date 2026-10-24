//! Backup-only inventory and cleanup; journals never grant or restrict deletion.
use super::{
    cache::Node,
    cache_lock::CacheLock,
    outcome::{CleanupErrorCode, CleanupFailure, CleanupStatus},
    service::ValidatedHostContext,
};
use crate::projection::DomainError;
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    io,
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct BackupInventory {
    pub total_bytes: u64,
    pub files: u64,
    pub failures: Vec<CleanupFailure>,
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct BackupCleanupResult {
    pub status: CleanupStatus,
    pub removed_bytes: u64,
    pub removed_files: u64,
    pub failures: Vec<CleanupFailure>,
}
pub struct BackupService<'a> {
    host: &'a ValidatedHostContext,
}
impl<'a> BackupService<'a> {
    pub fn new(host: &'a ValidatedHostContext) -> Self {
        Self { host }
    }
    pub fn inspect(&self) -> Result<BackupInventory, DomainError> {
        self.host.refresh_ro()?;
        let mut scan = Scan::default();
        self.scan(false, None, &mut scan);
        Ok(BackupInventory {
            total_bytes: scan.bytes,
            files: scan.files,
            failures: scan.failures,
        })
    }
    pub fn cleanup(&self) -> Result<BackupCleanupResult, DomainError> {
        self.host.refresh()?;
        let lock = CacheLock::exclusive(self.host)?;
        let mut scan = Scan::default();
        self.scan(true, Some(&lock), &mut scan);
        Ok(BackupCleanupResult {
            status: if scan.files == 0
                && scan
                    .failures
                    .iter()
                    .any(|f| f.code == CleanupErrorCode::InstallerBusy)
            {
                CleanupStatus::Busy
            } else if scan.failed == 0 {
                CleanupStatus::Completed
            } else if scan.files > 0 {
                CleanupStatus::Partial
            } else {
                CleanupStatus::Failed
            },
            removed_bytes: scan.bytes,
            removed_files: scan.files,
            failures: scan.failures,
        })
    }
    fn scan(&self, delete: bool, lock: Option<&CacheLock>, scan: &mut Scan) {
        let mut roots = Vec::new();
        for path in [
            self.host.installer_paths().root().to_path_buf(),
            self.host.state_dir().to_path_buf(),
        ] {
            if let Some(store) = scan.root(&path) {
                collect_jobs(store, &mut roots, scan);
            }
        }
        if let Some(publisher) = scan.root(&self.host.state_dir().join("publisher")) {
            children(&publisher, scan, |release, scan| {
                children(release, scan, |instance, scan| {
                    if let Some(store) = scan.root(instance.path()) {
                        collect_jobs(store, &mut roots, scan);
                    }
                });
            });
        }
        // Pin all stores and existing legacy journal locks for the entire sweep.
        // Current production writers additionally contend on CacheLock.
        let mut journal_locks = Vec::new();
        if delete {
            for (store, _) in &roots {
                match CacheLock::legacy_journal(store.path()) {
                    Ok(guard) => journal_locks.push(guard),
                    Err(e) => {
                        scan.fail(store.path(), &io::Error::other(e.code));
                        return;
                    }
                }
            }
        }
        for (_, root) in &roots {
            if journal_locks.iter().any(|guard| guard.validate().is_err()) {
                scan.fail(root.path(), &io::Error::other("cleanup_linked_path"));
                break;
            }
            jobs(root, delete, lock, scan);
        }
    }
}
fn collect_jobs(store: Node, roots: &mut Vec<(Node, Node)>, scan: &mut Scan) {
    match store.child(OsStr::new("jobs"), false) {
        Ok(root) => roots.push((store, root)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (),
        Err(e) => scan.fail(store.path(), &e),
    }
}
#[derive(Default)]
struct Scan {
    bytes: u64,
    files: u64,
    failed: u64,
    failures: Vec<CleanupFailure>,
}
impl Scan {
    fn root(&mut self, path: &Path) -> Option<Node> {
        match Node::absolute(path) {
            Ok(n) => Some(n),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => {
                self.fail(path, &e);
                None
            }
        }
    }
    fn fail(&mut self, path: &Path, e: &io::Error) {
        self.failed += 1;
        let code = if e.to_string() == "installer_busy"
            || e.kind() == io::ErrorKind::WouldBlock
            || (cfg!(windows) && e.raw_os_error() == Some(32))
            || e.raw_os_error() == fs2::lock_contended_error().raw_os_error()
        {
            CleanupErrorCode::InstallerBusy
        } else if e.to_string().contains("cleanup_linked_path") || e.raw_os_error() == Some(40) {
            CleanupErrorCode::CleanupLinkedPath
        } else {
            CleanupErrorCode::CleanupFailed
        };
        let f = CleanupFailure {
            code,
            detail: DomainError::new("", format!("{}: {e}", path.display())).detail,
        };
        self.failures.push(f);
    }
}
fn children(node: &Node, scan: &mut Scan, mut action: impl FnMut(&Node, &mut Scan)) {
    let names = match node.names() {
        Ok(v) => v,
        Err(e) => {
            scan.fail(node.path(), &e);
            return;
        }
    };
    for name in names {
        match node.child(&name, false) {
            Ok(child) => match child.metadata() {
                Ok(m) if m.is_dir() => action(&child, scan),
                Ok(_) => (),
                Err(e) => scan.fail(child.path(), &e),
            },
            Err(e) => scan.fail(&node.path().join(&name), &e),
        }
    }
}
fn jobs(root: &Node, delete: bool, lock: Option<&CacheLock>, scan: &mut Scan) {
    children(root, scan, |job, scan| {
        let names = match job.names() {
            Ok(n) => n,
            Err(e) => {
                scan.fail(job.path(), &e);
                return;
            }
        };
        for name in names {
            let Some(text) = name.to_str() else { continue };
            // Known backup directory and indexed transaction files only, never arbitrary .backup elsewhere.
            let indexed = text
                .strip_suffix(".backup")
                .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
            let interrupted_cleanup = text
                .strip_prefix(".backup-cleanup-")
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok_and(|v| v.to_string() == id));
            if text == "backup" || indexed || interrupted_cleanup {
                material(job, &name, delete, lock, scan);
            }
        }
    });
}
enum MaterialEntry {
    File {
        node: Node,
        path: PathBuf,
        bytes: u64,
    },
    Dir {
        node: Node,
        name: OsString,
        path: PathBuf,
    },
}
/// Opens one backup entry and validates the sweep lock before touching it.
fn open_material(
    parent: &Node,
    name: &OsStr,
    delete: bool,
    lock: Option<&CacheLock>,
    scan: &mut Scan,
) -> Option<MaterialEntry> {
    let path = parent.path().join(name);
    if let Some(lock) = lock {
        if let Err(e) = lock.validate().map_err(io::Error::other) {
            scan.fail(&path, &e);
            return None;
        }
    }
    let node = match parent.child(name, delete) {
        Ok(node) => node,
        Err(e) => {
            scan.fail(&path, &e);
            return None;
        }
    };
    match node.metadata() {
        Ok(meta) if meta.is_dir() => Some(MaterialEntry::Dir {
            node,
            name: name.to_os_string(),
            path,
        }),
        Ok(meta) => Some(MaterialEntry::File {
            node,
            path,
            bytes: meta.len(),
        }),
        Err(e) => {
            scan.fail(&path, &e);
            None
        }
    }
}
fn count_file(
    node: Node,
    parent: &Node,
    name: &OsStr,
    path: &Path,
    bytes: u64,
    delete: bool,
    scan: &mut Scan,
) {
    let result = (|| -> io::Result<()> {
        scan.bytes
            .checked_add(bytes)
            .ok_or_else(|| io::Error::other("cleanup_size_overflow"))?;
        let bytes = if delete {
            node.remove_empty(parent, name)?
        } else {
            bytes
        };
        let total = scan
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| io::Error::other("cleanup_size_overflow"))?;
        scan.bytes = total;
        scan.files += 1;
        Ok(())
    })();
    if let Err(e) = result {
        scan.fail(path, &e);
    }
}
/// One backup directory awaiting post-order teardown. `node` is also the parent
/// handle for the level below, so a local stack is enough to carry depth.
struct MaterialFrame {
    node: Node,
    name: OsString,
    path: PathBuf,
    names: Vec<OsString>,
    index: usize,
    loaded: bool,
}
fn finish_dir(frame: MaterialFrame, parent: &Node, delete: bool, scan: &mut Scan) {
    if !delete {
        return;
    }
    let result = (|| -> io::Result<()> {
        if frame.node.names()?.is_empty() {
            frame.node.remove_empty(parent, &frame.name)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        scan.fail(&frame.path, &e);
    }
}
fn material(parent: &Node, name: &OsStr, delete: bool, lock: Option<&CacheLock>, scan: &mut Scan) {
    let mut stack: Vec<MaterialFrame> = Vec::new();
    match open_material(parent, name, delete, lock, scan) {
        Some(MaterialEntry::File { node, path, bytes }) => {
            count_file(node, parent, name, &path, bytes, delete, scan);
        }
        Some(MaterialEntry::Dir { node, name, path }) => {
            stack.push(MaterialFrame {
                node,
                name,
                path,
                names: Vec::new(),
                index: 0,
                loaded: false,
            });
        }
        None => return,
    }
    while let Some(last) = stack.len().checked_sub(1) {
        if !stack[last].loaded {
            stack[last].loaded = true;
            match stack[last].node.names() {
                Ok(names) => stack[last].names = names,
                Err(e) => {
                    let frame = stack.pop().unwrap();
                    scan.fail(&frame.path, &e);
                    continue;
                }
            }
        }
        if stack[last].index < stack[last].names.len() {
            let child = stack[last].names[stack[last].index].clone();
            stack[last].index += 1;
            match open_material(&stack[last].node, &child, delete, lock, scan) {
                Some(MaterialEntry::File { node, path, bytes }) => {
                    count_file(node, &stack[last].node, &child, &path, bytes, delete, scan);
                }
                Some(MaterialEntry::Dir { node, name, path }) => stack.push(MaterialFrame {
                    node,
                    name,
                    path,
                    names: Vec::new(),
                    index: 0,
                    loaded: false,
                }),
                None => {}
            }
            continue;
        }
        let frame = stack.pop().unwrap();
        let up = stack.last().map(|f| &f.node).unwrap_or(parent);
        finish_dir(frame, up, delete, scan);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn all_scan_failures_are_retained_beyond_the_former_limit() {
        let mut scan = Scan::default();
        for _ in 0..600 {
            scan.fail(Path::new("jobs"), &io::Error::other("cleanup_failed"));
        }
        assert_eq!(scan.failures.len(), 600);
        assert_eq!(scan.failed, 600);
        assert!(scan.failures.iter().all(|failure| !failure
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("omitted")));
    }
    fn fixture() -> (tempfile::TempDir, ValidatedHostContext) {
        let dir = tempfile::tempdir().unwrap();
        let platform = if cfg!(windows) {
            crate::catalog::Platform::Windows
        } else {
            crate::catalog::Platform::Steamos
        };
        let host = ValidatedHostContext::new(
            &dir.path().join("state"),
            &[dir.path().join("no-steam")],
            platform,
        )
        .unwrap();
        (dir, host)
    }
    fn put(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    #[test]
    fn inventory_is_readonly_when_state_missing() {
        let (_dir, host) = fixture();
        let inventory = BackupService::new(&host).inspect().unwrap();
        assert_eq!((inventory.files, inventory.total_bytes), (0, 0));
        assert!(!host.state_dir().exists());
    }
    #[test]
    fn all_layouts_states_corrupt_and_orphan_journals_are_cleared_only_backups() {
        let (_dir, host) = fixture();
        let roots = [
            host.installer_paths().jobs_dir(),
            host.state_dir().join("jobs"),
            host.state_dir().join("publisher/release/instance/jobs"),
        ];
        let states = [
            "committed",
            "failed",
            "rolled_back",
            "planned",
            "applying",
            "needs_recovery",
            "orphan",
        ];
        for root in &roots {
            for state in states {
                let job = root.join(state);
                put(&job.join("0.backup"), b"original");
                put(&job.join("backup/nested/original.dat"), b"old");
                put(&job.join("0.stage"), b"stage");
                put(&job.join("authorization.json"), b"authorization");
                if state != "orphan" {
                    put(&job.join("journal.json"), state.as_bytes());
                }
            }
        }
        for file in [
            "journal.sqlite",
            "receipts/receipt",
            "downloads/archive.zip",
            "sources/cache/record",
            "settings.json",
            "publisher/release/instance/journal.sqlite",
        ] {
            put(&host.state_dir().join(file), b"sentinel");
        }
        let service = BackupService::new(&host);
        let before = service.inspect().unwrap();
        assert_eq!((before.files, before.total_bytes), (42, 231));
        assert!(before.failures.is_empty());
        let result = service.cleanup().unwrap();
        assert_eq!(result.status, CleanupStatus::Completed);
        assert_eq!((result.removed_files, result.removed_bytes), (42, 231));
        for root in &roots {
            for state in states {
                let job = root.join(state);
                assert!(!job.join("0.backup").exists());
                assert!(!job.join("backup").exists());
                assert_eq!(fs::read(job.join("0.stage")).unwrap(), b"stage");
                assert_eq!(
                    fs::read(job.join("authorization.json")).unwrap(),
                    b"authorization"
                );
                if state != "orphan" {
                    assert_eq!(
                        fs::read(job.join("journal.json")).unwrap(),
                        state.as_bytes()
                    );
                }
            }
        }
        assert_eq!(
            fs::read(host.state_dir().join("journal.sqlite")).unwrap(),
            b"sentinel"
        );
        let empty = service.cleanup().unwrap();
        assert_eq!((empty.removed_files, empty.removed_bytes), (0, 0));
        assert_eq!(service.inspect().unwrap().files, 0);
    }
    #[test]
    fn install_and_cleanup_lock_compete_before_any_deletion() {
        let (_dir, host) = fixture();
        let backup = host
            .installer_paths()
            .jobs_dir()
            .join("interrupted/0.backup");
        put(&backup, b"original");
        let writer = CacheLock::shared(&host).unwrap();
        assert_eq!(
            BackupService::new(&host).cleanup().unwrap_err().code,
            "installer_busy"
        );
        assert!(backup.exists());
        drop(writer);
        assert_eq!(
            BackupService::new(&host).cleanup().unwrap().removed_files,
            1
        );
    }
    #[test]
    fn hardlinks_are_reported_partial_and_retry_clears_remaining() {
        let (dir, host) = fixture();
        let job = host.installer_paths().jobs_dir().join("orphan");
        put(&job.join("0.backup"), b"safe");
        put(&dir.path().join("external"), b"external");
        fs::hard_link(dir.path().join("external"), job.join("1.backup")).unwrap();
        let result = BackupService::new(&host).cleanup().unwrap();
        assert_eq!(result.status, CleanupStatus::Partial);
        assert_eq!((result.removed_files, result.removed_bytes), (1, 4));
        assert_eq!(result.failures[0].code, CleanupErrorCode::CleanupLinkedPath);
        assert_eq!(fs::read(dir.path().join("external")).unwrap(), b"external");
        fs::remove_file(dir.path().join("external")).unwrap();
        assert_eq!(
            BackupService::new(&host).cleanup().unwrap().removed_files,
            1
        );
    }
    #[test]
    fn interrupted_cleanup_capture_is_counted_and_reclaimed_without_ambiguous_stage_deletion() {
        let (_dir, host) = fixture();
        let job = host.installer_paths().jobs_dir().join("orphan");
        let capture = job.join(format!(".backup-cleanup-{}", uuid::Uuid::new_v4()));
        put(&capture.join("item"), b"original");
        put(&job.join(".cleanup-unknown/item"), b"could be stage");
        let service = BackupService::new(&host);
        assert_eq!(service.inspect().unwrap().files, 1);
        assert_eq!(service.cleanup().unwrap().removed_bytes, 8);
        assert!(!capture.exists());
        assert!(job.join(".cleanup-unknown/item").exists());
        assert_eq!(service.inspect().unwrap().files, 0);
    }
    #[test]
    fn legacy_journal_lock_blocks_entire_sweep() {
        use fs2::FileExt;
        let (_dir, host) = fixture();
        put(&host.state_dir().join("journal.lock"), b"");
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(host.state_dir().join("journal.lock"))
            .unwrap();
        file.try_lock_exclusive().unwrap();
        let backup = host.state_dir().join("jobs/interrupted/0.backup");
        let current = host.installer_paths().jobs_dir().join("orphan/0.backup");
        put(&backup, b"old");
        put(&current, b"new");
        let result = BackupService::new(&host).cleanup().unwrap();
        assert_eq!(result.removed_files, 0);
        assert_eq!(result.failures[0].code, CleanupErrorCode::InstallerBusy);
        assert!(backup.exists() && current.exists());
        drop(file);
        assert_eq!(
            BackupService::new(&host).cleanup().unwrap().removed_files,
            2
        );
    }
    #[cfg(unix)]
    #[test]
    fn cleanup_child() {
        let Some(state) = std::env::var_os("PATCHHUB_TEST_BACKUP_STATE") else {
            return;
        };
        let state = std::path::PathBuf::from(state);
        let host = ValidatedHostContext::new(
            &state,
            &[state.with_file_name("no-steam")],
            crate::catalog::Platform::Steamos,
        )
        .unwrap();
        BackupService::new(&host).cleanup().unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn killed_cleanup_after_rename_is_visible_and_retryable() {
        let (dir, host) = fixture();
        put(
            &host.installer_paths().jobs_dir().join("orphan/0.backup"),
            b"original",
        );
        let marker = dir.path().join("captured");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "installer::backup::tests::cleanup_child"])
            .env("PATCHHUB_TEST_BACKUP_STATE", host.state_dir())
            .env("PATCHHUB_TEST_BACKUP_CAPTURE", &marker)
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let reached = marker.exists();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(reached, "child did not reach capture crash point");
        let service = BackupService::new(&host);
        assert_eq!(
            (
                service.inspect().unwrap().files,
                service.inspect().unwrap().total_bytes
            ),
            (1, 8)
        );
        assert_eq!(service.cleanup().unwrap().removed_files, 1);
        assert_eq!(service.inspect().unwrap().files, 0);
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_legacy_root_does_not_touch_external_material() {
        let (dir, host) = fixture();
        let external = dir.path().join("external");
        put(&external.join("jobs/orphan/0.backup"), b"external");
        fs::create_dir_all(host.state_dir()).unwrap();
        std::os::unix::fs::symlink(&external, host.state_dir().join("publisher")).unwrap();
        let result = BackupService::new(&host).cleanup().unwrap();
        assert_eq!(result.status, CleanupStatus::Failed);
        assert_eq!(
            fs::read(external.join("jobs/orphan/0.backup")).unwrap(),
            b"external"
        );
    }
}
