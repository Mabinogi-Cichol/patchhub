//! Conservative local ZIP installer. Callers must authenticate the catalog before prepare/apply.
//! SQLite journals intent before mutation; recovery is hash guarded, not globally atomic.
use crate::operation;
use crate::{
    catalog::{self, Catalog, PatchRelease, SupportLevel},
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

pub const MAX_ARCHIVE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub const MAX_EXPANDED_BYTES: u64 = 16 * 1024 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 8192;
pub const PLAN_TTL_SECONDS: u64 = 900;

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum InstallOperation {
    #[default]
    Install,
    Reinstall,
}

/// Content identity excludes pre-operation hashes, which are snapshots during reinstall.
pub fn recipe_identity(release: &PatchRelease) -> io::Result<String> {
    let recipe = release
        .recipe
        .as_ref()
        .ok_or_else(|| invalid("recipe_missing"))?;
    let mappings: Vec<_> = recipe
        .files
        .iter()
        .map(|m| (&m.source, &m.destination, &m.installed_sha256))
        .collect();
    let bytes = serde_json::to_vec(&(
        release.appid,
        &release.release_id,
        &release.language,
        &recipe.artifact_sha256,
        mappings,
    ))
    .map_err(|e| invalid(e.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlannedChange {
    pub path: String,
    pub action: String,
    pub original_sha256: Option<String>,
    pub installed_sha256: String,
    pub size: u64,
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
    pub expires_at: u64,
    pub install: GameInstall,
    pub changes: Vec<PlannedChange>,
    pub disk_requirement: u64,
    pub blockers: Vec<String>,
    /// Legacy diagnostic field, empty for new automatic tasks. Cooperating
    /// locks do not detect or stop a running game or Steam updater.
    #[serde(default)]
    pub preconditions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub transaction_id: String,
    pub plan_id: String,
    pub instance_id: String,
    pub release_id: String,
    pub state: String,
    pub error: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    /// Whether this operation temporarily snapshots existing bytes for failure
    /// recovery. Successful new installations do not retain those snapshots.
    #[serde(default)]
    pub has_replacements: bool,
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
        .any(|s| s.to_lowercase().starts_with(".steam-patch-"))
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
    Ok(file)
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
        Ok(mut f) => Ok(Some(digest_reader(&mut f, MAX_FILE_BYTES)?.0)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Compare an interrupted copy with an authenticated source without buffering
/// either file in memory. A complete copy is handled by its destination hash;
/// this permits only a *strict* prefix of the expected source image.
fn is_verified_prefix(temp: &Path, source: &Path, expected: &str) -> io::Result<bool> {
    let mut input = regular_file(source)?;
    let (actual, source_len) = digest_reader(&mut input, MAX_FILE_BYTES)?;
    if actual != expected {
        return Err(invalid("temporary_source_changed"));
    }
    let mut partial = regular_file(temp)?;
    let partial_len = partial.metadata()?.len();
    if partial_len >= source_len || partial_len > MAX_FILE_BYTES {
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

/// Legacy/default transactional root for a state dir: the state dir itself.
///
/// P1's `installer/rust-v1/` schema roots transactions under
/// [`crate::installer::InstallerPaths::root`] instead; callers that opt in pass
/// that root to [`Store::open_at`].
pub fn transaction_root(state_dir: &Path) -> PathBuf {
    state_dir.to_path_buf()
}

pub struct Store {
    root: PathBuf,
    conn: Connection,
    _lock: File,
}
impl Store {
    /// Opens the legacy layout, unchanged for existing callers.
    fn open(state_dir: &Path) -> io::Result<Self> {
        Self::open_at(&transaction_root(state_dir))
    }

    /// Opens the transactional store with an explicit root, letting callers
    /// point `journal.lock` / `journal.sqlite` / `jobs/` at the P1 installer
    /// directory (`<state_dir>/installer/rust-v1/`).
    pub fn open_at(root: &Path) -> io::Result<Self> {
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
        sql(conn.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY NOT NULL, document TEXT NOT NULL);"))?;
        fs::create_dir_all(root.join("jobs"))?;
        Ok(Self {
            root,
            conn,
            _lock: lock,
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
        sql(self.conn.execute("INSERT INTO jobs(id,document) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET document=excluded.document", params![job.plan.plan_id, document]))?;
        Ok(())
    }
    fn load(&self, id: &str) -> io::Result<Job> {
        valid_id(id)?;
        let document: String = sql(self.conn.query_row(
            "SELECT document FROM jobs WHERE id=?1 AND length(document)<8388608",
            [id],
            |row| row.get(0),
        ))?;
        let job: Job = serde_json::from_str(&document).map_err(|_| invalid("journal_invalid"))?;
        validate_job(&job, id)?;
        Ok(job)
    }
    fn ids(&self) -> io::Result<Vec<String>> {
        let mut statement = sql(self.conn.prepare("SELECT id FROM jobs ORDER BY id"))?;
        let rows = sql(statement.query_map([], |row| row.get(0)))?;
        rows.map(sql).collect()
    }
}

fn validate_job(job: &Job, id: &str) -> io::Result<()> {
    let p = &job.plan;
    if p.plan_id != id
        || job.transaction.plan_id != id
        || job.transaction.transaction_id != id
        || p.instance_id != p.install.instance_id
        || job.transaction.instance_id != p.instance_id
        || job.transaction.release_id != p.release_id
        || p.changes.is_empty()
        || p.changes.len() > MAX_ENTRIES
        || job.progress.len() != p.changes.len()
        || p.expires_at < p.created_at
        || p.expires_at - p.created_at > PLAN_TTL_SECONDS
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
            || !valid_hash(&change.installed_sha256)
            || change
                .original_sha256
                .as_ref()
                .is_some_and(|h| !valid_hash(h))
            || change.size > MAX_FILE_BYTES
        {
            return Err(invalid("journal_invalid"));
        }
    }
    for dir in &job.created_dirs {
        relative(dir)?;
        if !p
            .changes
            .iter()
            .any(|c| c.path.starts_with(&format!("{dir}/")))
        {
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
    let path = install.install_path.join(".steam-patch-manager.lock");
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
            && !["planned", "rolled_back", "uninstalled", "superseded"]
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
pub fn prepare(
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

pub fn prepare_with_operation(
    install: &GameInstall,
    release: &PatchRelease,
    archive: &Path,
    state_dir: &Path,
    catalog_revision: u64,
    operation_kind: InstallOperation,
) -> io::Result<InstallPlan> {
    Catalog {
        schema_version: 1,
        revision: catalog_revision,
        releases: vec![release.clone()],
    }
    .validate()?;
    if install.appid != release.appid
        || release.support_level != SupportLevel::ManagedZip
        || !install
            .buildid
            .as_ref()
            .is_some_and(|b| release.verified_buildids.contains(b))
    {
        return Err(invalid("release_not_compatible"));
    }
    let recipe = release
        .recipe
        .as_ref()
        .ok_or_else(|| invalid("recipe_missing"))?;
    if recipe.files.len() > MAX_ENTRIES {
        return Err(invalid("recipe_entry_limit"));
    }
    let requested_state = canonical_future(&std::path::absolute(state_dir)?)?;
    let game_root = fs::canonicalize(&install.install_path)?;
    if requested_state.starts_with(&game_root) || game_root.starts_with(&requested_state) {
        return Err(invalid("state_overlaps_game"));
    }
    let store = Store::open(state_dir)?;
    validate_install(install, &store.root)?;
    ensure_no_active(
        &store,
        &install.instance_id,
        None,
        operation_kind == InstallOperation::Reinstall,
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
                        c.path == m.destination && c.installed_sha256 == m.installed_sha256
                    });
            if previous.uninstall_requested
                || previous.plan.release_id != release.release_id
                || previous.plan.install.appid != install.appid
                || previous.plan.install.buildid != install.buildid
                || previous.plan.install.install_path != install.install_path
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
            for mapping in &recipe.files {
                if hash_at(&target(&install.install_path, &mapping.destination)?)?.as_deref()
                    != Some(mapping.installed_sha256.as_str())
                {
                    return Err(invalid("reinstall_unowned_or_incomplete"));
                }
            }
        }
    }
    let archive_path = std::path::absolute(archive)?;
    let mut file = regular_file(&archive_path)?;
    if file.metadata()?.len() > MAX_ARCHIVE_BYTES {
        return Err(invalid("archive_too_large"));
    }
    if release.artifact.as_ref().is_some_and(|artifact| {
        artifact.size != file.metadata().map(|m| m.len()).unwrap_or(u64::MAX)
    }) {
        return Err(invalid("artifact_size_mismatch"));
    }
    operation::archive(file.metadata()?.len());
    operation::stage("validating", Some(file.metadata()?.len()), Some(1));
    operation::file(0, &archive_path.to_string_lossy());
    if digest_measured(&mut file, MAX_ARCHIVE_BYTES)?.0 != recipe.artifact_sha256 {
        return Err(invalid("artifact_hash_mismatch"));
    }
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| invalid(format!("invalid_zip:{e}")))?;
    if zip.len() > MAX_ENTRIES {
        return Err(invalid("archive_entry_limit"));
    }
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
        if member.name_raw() != member.name().as_bytes() {
            return Err(invalid("zip_name_encoding"));
        }
        let mode = member.unix_mode().unwrap_or(0) & 0o170000;
        if (member.is_dir() && mode != 0 && mode != 0o040000)
            || (!member.is_dir() && mode != 0 && mode != 0o100000)
        {
            return Err(invalid("zip_link_or_special_file"));
        }
        // Only recognize the fixed NTFS timestamp record. Never apply archive attributes.
        if let Some(extra) = member.extra_data() {
            let mut cursor = 0;
            while cursor + 4 <= extra.len() {
                let tag = u16::from_le_bytes([extra[cursor], extra[cursor + 1]]);
                let size = u16::from_le_bytes([extra[cursor + 2], extra[cursor + 3]]) as usize;
                cursor += 4;
                if [0x000d, 0x756e].contains(&tag) || cursor + size > extra.len() {
                    return Err(invalid("zip_unsafe_extra_metadata"));
                }
                if tag == 0x000a {
                    let record = &extra[cursor..cursor + size];
                    if record.len() != 32
                        || record[..4] != [0, 0, 0, 0]
                        || record[4..8] != [1, 0, 24, 0]
                    {
                        return Err(invalid("zip_unsafe_extra_metadata"));
                    }
                }
                cursor += size;
            }
            if cursor != extra.len() {
                return Err(invalid("zip_invalid_extra_metadata"));
            }
        }
        if member.size() > MAX_FILE_BYTES
            || (member.size() > 1024 * 1024
                && member.size() / member.compressed_size().max(1) > 200)
        {
            return Err(invalid("zip_expansion_limit"));
        }
        total = total
            .checked_add(member.size())
            .ok_or_else(|| invalid("zip_expansion_limit"))?;
        if total > MAX_EXPANDED_BYTES {
            return Err(invalid("zip_expansion_limit"));
        }
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
    let mut required_bytes = 0_u64;
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
            .ok_or_else(|| invalid("zip_expansion_limit"))?;
        if mapped_bytes > MAX_EXPANDED_BYTES {
            return Err(invalid("zip_expansion_limit"));
        }
        let destination = target(&install.install_path, &mapping.destination)?;
        let original_bytes = match regular_file(&destination) {
            Ok(file) => file.metadata()?.len(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error),
        };
        if original_bytes > MAX_FILE_BYTES {
            return Err(invalid("size_limit"));
        }
        required_bytes = required_bytes
            .checked_add(original_bytes + member.size() * 2)
            .ok_or_else(|| invalid("space_overflow"))?;
    }
    if fs2::available_space(&store.root)? < required_bytes
        || fs2::available_space(&install.install_path)? < required_bytes
    {
        return Err(invalid("insufficient_space"));
    }
    let id = Uuid::new_v4().to_string();
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
            for other in &recipe.files {
                if mapping
                    .destination
                    .to_lowercase()
                    .starts_with(&(other.destination.to_lowercase() + "/"))
                {
                    return Err(invalid("recipe_file_directory_collision"));
                }
            }
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
            let stage = store.item(&id, index, "stage")?;
            let mut out = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stage)?;
            let copied = operation::copy(&mut member.by_ref().take(MAX_FILE_BYTES + 1), &mut out)?;
            out.sync_all()?;
            drop(out);
            if copied > MAX_FILE_BYTES
                || copied != member.size()
                || hash_at(&stage)?.as_deref() != Some(&mapping.installed_sha256)
            {
                return Err(invalid("installed_hash_or_size_mismatch"));
            }
            disk = disk
                .checked_add(old_size + copied * 2)
                .ok_or_else(|| invalid("space_overflow"))?;
            operation::finish_file(index + 1);
            changes.push(PlannedChange {
                path: mapping.destination.clone(),
                action: if actual.is_some() { "replace" } else { "add" }.into(),
                original_sha256: actual,
                installed_sha256: mapping.installed_sha256.clone(),
                size: copied,
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
    if fs2::available_space(&store.root)? < disk
        || fs2::available_space(&install.install_path)? < disk
    {
        blockers.push("insufficient_space".into());
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
        expires_at: now() + PLAN_TTL_SECONDS,
        install: serde_json::from_value(
            serde_json::to_value(install).map_err(|e| invalid(e.to_string()))?,
        )
        .map_err(|e| invalid(e.to_string()))?,
        changes,
        disk_requirement: disk,
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
    };
    let mut job = Job {
        progress: vec!["pending".into(); plan.changes.len()],
        plan,
        transaction,
        created_dirs: Vec::new(),
        uninstall_requested: false,
        transient_recovery: true,
    };
    operation::stage("planning", None, None);
    if let Err(error) = operation::check().and_then(|()| store.save(&mut job)) {
        for index in 0..job.plan.changes.len() {
            let path = store.item(&job.plan.plan_id, index, "stage")?;
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir(&directory);
        return Err(error);
    }
    Ok(job.plan)
}

pub fn get_plan(plan_id: &str, state_dir: &Path) -> io::Result<InstallPlan> {
    Ok(Store::open(state_dir)?.load(plan_id)?.plan)
}

/// Invalidate unexecuted preparation, without ever touching game contents.
pub fn discard_plan(plan_id: &str, state_dir: &Path) -> io::Result<Transaction> {
    let store = Store::open(state_dir)?;
    let mut job = store.load(plan_id)?;
    let _lock = game_lock(&job.plan.install)?;
    if job.transaction.state != "planned" {
        return Err(invalid("plan_already_consumed"));
    }
    revoke(&store, plan_id)?;
    job.transaction.state = "rolled_back".into();
    job.transaction.error = Some("task_cancelled".into());
    store.save(&mut job)?;
    for i in 0..job.plan.changes.len() {
        let path = store.item(plan_id, i, "stage")?;
        safe_path(&path)?;
        match fs::remove_file(path) {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
    }
    Ok(job.transaction)
}

fn copy_checked(source: &Path, destination: &Path, expected: &str) -> io::Result<()> {
    let mut input = regular_file(source)?;
    safe_path(destination)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let copied = operation::copy(
        &mut Read::by_ref(&mut input).take(MAX_FILE_BYTES + 1),
        &mut output,
    );
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
    if bytes > MAX_FILE_BYTES || hash_at(destination)?.as_deref() != Some(expected) {
        return Err(invalid("copy_hash_mismatch"));
    }
    sync_dir(
        destination
            .parent()
            .ok_or_else(|| invalid("missing_parent"))?,
    )?;
    Ok(())
}

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
            store.save(job)?;
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
        .join(format!(".steam-patch-{}-{index}.tmp", job.plan.plan_id));
    safe_path(&temp)?;
    Ok(temp)
}

fn verify_all(job: &Job, installed: bool) -> io::Result<()> {
    for change in &job.plan.changes {
        let actual = hash_at(&target(&job.plan.install.install_path, &change.path)?)?;
        let expected = if installed {
            Some(change.installed_sha256.as_str())
        } else {
            change.original_sha256.as_deref()
        };
        if actual.as_deref() != expected {
            return Err(invalid(format!("file_changed:{}", change.path)));
        }
    }
    Ok(())
}

pub fn apply(plan_id: &str, state_dir: &Path) -> io::Result<Transaction> {
    let store = Store::open(state_dir)?;
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
    if now() < job.plan.created_at || now() > job.plan.expires_at {
        return Err(invalid("plan_expired"));
    }
    if !job.plan.blockers.is_empty() {
        return Err(invalid("plan_has_blockers"));
    }
    if job.plan.operation == InstallOperation::Reinstall {
        for id in store.ids()? {
            let previous = store.load(&id)?;
            if previous.plan.instance_id == job.plan.instance_id
                && previous.transaction.state == "committed"
                && (previous.plan.release_id != job.plan.release_id
                    || previous.plan.install.appid != job.plan.install.appid
                    || previous.plan.install.buildid != job.plan.install.buildid
                    || previous.plan.install.install_path != job.plan.install.install_path
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
    }
    operation::stage("validating", None, Some(job.plan.changes.len()));
    operation::check()?;
    ensure_no_active(
        &store,
        &job.plan.instance_id,
        Some(plan_id),
        job.plan.operation == InstallOperation::Reinstall,
    )?;
    verify_all(&job, false)?;
    if fs2::available_space(&store.root)? < job.plan.disk_requirement
        || fs2::available_space(&job.plan.install.install_path)? < job.plan.disk_requirement
    {
        return Err(invalid("insufficient_space"));
    }
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

fn apply_files(store: &Store, job: &mut Job) -> io::Result<()> {
    let mut backup_bytes = 0;
    for change in &job.plan.changes {
        if change.original_sha256.is_some() {
            backup_bytes += regular_file(&target(&job.plan.install.install_path, &change.path)?)?
                .metadata()?
                .len();
        }
    }
    operation::stage(
        "backing_up",
        Some(backup_bytes),
        Some(job.plan.changes.len()),
    );
    for i in 0..job.plan.changes.len() {
        operation::check()?;
        let change = &job.plan.changes[i];
        operation::file(i, &change.path);
        let stage = store.item(&job.plan.plan_id, i, "stage")?;
        if hash_at(&stage)?.as_deref() != Some(&change.installed_sha256) {
            return Err(invalid("staged_file_changed"));
        }
        if let Some(hash) = &change.original_sha256 {
            copy_checked(
                &target(&job.plan.install.install_path, &change.path)?,
                &store.item(&job.plan.plan_id, i, "backup")?,
                hash,
            )?;
        }
        job.progress[i] = "backed_up".into();
        store.save(job)?;
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
        store.save(job)?;
        let change = &job.plan.changes[i];
        let dest = target(&job.plan.install.install_path, &change.path)?;
        let temp = temporary(job, i)?;
        copy_checked(
            &store.item(&job.plan.plan_id, i, "stage")?,
            &temp,
            &change.installed_sha256,
        )?;
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
        if hash_at(&dest)?.as_deref() != Some(&change.installed_sha256) {
            return Err(invalid("post_install_hash_mismatch"));
        }
        job.progress[i] = "installed".into();
        store.save(job)?;
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
    if hash == change.installed_sha256 || change.original_sha256.as_ref() == Some(&hash) {
        return Ok(true);
    }
    let source = match job.progress[index].as_str() {
        "writing" => Some((
            store.item(&job.plan.plan_id, index, "stage")?,
            change.installed_sha256.as_str(),
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
        if live != change.original_sha256 && live.as_deref() != Some(&change.installed_sha256) {
            return Err(invalid(format!("recovery_conflict:{}", change.path)));
        }
        let backup = hash_at(&store.item(&job.plan.plan_id, i, "backup")?)?;
        if backup.is_some() && backup != change.original_sha256 {
            return Err(invalid("backup_missing_or_changed"));
        }
        // A same-content reinstall can have identical original/installed
        // images. Before the first write no backup is needed to preserve it.
        if live != change.original_sha256
            && live.as_deref() == Some(&change.installed_sha256)
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
            if live.as_deref() != Some(&change.installed_sha256) {
                return Err(invalid("recovery_conflict"));
            }
            job.progress[i] = "restoring".into();
            store.save(job)?;
            let change = &job.plan.changes[i];
            if let Some(hash) = &change.original_sha256 {
                copy_checked(&store.item(&job.plan.plan_id, i, "backup")?, &temp, hash)?;
                if hash_at(&dest)?.as_deref() != Some(&change.installed_sha256) {
                    return Err(invalid("recovery_conflict"));
                }
                safe_path(&dest)?;
                fs::rename(&temp, &dest)?;
            } else {
                if hash_at(&dest)?.as_deref() != Some(&change.installed_sha256) {
                    return Err(invalid("recovery_conflict"));
                }
                fs::remove_file(&dest)?;
            }
            sync_dir(dest.parent().ok_or_else(|| invalid("missing_parent"))?)?;
        }
        job.progress[i] = "restored".into();
        store.save(job)?;
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

pub fn recover(state_dir: &Path) -> io::Result<Vec<Transaction>> {
    let store = Store::open(state_dir)?;
    let mut result = Vec::new();
    for id in store.ids()? {
        let mut job = store.load(&id)?;
        job.transaction.has_replacements = job.plan.changes.iter().any(|c| c.action != "add");
        if ["planned", "rolled_back", "uninstalled", "superseded"]
            .contains(&job.transaction.state.as_str())
            || (job.transaction.state == "committed" && !job.uninstall_requested)
        {
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
    let store = Store::open(state_dir)?;
    let mut result = Vec::new();
    for id in store.ids()? {
        let mut job = store.load(&id)?;
        job.transaction.has_replacements =
            job.plan.changes.iter().any(|change| change.action != "add");
        result.push(job.transaction);
    }
    Ok(result)
}

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
        sync_dir(&store.directory(&job.plan.plan_id)?)
    })();
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
    Ok(())
}

/// One-time upgrade invalidates old executable plans. Subsequent calls only
/// retry safe terminal cleanup and never revoke current-version preparations.
pub fn migrate_state(state_dir: &Path) -> io::Result<Vec<Transaction>> {
    let store = Store::open(state_dir)?;
    sql(store
        .conn
        .execute_batch("CREATE TABLE IF NOT EXISTS migrations(name TEXT PRIMARY KEY NOT NULL)"))?;
    let migrated: bool = sql(store.conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM migrations WHERE name='simplified_install_v1')",
        [],
        |r| r.get(0),
    ))?;
    let ids = store.ids()?;
    // Validate the complete journal before deleting any material.
    for id in &ids {
        let _ = store.load(id)?;
    }
    let mut result = Vec::new();
    for id in ids {
        let mut job = store.load(&id)?;
        if !migrated && job.transaction.state == "planned" {
            revoke(&store, &id)?;
            job.transaction.state = "rolled_back".into();
            job.transaction.error = Some("upgrade_plan_revoked".into());
        }
        // Legacy unfinished operations keep their materials until recovery;
        // once settled, their compatibility backups have no further product use.
        job.transient_recovery = true;
        store.save(&mut job)?;
        if ["committed", "superseded", "rolled_back", "uninstalled"]
            .contains(&job.transaction.state.as_str())
            && (!job.uninstall_requested || job.transaction.state == "uninstalled")
        {
            // Check the recorded instance still resolves to this Steam game.
            match validate_install(&job.plan.install, &store.root) {
                Ok(()) => {
                    let _lock = game_lock(&job.plan.install)?;
                    job.transient_recovery = true;
                    store.save(&mut job)?;
                    cleanup_terminal(&store, &mut job)?;
                }
                Err(error) => {
                    job.transaction.error = Some(format!("migration_cleanup_blocked:{error}"));
                    store.save(&mut job)?;
                }
            }
        }
        result.push(job.transaction);
    }
    if !migrated {
        sql(store.conn.execute(
            "INSERT INTO migrations(name) VALUES('simplified_install_v1')",
            [],
        ))?;
    }
    Ok(result)
}
