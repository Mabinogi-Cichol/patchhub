//! Core-only, allowlisted cache inventory and deletion. No caller-supplied paths.
use super::{
    cache_lock::CacheLock,
    outcome::{CleanupErrorCode, CleanupFailure, CleanupResult, CleanupStatus},
    service::ValidatedHostContext,
};
use crate::projection::DomainError;
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

use native::Captured;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct CacheInventory {
    pub total_bytes: u64,
    pub entries: u64,
    pub diagnostics: Vec<CleanupFailure>,
}
pub struct CacheService<'a> {
    host: &'a ValidatedHostContext,
}
impl<'a> CacheService<'a> {
    pub fn new(host: &'a ValidatedHostContext) -> Self {
        Self { host }
    }
    pub fn inspect(&self) -> Result<CacheInventory, DomainError> {
        self.host.refresh_ro()?;
        let roots = self.roots()?;
        let mut state = Walk::default();
        for root in roots {
            inspect_tree(root, &mut state);
        }
        Ok(CacheInventory {
            total_bytes: state.bytes,
            entries: state.entries,
            diagnostics: state.failures,
        })
    }
    pub fn cleanup(&self) -> Result<CleanupResult, DomainError> {
        self.host.refresh()?;
        let lock = CacheLock::exclusive(self.host)?;
        // All root/ancestor checks precede deletion. Root handles stay pinned throughout.
        let roots = self.roots()?;
        let tasks = self.task_root()?;
        lock.validate()?;
        let mut state = Walk {
            retained_archives: crate::install::protected_archive_sources_at(
                self.host.installer_paths().root(),
            )
            .map_err(|e| DomainError::new("cleanup_failed", format!("archive retention: {e}")))?,
            ..Walk::default()
        };
        for root in roots {
            if !validate_lock(Some(&lock), &mut state) {
                break;
            }
            remove_children(&root, false, &mut state);
        }
        self.reclaim_tasks(Some(&lock), tasks.as_ref(), &mut state);
        let status = if state.failure_count == 0 {
            CleanupStatus::Completed
        } else if state.deleted != 0 {
            CleanupStatus::Partial
        } else {
            CleanupStatus::Failed
        };
        Ok(CleanupResult {
            status,
            performed: true,
            removed_bytes: state.bytes,
            failures: state.failures,
        })
    }
    fn roots(&self) -> Result<Vec<Node>, DomainError> {
        let mut roots = Vec::new();
        for path in self.host.installer_paths().cleanable_dirs() {
            match Node::absolute(&path) {
                Ok(node) if node.file.metadata()?.is_dir() => roots.push(node),
                Ok(_) => return Err(DomainError::code("cleanup_unknown_path")),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => {
                    let failure = failure(&path, &e);
                    return Err(DomainError {
                        code: code(failure.code).into(),
                        detail: failure.detail,
                    });
                }
            }
        }
        Ok(roots)
    }
    fn task_root(&self) -> Result<Option<Node>, DomainError> {
        let path = self.host.installer_paths().tasks_dir();
        match Node::absolute(&path) {
            Ok(node) if node.file.metadata()?.is_dir() => Ok(Some(node)),
            Ok(_) => Err(DomainError::code("cleanup_unknown_path")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => {
                let f = failure(&path, &e);
                Err(DomainError {
                    code: code(f.code).into(),
                    detail: f.detail,
                })
            }
        }
    }
    fn reclaim_tasks(&self, lock: Option<&CacheLock>, root: Option<&Node>, state: &mut Walk) {
        if !validate_lock(lock, state) {
            return;
        }
        let Some(root) = root else { return };
        let paths = self.host.installer_paths();
        let names = match root.names() {
            Ok(n) => n,
            Err(e) => {
                state.fail(failure(&root.path, &e));
                return;
            }
        };
        // Journal failure gives no terminal proof: preserve every task.
        let transactions = match crate::install::list_transactions_at(paths.root()) {
            Ok(t) => t,
            Err(_) => return,
        };
        for name in names {
            let Some(id) = name.to_str() else { continue };
            if uuid::Uuid::parse_str(id).map_or(true, |v| v.to_string() != id) {
                continue;
            }
            let task = match root.child(&name, true) {
                Ok(t) if t.file.metadata().is_ok_and(|m| m.is_dir()) => t,
                _ => continue,
            };
            let children = match task.names() {
                Ok(n) => n,
                _ => continue,
            };
            if children
                .iter()
                .any(|n| !matches!(n.to_str(), Some("task.json" | "owner.lock" | "revoked")))
            {
                continue;
            }
            let document = match task.json("task.json", 65536) {
                Ok(v) => v,
                _ => continue,
            };
            let Some(doc) = document.as_object() else {
                continue;
            };
            if doc.len() != 6
                || doc.keys().any(|k| {
                    !matches!(
                        k.as_str(),
                        "schema_version"
                            | "task_id"
                            | "request_id"
                            | "instance_id"
                            | "selection_id"
                            | "created_at_ms"
                    )
                })
                || doc.get("schema_version").and_then(|v| v.as_u64()) != Some(1)
                || doc.get("task_id").and_then(|v| v.as_str()) != Some(id)
            {
                continue;
            }
            let Some(request) = doc.get("request_id").and_then(|v| v.as_str()) else {
                continue;
            };
            if request.is_empty() || request.len() > 128 || request.chars().any(char::is_control) {
                continue;
            }
            let plan = super::requests::request_plan_id(request);
            if !transactions.iter().any(|t| {
                t.plan_id == plan
                    && matches!(
                        t.state.as_str(),
                        "committed" | "rolled_back" | "superseded" | "uninstalled"
                    )
                    && document.get("instance_id").and_then(|v| v.as_str())
                        == Some(t.instance_id.as_str())
                    && document.get("selection_id").and_then(|v| v.as_str())
                        == Some(t.selection_id.as_deref().unwrap_or(&t.release_id))
            }) {
                continue;
            }
            // The owner lock is the liveness signal: if it is still held, a live
            // task owns the directory and reclamation is refused.
            let owner = match task.child(OsStr::new("owner.lock"), true) {
                Ok(n) => n,
                _ => continue,
            };
            if fs2::FileExt::try_lock_exclusive(&owner.file).is_err() {
                continue;
            }
            if !owner.file.metadata().is_ok_and(|m| m.is_file()) || !validate_lock(lock, state) {
                continue;
            }
            // Keep the owner lock until every other task file has been removed.
            // Windows uses this very handle for its final owner.lock deletion.
            let failures_before = state.failure_count;
            for child in children
                .iter()
                .filter(|n| n.as_os_str() != OsStr::new("owner.lock"))
            {
                remove_entry(&task, child, 1, true, state);
            }
            if state.failure_count != failures_before {
                continue;
            }
            finish_open_entry(owner, &task, OsStr::new("owner.lock"), 1, true, state);
            if state.failure_count != failures_before {
                continue;
            }
            finish_open_entry(task, root, &name, 0, true, state);
        }
    }
}
/// A task's own terminal teardown uses the directory captured before work starts.
/// This private path never authorizes stale-task reclamation or caller paths.
pub(crate) struct OwnedTaskDirectory {
    parent: Node,
    task: Node,
    name: std::ffi::OsString,
}
impl OwnedTaskDirectory {
    pub(crate) fn capture(path: &Path) -> io::Result<Self> {
        let name = path.file_name().ok_or_else(unknown)?.to_os_string();
        let parent = Node::absolute(path.parent().ok_or_else(unknown)?)?;
        let task = parent.child(&name, false)?;
        if !task.file.metadata()?.is_dir() {
            return Err(unknown());
        }
        Ok(Self { parent, task, name })
    }
    pub(crate) fn cleanup(self) {
        // During work Windows needs only a read-only pin. DELETE access on the
        // directory would conflict with atomic lease publication. At teardown,
        // release that pin, reopen under the still-pinned parent and verify the
        // saved volume/file identity before any deletion; the new handle pins it.
        #[cfg(windows)]
        let task = {
            let Ok(expected) = self.task.identity() else {
                return;
            };
            drop(self.task);
            let Ok(task) = self.parent.child(&self.name, true) else {
                return;
            };
            if task.identity().ok() != Some(expected) {
                return;
            }
            task
        };
        #[cfg(not(windows))]
        let task = self.task;
        let mut state = Walk::default();
        let Ok(names) = task.names() else { return };
        if names
            .iter()
            .any(|name| !matches!(name.to_str(), Some("task.json" | "owner.lock" | "revoked")))
        {
            return;
        }
        for name in names {
            remove_entry(&task, &name, 1, true, &mut state);
        }
        if state.failure_count == 0 {
            finish_open_entry(task, &self.parent, &self.name, 0, true, &mut state);
        }
    }
}
fn code(code: CleanupErrorCode) -> &'static str {
    match code {
        CleanupErrorCode::CleanupFailed => "cleanup_failed",
        CleanupErrorCode::CleanupLinkedPath => "cleanup_linked_path",
        CleanupErrorCode::CleanupUnknownPath => "cleanup_unknown_path",
        CleanupErrorCode::CleanupSizeOverflow => "cleanup_size_overflow",
        CleanupErrorCode::InstallerBusy => "installer_busy",
    }
}
fn failure(path: &Path, e: &io::Error) -> CleanupFailure {
    let code = if e.to_string().starts_with("cleanup_linked_path") || e.raw_os_error() == Some(40) {
        CleanupErrorCode::CleanupLinkedPath
    } else if e.to_string().starts_with("cleanup_unknown_path") {
        CleanupErrorCode::CleanupUnknownPath
    } else if e.to_string().starts_with("cleanup_size_overflow") {
        CleanupErrorCode::CleanupSizeOverflow
    } else {
        CleanupErrorCode::CleanupFailed
    };
    let detail = DomainError::new("", format!("{}: {e}", path.display())).detail;
    CleanupFailure { code, detail }
}
fn unsafe_path() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "cleanup_linked_path")
}
fn unknown() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "cleanup_unknown_path")
}
fn overflow() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "cleanup_size_overflow")
}
fn protected(name: &OsStr) -> bool {
    let name = name.to_string_lossy();
    matches!(
        name.as_ref(),
        "stage" | "backup" | "authorization.json" | "revoked"
    ) || name.ends_with(".stage")
        || name.ends_with(".backup")
}
#[derive(Default)]
struct Walk {
    retained_archives: Vec<PathBuf>,
    bytes: u64,
    entries: u64,
    deleted: u64,
    failures: Vec<CleanupFailure>,
    failure_count: u64,
}
impl Walk {
    fn fail(&mut self, failure: CleanupFailure) {
        self.failure_count = self.failure_count.saturating_add(1);
        self.failures.push(failure);
    }

