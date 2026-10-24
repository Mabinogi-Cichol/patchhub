//! Conservative local installer state, recovery and direct-extraction targets.
//! SQLite journals intent before mutation; recovery is hash guarded, not globally atomic.
#[cfg(test)]
use crate::catalog::{Catalog, SupportLevel};
use crate::operation;
use crate::{
    catalog::{self, FileMapping, PatchRelease},
    steam::{self, GameInstall},
};
use fs2::FileExt;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub const MAX_PLAN_METADATA_BYTES: usize = crate::zip_metadata::MAX_METADATA_BYTES as usize;

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum InstallOperation {
    #[default]
    Install,
    Reinstall,
    Overwrite,
}

/// Content identity excludes pre-operation hashes, which are snapshots during reinstall.
pub fn recipe_identity(release: &PatchRelease) -> io::Result<String> {
    let recipe = release
        .recipe
        .as_ref()
        .ok_or_else(|| invalid("recipe_missing"))?;
    mapping_identity(
        release.appid,
        &release.release_id,
        &release.language,
        &recipe.artifact_sha256,
        &recipe.files,
    )
}

/// Same identity shape as [`recipe_identity`] for callers whose mapping is not
/// a signed catalog recipe (dynamic publisher ZIP).
pub fn mapping_identity(
    appid: u32,
    release_id: &str,
    language: &str,
    artifact_sha256: &str,
    mappings: &[FileMapping],
) -> io::Result<String> {
    let mappings: Vec<_> = mappings
        .iter()
        .map(|m| (&m.source, &m.destination, &m.installed_sha256))
        .collect();
    let bytes = serde_json::to_vec(&(appid, release_id, language, artifact_sha256, mappings))
        .map_err(|e| invalid(e.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlannedChange {
    pub path: String,
    pub action: String,
    pub original_sha256: Option<String>,
    /// Digest of the installed bytes. `None` before a direct extraction has
    /// produced them; archive-backed plans always set it during preparation.
    #[serde(default)]
    pub installed_sha256: Option<String>,
    pub size: u64,
    /// Archive member this change was extracted from (`None` for journals written
    /// before the field existed, or for non-recipe callers).
    ///
    /// The member *name* is the stable locator (design §4.2): prepare rejects
    /// duplicate or case-colliding member names and verifies that the name maps
    /// to exactly one member, so a later re-open (`write_archive_member` /
    /// `verified_archive_prefix`) resolves the same member by name. No persisted
    /// member index is needed while that uniqueness check holds.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallPlan {
    #[serde(default)]
    pub operation: InstallOperation,
    #[serde(default)]
    pub recipe_sha256: Option<String>,
    pub plan_id: String,
    pub instance_id: String,
    pub release_id: String,
    pub catalog_revision: u64,
    pub created_at: u64,
    /// Legacy plan-expiry timestamp written before the local plan TTL was
    /// removed (design D01). Read-only compatibility for journals created by
    /// older builds: it is never written again and never gates anything.
    #[serde(default, skip_serializing)]
    pub expires_at: Option<u64>,
    pub install: GameInstall,
    pub changes: Vec<PlannedChange>,
    pub disk_requirement: u64,
    #[serde(default)]
    pub space_budget: Option<crate::installer::space::ZipSpaceBudget>,
    pub blockers: Vec<String>,
    /// Legacy diagnostic field, empty for new automatic tasks. Cooperating
    /// locks do not detect or stop a running game or Steam updater.
    #[serde(default)]
    pub preconditions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub transaction_id: String,
    pub plan_id: String,
    pub instance_id: String,
    pub release_id: String,
    #[ts(type = "TransactionState")]
    pub state: String,
    pub error: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    /// Whether this operation temporarily snapshots existing bytes for failure
    /// recovery. Successful new installations do not retain those snapshots.
    #[serde(default)]
    pub has_replacements: bool,
    /// Dynamic publisher selection identity (`zip-selection-*` / `exe-selection-*`).
    /// `None` for signed (`managed_zip`) transactions and for journals written
    /// before the field existed (p1-host-contract §1.1/§6.3).
    #[serde(default)]
    pub selection_id: Option<String>,
    /// Dynamic publisher content identity (`exe-content-*`). `None` for ZIP and
    /// signed transactions, and for journals written before the field existed.
    #[serde(default)]
    pub content_id: Option<String>,
    /// Direct EXE extraction: an unfinished journal is reported as incomplete
    /// and never blocks a fresh install. Legacy rows omit this field.
    #[serde(default)]
    pub direct: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveSource {
    path: PathBuf,
    sha256: String,
    size: u64,
    #[serde(default)]
    managed: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    plan: InstallPlan,
    transaction: Transaction,
    // pending -> backed_up -> writing -> installed -> restoring -> restored.
    progress: Vec<String>,
    // Relative directories whose creation intent was durably recorded.
    created_dirs: Vec<String>,
    #[serde(default)]
    uninstall_requested: bool,
    #[serde(default)]
    transient_recovery: bool,
    /// New jobs opt into durable file progress overlays. Old documents keep
    /// their original recovery representation when this field is absent.
    #[serde(default)]
    progress_format: u32,
    /// New task plans read members from this content-bound archive. Legacy
    /// plans omit it and keep the original stage-based recovery representation.
    #[serde(default)]
    archive_source: Option<ArchiveSource>,
}

fn invalid(code: impl Into<String>) -> io::Error {
    io::Error::other(code.into())
}
fn sql<T>(result: rusqlite::Result<T>) -> io::Result<T> {
    result.map_err(|e| invalid(format!("journal_error:{e}")))
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn valid_id(id: &str) -> io::Result<()> {
    if Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id) {
        Ok(())
    } else {
        Err(invalid("invalid_transaction_id"))
    }
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn linked(m: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        m.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        m.file_type().is_symlink()
    }
}

/// Check all existing ancestors, including ancestors outside the Steam library.
fn safe_path(path: &Path) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(invalid("absolute_path_required"));
    }
    let mut current = PathBuf::new();
    let mut missing = false;
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(invalid("unsafe_path_component"));
        }
        current.push(component);
        // A bare Windows prefix is not a filesystem object.
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        if missing {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(m) if linked(&m) => return Err(invalid("linked_path")),
            Ok(_) => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => missing = true,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

// Resolve the existing ancestor first so Windows verbatim prefixes/casing cannot
// defeat overlap checks for a state directory that has not been created yet.
fn canonical_future(path: &Path) -> io::Result<PathBuf> {
    safe_path(path)?;
    let mut ancestor = path.to_owned();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(
            ancestor
                .file_name()
                .ok_or_else(|| invalid("invalid_state_path"))?
                .to_owned(),
        );
        if !ancestor.pop() {
            return Err(invalid("invalid_state_path"));
        }
    }
    let mut resolved = fs::canonicalize(ancestor)?;
    for part in suffix.into_iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}

fn relative(path: &str) -> io::Result<PathBuf> {
    let result = catalog::relative_path(path)?;
    if path
        .split('/')
        .any(|s| s.to_lowercase().starts_with(".patchhub-"))
    {
        return Err(invalid("reserved_installer_path"));
    }
    Ok(result)
}

fn target(root: &Path, relative_name: &str) -> io::Result<PathBuf> {
    let rel = relative(relative_name)?;
    let mut current = root.to_owned();
    safe_path(&current)?;
    for part in rel.components() {
        if current.is_dir() {
            let expected = part.as_os_str().to_string_lossy();
            for entry in fs::read_dir(&current)? {
                let name = entry?.file_name();
                if name.to_string_lossy().to_lowercase() == expected.to_lowercase()
                    && name != part.as_os_str()
                {
                    return Err(invalid("filesystem_case_collision"));
                }
            }
        }
        current.push(part);
        safe_path(&current)?;
    }
    Ok(current)
}

fn regular_file(path: &Path) -> io::Result<File> {
    safe_path(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Linux O_NOFOLLOW prevents the final component becoming a link between check and open.
        options.custom_flags(0x20000);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Open the reparse point itself, disallow other writers while hashing/copying.
        options.custom_flags(0x0020_0000).share_mode(1);
    }
    let file = options.open(path)?;
    let m = file.metadata()?;
    if !m.is_file() || linked(&m) {
        return Err(invalid("not_regular_file"));
    }
    ensure_single_link(&file, &m)?;
    Ok(file)
}

fn ensure_single_link(file: &File, m: &fs::Metadata) -> io::Result<()> {
    #[cfg(unix)]
    let _ = file;
    #[cfg(windows)]
    let _ = m;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if m.nlink() != 1 {
            return Err(invalid("hard_link_rejected"));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
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
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetFileInformationByHandle(
                handle: *mut std::ffi::c_void,
                info: *mut FileInformation,
            ) -> i32;
        }
        let mut info: FileInformation = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if info.links != 1 {
            return Err(invalid("hard_link_rejected"));
        }
    }
    Ok(())
}

fn digest_reader(reader: &mut impl Read, limit: u64) -> io::Result<(String, u64)> {
    let mut hash = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        operation::check()?;
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or_else(|| invalid("size_limit"))?;
        if total > limit {
            return Err(invalid("size_limit"));
        }
        hash.update(&buffer[..count]);
    }
    Ok((format!("{:x}", hash.finalize()), total))
}

#[cfg(test)]
fn digest_measured(reader: &mut impl Read, limit: u64) -> io::Result<(String, u64)> {
    struct Measured<'a, R>(&'a mut R);
    impl<R: Read> Read for Measured<'_, R> {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let count = self.0.read(bytes)?;
            operation::advance(count as u64);
            Ok(count)
        }
    }
    digest_reader(&mut Measured(reader), limit)
}

fn hash_at(path: &Path) -> io::Result<Option<String>> {
    match regular_file(path) {
        Ok(mut f) => {
            let length = f.metadata()?.len();
            let (hash, actual) = digest_reader(&mut f, length)?;
            if actual != length {
                return Err(invalid("file_size_changed"));
            }
            Ok(Some(hash))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Compare an interrupted copy with an authenticated source without buffering
/// either file in memory. A complete copy is handled by its destination hash;
/// this permits only a *strict* prefix of the expected source image.
fn is_verified_prefix(temp: &Path, source: &Path, expected: &str) -> io::Result<bool> {
    let mut input = regular_file(source)?;
    let expected_len = input.metadata()?.len();
    let (actual, source_len) = digest_reader(&mut input, expected_len)?;
    if source_len != expected_len {
        return Err(invalid("temporary_source_changed"));
    }
    if actual != expected {
        return Err(invalid("temporary_source_changed"));
    }
    let mut partial = regular_file(temp)?;
    let partial_len = partial.metadata()?.len();
    if partial_len >= source_len {
        return Ok(false);
    }
    input.seek(SeekFrom::Start(0))?;
    let mut remaining = partial_len;
    let mut from_source = [0_u8; 65536];
    let mut from_partial = [0_u8; 65536];
    while remaining > 0 {
        let count = remaining.min(from_source.len() as u64) as usize;
        input.read_exact(&mut from_source[..count])?;
        partial.read_exact(&mut from_partial[..count])?;
        if from_source[..count] != from_partial[..count] {
            return Ok(false);
        }
        remaining -= count as u64;
    }
    // Detect an append while reading, including an empty prefix that grew.
    Ok(partial.read(&mut from_partial[..1])? == 0)
}

fn sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    sync_dir(path)
}

/// Open a direct-extraction target for overwrite, or create it after making its
/// parents. Existing permissions are preserved by opening the existing inode;
/// links and non-regular files are rejected before any truncation.
pub(crate) fn open_direct_target(path: &Path) -> io::Result<File> {
    safe_path(path)?;
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() && !linked(&m) => {
            let mut opts = OpenOptions::new();
            opts.write(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                opts.custom_flags(0x0020_0000);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.custom_flags(0x2_0000);
            }
            let file = opts.open(path)?;
            if linked(&fs::symlink_metadata(path)?) {
                return Err(invalid("target_not_file"));
            }
            let m = file.metadata()?;
            if !m.is_file() || linked(&m) {
                return Err(invalid("target_not_file"));
            }
            // Reject extra links before truncating: a hard-linked game file is
            // shared with other paths and must not be emptied by a failed run.
            ensure_single_link(&file, &m)?;
            file.set_len(0)?;
            Ok(file)
        }
        Ok(_) => Err(invalid("target_not_file")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .ok_or_else(|| invalid("unsafe_path_component"))?;
            safe_path(parent)?;
            let mut missing = Vec::new();
            let mut current = parent;
            while !current.exists() {
                missing.push(current.to_path_buf());
                current = current
                    .parent()
                    .ok_or_else(|| invalid("unsafe_path_component"))?;
            }
            for dir in missing.iter().rev() {
                fs::create_dir(dir)?;
            }
            if let Some(top) = missing.last().and_then(|p| p.parent()) {
                sync_dir(top)?;
            }
            let mut opts = OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                opts.custom_flags(0x0020_0000);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.custom_flags(0x2_0000);
            }
            Ok(opts.open(path)?)
        }
        Err(e) => Err(e),
    }
}