    /// Counts a visited entry; the running total is the only bounded arithmetic left.
    fn visit(&mut self) -> io::Result<()> {
        self.entries = self.entries.checked_add(1).ok_or_else(overflow)?;
        Ok(())
    }
    fn record(&mut self, bytes: u64) -> io::Result<()> {
        self.bytes = self.bytes.checked_add(bytes).ok_or_else(overflow)?;
        Ok(())
    }
    fn count_removed(&mut self) -> io::Result<()> {
        self.deleted = self.deleted.checked_add(1).ok_or_else(overflow)?;
        Ok(())
    }
}
/// Read-only inventory. Directories are walked depth-first with a local stack of
/// in-progress frames: each frame keeps one open directory handle plus its
/// enumeration cursor, so a directory's whole subtree is drained before its next
/// sibling is opened. Open handles therefore stay bounded by the current depth
/// (not by the number of sibling directories), and the OS thread stack is unused.
struct InspectFrame {
    node: Node,
    names: Vec<OsString>,
    index: usize,
    loaded: bool,
}
fn inspect_tree(root: Node, state: &mut Walk) {
    let mut stack = vec![InspectFrame {
        node: root,
        names: Vec::new(),
        index: 0,
        loaded: false,
    }];
    while let Some(last) = stack.len().checked_sub(1) {
        if !stack[last].loaded {
            stack[last].loaded = true;
            match stack[last].node.names() {
                Ok(names) => stack[last].names = names,
                Err(e) => {
                    let frame = stack.pop().unwrap();
                    state.fail(failure(&frame.node.path, &e));
                    continue;
                }
            }
        }
        if stack[last].index >= stack[last].names.len() {
            stack.pop();
            continue;
        }
        let name = stack[last].names[stack[last].index].clone();
        stack[last].index += 1;
        let result = (|| -> io::Result<Option<Node>> {
            state.visit()?;
            if protected(&name) {
                return Err(unknown());
            }
            let node = stack[last].node.child(&name, false)?;
            let meta = node.file.metadata()?;
            if meta.is_dir() {
                Ok(Some(node))
            } else {
                state.record(meta.len())?;
                Ok(None)
            }
        })();
        match result {
            Ok(Some(node)) => stack.push(InspectFrame {
                node,
                names: Vec::new(),
                index: 0,
                loaded: false,
            }),
            Ok(None) => {}
            Err(e) => {
                let path = stack[last].node.path.join(&name);
                state.fail(failure(&path, &e));
            }
        }
    }
}
/// Deletes every child of a root directory (the root itself is retained).
fn remove_children(root: &Node, task: bool, state: &mut Walk) {
    match root.names() {
        Ok(names) => {
            for name in names {
                remove_entry(root, &name, 0, task, state);
            }
        }
        Err(e) => state.fail(failure(&root.path, &e)),
    }
}
fn remove_entry(parent: &Node, name: &OsStr, depth: usize, task: bool, state: &mut Walk) {
    let path = parent.path.join(name);
    if !state.retained_archives.is_empty() {
        if let Ok(candidate) = fs::canonicalize(&path) {
            // Preserve the referenced file and any containing cache directory.
            if state
                .retained_archives
                .iter()
                .any(|source| source.starts_with(&candidate))
            {
                return;
            }
        }
    }
    let result = (|| {
        state.visit()?;
        let allowed =
            matches!(name.to_str(), Some("task.json" | "owner.lock" | "revoked")) || depth == 0;
        if (!task && protected(name)) || (task && !allowed) {
            return Err(unknown());
        }
        let node = parent.child(name, true)?;
        delete_open(node, parent, name, depth, task, state);
        Ok(())
    })();
    if let Err(e) = result {
        state.fail(failure(&path, &e));
    }
}
fn validate_lock(lock: Option<&CacheLock>, state: &mut Walk) -> bool {
    match lock.map(CacheLock::validate).transpose() {
        Ok(_) => true,
        Err(e) => {
            state.fail(CleanupFailure {
                code: CleanupErrorCode::CleanupLinkedPath,
                detail: DomainError::new("", e.to_string()).detail,
            });
            false
        }
    }
}
fn finish_open_entry(
    node: Node,
    parent: &Node,
    name: &OsStr,
    depth: usize,
    task: bool,
    state: &mut Walk,
) {
    let result = state.visit().and_then(|()| {
        let removed = delete_subtree(node, parent, name, depth, task, false, state)?;
        state.record(removed)?;
        state.count_removed()
    });
    if let Err(e) = result {
        state.fail(failure(&parent.path.join(name), &e));
    }
}
/// Accounts one already-open entry (its own bytes and removal count) and reports
/// the entry's own failure; nested failures are reported inside [`delete_subtree`].
fn delete_open(
    node: Node,
    parent: &Node,
    name: &OsStr,
    depth: usize,
    task: bool,
    state: &mut Walk,
) {
    let path = parent.path.join(name);
    let result =
        delete_subtree(node, parent, name, depth, task, false, state).and_then(|removed| {
            state.record(removed)?;
            state.count_removed()
        });
    if let Err(e) = result {
        state.fail(failure(&path, &e));
    }
}
/// One directory level awaiting deletion. The `captured` handle is kept until the
/// level's children have been handled, so deletion proceeds post-order without
/// relying on the call stack for depth.
struct Frame {
    captured: Captured,
    name: OsString,
    report_path: PathBuf,
    depth: usize,
    task: bool,
    directory: bool,
    names: Vec<OsString>,
    index: usize,
    loaded: bool,
}
/// Deletes `node` (and its subtree) with a local explicit stack. The removed bytes
/// of the entry itself are returned; failures below the entry are recorded on
/// `state`, while a failure of the entry's own platform step is returned.
fn delete_subtree(
    node: Node,
    parent: &Node,
    name: &OsStr,
    depth: usize,
    task: bool,
    backup: bool,
    state: &mut Walk,
) -> io::Result<u64> {
    let captured = node.capture(parent, name, backup)?;
    let directory = match captured.is_dir() {
        Ok(directory) => directory,
        Err(e) => {
            captured.abort(parent, name);
            return Err(e);
        }
    };
    let mut stack = vec![Frame {
        captured,
        name: name.to_os_string(),
        report_path: parent.path.join(name),
        depth,
        task,
        directory,
        names: Vec::new(),
        index: 0,
        loaded: false,
    }];
    loop {
        let last = stack.len() - 1;
        if !stack[last].loaded {
            stack[last].loaded = true;
            if stack[last].directory {
                match stack[last].captured.names() {
                    Ok(names) => stack[last].names = names,
                    Err(e) => {
                        let frame = stack.pop().unwrap();
                        if let Some(below) = stack.last() {
                            let up = below.captured.parent_ref();
                            frame.captured.abort(up, &frame.name);
                            state.fail(failure(&frame.report_path, &e));
                            continue;
                        }
                        frame.captured.abort(parent, &frame.name);
                        return Err(e);
                    }
                }
            }
        }
        if stack[last].index < stack[last].names.len() {
            let child = stack[last].names[stack[last].index].clone();
            stack[last].index += 1;
            process_child(&mut stack, last, &child, backup, state);
            continue;
        }
        let frame = stack.pop().unwrap();
        let Some(below) = stack.last() else {
            return frame.captured.finish(parent, &frame.name, frame.directory);
        };
        let up = below.captured.parent_ref();
        match frame.captured.finish(up, &frame.name, frame.directory) {
            Ok(removed) => {
                if let Err(e) = state.record(removed).and_then(|()| state.count_removed()) {
                    state.fail(failure(&frame.report_path, &e));
                }
            }
            Err(e) => state.fail(failure(&frame.report_path, &e)),
        }
    }
}
fn process_child(
    stack: &mut Vec<Frame>,
    parent_index: usize,
    name: &OsStr,
    backup: bool,
    state: &mut Walk,
) {
    let report = stack[parent_index].captured.parent_ref().path.join(name);
    // A child is gated with the containing frame's depth, i.e. one less than the
    // original recursion's `depth + 1`. The current task callers are equivalent
    // because every task child is opened via `remove_entry`/`finish_open_entry`
    // at depth 1 — `process_child` only ever sees a non-zero containing depth —
    // and the task directory itself reaches `finish_open_entry(..., 0, true)`
    // only after its whitelisted children were removed. `depth == 0` therefore
    // still denotes the task directory root, never one of its entries.
    let depth = stack[parent_index].depth;
    let task = stack[parent_index].task;
    if !state.retained_archives.is_empty() {
        if let Ok(candidate) = fs::canonicalize(&report) {
            if state
                .retained_archives
                .iter()
                .any(|source| source.starts_with(&candidate))
            {
                return;
            }
        }
    }
    if let Err(e) = state.visit() {
        state.fail(failure(&report, &e));
        return;
    }
    let allowed =
        matches!(name.to_str(), Some("task.json" | "owner.lock" | "revoked")) || depth == 0;
    if (!task && protected(name)) || (task && !allowed) {
        state.fail(failure(&report, &unknown()));
        return;
    }
    let node = match stack[parent_index].captured.child(name, true) {
        Ok(node) => node,
        Err(e) => {
            state.fail(failure(&report, &e));
            return;
        }
    };
    let captured = match node.capture(stack[parent_index].captured.parent_ref(), name, backup) {
        Ok(captured) => captured,
        Err(e) => {
            state.fail(failure(&report, &e));
            return;
        }
    };
    let directory = match captured.is_dir() {
        Ok(directory) => directory,
        Err(e) => {
            captured.abort(stack[parent_index].captured.parent_ref(), name);
            state.fail(failure(&report, &e));
            return;
        }
    };
    if directory {
        stack.push(Frame {
            captured,
            name: name.to_os_string(),
            report_path: report,
            depth: depth + 1,
            task,
            directory,
            names: Vec::new(),
            index: 0,
            loaded: false,
        });
        return;
    }
    match captured.finish(stack[parent_index].captured.parent_ref(), name, false) {
        Ok(removed) => {
            if let Err(e) = state.record(removed).and_then(|()| state.count_removed()) {
                state.fail(failure(&report, &e));
            }
        }
        Err(e) => state.fail(failure(&report, &e)),
    }
}