/// Legacy/default transactional root for a state dir: the state dir itself.
///
/// P1's `installer/rust-v1/` schema roots transactions under
/// [`crate::installer::InstallerPaths::root`] instead; callers that opt in pass
/// that root to [`Store::open_at`].
pub fn transaction_root(state_dir: &Path) -> PathBuf {
    state_dir.to_path_buf()
}

pub(crate) struct Store {
    root: PathBuf,
    conn: Connection,
    has_progress: bool,
    _lock: Option<File>,
}
impl Store {
    /// Opens the transactional store with an explicit root, letting callers
    /// point `journal.lock` / `journal.sqlite` / `jobs/` at the P1 installer
    /// directory (`<state_dir>/installer/rust-v1/`).
    pub(crate) fn open_at(root: &Path) -> io::Result<Self> {
        let root = std::path::absolute(root)?;
        safe_path(&root)?;
        fs::create_dir_all(&root)?;
        safe_path(&root)?;
        let root = fs::canonicalize(root)?;
        for name in [
            "journal.lock",
            "journal.sqlite",
            "journal.sqlite-wal",
            "journal.sqlite-shm",
            "journal.sqlite-journal",
            "jobs",
        ] {
            let path = root.join(name);
            safe_path(&path)?;
            if name != "jobs" && path.exists() {
                match regular_file(&path) {
                    Ok(file) => drop(file),
                    Err(error) if name == "journal.lock" && error.raw_os_error() == Some(32) => {
                        return Err(invalid("installer_busy"))
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("journal.lock"))?;
        lock.try_lock_exclusive()
            .map_err(|_| invalid("installer_busy"))?;
        // A single cooperating writer and DELETE journaling avoid unvalidated WAL files.
        let conn = sql(Connection::open(root.join("journal.sqlite")))?;
        sql(conn.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY NOT NULL, document TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS job_progress(job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE, item INTEGER NOT NULL, state TEXT NOT NULL, updated_at INTEGER NOT NULL, PRIMARY KEY(job_id,item));
            CREATE TABLE IF NOT EXISTS job_directories(job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE, item INTEGER NOT NULL, path TEXT NOT NULL, PRIMARY KEY(job_id,item));"))?;
        fs::create_dir_all(root.join("jobs"))?;
        Ok(Self {
            root,
            conn,
            has_progress: true,
            _lock: Some(lock),
        })
    }

    /// Opens a **read-only** snapshot of the journal without taking the
    /// cross-process write lock (p0-contracts §4.1). Used by `scan` /
    /// `list_transactions` so a cooperating writer is not reported as busy.
    pub(crate) fn open_ro(root: &Path) -> io::Result<Self> {
        let root = std::path::absolute(root)?;
        safe_path(&root)?;
        let database = root.join("journal.sqlite");
        if !database.exists() {
            return Err(invalid("installer_state_missing"));
        }
        let root = fs::canonicalize(&root)?;
        let database = root.join("journal.sqlite");
        let conn =
            Connection::open_with_flags(&database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|e| invalid(format!("journal_error:{e}")))?;
        // An `init_atomic` bootstrap leaves an empty journal until the first
        // writer creates the schema; an absent `jobs` table means "no
        // transactions" for read-only callers.
        let has_jobs: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='jobs')",
                [],
                |row| row.get(0),
            )
            .map_err(|e| invalid(format!("journal_error:{e}")))?;
        if !has_jobs {
            return Err(invalid("installer_state_missing"));
        }
        let has_progress: bool = sql(conn.query_row(
            "SELECT count(*)=2 FROM sqlite_master WHERE type='table' AND name IN ('job_progress','job_directories')",
            [], |row| row.get(0),
        ))?;
        Ok(Self {
            root,
            conn,
            has_progress,
            _lock: None,
        })
    }
    fn directory(&self, id: &str) -> io::Result<PathBuf> {
        valid_id(id)?;
        let path = self.root.join("jobs").join(id);
        safe_path(&path)?;
        Ok(path)
    }
    fn item(&self, id: &str, index: usize, kind: &str) -> io::Result<PathBuf> {
        let path = self.directory(id)?.join(format!("{index}.{kind}"));
        safe_path(&path)?;
        Ok(path)
    }
    fn save(&self, job: &mut Job) -> io::Result<()> {
        job.transaction.updated_at = now();
        let document = serde_json::to_string(job).map_err(|e| invalid(e.to_string()))?;
        if document.len() > MAX_PLAN_METADATA_BYTES {
            return Err(invalid("plan_metadata_limit"));
        }
        sql(self.conn.execute_batch("SAVEPOINT save_job"))?;
        let result = (|| {
            sql(self.conn.execute("INSERT INTO jobs(id,document) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET document=excluded.document", params![job.plan.plan_id, document]))?;
            sql(self.conn.execute(
                "DELETE FROM job_progress WHERE job_id=?1",
                [&job.plan.plan_id],
            ))?;
            sql(self.conn.execute(
                "DELETE FROM job_directories WHERE job_id=?1",
                [&job.plan.plan_id],
            ))?;
            sql(self.conn.execute_batch("RELEASE save_job"))
        })();
        if result.is_err() {
            let _ = self
                .conn
                .execute_batch("ROLLBACK TO save_job; RELEASE save_job");
        }
        result
    }
    /// A writing/restoring intent must reach FULL-synchronous storage before
    /// live mutation; only the changed item is serialized between phases.
    fn save_progress(&self, job: &mut Job, index: usize) -> io::Result<()> {
        if job.progress_format == 0 {
            return self.save(job);
        }
        job.transaction.updated_at = now();
        sql(self.conn.execute(
            "INSERT INTO job_progress(job_id,item,state,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(job_id,item) DO UPDATE SET state=excluded.state,updated_at=excluded.updated_at",
            params![job.plan.plan_id, index as i64, job.progress[index], job.transaction.updated_at as i64],
        ))?;
        Ok(())
    }
    #[cfg(test)]
    fn save_directory(&self, job: &mut Job) -> io::Result<()> {
        if job.progress_format == 0 {
            return self.save(job);
        }
        let index = job.created_dirs.len() - 1;
        sql(self.conn.execute(
            "INSERT INTO job_directories(job_id,item,path) VALUES(?1,?2,?3)",
            params![job.plan.plan_id, index as i64, job.created_dirs[index]],
        ))?;
        Ok(())
    }
    fn load(&self, id: &str) -> io::Result<Job> {
        valid_id(id)?;
        // Base and overlay are read in one SQL snapshot. Old stores need no
        // migration for read-only scans; their documents default to format 0.
        let query = if self.has_progress {
            "SELECT document,
            CASE WHEN coalesce((SELECT sum(length(CAST(state AS BLOB))+32) FROM job_progress WHERE job_id=?1),0)<=?2
            THEN (SELECT json_group_array(json_array(item,state)) FROM (SELECT item,state FROM job_progress WHERE job_id=?1 ORDER BY item)) ELSE NULL END,
            (SELECT max(updated_at) FROM job_progress WHERE job_id=?1),
            CASE WHEN coalesce((SELECT sum(length(CAST(path AS BLOB))+32) FROM job_directories WHERE job_id=?1),0)<=?2
            THEN (SELECT json_group_array(json_array(item,path)) FROM (SELECT item,path FROM job_directories WHERE job_id=?1 ORDER BY item)) ELSE NULL END
            FROM jobs WHERE id=?1 AND length(CAST(document AS BLOB))<=?2"
        } else {
            "SELECT document,'[]',NULL,'[]' FROM jobs WHERE id=?1 AND length(CAST(document AS BLOB))<=?2"
        };
        let (document, progress, updated, directories): (
            String,
            Option<String>,
            Option<i64>,
            Option<String>,
        ) = sql(self
            .conn
            .query_row(query, params![id, MAX_PLAN_METADATA_BYTES as i64], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            }))?;
        let mut job: Job =
            serde_json::from_str(&document).map_err(|_| invalid("journal_invalid"))?;
        if job.progress_format == 1 {
            if !self.has_progress {
                return Err(invalid("journal_progress_missing"));
            }
            let progress = progress.ok_or_else(|| invalid("plan_metadata_limit"))?;
            let deltas: Vec<(usize, String)> =
                serde_json::from_str(&progress).map_err(|_| invalid("journal_invalid"))?;
            for (index, state) in deltas {
                *job.progress
                    .get_mut(index)
                    .ok_or_else(|| invalid("journal_invalid"))? = state;
            }
            let directories = directories.ok_or_else(|| invalid("plan_metadata_limit"))?;
            let dirs: Vec<(usize, String)> = serde_json::from_str(&directories)
                .map_err(|_| invalid("journal_invalid_directory"))?;
            for (index, path) in dirs {
                if index != job.created_dirs.len() {
                    return Err(invalid("journal_invalid_directory"));
                }
                job.created_dirs.push(path);
            }
            if let Some(updated) = updated {
                let updated = u64::try_from(updated).map_err(|_| invalid("journal_invalid"))?;
                job.transaction.updated_at = job.transaction.updated_at.max(updated);
            }
        }
        validate_job(&job, id)?;
        Ok(job)
    }
    fn ids(&self) -> io::Result<Vec<String>> {
        let mut statement = sql(self.conn.prepare("SELECT id FROM jobs ORDER BY id"))?;
        let rows = sql(statement.query_map([], |row| row.get(0)))?;
        rows.map(sql).collect()
    }
}

/// Reserve the largest normal progress/state strings, every possible created
/// parent and bounded diagnostic slack before accepting a live write. Recovery
/// must not be the first phase to discover the metadata budget is exhausted.
#[cfg(test)]
fn validate_metadata_budget(job: &Job) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(job)
        .map_err(|e| invalid(e.to_string()))?
        .len();
    // Account for each durable overlay row exactly as `load` does
    // (`sum(length(CAST(state AS BLOB))+32)`): the serialized state bytes plus
    // the 32-byte per-row SQLite overhead. A pre-check that undercounted these
    // rows could accept a plan that only `plan_metadata_limit` rejects during
    // recovery, making the write unrecoverable.
    let mut overlay = 0_usize;
    for state in &job.progress {
        overlay = overlay
            .checked_add(state.len())
            .and_then(|v| v.checked_add(32))
            .ok_or_else(|| invalid("plan_metadata_limit"))?;
    }
    bytes = bytes
        .checked_add(overlay)
        .and_then(|v| v.checked_add(256 * 1024))
        .ok_or_else(|| invalid("plan_metadata_limit"))?;
    let mut parents = HashSet::new();
    for change in &job.plan.changes {
        operation::check()?;
        for (offset, _) in change.path.match_indices('/') {
            let parent = &change.path[..offset];
            if parents.insert(parent) {
                bytes = bytes
                    .checked_add(
                        serde_json::to_vec(parent)
                            .map_err(|e| invalid(e.to_string()))?
                            .len()
                            + 1,
                    )
                    .ok_or_else(|| invalid("plan_metadata_limit"))?;
                if bytes > MAX_PLAN_METADATA_BYTES {
                    return Err(invalid("plan_metadata_limit"));
                }
            }
        }
    }
    if bytes > MAX_PLAN_METADATA_BYTES {
        return Err(invalid("plan_metadata_limit"));
    }
    Ok(())
}

fn validate_job(job: &Job, id: &str) -> io::Result<()> {
    if let Some(source) = &job.archive_source {
        if !source.path.is_absolute()
            || !valid_hash(&source.sha256)
            || source.size == 0
            || job.plan.changes.iter().any(|c| c.source.is_none())
        {
            return Err(invalid("journal_invalid_archive_source"));
        }
    }
    let p = &job.plan;
    if p.plan_id != id
        || job.transaction.plan_id != id
        || job.transaction.transaction_id != id
        || p.instance_id != p.install.instance_id
        || job.transaction.instance_id != p.instance_id
        || job.transaction.release_id != p.release_id
        || job.progress_format > 1
        || p.changes.is_empty()
        || job.progress.len() != p.changes.len()
    {
        return Err(invalid("journal_invalid"));
    }
    if p.recipe_sha256.as_ref().is_some_and(|h| !valid_hash(h)) {
        return Err(invalid("journal_invalid_identity"));
    }
    if ![
        "planned",
        "backing_up",
        "applying",
        "committed",
        "superseded",
        "uninstalling",
        "rolling_back",
        "rolled_back",
        "uninstalled",
        "needs_recovery",
        "failed",
    ]
    .contains(&job.transaction.state.as_str())
        || job.progress.iter().any(|state| {
            ![
                "pending",
                "backed_up",
                "writing",
                "installed",
                "restoring",
                "restored",
            ]
            .contains(&state.as_str())
        })
    {
        return Err(invalid("journal_invalid_state"));
    }
    if ["committed", "superseded"].contains(&job.transaction.state.as_str())
        && !job.uninstall_requested
        && job.progress.iter().any(|state| state != "installed")
    {
        return Err(invalid("journal_invalid_terminal"));
    }
    if ["rolled_back", "uninstalled"].contains(&job.transaction.state.as_str())
        && !job.progress.iter().all(|state| state == "restored")
        && !(job.transaction.state == "rolled_back"
            && job.progress.iter().all(|state| state == "pending"))
    {
        return Err(invalid("journal_invalid_terminal"));
    }
    let mut names = HashSet::new();
    for change in &p.changes {
        relative(&change.path)?;
        if !names.insert(change.path.to_lowercase())
            || change
                .installed_sha256
                .as_ref()
                .is_some_and(|h| !valid_hash(h))
            || change
                .original_sha256
                .as_ref()
                .is_some_and(|h| !valid_hash(h))
        {
            return Err(invalid("journal_invalid"));
        }
    }
    let parents: HashSet<&str> = p
        .changes
        .iter()
        .flat_map(|c| c.path.match_indices('/').map(|(at, _)| &c.path[..at]))
        .collect();
    let mut seen_dirs = HashSet::new();
    for dir in &job.created_dirs {
        relative(dir)?;
        if !parents.contains(dir.as_str()) || !seen_dirs.insert(dir) {
            return Err(invalid("journal_invalid_directory"));
        }
    }
    Ok(())
}

fn validate_install(install: &GameInstall, state: &Path) -> io::Result<()> {
    safe_path(&install.library_path)?;
    safe_path(&install.install_path)?;
    let library = fs::canonicalize(&install.library_path)?;
    let root = fs::canonicalize(&install.install_path)?;
    let common = library.join("steamapps/common");
    if !root.starts_with(&common)
        || root == common
        || root.starts_with(state)
        || state.starts_with(&root)
    {
        return Err(invalid("install_or_state_path_outside_boundary"));
    }
    let manifest = library
        .join("steamapps")
        .join(format!("appmanifest_{}.acf", install.appid));
    drop(regular_file(&manifest)?);
    let scan = steam::scan(&[library]);
    if !scan.installs.iter().any(|fresh| {
        fresh.instance_id == install.instance_id
            && fresh.appid == install.appid
            && fresh.buildid == install.buildid
            && fresh.availability == "installed"
            && fresh.install_path == install.install_path
    }) {
        return Err(invalid("game_changed_or_unavailable"));
    }
    Ok(())
}

fn game_lock(install: &GameInstall) -> io::Result<File> {
    let path = install.install_path.join(".patchhub.lock");
    safe_path(&path)?;
    if path.exists() {
        drop(regular_file(&path)?);
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock_exclusive()
        .map_err(|_| invalid("game_busy"))?;
    Ok(file)
}

fn ensure_no_active(
    store: &Store,
    instance: &str,
    except: Option<&str>,
    reinstall: bool,
) -> io::Result<()> {
    for id in store.ids()? {
        let job = store.load(&id)?;
        if Some(id.as_str()) != except
            && job.plan.instance_id == instance
            && job.transaction.state == "needs_recovery"
        {
            return Err(invalid("needs_recovery"));
        }
        if Some(id.as_str()) != except
            && job.plan.instance_id == instance
            && job.transaction.direct
            && job.transaction.state == "applying"
        {
            // A direct install has no recoverable original image. An interrupted
            // run is reported as incomplete by recovery but must not block a
            // fresh install.
            continue;
        }
        if Some(id.as_str()) != except
            && job.plan.instance_id == instance
            && ![
                "planned",
                "rolled_back",
                "uninstalled",
                "superseded",
                "failed",
            ]
            .contains(&job.transaction.state.as_str())
            && !(reinstall && job.transaction.state == "committed" && !job.uninstall_requested)
        {
            return Err(invalid("managed_install_exists_or_needs_recovery"));
        }
    }
    Ok(())
}

/// Validate every ZIP member, including members not referenced by the recipe.
/// Signature, expiry, revocation and platform policy remain the trusted caller's responsibility.
#[cfg(test)]
pub(crate) fn prepare(
    install: &GameInstall,
    release: &PatchRelease,
    archive: &Path,
    state_dir: &Path,
    catalog_revision: u64,
) -> io::Result<InstallPlan> {
    prepare_with_operation(
        install,
        release,
        archive,
        state_dir,
        catalog_revision,
        InstallOperation::Install,
    )
}

#[cfg(test)]
pub(crate) fn prepare_with_operation(
    install: &GameInstall,
    release: &PatchRelease,
    archive: &Path,
    state_dir: &Path,
    catalog_revision: u64,
    operation_kind: InstallOperation,
) -> io::Result<InstallPlan> {
    prepare_with_operation_at(
        &transaction_root(state_dir),
        install,
        release,
        archive,
        catalog_revision,
        operation_kind,
    )
}

/// Explicit-root variant of [`prepare_with_operation`] for the P1 installer
/// layout (`<state_dir>/installer/rust-v1/`). The root is the transactional
/// `journal.lock` / `journal.sqlite` / `jobs/` location. Signed and legacy
/// callers do not bind a dynamic selection/content identity, so both
/// [`Transaction`] fields stay `None`.
#[cfg(test)]
pub(crate) fn prepare_with_operation_at(
    root: &Path,
    install: &GameInstall,
    release: &PatchRelease,
    archive: &Path,
    catalog_revision: u64,
    operation_kind: InstallOperation,
) -> io::Result<InstallPlan> {
    prepare_with_binding_at(
        root,
        install,
        release,
        archive,
        catalog_revision,
        operation_kind,
        None,
        None,
    )
}

/// Explicit-root prepare that additionally records the dynamic publisher
/// `selection_id`/`content_id` binding on the journaled [`Transaction`]
/// (p1-host-contract §1.1/§6.3). The binding is persisted with the plan in the
/// same journal write, so a created transaction never exposes half a binding.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_with_binding_at(
    root: &Path,
    install: &GameInstall,
    release: &PatchRelease,
    archive: &Path,
    catalog_revision: u64,
    operation_kind: InstallOperation,
    selection_id: Option<&str>,
    content_id: Option<&str>,
) -> io::Result<InstallPlan> {
    prepare_for_request_at(
        root,
        install,
        release,
        archive,
        catalog_revision,
        operation_kind,
        selection_id,
        content_id,
        None,
    )
}