pub(crate) struct Node {
    file: File,
    path: PathBuf,
    _ancestors: Vec<File>,
}
impl Node {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn metadata(&self) -> io::Result<fs::Metadata> {
        self.file.metadata()
    }
    pub(crate) fn remove_empty(self, parent: &Node, name: &OsStr) -> io::Result<u64> {
        if self.file.metadata()?.is_dir() && !self.names()?.is_empty() {
            return Err(unknown());
        }
        // Nested failures are not this caller's concern; only the entry's own
        // platform result is returned (as before, via a throwaway `Walk`).
        delete_subtree(self, parent, name, 0, false, true, &mut Walk::default())
    }
    fn json(&self, name: &str, limit: u64) -> io::Result<serde_json::Value> {
        let node = self.child(OsStr::new(name), false)?;
        if !node.file.metadata()?.is_file() || node.file.metadata()?.len() > limit {
            return Err(unknown());
        }
        let mut bytes = Vec::new();
        node.file.take(limit + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            return Err(unknown());
        }
        serde_json::from_slice(&bytes).map_err(|_| unknown())
    }
}

#[cfg(unix)]
mod native {
    use super::*;
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::MetadataExt},
        },
    };
    fn c(name: &OsStr) -> io::Result<CString> {
        CString::new(name.as_bytes()).map_err(|_| unknown())
    }
    fn open(parent: i32, name: &OsStr, directory: bool) -> io::Result<File> {
        let mut before: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                parent,
                c(name)?.as_ptr(),
                &mut before,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        if before.st_mode & libc::S_IFMT == libc::S_IFLNK {
            return Err(unsafe_path());
        }
        if directory && before.st_mode & libc::S_IFMT != libc::S_IFDIR {
            return Err(unknown());
        }
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if directory { libc::O_DIRECTORY } else { 0 };
        let fd = unsafe { libc::openat(parent, c(name)?.as_ptr(), flags) };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        check(&file)?;
        Ok(file)
    }
    fn check(file: &File) -> io::Result<()> {
        let m = file.metadata()?;
        if !m.is_dir() && !m.is_file() {
            return Err(unknown());
        }
        if m.is_file() && m.nlink() != 1 {
            return Err(unsafe_path());
        }
        Ok(())
    }
    fn same(a: &File, b: &File) -> io::Result<bool> {
        let a = a.metadata()?;
        let b = b.metadata()?;
        Ok(a.dev() == b.dev() && a.ino() == b.ino() && a.file_type() == b.file_type())
    }
    fn unlink(parent: &File, name: &OsStr, directory: bool) -> io::Result<()> {
        if unsafe {
            libc::unlinkat(
                parent.as_raw_fd(),
                c(name)?.as_ptr(),
                if directory { libc::AT_REMOVEDIR } else { 0 },
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    impl Node {
        pub(crate) fn absolute(path: &Path) -> io::Result<Self> {
            let path = std::path::absolute(path)?;
            let mut current = open(libc::AT_FDCWD, OsStr::new("/"), true)?;
            let mut ancestors = Vec::new();
            for component in path.components() {
                match component {
                    std::path::Component::RootDir => {}
                    std::path::Component::Normal(n) => {
                        let next = open(current.as_raw_fd(), n, true)?;
                        ancestors.push(current);
                        current = next;
                    }
                    _ => return Err(unknown()),
                }
            }
            Ok(Self {
                file: current,
                path,
                _ancestors: ancestors,
            })
        }
        pub(crate) fn child(&self, name: &OsStr, _delete: bool) -> io::Result<Self> {
            Ok(Self {
                file: open(self.file.as_raw_fd(), name, false)?,
                path: self.path.join(name),
                _ancestors: Vec::new(),
            })
        }
        pub(crate) fn names(&self) -> io::Result<Vec<std::ffi::OsString>> {
            // /proc resolves this already-held directory descriptor, never a mutable path ancestor.
            let path = format!("/proc/self/fd/{}", self.file.as_raw_fd());
            let mut result = Vec::new();
            for entry in fs::read_dir(path)? {
                result.push(entry?.file_name());
            }
            Ok(result)
        }
        /// Captures the open node by rename into a fresh private directory. A
        /// replacement between open and rename is preserved, never unlinked based
        /// on the old stat. The returned handle addresses the captured item.
        pub(super) fn capture(
            self,
            parent: &Node,
            name: &OsStr,
            backup: bool,
        ) -> io::Result<Captured> {
            let prefix = if backup {
                ".backup-cleanup-"
            } else {
                ".cleanup-"
            };
            let quarantine = format!("{prefix}{}", uuid::Uuid::new_v4());
            if unsafe {
                libc::mkdirat(
                    parent.file.as_raw_fd(),
                    c(OsStr::new(&quarantine))?.as_ptr(),
                    0o700,
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
            let q = parent.child(OsStr::new(&quarantine), false)?;
            let item = OsStr::new("item");
            if unsafe {
                libc::renameat(
                    parent.file.as_raw_fd(),
                    c(name)?.as_ptr(),
                    q.file.as_raw_fd(),
                    c(item)?.as_ptr(),
                )
            } != 0
            {
                let error = io::Error::last_os_error();
                let _ = unlink(&parent.file, OsStr::new(&quarantine), true);
                return Err(error);
            }
            #[cfg(test)]
            if backup {
                if let Some(marker) = std::env::var_os("STEAM_PATCH_TEST_BACKUP_CAPTURE") {
                    fs::write(marker, b"captured")?;
                    loop {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
            }
            let captured = q.child(item, false).and_then(|captured| {
                if !same(&self.file, &captured.file)? {
                    return Err(unsafe_path());
                }
                Ok(captured)
            });
            match captured {
                Ok(item) => Ok(Captured {
                    q,
                    item,
                    quarantine: OsString::from(quarantine),
                }),
                Err(error) => {
                    // Any failure after the rename restores the original name and
                    // removes the quarantine, matching the pre-rewrite recursion.
                    restore(&q, parent, name);
                    let _ = unlink(&parent.file, OsStr::new(&quarantine), true);
                    Err(error)
                }
            }
        }
    }
    /// No-replace restore keeps racing new cache entries untouched (Linux only).
    fn restore(q: &Node, parent: &Node, name: &OsStr) {
        #[cfg(target_os = "linux")]
        unsafe {
            if let (Ok(item), Ok(target)) = (c(OsStr::new("item")), c(name)) {
                libc::renameat2(
                    q.file.as_raw_fd(),
                    item.as_ptr(),
                    parent.file.as_raw_fd(),
                    target.as_ptr(),
                    libc::RENAME_NOREPLACE,
                );
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (q, parent, name);
    }
    /// The renamed capture kept open while its children (if any) are deleted and
    /// its quarantine directory removed afterwards.
    pub(super) struct Captured {
        q: Node,
        item: Node,
        quarantine: OsString,
    }
    impl Captured {
        pub(super) fn parent_ref(&self) -> &Node {
            &self.item
        }
        pub(super) fn is_dir(&self) -> io::Result<bool> {
            Ok(self.item.file.metadata()?.is_dir())
        }
        pub(super) fn names(&self) -> io::Result<Vec<std::ffi::OsString>> {
            self.item.names()
        }
        pub(super) fn child(&self, name: &OsStr, delete: bool) -> io::Result<Node> {
            self.item.child(name, delete)
        }
        /// Verifies identity, unlinks the captured name and removes the quarantine.
        pub(super) fn finish(
            self,
            parent: &Node,
            name: &OsStr,
            directory: bool,
        ) -> io::Result<u64> {
            let result = (|| {
                check(&self.item.file)?;
                let item = OsStr::new("item");
                // Revalidate identity after traversal before unlinking the captured name.
                if !same(&self.item.file, &self.q.child(item, false)?.file)? {
                    return Err(unsafe_path());
                }
                let bytes = if directory {
                    0
                } else {
                    self.item.file.metadata()?.len()
                };
                unlink(&self.q.file, item, directory)?;
                Ok(bytes)
            })();
            if result.is_err() {
                restore(&self.q, parent, name);
            }
            let _ = unlink(&parent.file, OsStr::new(&self.quarantine), true);
            result
        }
        /// Discards a capture whose children were never enumerated.
        pub(super) fn abort(self, parent: &Node, name: &OsStr) {
            restore(&self.q, parent, name);
            let _ = unlink(&parent.file, OsStr::new(&self.quarantine), true);
        }
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    #[repr(C)]
    struct Information {
        attributes: u32,
        times: [u32; 6],
        volume: u32,
        high: u32,
        low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[repr(C)]
    struct Disposition {
        delete: u8,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut Information)
            -> i32;
        fn SetFileInformationByHandle(
            handle: *mut std::ffi::c_void,
            class: i32,
            info: *const Disposition,
            size: u32,
        ) -> i32;
    }
    fn open(path: &Path, delete: bool, directory: bool) -> io::Result<File> {
        // No FILE_SHARE_DELETE on any ancestor; files additionally deny writes.
        let file = fs::OpenOptions::new()
            .access_mode(0x80000000 | if delete { 0x10000 } else { 0 })
            .share_mode(if directory { 1 | 2 } else { 1 })
            .custom_flags(0x00200000 | 0x02000000)
            .open(path)?;
        check(&file)?;
        Ok(file)
    }
    fn check(file: &File) -> io::Result<()> {
        let meta = file.metadata()?;
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(unsafe_path());
        }
        if !meta.is_dir() && !meta.is_file() {
            return Err(unknown());
        }
        let mut info: Information = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if meta.is_file() && info.links != 1 {
            return Err(unsafe_path());
        }
        Ok(())
    }
    impl Node {
        pub(crate) fn identity(&self) -> io::Result<(u64, u64)> {
            let mut info: Information = unsafe { std::mem::zeroed() };
            if unsafe { GetFileInformationByHandle(self.file.as_raw_handle(), &mut info) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok((
                u64::from(info.volume),
                (u64::from(info.index_high) << 32) | u64::from(info.index_low),
            ))
        }
        pub(crate) fn absolute(path: &Path) -> io::Result<Self> {
            let path = std::path::absolute(path)?;
            let mut current = PathBuf::new();
            let mut ancestors = Vec::new();
            for part in path.components() {
                match part {
                    std::path::Component::Prefix(_) => current.push(part),
                    std::path::Component::RootDir | std::path::Component::Normal(_) => {
                        current.push(part);
                        ancestors.push(open(&current, false, true)?);
                    }
                    _ => return Err(unknown()),
                }
            }
            let file = ancestors.pop().ok_or_else(unknown)?;
            Ok(Self {
                file,
                path,
                _ancestors: ancestors,
            })
        }
        pub(crate) fn child(&self, name: &OsStr, delete: bool) -> io::Result<Self> {
            let path = self.path.join(name);
            let directory = fs::symlink_metadata(&path)?.is_dir();
            Ok(Self {
                file: open(&path, delete, directory)?,
                path,
                _ancestors: Vec::new(),
            })
        }
        pub(crate) fn names(&self) -> io::Result<Vec<std::ffi::OsString>> {
            let mut result = Vec::new();
            for entry in fs::read_dir(&self.path)? {
                result.push(entry?.file_name());
            }
            Ok(result)
        }
        /// Windows deletes through the checked handle itself, so there is nothing
        /// to capture before traversal; the returned handle is the directory pin.
        pub(super) fn capture(
            self,
            _parent: &Node,
            _name: &OsStr,
            _backup: bool,
        ) -> io::Result<Captured> {
            Ok(Captured { node: self })
        }
    }
    /// The open handle kept pinned while its children (if any) are deleted and
    /// the handle itself is marked for deletion afterwards.
    pub(super) struct Captured {
        node: Node,
    }
    impl Captured {
        pub(super) fn parent_ref(&self) -> &Node {
            &self.node
        }
        pub(super) fn is_dir(&self) -> io::Result<bool> {
            Ok(self.node.file.metadata()?.is_dir())
        }
        pub(super) fn names(&self) -> io::Result<Vec<std::ffi::OsString>> {
            self.node.names()
        }
        pub(super) fn child(&self, name: &OsStr, delete: bool) -> io::Result<Node> {
            self.node.child(name, delete)
        }
        pub(super) fn finish(
            self,
            _parent: &Node,
            _name: &OsStr,
            directory: bool,
        ) -> io::Result<u64> {
            let bytes = if directory {
                0
            } else {
                self.node.file.metadata()?.len()
            };
            check(&self.node.file)?;
            let disposition = Disposition { delete: 1 };
            // Delete exactly the checked handle. Path/ancestor replacement cannot redirect it.
            if unsafe {
                SetFileInformationByHandle(
                    self.node.file.as_raw_handle(),
                    4,
                    &disposition,
                    std::mem::size_of::<Disposition>() as u32,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            drop(self.node);
            Ok(bytes)
        }
        /// Windows has no quarantine to remove for a capture that was never traversed.
        pub(super) fn abort(self, _parent: &Node, _name: &OsStr) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn host(dir: &Path) -> ValidatedHostContext {
        let platform = if cfg!(windows) {
            crate::catalog::Platform::Windows
        } else {
            crate::catalog::Platform::Steamos
        };
        ValidatedHostContext::new(&dir.join("state"), &[dir.join("no-steam")], platform).unwrap()
    }
    fn fixture() -> (tempfile::TempDir, ValidatedHostContext) {
        let dir = tempfile::tempdir().unwrap();
        let host = host(dir.path());
        for path in host.installer_paths().cleanable_dirs() {
            fs::create_dir_all(path).unwrap();
        }
        (dir, host)
    }
    fn local_cleanup(host: &ValidatedHostContext) -> Walk {
        let mut state = Walk::default();
        for root in CacheService::new(host).roots().unwrap() {
            remove_children(&root, false, &mut state);
        }
        state
    }
    #[test]
    fn owned_terminal_teardown_cannot_follow_replaced_task_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        let tasks = dir.path().join("tasks");
        let task = tasks.join("owned");
        fs::create_dir_all(&task).unwrap();
        fs::write(task.join("task.json"), b"own").unwrap();
        let owned = OwnedTaskDirectory::capture(&task).unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir_all(outside.join("owned")).unwrap();
        fs::write(outside.join("owned/original.backup"), b"external").unwrap();
        #[cfg(unix)]
        {
            fs::rename(&tasks, dir.path().join("old-tasks")).unwrap();
            std::os::unix::fs::symlink(&outside, &tasks).unwrap();
        }
        #[cfg(windows)]
        assert!(fs::rename(&tasks, dir.path().join("old-tasks")).is_err());
        owned.cleanup();
        assert_eq!(
            fs::read(outside.join("owned/original.backup")).unwrap(),
            b"external"
        );
        #[cfg(unix)]
        {
            assert!(!dir.path().join("old-tasks/owned").exists());
            fs::remove_file(&tasks).unwrap();
        }
        #[cfg(windows)]
        assert!(!task.exists());
    }
    #[test]
    fn inspect_missing_state_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let host = host(dir.path());
        let inventory = CacheService::new(&host).inspect().unwrap();
        assert_eq!((inventory.entries, inventory.total_bytes), (0, 0));
        assert!(inventory.diagnostics.is_empty());
        assert!(!host.state_dir().exists());
    }
    #[test]
    fn inventory_and_cleanup_count_files_not_protected_state() {
        let (_dir, host) = fixture();
        let paths = host.installer_paths();
        fs::create_dir(paths.content_dir().join("a")).unwrap();
        fs::write(paths.content_dir().join("a/one"), b"123").unwrap();
        fs::write(paths.downloads_dir().join("two"), b"45678").unwrap();
        for name in [
            "jobs", "receipts", "runtime", "history", "sources", "settings", "trust",
        ] {
            fs::create_dir_all(paths.root().join(name)).unwrap();
            fs::write(paths.root().join(name).join("keep"), b"protected").unwrap();
        }
        for name in [
            "journal.sqlite",
            "journal.sqlite-wal",
            "requests.sqlite",
            "schema.json",
            "cache.lock",
            "journal.lock",
        ] {
            fs::write(paths.root().join(name), b"protected").unwrap();
        }
        let inventory = CacheService::new(&host).inspect().unwrap();
        assert_eq!((inventory.total_bytes, inventory.entries), (8, 3));
        let result = local_cleanup(&host);
        assert!(result.failures.is_empty());
        assert_eq!(result.bytes, 8);
        for name in [
            "jobs", "receipts", "runtime", "history", "sources", "settings", "trust",
        ] {
            assert_eq!(
                fs::read(paths.root().join(name).join("keep")).unwrap(),
                b"protected"
            );
        }
        for name in [
            "journal.sqlite",
            "journal.sqlite-wal",
            "requests.sqlite",
            "schema.json",
            "cache.lock",
            "journal.lock",
        ] {
            assert_eq!(fs::read(paths.root().join(name)).unwrap(), b"protected");
        }
    }
    #[test]
    fn hard_link_failure_reports_only_real_removed_bytes() {
        let (dir, host) = fixture();
        let paths = host.installer_paths();
        let external = dir.path().join("external");
        fs::write(&external, b"outside").unwrap();
        fs::hard_link(&external, paths.content_dir().join("link")).unwrap();
        fs::write(paths.downloads_dir().join("safe"), b"safe").unwrap();
        let inventory = CacheService::new(&host).inspect().unwrap();
        assert_eq!(inventory.total_bytes, 4);
        assert!(inventory
            .diagnostics
            .iter()
            .any(|f| f.code == CleanupErrorCode::CleanupLinkedPath));
        let result = local_cleanup(&host);
        assert_eq!(result.bytes, 4);
        assert_eq!(result.deleted, 1);
        assert!(result
            .failures
            .iter()
            .any(|f| f.code == CleanupErrorCode::CleanupLinkedPath));
        assert_eq!(fs::read(&external).unwrap(), b"outside");
        assert!(paths.content_dir().join("link").exists());
    }
    #[test]
    fn recovery_names_inside_cache_are_preserved() {
        let (_dir, host) = fixture();
        let dir = host.installer_paths().content_dir();
        fs::create_dir_all(dir.join("nested/save.backup")).unwrap();
        fs::write(dir.join("nested/save.backup/original"), b"recover").unwrap();
        fs::write(dir.join("nested/safe"), b"ok").unwrap();
        let result = local_cleanup(&host);
        assert_eq!(result.bytes, 2);
        assert!(!result.failures.is_empty());
        assert_eq!(
            fs::read(dir.join("nested/save.backup/original")).unwrap(),
            b"recover"
        );
    }
    #[cfg(unix)]
    #[test]
    fn linked_root_and_nested_links_preserve_external_bytes() {
        use std::os::unix::fs::symlink;
        let (dir, host) = fixture();
        let paths = host.installer_paths();
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), b"external").unwrap();
        fs::create_dir(paths.content_dir().join("nested")).unwrap();
        symlink(&outside, paths.content_dir().join("nested/link")).unwrap();
        fs::write(paths.downloads_dir().join("safe"), b"ok").unwrap();
        let result = local_cleanup(&host);
        assert_eq!(result.bytes, 2);
        assert!(!result.failures.is_empty());
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"external");
        fs::remove_dir(paths.reports_dir()).unwrap();
        symlink(&outside, paths.reports_dir()).unwrap();
        assert_eq!(
            CacheService::new(&host).roots().err().unwrap().code,
            "cleanup_linked_path"
        );
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"external");
    }
    #[test]
    fn stale_task_without_journal_terminal_proof_is_preserved() {
        let (_dir, host) = fixture();
        let tasks = host.installer_paths().tasks_dir();
        fs::create_dir(&tasks).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let task = tasks.join(&id);
        fs::create_dir(&task).unwrap();
        fs::write(task.join("owner.lock"), b"").unwrap();
        fs::write(task.join("task.json"), serde_json::to_vec(&serde_json::json!({"schema_version":1,"task_id":id,"request_id":"request","instance_id":"instance","selection_id":"release","created_at_ms":0})).unwrap()).unwrap();
        fs::write(task.join("revoked"), b"revoked").unwrap();
        let unknown = tasks.join("unknown");
        fs::create_dir(&unknown).unwrap();
        fs::write(unknown.join("original"), b"keep").unwrap();
        let mut state = Walk::default();
        let service = CacheService::new(&host);
        let tasks = service.task_root().unwrap();
        service.reclaim_tasks(None, tasks.as_ref(), &mut state);
        assert!(task.exists());
        assert!(unknown.join("original").exists());
        assert_eq!(state.deleted, 0);
    }
    fn task_with_journal(host: &ValidatedHostContext, request: &str, terminal: &str) -> PathBuf {
        let paths = host.installer_paths();
        let id = uuid::Uuid::new_v4().to_string();
        let task = paths.tasks_dir().join(&id);
        fs::create_dir_all(&task).unwrap();
        fs::write(task.join("owner.lock"), b"").unwrap();
        fs::write(task.join("task.json"), serde_json::to_vec(&serde_json::json!({"schema_version":1,"task_id":id,"request_id":request,"instance_id":"instance","selection_id":"release","created_at_ms":0})).unwrap()).unwrap();
        let plan = super::super::requests::request_plan_id(request);
        let document = serde_json::json!({
            "plan":{"plan_id":plan,"instance_id":"instance","release_id":"release","catalog_revision":1,"created_at":0,
                "install":{"instance_id":"instance","appid":1,"name":"fixture","localized_name":null,"name_language":null,"library_path":host.state_dir().join("unused-library"),"install_path":host.state_dir().join("unused-game"),"buildid":null,"availability":"installed","kind":"game"},
                "changes":[{"path":"patch","action":"add","original_sha256":null,"installed_sha256":"1".repeat(64),"size":1}],"disk_requirement":1,"blockers":[]},
            "transaction":{"transaction_id":plan,"plan_id":plan,"instance_id":"instance","release_id":"release","state":terminal,"error":null,"created_at":0,"updated_at":1},
            "progress":[if terminal == "committed" {"installed"} else {"pending"}],"created_dirs":[]
        });
        let conn = rusqlite::Connection::open(paths.journal()).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY, document TEXT NOT NULL)",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO jobs(id,document) VALUES(?1,?2)",
            rusqlite::params![plan, document.to_string()],
        )
        .unwrap();
        drop(conn);
        task
    }
    #[test]
    fn terminal_task_reclaimed_owner_lock_and_recovery_tasks_preserved() {
        let (_dir, host) = fixture();
        let free = task_with_journal(&host, "free", "committed");
        let held = task_with_journal(&host, "held", "committed");
        let recovery = task_with_journal(&host, "recovery", "needs_recovery");
        let material = task_with_journal(&host, "material", "committed");
        fs::write(material.join("original.backup"), b"recover").unwrap();
        let owner = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(held.join("owner.lock"))
            .unwrap();
        fs2::FileExt::try_lock_exclusive(&owner).unwrap();
        assert_eq!(
            crate::install::list_transactions_at(host.installer_paths().root())
                .unwrap()
                .len(),
            4
        );
        let mut state = Walk::default();
        let service = CacheService::new(&host);
        let tasks = service.task_root().unwrap();
        service.reclaim_tasks(None, tasks.as_ref(), &mut state);
        assert!(state.failures.is_empty(), "{:?}", state.failures);
        assert!(!free.exists());
        assert!(state.bytes > 0);
        assert!(held.exists());
        assert!(recovery.exists());
        assert_eq!(
            fs::read(material.join("original.backup")).unwrap(),
            b"recover"
        );
        drop(owner);
    }
    #[test]
    fn unbounded_utf8_failure_keeps_full_detail_and_checked_inventory() {
        let path_text = "测".repeat(3000);
        let f = failure(Path::new(&path_text), &unknown());
        let detail = f.detail.unwrap();
        assert!(detail.contains(&path_text));
        assert!(detail.ends_with(": cleanup_unknown_path"));
        let mut state = Walk {
            bytes: u64::MAX,
            ..Walk::default()
        };
        assert!(state.record(1).is_err());
        assert_eq!(state.bytes, u64::MAX);
        let mut state = Walk {
            entries: u64::MAX,
            ..Walk::default()
        };
        assert!(state.visit().is_err());
        assert_eq!(state.entries, u64::MAX);
    }
    #[test]
    fn deep_cache_tree_is_inventoried_and_cleaned() {
        let (_dir, host) = fixture();
        let paths = host.installer_paths();
        // 160 levels exceeds the removed 128-deep recursion gate; the traversal
        // must complete without relying on the OS thread stack.
        let mut nested = paths.content_dir().to_path_buf();
        for _ in 0..160 {
            nested.push("d");
        }
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("payload"), b"deep").unwrap();
        let inventory = CacheService::new(&host).inspect().unwrap();
        assert_eq!(inventory.entries, 161);
        assert_eq!(inventory.total_bytes, 4);
        assert!(
            inventory.diagnostics.is_empty(),
            "{:?}",
            inventory.diagnostics
        );
        let result = local_cleanup(&host);
        assert!(result.failures.is_empty(), "{:?}", result.failures);
        assert_eq!(result.bytes, 4);
        assert_eq!(result.deleted, 161);
        assert!(!paths.content_dir().join("d").exists());
        assert_eq!(CacheService::new(&host).inspect().unwrap().entries, 0);
    }
    #[test]
    fn wide_and_deep_cache_tree_is_fully_inventoried() {
        let (_dir, host) = fixture();
        let paths = host.installer_paths();
        // More sibling directories than a typical low fd/handle limit: the walk
        // must open one directory at a time, not every sibling up front.
        for index in 0..300 {
            let dir = paths.content_dir().join(format!("wide-{index}"));
            fs::create_dir(&dir).unwrap();
            fs::write(dir.join("payload"), b"w").unwrap();
        }
        // A nested chain keeps the deep case covered alongside the wide level.
        let mut deep = paths.content_dir().join("chain");
        for _ in 0..150 {
            deep.push("d");
        }
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("payload"), b"z").unwrap();
        let inventory = CacheService::new(&host).inspect().unwrap();
        assert_eq!(inventory.entries, 300 + 300 + 1 + 150 + 1);
        assert_eq!(inventory.total_bytes, 300 + 1);
        assert!(
            inventory.diagnostics.is_empty(),
            "{:?}",
            inventory.diagnostics
        );
    }
    #[test]
    #[ignore = "creates and clears >100k real entries; run explicitly with --ignored"]
    fn large_flat_cache_directory_is_inventoried_and_cleaned() {
        let (_dir, host) = fixture();
        let paths = host.installer_paths();
        const COUNT: u64 = 100_001;
        for index in 0..COUNT {
            fs::File::create(paths.downloads_dir().join(format!("f{index}"))).unwrap();
        }
        let inventory = CacheService::new(&host).inspect().unwrap();
        assert_eq!(inventory.entries, COUNT);
        assert_eq!(inventory.total_bytes, 0);
        assert!(
            inventory.diagnostics.is_empty(),
            "{:?}",
            inventory.diagnostics
        );
        let result = local_cleanup(&host);
        assert!(result.failures.is_empty(), "{:?}", result.failures);
        assert_eq!(result.deleted, COUNT);
        assert!(fs::read_dir(paths.downloads_dir())
            .unwrap()
            .next()
            .is_none());
        assert_eq!(CacheService::new(&host).inspect().unwrap().entries, 0);
    }
    #[test]
    fn diagnostics_are_complete_and_keep_total_failure_count() {
        const COUNT: usize = 600;
        let mut state = Walk::default();
        for _ in 0..COUNT {
            state.fail(failure(Path::new("cache"), &unknown()));
        }
        assert_eq!(state.failures.len(), COUNT);
        assert_eq!(state.failure_count, COUNT as u64);
        assert!(state.failures.iter().all(|failure| !failure
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("omitted")));
        let before = state.failure_count;
        state.fail(failure(Path::new("task"), &unknown()));
        assert_ne!(
            state.failure_count, before,
            "task protection must observe failures after diagnostic truncation"
        );
        assert_eq!(state.failures.len(), COUNT + 1);
    }
    #[test]
    fn shared_installer_blocks_manual_cleanup_without_deleting() {
        if super::super::host_identity_blocker().is_some() {
            return;
        }
        let (_dir, host) = fixture();
        let path = host.installer_paths().downloads_dir().join("safe");
        fs::write(&path, b"keep").unwrap();
        let _shared = CacheLock::shared(&host).unwrap();
        assert_eq!(
            CacheService::new(&host).cleanup().unwrap_err().code,
            "installer_busy"
        );
        assert_eq!(fs::read(path).unwrap(), b"keep");
    }
}