/// Unified tasks reserve a request-derived plan identity in the existing journal.
/// Retained as the legacy-plan fixture for recovery tests; production ZIP and
/// EXE routes install directly instead.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_for_request_at(
    root: &Path,
    install: &GameInstall,
    release: &PatchRelease,
    archive: &Path,
    catalog_revision: u64,
    operation_kind: InstallOperation,
    selection_id: Option<&str>,
    content_id: Option<&str>,
    request_id: Option<&str>,
) -> io::Result<InstallPlan> {
    let validation_catalog = Catalog {
        schema_version: 1,
        revision: catalog_revision,
        releases: vec![release.clone()],
    };
    if selection_id.is_some() {
        validation_catalog.validate_dynamic()?;
    } else {
        validation_catalog.validate()?;
    }
    if install.appid != release.appid
        || release.support_level != SupportLevel::ManagedZip
        || (selection_id.is_none()
            && !install
                .buildid
                .as_ref()
                .is_some_and(|b| release.verified_buildids.contains(b)))
    {
        return Err(invalid("release_not_compatible"));
    }
    let recipe = release
        .recipe
        .as_ref()
        .ok_or_else(|| invalid("recipe_missing"))?;
    if serde_json::to_vec(recipe)
        .map_err(|e| invalid(e.to_string()))?
        .len()
        > MAX_PLAN_METADATA_BYTES
    {
        return Err(invalid("plan_metadata_limit"));
    }
    let mut targets = HashSet::new();
    for mapping in &recipe.files {
        relative(&mapping.destination)?;
        if !targets.insert(mapping.destination.to_lowercase()) {
            return Err(invalid("recipe_duplicate_target"));
        }
    }
    for path in &targets {
        for (offset, _) in path.match_indices('/') {
            if targets.contains(&path[..offset]) {
                return Err(invalid("recipe_file_directory_collision"));
            }
        }
    }
    let requested_state = canonical_future(&std::path::absolute(root)?)?;
    let game_root = fs::canonicalize(&install.install_path)?;
    if requested_state.starts_with(&game_root) || game_root.starts_with(&requested_state) {
        return Err(invalid("state_overlaps_game"));
    }
    let store = Store::open_at(root)?;
    validate_install(install, &store.root)?;
    ensure_no_active(
        &store,
        &install.instance_id,
        None,
        operation_kind != InstallOperation::Install,
    )?;
    let identity = recipe_identity(release)?;
    if operation_kind == InstallOperation::Reinstall {
        let mut owned = false;
        for id in store.ids()? {
            let previous = store.load(&id)?;
            if previous.plan.instance_id != install.instance_id
                || previous.transaction.state != "committed"
            {
                continue;
            }
            let same_targets = previous.plan.changes.len() == recipe.files.len()
                && previous
                    .plan
                    .changes
                    .iter()
                    .zip(&recipe.files)
                    .all(|(c, m)| {
                        c.path == m.destination
                            && c.installed_sha256.as_deref() == Some(m.installed_sha256.as_str())
                    });
            if previous.uninstall_requested
                || previous.plan.release_id != release.release_id
                || previous.plan.install.appid != install.appid
                || previous.plan.install.buildid != install.buildid
                || previous.plan.install.install_path != install.install_path
                || previous.plan.install.library_path != install.library_path
                || !same_targets
                || previous
                    .plan
                    .recipe_sha256
                    .as_ref()
                    .is_some_and(|h| h != &identity)
            {
                return Err(invalid("reinstall_content_identity_changed"));
            }
            owned = true;
        }
        if !owned {
            return Err(invalid("reinstall_unowned_or_incomplete"));
        }
        for mapping in &recipe.files {
            if hash_at(&target(&install.install_path, &mapping.destination)?)?.as_deref()
                != Some(mapping.installed_sha256.as_str())
            {
                return Err(invalid("reinstall_unowned_or_incomplete"));
            }
        }
    }
    let archive_path = std::path::absolute(archive)?;
    let mut file = regular_file(&archive_path)?;
    if release.artifact.as_ref().is_some_and(|artifact| {
        artifact.size != file.metadata().map(|m| m.len()).unwrap_or(u64::MAX)
    }) {
        return Err(invalid("artifact_size_mismatch"));
    }
    let file_len = file.metadata()?.len();
    operation::archive(file_len);
    operation::stage("validating", Some(file.metadata()?.len()), Some(1));
    operation::file(0, &archive_path.to_string_lossy());
    if digest_measured(&mut file, file_len)?.0 != recipe.artifact_sha256 {
        return Err(invalid("artifact_hash_mismatch"));
    }
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0))?;
    crate::zip_metadata::preflight(&mut file)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| invalid(format!("invalid_zip:{e}")))?;
    let mut names = HashMap::new();
    let mut prefixes = HashMap::new();
    let mut total = 0_u64;
    for i in 0..zip.len() {
        let member = zip
            .by_index(i)
            .map_err(|e| invalid(format!("zip_member:{e}")))?;
        let name = member.name().strip_suffix('/').unwrap_or(member.name());
        relative(name)?;
        let mut prefix = String::new();
        for part in name.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            if prefixes
                .insert(prefix.to_lowercase(), prefix.clone())
                .is_some_and(|old| old != prefix)
            {
                return Err(invalid("zip_duplicate_or_case_collision"));
            }
        }
        if !crate::installer::zip_recipe::name_encoding_ok(member.name_raw(), member.name()) {
            return Err(invalid("zip_name_encoding"));
        }
        let mode = member.unix_mode().unwrap_or(0) & 0o170000;
        if (member.is_dir() && mode != 0 && mode != 0o040000)
            || (!member.is_dir() && mode != 0 && mode != 0o100000)
        {
            return Err(invalid("zip_link_or_special_file"));
        }
        if let Some(extra) = member.extra_data() {
            crate::installer::zip_recipe::extra_safe(extra)?;
        }
        total = total
            .checked_add(member.size())
            .ok_or_else(|| invalid("zip_size_overflow"))?;
        if names
            .insert(name.to_lowercase(), (i, name.to_owned(), member.is_dir()))
            .is_some()
        {
            return Err(invalid("zip_duplicate_or_case_collision"));
        }
    }
    for (_, name, _) in names.values() {
        let mut parent = Path::new(name).parent();
        while let Some(p) = parent {
            if let Some((_, _, false)) = names.get(&p.to_string_lossy().to_lowercase()) {
                return Err(invalid("zip_file_directory_collision"));
            }
            parent = p.parent();
        }
    }
    // Check space before writing potentially multi-GiB staged data. Count mapped
    // output independently: multiple mappings of one member must not bypass the
    // aggregate expansion cap.
    let mut mapped_bytes = 0_u64;
    let mut original_total = 0_u64;
    let archive_backed = request_id.is_some();
    let mut file_sizes = Vec::with_capacity(recipe.files.len());
    for mapping in &recipe.files {
        let (index, name, directory) = names
            .get(&mapping.source.to_lowercase())
            .ok_or_else(|| invalid("recipe_source_missing"))?;
        if *directory || name != &mapping.source {
            return Err(invalid("recipe_source_mismatch"));
        }
        let member = zip.by_index(*index).map_err(|e| invalid(e.to_string()))?;
        mapped_bytes = mapped_bytes
            .checked_add(member.size())
            .ok_or_else(|| invalid("zip_size_overflow"))?;
        let destination = target(&install.install_path, &mapping.destination)?;
        let original_bytes = match regular_file(&destination) {
            Ok(file) => file.metadata()?.len(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error),
        };
        file_sizes.push((member.size(), original_bytes));
        original_total = original_total
            .checked_add(original_bytes)
            .ok_or_else(|| invalid("space_overflow"))?;
    }
    let space_budget = if archive_backed {
        crate::installer::space::ZipSpaceBudget::for_archive(&file_sizes)?
    } else {
        crate::installer::space::ZipSpaceBudget::for_stage(mapped_bytes, original_total)?
    };
    space_budget.inspect(&store.root, &install.install_path)?;
    let id = request_id
        .map(crate::installer::requests::request_plan_id)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let directory = store.directory(&id)?;
    fs::create_dir(&directory)?;
    let mut changes = Vec::new();
    let mut disk = 0_u64;
    let mut blockers = Vec::new();
    operation::stage("validating", Some(mapped_bytes), Some(recipe.files.len()));
    let prepared = (|| -> io::Result<()> {
        for (index, mapping) in recipe.files.iter().enumerate() {
            operation::check()?;
            operation::file(index, &mapping.destination);
            relative(&mapping.destination)?;
            let (member_index, name, is_dir) = names
                .get(&mapping.source.to_lowercase())
                .ok_or_else(|| invalid("recipe_source_missing"))?;
            if *is_dir || name != &mapping.source {
                return Err(invalid("recipe_source_mismatch"));
            }
            let path = target(&install.install_path, &mapping.destination)?;
            let actual = hash_at(&path)?;
            if operation_kind == InstallOperation::Install && actual != mapping.original_sha256 {
                blockers.push(format!("baseline_conflict:{}", mapping.destination));
            }
            let old_size = if actual.is_some() {
                fs::metadata(&path)?.len()
            } else {
                0
            };
            let mut member = zip
                .by_index(*member_index)
                .map_err(|e| invalid(e.to_string()))?;
            let declared_size = member.size();
            let copied = if archive_backed {
                let (hash, size) = digest_measured(&mut member, declared_size)?;
                if size != declared_size || hash != mapping.installed_sha256 {
                    return Err(invalid("installed_hash_or_size_mismatch"));
                }
                size
            } else {
                let stage = store.item(&id, index, "stage")?;
                let mut out = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&stage)?;
                let read_limit = declared_size
                    .checked_add(1)
                    .ok_or_else(|| invalid("size_overflow"))?;
                let copied = operation::copy(&mut member.by_ref().take(read_limit), &mut out)?;
                out.sync_all()?;
                drop(out);
                if copied != declared_size
                    || hash_at(&stage)?.as_deref() != Some(&mapping.installed_sha256)
                {
                    return Err(invalid("installed_hash_or_size_mismatch"));
                }
                copied
            };
            disk = disk
                .checked_add(
                    old_size
                        .checked_add(
                            copied
                                .checked_mul(2)
                                .ok_or_else(|| invalid("space_overflow"))?,
                        )
                        .ok_or_else(|| invalid("space_overflow"))?,
                )
                .ok_or_else(|| invalid("space_overflow"))?;
            operation::finish_file(index + 1);
            changes.push(PlannedChange {
                path: mapping.destination.clone(),
                action: if actual.is_some() { "replace" } else { "add" }.into(),
                original_sha256: actual,
                installed_sha256: Some(mapping.installed_sha256.clone()),
                size: copied,
                source: Some(mapping.source.clone()),
            });
        }
        operation::check()?;
        Ok(())
    })();
    if let Err(error) = prepared {
        // Only individually derived files in this newly created UUID directory are cleaned.
        for i in 0..recipe.files.len() {
            let _ = fs::remove_file(store.item(&id, i, "stage")?);
        }
        let _ = fs::remove_dir(&directory);
        return Err(error);
    }
    if let Err(error) = space_budget
        .after_staging()
        .inspect(&store.root, &install.install_path)
    {
        // A plan with a capacity blocker is discarded by the normal caller.
        blockers.push(error.to_string());
    }
    sync_dir(&directory)?;
    let plan = InstallPlan {
        operation: operation_kind,
        recipe_sha256: Some(identity),
        plan_id: id.clone(),
        instance_id: install.instance_id.clone(),
        release_id: release.release_id.clone(),
        catalog_revision,
        created_at: now(),
        expires_at: None,
        install: serde_json::from_value(
            serde_json::to_value(install).map_err(|e| invalid(e.to_string()))?,
        )
        .map_err(|e| invalid(e.to_string()))?,
        changes,
        disk_requirement: if archive_backed {
            space_budget.required(true)?.0
        } else {
            disk
        },
        space_budget: Some(space_budget),
        blockers,
        preconditions: Vec::new(),
    };
    let transaction = Transaction {
        transaction_id: id.clone(),
        plan_id: id,
        instance_id: plan.instance_id.clone(),
        release_id: plan.release_id.clone(),
        state: "planned".into(),
        error: None,
        created_at: now(),
        updated_at: now(),
        has_replacements: plan.changes.iter().any(|change| change.action != "add"),
        selection_id: selection_id.map(str::to_string),
        content_id: content_id.map(str::to_string),
        direct: false,
    };
    let mut job = Job {
        progress: vec!["pending".into(); plan.changes.len()],
        plan,
        transaction,
        created_dirs: Vec::new(),
        uninstall_requested: false,
        transient_recovery: true,
        progress_format: 1,
        archive_source: archive_backed.then(|| ArchiveSource {
            path: archive_path,
            sha256: recipe.artifact_sha256.clone(),
            size: file_len,
            managed: false,
        }),
    };
    operation::stage("planning", None, None);
    if let Err(error) = operation::check()
        .and_then(|()| validate_metadata_budget(&job))
        .and_then(|()| store.save(&mut job))
    {
        for index in 0..job.plan.changes.len() {
            let path = store.item(&job.plan.plan_id, index, "stage")?;
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir(&directory);
        return Err(error);
    }
    Ok(job.plan)
}

#[cfg(test)]
pub(crate) fn get_plan(plan_id: &str, state_dir: &Path) -> io::Result<InstallPlan> {
    get_plan_at(&transaction_root(state_dir), plan_id)
}

/// Explicit-root variant of [`get_plan`].
#[cfg(test)]
pub(crate) fn get_plan_at(root: &Path, plan_id: &str) -> io::Result<InstallPlan> {
    Ok(Store::open_at(root)?.load(plan_id)?.plan)
}

/// Read-only variant of [`get_plan_at`] for `scan`/`preview` projections
/// (p0-contracts §4.1). Uses the shared read-only snapshot ([`Store::open_ro`]):
/// it never takes the exclusive `journal.lock` and never creates the installer
/// root, so a cooperating writer is not reported as `installer_busy`.
pub fn get_plan_ro(root: &Path, plan_id: &str) -> io::Result<InstallPlan> {
    Ok(Store::open_ro(root)?.load(plan_id)?.plan)
}

/// Invalidate unexecuted preparation, without ever touching game contents.
#[cfg(test)]
pub(crate) fn discard_plan(plan_id: &str, state_dir: &Path) -> io::Result<Transaction> {
    discard_plan_at(&transaction_root(state_dir), plan_id)
}

/// Explicit-root variant of [`discard_plan`].
#[cfg(test)]
pub(crate) fn discard_plan_at(root: &Path, plan_id: &str) -> io::Result<Transaction> {
    discard_plan_checked_at(root, plan_id, |_| Ok(()))
}

pub(crate) fn discard_plan_checked_at(
    root: &Path,
    plan_id: &str,
    validate: impl Fn(&InstallPlan) -> io::Result<()>,
) -> io::Result<Transaction> {
    let store = Store::open_at(root)?;
    let mut job = store.load(plan_id)?;
    validate(&job.plan)?;
    let _lock = game_lock(&job.plan.install)?;
    if job.transaction.state != "planned" {
        return Err(invalid("plan_already_consumed"));
    }
    revoke(&store, plan_id)?;
    job.transaction.state = "rolled_back".into();
    job.transaction.error = Some("task_cancelled".into());
    store.save(&mut job)?;
    let cleanup = (|| {
        for i in 0..job.plan.changes.len() {
            let path = store.item(plan_id, i, "stage")?;
            safe_path(&path)?;
            match fs::remove_file(path) {
                Ok(()) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e),
            }
        }
        Ok(())
    })();
    record_cleanup_result(&store, &mut job, cleanup);
    Ok(job.transaction)
}

/// Cache cleanup must retain sources referenced by unfinished new journals.
pub(crate) fn protected_archive_sources_at(root: &Path) -> io::Result<Vec<PathBuf>> {
    let store = match Store::open_ro(root) {
        Ok(store) => store,
        Err(e)
            if e.kind() == io::ErrorKind::NotFound
                || e.to_string() == "installer_state_missing" =>
        {
            return Ok(Vec::new())
        }
        Err(e) => return Err(e),
    };
    let mut sources = Vec::new();
    for id in store.ids()? {
        let job = store.load(&id)?;
        if !["committed", "superseded", "rolled_back", "uninstalled"]
            .contains(&job.transaction.state.as_str())
        {
            if let Some(source) = job.archive_source {
                // Only non-managed sources point into the shared download cache
                // and must be retained for cache cleanup. A managed source is the
                // journal-owned plan archive (`jobs/<id>/payload.zip`) and is
                // released with its own journal, never by the cache cleaner.
                if source.managed {
                    continue;
                }
                match fs::canonicalize(&source.path) {
                    Ok(path) => sources.push(path),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e),
                }
            }
        }
    }
    Ok(sources)
}

/// Adopt an external (non-managed) local source archive into the journal-owned
/// plan-archive path (`jobs/<plan_id>/payload.zip`). The user's own file is
/// **copied**, never renamed, and the copy is re-verified for exact size and
/// full SHA-256 before the journal adopts it (design §4.3). A task plan must never keep depending on a
/// user-controlled absolute path that a later move/delete/rename can invalidate.
#[cfg(test)]
pub(crate) fn adopt_external_plan_archive_at(root: &Path, id: &str) -> io::Result<()> {
    let store = Store::open_at(root)?;
    let mut job = store.load(id)?;
    if job.transaction.state != "planned" {
        return Err(invalid("plan_already_consumed"));
    }
    let Some(source) = job.archive_source.as_mut() else {
        return Ok(());
    };
    if source.managed {
        return Ok(());
    }
    let destination = store.directory(id)?.join("payload.zip");
    if source.path == destination {
        return Ok(());
    }
    let expected_size = source.size;
    let expected_hash = source.sha256.clone();
    safe_path(&destination)?;
    // A partial copy from an interrupted attempt is never trusted; re-copy from
    // the still-verified source instead.
    match fs::remove_file(&destination) {
        Ok(()) => (),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (),
        Err(e) => return Err(e),
    }
    copy_external_archive(&source.path, &destination, expected_size, &expected_hash)?;
    source.path = destination;
    source.managed = true;
    store.save(&mut job)
}

/// Copy an external source archive into its managed location, verifying the
/// recorded size and full SHA-256 of the copy. On any failure the partial copy
/// is removed so a retry starts clean.
#[cfg(test)]
fn copy_external_archive(
    source: &Path,
    destination: &Path,
    expected_size: u64,
    expected_hash: &str,
) -> io::Result<()> {
    let fail = |error: io::Error| {
        let _ = fs::remove_file(destination);
        error
    };
    let mut input = match regular_file(source) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(fail(archive_source_missing(source)))
        }
        Err(e) => return Err(fail(e)),
    };
    if input.metadata()?.len() != expected_size {
        return Err(fail(invalid("archive_source_changed")));
    }
    let mut output = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
    {
        Ok(file) => file,
        Err(e) => return Err(fail(e)),
    };
    let read_limit = expected_size
        .checked_add(1)
        .ok_or_else(|| invalid("size_overflow"))?;
    let copied = operation::copy(&mut Read::by_ref(&mut input).take(read_limit), &mut output);
    let bytes = match copied {
        Ok(bytes) => bytes,
        Err(error) => {
            drop(output);
            return Err(fail(error));
        }
    };
    // The plan archive is internal: inherit no source permission bits, so a
    // read-only user ZIP cannot make the owned copy undeletable at cleanup.
    if let Err(error) = output.sync_all() {
        drop(output);
        return Err(fail(error));
    }
    drop(output);
    // Re-verify the copy itself: exact size and full SHA-256 of the payload.
    let mut copied_file = match regular_file(destination) {
        Ok(file) => file,
        Err(e) => return Err(fail(e)),
    };
    let actual_size = match copied_file.metadata() {
        Ok(metadata) => metadata.len(),
        Err(e) => return Err(fail(e)),
    };
    let digest = match digest_reader(&mut copied_file, expected_size) {
        Ok((hash, _)) => hash,
        Err(e) => return Err(fail(e)),
    };
    drop(copied_file);
    if bytes != expected_size || actual_size != expected_size || digest != expected_hash {
        return Err(fail(invalid("archive_source_changed")));
    }
    sync_dir(
        destination
            .parent()
            .ok_or_else(|| invalid("missing_parent"))?,
    )?;
    Ok(())
}

/// A recorded archive path that cannot be read as an owned regular file: it is
/// missing, or present but not a regular file (directory, link, ...). Distinct
/// from a present-but-changed archive (`archive_source_changed`).
fn archive_path_unusable(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(metadata) => !metadata.is_file(),
        Err(error) => error.kind() == io::ErrorKind::NotFound,
    }
}

fn archive_source_missing(path: &Path) -> io::Error {
    invalid(format!("archive_source_missing:{}", path.display()))
}

fn open_payload_archive(store: &Store, job: &Job) -> io::Result<Option<zip::ZipArchive<File>>> {
    let Some(source) = &job.archive_source else {
        return Ok(None);
    };
    // A crash after the owned artifact rename/copy but before the journal update
    // is recoverable from the deterministic job path, still checked by digest.
    let managed = store.directory(&job.plan.plan_id)?.join("payload.zip");
    let mut file = match regular_file(&source.path) {
        Ok(file) => file,
        Err(_) if archive_path_unusable(&source.path) => match regular_file(&managed) {
            Ok(file) => file,
            Err(_) => return Err(archive_source_missing(&source.path)),
        },
        Err(error) => return Err(error),
    };
    if file.metadata()?.len() != source.size
        || digest_reader(&mut file, source.size)?.0 != source.sha256
    {
        return Err(invalid("archive_source_changed"));
    }
    file.seek(SeekFrom::Start(0))?;
    crate::zip_metadata::preflight(&mut file)?;
    Ok(Some(
        zip::ZipArchive::new(file).map_err(|e| invalid(format!("invalid_zip:{e}")))?,
    ))
}

#[cfg(test)]
fn write_archive_member(
    zip: &mut zip::ZipArchive<File>,
    change: &PlannedChange,
    temp: &Path,
) -> io::Result<()> {
    let source = change
        .source
        .as_deref()
        .ok_or_else(|| invalid("recipe_source_missing"))?;
    let mut member = zip.by_name(source).map_err(|e| invalid(e.to_string()))?;
    if member.name() != source || member.size() != change.size {
        return Err(invalid("recipe_source_mismatch"));
    }
    safe_path(temp)?;
    let mut output = OpenOptions::new().write(true).create_new(true).open(temp)?;
    // Keep a failed partial for authenticated recovery, just as a crashed copy.
    let limit = change
        .size
        .checked_add(1)
        .ok_or_else(|| invalid("size_overflow"))?;
    let copied = operation::copy(&mut member.by_ref().take(limit), &mut output)?;
    output.sync_all()?;
    drop(output);
    if copied != change.size || hash_at(temp)?.as_deref() != change.installed_sha256.as_deref() {
        return Err(invalid("installed_hash_or_size_mismatch"));
    }
    sync_dir(temp.parent().ok_or_else(|| invalid("missing_parent"))?)
}

fn verified_archive_prefix(
    zip: &mut zip::ZipArchive<File>,
    change: &PlannedChange,
    temp: &Path,
) -> io::Result<bool> {
    let source = change
        .source
        .as_deref()
        .ok_or_else(|| invalid("recipe_source_missing"))?;
    let mut member = zip.by_name(source).map_err(|e| invalid(e.to_string()))?;
    if member.name() != source || member.size() != change.size {
        return Err(invalid("recipe_source_mismatch"));
    }
    let mut partial = regular_file(temp)?;
    let length = partial.metadata()?.len();
    if length >= change.size {
        return Ok(false);
    }
    let mut matched = true;
    let mut total = 0u64;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut compare = [0u8; 65536];
    loop {
        operation::check()?;
        let count = member.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        if total < length {
            let n = (length - total).min(count as u64) as usize;
            partial.read_exact(&mut compare[..n])?;
            matched &= buffer[..n] == compare[..n];
        }
        total = total
            .checked_add(count as u64)
            .ok_or_else(|| invalid("size_overflow"))?;
        if total > change.size {
            return Err(invalid("temporary_source_changed"));
        }
        hash.update(&buffer[..count]);
    }
    if total != change.size
        || format!("{:x}", hash.finalize())
            != change.installed_sha256.as_deref().unwrap_or_default()
    {
        return Err(invalid("temporary_source_changed"));
    }
    Ok(matched && partial.read(&mut compare[..1])? == 0)
}

fn copy_checked(source: &Path, destination: &Path, expected: &str) -> io::Result<()> {
    let mut input = regular_file(source)?;
    safe_path(destination)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let expected_size = input.metadata()?.len();
    let read_limit = expected_size
        .checked_add(1)
        .ok_or_else(|| invalid("size_overflow"))?;
    let copied = operation::copy(&mut Read::by_ref(&mut input).take(read_limit), &mut output);
    let bytes = match copied {
        Ok(bytes) => bytes,
        Err(error) => {
            drop(output);
            fs::remove_file(destination)?;
            return Err(error);
        }
    };
    output.set_permissions(input.metadata()?.permissions())?;
    output.sync_all()?;
    drop(output);
    if bytes != expected_size || hash_at(destination)?.as_deref() != Some(expected) {
        return Err(invalid("copy_hash_mismatch"));
    }
    sync_dir(
        destination
            .parent()
            .ok_or_else(|| invalid("missing_parent"))?,
    )?;
    Ok(())
}

#[cfg(test)]
fn make_parents(store: &Store, job: &mut Job, index: usize) -> io::Result<()> {
    let name = job.plan.changes[index].path.clone();
    let mut rel = String::new();
    let parts: Vec<_> = name.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        if !rel.is_empty() {
            rel.push('/');
        }
        rel.push_str(part);
        let path = target(&job.plan.install.install_path, &rel)?;
        if !path.exists() {
            job.created_dirs.push(rel.clone());
            store.save_directory(job)?;
            fs::create_dir(&path)?;
            sync_dir(path.parent().ok_or_else(|| invalid("missing_parent"))?)?;
        }
        if !path.is_dir() {
            return Err(invalid("parent_not_directory"));
        }
    }
    Ok(())
}

fn temporary(job: &Job, index: usize) -> io::Result<PathBuf> {
    let dest = target(
        &job.plan.install.install_path,
        &job.plan.changes[index].path,
    )?;
    let temp = dest
        .parent()
        .ok_or_else(|| invalid("missing_parent"))?
        .join(format!(".patchhub-{}-{index}.tmp", job.plan.plan_id));
    safe_path(&temp)?;
    Ok(temp)
}

fn verify_all(job: &Job, installed: bool) -> io::Result<()> {
    for change in &job.plan.changes {
        let actual = hash_at(&target(&job.plan.install.install_path, &change.path)?)?;
        let expected = if installed {
            change.installed_sha256.as_deref()
        } else {
            change.original_sha256.as_deref()
        };
        if actual.as_deref() != expected {
            return Err(invalid(format!("file_changed:{}", change.path)));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn apply(plan_id: &str, state_dir: &Path) -> io::Result<Transaction> {
    apply_at(&transaction_root(state_dir), plan_id)
}

/// Explicit-root variant of [`apply`].
#[cfg(test)]
pub(crate) fn apply_at(root: &Path, plan_id: &str) -> io::Result<Transaction> {
    let store = Store::open_at(root)?;
    let mut job = store.load(plan_id)?;
    validate_install(&job.plan.install, &store.root)?;
    let _game_lock = game_lock(&job.plan.install)?;
    if job.transaction.state == "committed" {
        operation::committed();
        verify_all(&job, true)?;
        return Ok(job.transaction);
    }
    if job.transaction.state != "planned" {
        return Err(invalid("plan_already_consumed"));
    }
    check_revoked(&store, plan_id)?;
    if !job.plan.blockers.is_empty() {
        return Err(invalid("plan_has_blockers"));
    }
    if job.plan.operation == InstallOperation::Reinstall {
        let mut owned = false;
        for id in store.ids()? {
            let previous = store.load(&id)?;
            if previous.plan.instance_id != job.plan.instance_id
                || previous.transaction.state != "committed"
            {
                continue;
            }
            owned = true;
            if previous.uninstall_requested
                || (previous.plan.release_id != job.plan.release_id
                    || previous.plan.install.appid != job.plan.install.appid
                    || previous.plan.install.buildid != job.plan.install.buildid
                    || previous.plan.install.install_path != job.plan.install.install_path
                    || previous.plan.install.library_path != job.plan.install.library_path
                    || previous.plan.changes.len() != job.plan.changes.len()
                    || previous
                        .plan
                        .changes
                        .iter()
                        .zip(&job.plan.changes)
                        .any(|(old, new)| {
                            old.path != new.path || old.installed_sha256 != new.installed_sha256
                        })
                    || previous
                        .plan
                        .recipe_sha256
                        .as_ref()
                        .is_some_and(|identity| Some(identity) != job.plan.recipe_sha256.as_ref()))
            {
                return Err(invalid("reinstall_content_identity_changed"));
            }
        }
        if !owned {
            return Err(invalid("reinstall_unowned_or_incomplete"));
        }
        for change in &job.plan.changes {
            if hash_at(&target(&job.plan.install.install_path, &change.path)?)?.as_deref()
                != change.installed_sha256.as_deref()
            {
                return Err(invalid("reinstall_unowned_or_incomplete"));
            }
        }
    }
    operation::stage("validating", None, Some(job.plan.changes.len()));
    operation::check()?;
    ensure_no_active(
        &store,
        &job.plan.instance_id,
        Some(plan_id),
        job.plan.operation != InstallOperation::Install,
    )?;
    verify_all(&job, false)?;
    if let Some(budget) = &job.plan.space_budget {
        budget
            .after_staging()
            .inspect(&store.root, &job.plan.install.install_path)?;
    } else if fs2::available_space(&store.root)? < job.plan.disk_requirement
        || fs2::available_space(&job.plan.install.install_path)? < job.plan.disk_requirement
    {
        return Err(invalid("insufficient_space"));
    }
    validate_metadata_budget(&job)?;
    job.transaction.state = "backing_up".into();
    store.save(&mut job)?;
    let result = apply_files(&store, &mut job);
    if let Err(error) = result {
        job.transaction.error = Some(error.to_string());
        operation::restoring();
        let recovery = restore(&store, &mut job, false);
        if let Err(recovery_error) = recovery {
            job.transaction.state = "needs_recovery".into();
            job.transaction.error = Some(format!("{error}; {recovery_error}"));
            store.save(&mut job)?;
        }
    }
    if job.transaction.state == "committed" {
        operation::committed();
    }
    cleanup_terminal(&store, &mut job)?;
    Ok(job.transaction)
}

/// One resolved direct-extraction target, already checked to be inside the game.
#[derive(Debug)]
pub(crate) struct DirectTarget {
    pub path: String,
    pub file: PathBuf,
}

/// Held while the worker streams members into the game directory.
#[derive(Debug)]
pub(crate) struct DirectPlan {
    pub plan_id: String,
    pub targets: Vec<DirectTarget>,
    pub _lock: File,
}

/// Record a direct install before any game file is changed. No backup, staging
/// image, archive reference or per-file override is created.
#[allow(clippy::too_many_arguments)]
pub(crate) fn begin_direct_at(
    root: &Path,
    install: &GameInstall,
    release_id: &str,
    selection_id: Option<&str>,
    content_id: Option<&str>,
    operation: InstallOperation,
    files: &[(String, u64)],
    recipe_sha256: Option<&str>,
) -> io::Result<DirectPlan> {
    let store = Store::open_at(root)?;
    validate_install(install, &store.root)?;
    ensure_no_active(
        &store,
        &install.instance_id,
        None,
        operation != InstallOperation::Install,
    )?;
    if files.is_empty() {
        return Err(invalid("exe_manifest_invalid"));
    }
    let mut changes = Vec::with_capacity(files.len());
    let mut targets = Vec::with_capacity(files.len());
    let mut seen = HashSet::new();
    let mut disk = 0u64;
    for (path, size) in files {
        relative(path)?;
        if !seen.insert(path.to_lowercase()) {
            return Err(invalid("journal_invalid"));
        }
        let dest = target(&install.install_path, path)?;
        let action = match fs::symlink_metadata(&dest) {
            Ok(m) if m.is_file() && !linked(&m) => "replace",
            Ok(_) => return Err(invalid("target_not_file")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => "add",
            Err(e) => return Err(e),
        };
        disk = disk
            .checked_add(*size)
            .ok_or_else(|| invalid("size_overflow"))?;
        changes.push(PlannedChange {
            path: path.clone(),
            action: action.into(),
            original_sha256: None,
            installed_sha256: None,
            size: *size,
            source: None,
        });
        targets.push(DirectTarget {
            path: path.clone(),
            file: dest,
        });
    }
    let lock = game_lock(install)?;
    let id = Uuid::new_v4().to_string();
    let directory = store.directory(&id)?;
    fs::create_dir(&directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    }
    if let Some(parent) = directory.parent() {
        sync_dir(parent)?;
    }
    let plan = InstallPlan {
        operation,
        recipe_sha256: recipe_sha256.map(str::to_owned),
        plan_id: id.clone(),
        instance_id: install.instance_id.clone(),
        release_id: release_id.to_owned(),
        catalog_revision: 0,
        created_at: now(),
        expires_at: None,
        install: serde_json::from_value(
            serde_json::to_value(install).map_err(|e| invalid(e.to_string()))?,
        )
        .map_err(|e| invalid(e.to_string()))?,
        changes,
        disk_requirement: disk,
        space_budget: None,
        blockers: Vec::new(),
        preconditions: Vec::new(),
    };
    let transaction = Transaction {
        transaction_id: id.clone(),
        plan_id: id.clone(),
        instance_id: plan.instance_id.clone(),
        release_id: plan.release_id.clone(),
        state: "applying".into(),
        error: None,
        created_at: now(),
        updated_at: now(),
        has_replacements: plan.changes.iter().any(|change| change.action != "add"),
        selection_id: selection_id.map(str::to_string),
        content_id: content_id.map(str::to_string),
        direct: true,
    };
    let mut job = Job {
        progress: vec!["pending".into(); plan.changes.len()],
        plan,
        transaction,
        created_dirs: Vec::new(),
        uninstall_requested: false,
        transient_recovery: false,
        progress_format: 1,
        archive_source: None,
    };
    store.save(&mut job)?;
    Ok(DirectPlan {
        plan_id: id,
        targets,
        _lock: lock,
    })
}

/// Persist the digests observed while streaming and commit atomically.
pub(crate) fn commit_direct_at(
    root: &Path,
    plan_id: &str,
    installed: &[(String, String)],
) -> io::Result<Transaction> {
    let store = Store::open_at(root)?;
    let mut job = store.load(plan_id)?;
    if job.transaction.state == "committed" {
        return Ok(job.transaction);
    }
    if job.transaction.state != "applying" {
        return Err(invalid("plan_already_consumed"));
    }
    let observed: std::collections::HashMap<&str, &str> = installed
        .iter()
        .map(|(path, sha)| (path.as_str(), sha.as_str()))
        .collect();
    if observed.len() != job.plan.changes.len() {
        return Err(invalid("journal_invalid"));
    }
    for change in &mut job.plan.changes {
        let sha = observed
            .get(change.path.as_str())
            .ok_or_else(|| invalid("journal_invalid"))?;
        if !valid_hash(sha) {
            return Err(invalid("journal_invalid"));
        }
        change.installed_sha256 = Some((*sha).to_owned());
    }
    job.progress = vec!["installed".into(); job.plan.changes.len()];
    job.transaction.state = "committed".into();
    job.transaction.error = None;
    job.transaction.updated_at = now();
    sql(store.conn.execute_batch("BEGIN IMMEDIATE"))?;
    let outcome = (|| {
        for id in store.ids()? {
            if id == job.plan.plan_id {
                continue;
            }
            let mut previous = store.load(&id)?;
            if previous.plan.instance_id == job.plan.instance_id
                && previous.transaction.state == "committed"
            {
                previous.transaction.state = "superseded".into();
                store.save(&mut previous)?;
            }
        }
        store.save(&mut job)
    })();
    if let Err(error) = outcome {
        let _ = store.conn.execute_batch("ROLLBACK");
        return Err(error);
    }
    if let Err(error) = sql(store.conn.execute_batch("COMMIT")) {
        let _ = store.conn.execute_batch("ROLLBACK");
        return Err(error);
    }
    operation::committed();
    Ok(job.transaction)
}

/// Record an interrupted or failed direct install. Written game files stay as
/// they are; there is no restore image and no recovery requirement.
pub(crate) fn fail_direct_at(root: &Path, plan_id: &str, error: &str) -> io::Result<Transaction> {
    let store = Store::open_at(root)?;
    let mut job = store.load(plan_id)?;
    if job.transaction.state == "committed" {
        return Ok(job.transaction);
    }
    job.transaction.state = "failed".into();
    job.transaction.error = Some(error.to_owned());
    job.transaction.updated_at = now();
    store.save(&mut job)?;
    Ok(job.transaction)
}

#[cfg(test)]
fn apply_files(store: &Store, job: &mut Job) -> io::Result<()> {
    let mut backup_bytes = 0u64;
    for change in &job.plan.changes {
        if change.original_sha256.is_some() {
            backup_bytes = backup_bytes
                .checked_add(
                    regular_file(&target(&job.plan.install.install_path, &change.path)?)?
                        .metadata()?
                        .len(),
                )
                .ok_or_else(|| invalid("space_overflow"))?;
        }
    }
    let mut archive = open_payload_archive(store, job)?;
    operation::stage(
        "backing_up",
        Some(backup_bytes),
        Some(job.plan.changes.len()),
    );
    for i in 0..job.plan.changes.len() {
        operation::check()?;
        let change = &job.plan.changes[i];
        operation::file(i, &change.path);
        if archive.is_none() {
            let stage = store.item(&job.plan.plan_id, i, "stage")?;
            if hash_at(&stage)?.as_deref() != change.installed_sha256.as_deref() {
                return Err(invalid("staged_file_changed"));
            }
        }
        if let Some(hash) = &change.original_sha256 {
            copy_checked(
                &target(&job.plan.install.install_path, &change.path)?,
                &store.item(&job.plan.plan_id, i, "backup")?,
                hash,
            )?;
        }
        job.progress[i] = "backed_up".into();
        store.save_progress(job, i)?;
        operation::finish_file(i + 1);
    }
    operation::stage(
        "writing",
        Some(job.plan.changes.iter().map(|c| c.size).sum()),
        Some(job.plan.changes.len()),
    );
    job.transaction.state = "applying".into();
    store.save(job)?;
    for i in 0..job.plan.changes.len() {
        operation::check()?;
        check_revoked(store, &job.plan.plan_id)?;
        operation::file(i, &job.plan.changes[i].path);
        validate_install(&job.plan.install, &store.root)?;
        make_parents(store, job, i)?;
        job.progress[i] = "writing".into();
        store.save_progress(job, i)?;
        let change = &job.plan.changes[i];
        let dest = target(&job.plan.install.install_path, &change.path)?;
        let temp = temporary(job, i)?;
        if let Some(archive) = archive.as_mut() {
            write_archive_member(archive, change, &temp)?;
        } else {
            copy_checked(
                &store.item(&job.plan.plan_id, i, "stage")?,
                &temp,
                change
                    .installed_sha256
                    .as_deref()
                    .ok_or_else(|| invalid("journal_invalid"))?,
            )?;
        }
        if hash_at(&dest)? != change.original_sha256 {
            return Err(invalid(format!("file_changed:{}", change.path)));
        }
        if change.original_sha256.is_some() {
            // Preserve the game's executable/permission bits instead of inheriting
            // the staged archive file's default mode on SteamOS.
            fs::set_permissions(&temp, fs::metadata(&dest)?.permissions())?;
            OpenOptions::new().write(true).open(&temp)?.sync_all()?;
        }
        operation::check()?;
        safe_path(&dest)?;
        fs::rename(&temp, &dest)?;
        sync_dir(dest.parent().ok_or_else(|| invalid("missing_parent"))?)?;
        if hash_at(&dest)?.as_deref() != change.installed_sha256.as_deref() {
            return Err(invalid("post_install_hash_mismatch"));
        }
        job.progress[i] = "installed".into();
        store.save_progress(job, i)?;
        operation::finish_file(i + 1);
    }
    operation::stage("committing", None, None);
    validate_install(&job.plan.install, &store.root)?;
    verify_all(job, true)?;
    operation::check()?;
    check_revoked(store, &job.plan.plan_id)?;
    job.transaction.state = "committed".into();
    // Supersede old successful receipts and commit this receipt atomically.
    sql(store.conn.execute_batch("BEGIN IMMEDIATE"))?;
    let outcome = (|| {
        for id in store.ids()? {
            if id == job.plan.plan_id {
                continue;
            }
            let mut previous = store.load(&id)?;
            if previous.plan.instance_id == job.plan.instance_id
                && previous.transaction.state == "committed"
            {
                previous.transaction.state = "superseded".into();
                store.save(&mut previous)?;
            }
        }
        store.save(job)
    })();
    if let Err(error) = outcome {
        let _ = store.conn.execute_batch("ROLLBACK");
        job.transaction.state = "applying".into();
        return Err(error);
    }
    if let Err(error) = sql(store.conn.execute_batch("COMMIT")) {
        let _ = store.conn.execute_batch("ROLLBACK");
        job.transaction.state = "applying".into();
        return Err(error);
    }
    Ok(())
}

fn validate_temporary(store: &Store, job: &Job, index: usize) -> io::Result<bool> {
    let temp = temporary(job, index)?;
    let Some(hash) = hash_at(&temp)? else {
        return Ok(false);
    };
    let change = &job.plan.changes[index];
    if Some(&hash) == change.installed_sha256.as_ref()
        || change.original_sha256.as_ref() == Some(&hash)
    {
        return Ok(true);
    }
    if job.progress[index] == "writing" && job.archive_source.is_some() {
        let mut archive =
            open_payload_archive(store, job)?.ok_or_else(|| invalid("archive_source_missing"))?;
        if verified_archive_prefix(&mut archive, change, &temp)? {
            return Ok(true);
        }
        return Err(invalid("temporary_file_changed"));
    }
    let source = match job.progress[index].as_str() {
        "writing" => Some((
            store.item(&job.plan.plan_id, index, "stage")?,
            change
                .installed_sha256
                .as_deref()
                .ok_or_else(|| invalid("journal_invalid"))?,
        )),
        "restoring" => match change.original_sha256.as_deref() {
            Some(expected) => Some((store.item(&job.plan.plan_id, index, "backup")?, expected)),
            None => None,
        },
        _ => None,
    };
    if let Some((source, expected)) = source {
        if is_verified_prefix(&temp, &source, expected)? {
            return Ok(true);
        }
    }
    Err(invalid("temporary_file_changed"))
}

/// Restore only when every live, backup and temporary file has an approved image.
fn restore(store: &Store, job: &mut Job, uninstalling: bool) -> io::Result<()> {
    validate_install(&job.plan.install, &store.root)?;
    // Check the entire set first. Any conflict leaves all remaining live files untouched.
    for (i, change) in job.plan.changes.iter().enumerate() {
        let live = hash_at(&target(&job.plan.install.install_path, &change.path)?)?;
        if live != change.original_sha256 && live.as_deref() != change.installed_sha256.as_deref() {
            return Err(invalid(format!("recovery_conflict:{}", change.path)));
        }
        let backup = hash_at(&store.item(&job.plan.plan_id, i, "backup")?)?;
        if backup.is_some() && backup != change.original_sha256 {
            return Err(invalid("backup_missing_or_changed"));
        }
        // A same-content reinstall can have identical original/installed
        // images. Before the first write no backup is needed to preserve it.
        if live != change.original_sha256
            && live.as_deref() == change.installed_sha256.as_deref()
            && change.original_sha256.is_some()
            && backup != change.original_sha256
        {
            return Err(invalid("backup_missing_or_changed"));
        }
        if ["pending", "backed_up"].contains(&job.progress[i].as_str())
            && live != change.original_sha256
        {
            return Err(invalid("unwritten_file_changed"));
        }
        validate_temporary(store, job, i)?;
    }
    job.transaction.state = if uninstalling {
        "uninstalling"
    } else {
        "rolling_back"
    }
    .into();
    store.save(job)?;
    for i in (0..job.plan.changes.len()).rev() {
        validate_install(&job.plan.install, &store.root)?;
        let change = &job.plan.changes[i];
        let dest = target(&job.plan.install.install_path, &change.path)?;
        let temp = temporary(job, i)?;
        if validate_temporary(store, job, i)? {
            fs::remove_file(&temp)?;
        }
        let live = hash_at(&dest)?;
        if live != change.original_sha256 {
            if live.as_deref() != change.installed_sha256.as_deref() {
                return Err(invalid("recovery_conflict"));
            }
            job.progress[i] = "restoring".into();
            store.save_progress(job, i)?;
            let change = &job.plan.changes[i];
            if let Some(hash) = &change.original_sha256 {
                copy_checked(&store.item(&job.plan.plan_id, i, "backup")?, &temp, hash)?;
                if hash_at(&dest)?.as_deref() != change.installed_sha256.as_deref() {
                    return Err(invalid("recovery_conflict"));
                }
                safe_path(&dest)?;
                fs::rename(&temp, &dest)?;
            } else {
                if hash_at(&dest)?.as_deref() != change.installed_sha256.as_deref() {
                    return Err(invalid("recovery_conflict"));
                }
                fs::remove_file(&dest)?;
            }
            sync_dir(dest.parent().ok_or_else(|| invalid("missing_parent"))?)?;
        }
        job.progress[i] = "restored".into();
        store.save_progress(job, i)?;
    }
    for rel in job.created_dirs.iter().rev() {
        let path = target(&job.plan.install.install_path, rel)?;
        match fs::remove_dir(&path) {
            Ok(()) => sync_dir(path.parent().ok_or_else(|| invalid("missing_parent"))?)?,
            Err(e)
                if [io::ErrorKind::NotFound, io::ErrorKind::DirectoryNotEmpty]
                    .contains(&e.kind()) => {}
            Err(e) => return Err(e),
        }
    }
    verify_all(job, false)?;
    job.transaction.state = if uninstalling {
        "uninstalled"
    } else {
        "rolled_back"
    }
    .into();
    store.save(job)
}

#[cfg(test)]
pub(crate) fn recover(state_dir: &Path) -> io::Result<Vec<Transaction>> {
    recover_at(&transaction_root(state_dir))
}

/// Legacy unit-test entry; production recovery requires host validation or one target plan.
#[cfg(test)]
pub(crate) fn recover_at(root: &Path) -> io::Result<Vec<Transaction>> {
    recover_checked_at(root, |_| Ok(()))
}

pub(crate) fn recover_checked_at(
    root: &Path,
    validate: impl Fn(&InstallPlan) -> io::Result<()>,
) -> io::Result<Vec<Transaction>> {
    recover_selected_at(root, None, validate)
}

#[cfg(test)]
pub(crate) fn recover_plan_at(root: &Path, plan_id: &str) -> io::Result<Vec<Transaction>> {
    recover_selected_at(root, Some(plan_id), |_| Ok(()))
}

fn recover_selected_at(
    root: &Path,
    plan_id: Option<&str>,
    validate: impl Fn(&InstallPlan) -> io::Result<()>,
) -> io::Result<Vec<Transaction>> {
    let ids = match Store::open_ro(root) {
        Ok(store) => store.ids()?,
        Err(error) if error.to_string() == "installer_state_missing" => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let store = Store::open_at(root)?;
    // Select and validate the complete write set while the journal lock is held.
    // A rejected host boundary must not partially recover earlier candidates.
    let mut candidates = Vec::new();
    for id in store.ids()? {
        if plan_id.is_some_and(|selected| selected != id) {
            continue;
        }
        let job = store.load(&id)?;
        if [
            "planned",
            "rolled_back",
            "uninstalled",
            "superseded",
            "failed",
        ]
        .contains(&job.transaction.state.as_str())
            || (job.transaction.state == "committed" && !job.uninstall_requested)
        {
            continue;
        }
        if !job.transaction.direct {
            validate(&job.plan)?;
        }
        candidates.push(job);
    }
    let mut result = Vec::new();
    for mut job in candidates {
        job.transaction.has_replacements = job.plan.changes.iter().any(|c| c.action != "add");
        if [
            "planned",
            "rolled_back",
            "uninstalled",
            "superseded",
            "failed",
        ]
        .contains(&job.transaction.state.as_str())
            || (job.transaction.state == "committed" && !job.uninstall_requested)
        {
            continue;
        }
        // Direct extraction has no original-byte image. An interrupted write is
        // reported as an incomplete install and never restored or resumed.
        if job.transaction.direct {
            if !["committed", "superseded"].contains(&job.transaction.state.as_str()) {
                // A live install holds the exclusive game lock for the whole
                // write. Do not rewrite its journal from a concurrent recovery.
                match game_lock(&job.plan.install) {
                    Ok(lock) => drop(lock),
                    Err(error) if error.to_string() == "game_busy" => continue,
                    Err(_) => {}
                }
                job.transaction.state = "failed".into();
                job.transaction.error = Some("install_incomplete".into());
                job.transaction.updated_at = now();
                store.save(&mut job)?;
            }
            result.push(job.transaction);
            continue;
        }
        let uninstalling = job.uninstall_requested;
        let outcome = (|| {
            validate_install(&job.plan.install, &store.root)?;
            let _lock = game_lock(&job.plan.install)?;
            restore(&store, &mut job, uninstalling)
        })();
        if let Err(error) = outcome {
            job.transaction.state = "needs_recovery".into();
            job.transaction.error = Some(error.to_string());
            store.save(&mut job)?;
        }
        cleanup_terminal(&store, &mut job)?;
        result.push(job.transaction);
    }
    Ok(result)
}

pub fn list_transactions(state_dir: &Path) -> io::Result<Vec<Transaction>> {
    list_transactions_at(&transaction_root(state_dir))
}

/// Frozen read-only journal facts. Committed plans and transactions are read
/// through the same connection and SQL snapshot; projection never reopens it.
#[derive(Debug, Default)]
pub struct TransactionSnapshot {
    state_root: Option<PathBuf>,
    transactions: Vec<Transaction>,
    plans: HashMap<String, InstallPlan>,
    diagnostics: Vec<SnapshotDiagnostic>,
    incomplete_receipts: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotDiagnostic {
    pub code: String,
    pub path: PathBuf,
    pub message: String,
}

impl TransactionSnapshot {
    /// Uses the existing installer/rust-v1 store without initialization or writes.
    pub fn load_ro(state_dir: &Path, steam: &steam::ScanReport) -> io::Result<Self> {
        validate_projection_state(state_dir, &steam.installs)?;
        let root = crate::installer::InstallerPaths::new(state_dir)
            .root()
            .to_owned();
        let mut snapshot = Self {
            state_root: Some(std::path::absolute(state_dir)?),
            ..Self::default()
        };
        // Validate the actual database and journal paths as well as their root.
        for name in [
            "journal.sqlite",
            "journal.sqlite-wal",
            "journal.sqlite-shm",
            "journal.sqlite-journal",
        ] {
            safe_path(&root.join(name))?;
        }
        let store = match Store::open_ro(&root) {
            Ok(store) => store,
            Err(error) if error.to_string() == "installer_state_missing" => return Ok(snapshot),
            Err(error) => {
                snapshot.incomplete_receipts = true;
                snapshot.diagnostics.push(SnapshotDiagnostic {
                    code: "transaction_snapshot_unreadable".into(),
                    path: root,
                    message: error.to_string(),
                });
                return Ok(snapshot);
            }
        };
        sql(store.conn.execute_batch("BEGIN DEFERRED TRANSACTION"))?;
        let ids = match store.ids() {
            Ok(ids) => ids,
            Err(error) => {
                snapshot.incomplete_receipts = true;
                snapshot.diagnostics.push(SnapshotDiagnostic {
                    code: "transaction_snapshot_unreadable".into(),
                    path: root,
                    message: error.to_string(),
                });
                return Ok(snapshot);
            }
        };
        for id in ids {
            match store.load(&id) {
                Ok(mut job) => {
                    job.transaction.has_replacements =
                        job.plan.changes.iter().any(|change| change.action != "add");
                    if job.transaction.state == "committed" {
                        snapshot.plans.insert(job.plan.plan_id.clone(), job.plan);
                    }
                    snapshot.transactions.push(job.transaction);
                }
                Err(error) => {
                    // Preserve the current receipt identity if only its plan/overlay is
                    // corrupt. An unreadable newest plan must never select an older one.
                    #[derive(Deserialize)]
                    struct ReceiptOnly {
                        transaction: Transaction,
                    }
                    let document: rusqlite::Result<String> = store.conn.query_row(
                        "SELECT document FROM jobs WHERE id=?1 AND length(CAST(document AS BLOB))<=?2",
                        params![id, MAX_PLAN_METADATA_BYTES as i64], |row| row.get(0));
                    let mut recovered_identity = false;
                    if let Ok(document) = document {
                        if let Ok(mut receipt) = serde_json::from_str::<ReceiptOnly>(&document) {
                            if receipt.transaction.plan_id == id
                                && receipt.transaction.transaction_id == id
                            {
                                // Failed overlay validation must not discard its newer
                                // timestamp and make an older receipt win the selection.
                                let updated: rusqlite::Result<Option<i64>> = if store.has_progress {
                                    store.conn.query_row(
                                        "SELECT max(updated_at) FROM job_progress WHERE job_id=?1",
                                        [&id],
                                        |row| row.get(0),
                                    )
                                } else {
                                    Ok(None)
                                };
                                let updated = match updated {
                                    Ok(Some(value)) => u64::try_from(value).ok(),
                                    Ok(None) => Some(receipt.transaction.updated_at),
                                    Err(_) => None,
                                };
                                if let Some(updated) = updated {
                                    receipt.transaction.updated_at =
                                        receipt.transaction.updated_at.max(updated);
                                    snapshot.transactions.push(receipt.transaction);
                                    recovered_identity = true;
                                }
                            }
                        }
                    }
                    snapshot.incomplete_receipts |= !recovered_identity;
                    snapshot.diagnostics.push(SnapshotDiagnostic {
                        code: "transaction_plan_unreadable".into(),
                        path: root.join("jobs").join(id),
                        message: error.to_string(),
                    });
                }
            }
        }
        Ok(snapshot)
    }

    /// Builds owned fixed facts for callers which already hold a trusted snapshot.
    /// Projection still validates each matched plan's instance and game paths.
    pub fn from_records(transactions: Vec<Transaction>, plans: Vec<InstallPlan>) -> Self {
        Self {
            state_root: None,
            transactions,
            plans: plans
                .into_iter()
                .map(|plan| (plan.plan_id.clone(), plan))
                .collect(),
            diagnostics: Vec::new(),
            incomplete_receipts: false,
        }
    }
    pub fn receipts_complete(&self) -> bool {
        !self.incomplete_receipts
    }
    pub fn transactions(&self) -> &[Transaction] {
        &self.transactions
    }
    pub fn plan(&self, plan_id: &str) -> Option<&InstallPlan> {
        self.plans.get(plan_id)
    }
    pub fn diagnostics(&self) -> &[SnapshotDiagnostic] {
        &self.diagnostics
    }
    pub fn state_root(&self) -> Option<&Path> {
        self.state_root.as_deref()
    }
}

/// Shared direct-API state boundary for read-only snapshots and projection.
pub(crate) fn validate_projection_state(
    state_dir: &Path,
    installs: &[GameInstall],
) -> io::Result<()> {
    let state = canonical_future(&std::path::absolute(state_dir)?)?;
    for game in installs {
        let game_path = canonical_future(&std::path::absolute(&game.install_path)?)?;
        if state.starts_with(&game_path) || game_path.starts_with(&state) {
            return Err(invalid("state_overlaps_game"));
        }
    }
    Ok(())
}

/// Explicit-root variant of [`list_transactions`] using a read-only snapshot
/// that does not take the `journal.lock` exclusive lock (p0-contracts §4.1).
pub fn list_transactions_at(root: &Path) -> io::Result<Vec<Transaction>> {
    let store = match Store::open_ro(root) {
        Ok(store) => store,
        Err(error) if error.to_string() == "installer_state_missing" => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut result = Vec::new();
    for id in store.ids()? {
        let mut job = store.load(&id)?;
        job.transaction.has_replacements =
            job.plan.changes.iter().any(|change| change.action != "add");
        result.push(job.transaction);
    }
    Ok(result)
}

#[cfg(test)]
fn check_revoked(store: &Store, id: &str) -> io::Result<()> {
    let marker = store.directory(id)?.join("revoked");
    safe_path(&marker)?;
    if marker.exists() {
        return Err(invalid("plan_revoked"));
    }
    Ok(())
}

fn revoke(store: &Store, id: &str) -> io::Result<()> {
    let directory = store.directory(id)?;
    fs::create_dir_all(&directory)?;
    let marker = directory.join("revoked");
    safe_path(&marker)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(marker)?;
    file.sync_all()?;
    sync_dir(&directory)
}

/// Delete only derived temporary files for validated terminal records. Failures
/// preserve successful installation and remain visible as cleanup diagnostics.
fn cleanup_terminal(store: &Store, job: &mut Job) -> io::Result<()> {
    if !job.transient_recovery
        || job.uninstall_requested && job.transaction.state != "uninstalled"
        || !["committed", "superseded", "rolled_back", "uninstalled"]
            .contains(&job.transaction.state.as_str())
    {
        return Ok(());
    }
    let cleanup = (|| {
        for index in 0..job.plan.changes.len() {
            for kind in ["stage", "backup"] {
                let path = store.item(&job.plan.plan_id, index, kind)?;
                match regular_file(&path) {
                    Ok(file) => {
                        drop(file);
                        fs::remove_file(path)?;
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                    Err(error) => return Err(error),
                }
            }
        }
        let payload = store.directory(&job.plan.plan_id)?.join("payload.zip");
        match regular_file(&payload) {
            Ok(file) => {
                drop(file);
                fs::remove_file(payload)?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
        sync_dir(&store.directory(&job.plan.plan_id)?)
    })();
    record_cleanup_result(store, job, cleanup);
    Ok(())
}

fn record_cleanup_result(store: &Store, job: &mut Job, cleanup: io::Result<()>) {
    let previous_error = job.transaction.error.as_deref().and_then(|error| {
        if error.starts_with("cleanup_pending:") {
            None
        } else {
            Some(
                error
                    .split("; cleanup_pending:")
                    .next()
                    .unwrap_or(error)
                    .to_owned(),
            )
        }
    });
    if let Err(error) = cleanup {
        job.transaction.error = Some(format!(
            "{}cleanup_pending:{error}",
            previous_error
                .map(|error| format!("{error}; "))
                .unwrap_or_default()
        ));
        // Cleanup diagnostics must never change an already durable terminal outcome.
        let _ = store.save(job);
    } else if previous_error != job.transaction.error {
        job.transaction.error = previous_error;
        // Cleanup diagnostics must never change an already durable terminal outcome.
        let _ = store.save(job);
    }
}

#[cfg(test)]
mod transaction_binding_tests {
    use super::Transaction;

    /// Journals written before `selection_id`/`content_id` existed must still
    /// deserialize: `#[serde(default)]` fills both with `None`.
    #[test]
    fn legacy_transaction_document_loads_without_binding_fields() {
        let legacy = r#"{
            "transaction_id":"t","plan_id":"p","instance_id":"i","release_id":"r",
            "state":"committed","error":null,"created_at":1,"updated_at":2,
            "has_replacements":false
        }"#;
        let transaction: Transaction = serde_json::from_str(legacy).unwrap();
        assert_eq!(transaction.selection_id, None);
        assert_eq!(transaction.content_id, None);
    }

    /// `list_transactions` returns the struct verbatim, so both binding fields
    /// are part of its serialized output.
    #[test]
    fn transaction_binding_fields_are_serialized() {
        let transaction = Transaction {
            transaction_id: "t".into(),
            plan_id: "p".into(),
            instance_id: "i".into(),
            release_id: "r".into(),
            state: "planned".into(),
            error: None,
            created_at: 1,
            updated_at: 2,
            has_replacements: false,
            selection_id: Some("zip-selection-000000000000000000000000-42".into()),
            content_id: Some("exe-content-abc".into()),
            direct: false,
        };
        let value = serde_json::to_value(&transaction).unwrap();
        assert_eq!(
            value["selection_id"],
            "zip-selection-000000000000000000000000-42"
        );
        assert_eq!(value["content_id"], "exe-content-abc");
    }

    /// Journals written before the local plan TTL was removed still contain
    /// `plan.expires_at`. It must load read-only instead of failing as
    /// `journal_invalid` (design D01 legacy compat), and must not be written
    /// again by the new code.
    #[test]
    fn legacy_plan_document_with_expiry_loads_read_only() {
        let legacy = r#"{
            "plan": {
                "operation":"install","recipe_sha256":null,
                "plan_id":"11111111-1111-1111-1111-111111111111",
                "instance_id":"42|/lib|/lib/game","release_id":"rel",
                "catalog_revision":1,"created_at":10,"expires_at":910,
                "install":{"instance_id":"42|/lib|/lib/game","appid":42,"name":"N","localized_name":null,"name_language":null,"library_path":"/lib","install_path":"/lib/game","buildid":null,"availability":"installed","kind":"game"},
                "changes":[],"disk_requirement":0,"space_budget":null,"blockers":[],"preconditions":[]
            },
            "transaction":{"transaction_id":"t","plan_id":"11111111-1111-1111-1111-111111111111","instance_id":"42|/lib|/lib/game","release_id":"rel","state":"committed","error":null,"created_at":1,"updated_at":2,"has_replacements":false},
            "progress":[],"created_dirs":[],"uninstall_requested":false,"transient_recovery":false
        }"#;
        let job: super::Job = serde_json::from_str(legacy).unwrap();
        assert_eq!(job.plan.expires_at, Some(910));
        let value = serde_json::to_value(&job.plan).unwrap();
        assert!(value.get("expires_at").is_none());
    }
}
