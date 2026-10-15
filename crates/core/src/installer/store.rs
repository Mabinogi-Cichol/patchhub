//! Layout, atomic bootstrap and boundary helpers for `installer/rust-v1/`
//! (p0-contracts §3.1–§3.3).
//!
//! ```text
//! installer/rust-v1/
//! ├── schema.json                 # completion marker (written last)
//! ├── cache.lock                  # shared installer / exclusive cache lock
//! ├── journal.lock                # cross-process instance lock
//! ├── journal.sqlite              # transaction journal (schema owned by install::Store)
//! ├── requests.sqlite             # request-idempotency ledger
//! ├── jobs/<uuid>/...             # protected transaction material
//! ├── tasks/<uuid>/...            # transient task runtime (reclaimed)
//! ├── content/<content_id>/       # cleanable
//! ├── receipts/<instance_hash>/   # protected
//! ├── downloads/                  # cleanable
//! └── reports/<content_id>/       # cleanable
//! ```
//!
//! The atomic primitives mirror `sources::store::write_atomic`/`publish_tree`
//! (create_new tmp + `sync_all` + rename, bounded Windows retry, link checks).

use super::{invalid, Result, STORE_SCHEMA_VERSION};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const WRITE_RETRIES: u32 = 5;
const WRITE_RETRY_DELAY: Duration = Duration::from_millis(20);
const MAX_MARKER_BYTES: u64 = 4096;
const MAX_INIT_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Staging directory prefix used while publishing the base tree.
const INIT_STAGING_PREFIX: &str = ".init-";

/// Base directories created by [`init_atomic`] (p0-contracts §3.1).
const BASE_DIRS: &[&str] = &["jobs", "content", "receipts", "downloads", "reports"];

/// `schema.json` completion marker (p0-contracts §3.3).
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaMarker {
    schema_version: u32,
    created_at: u64,
}

/// Namespace layout rooted at `state_dir/installer/rust-v1/`.
#[derive(Debug, Clone)]
pub struct InstallerPaths {
    root: PathBuf,
}

impl InstallerPaths {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            root: state_dir.join("installer").join("rust-v1"),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    // --- base files --------------------------------------------------------

    /// Completion marker; its presence means initialization finished.
    pub fn schema(&self) -> PathBuf {
        self.root.join("schema.json")
    }

    /// Cross-process instance lock (p0-contracts §4.1).
    pub fn journal_lock(&self) -> PathBuf {
        self.root.join("journal.lock")
    }

    /// Fixed shared installer / exclusive cleanup lock (p0-contract §3).
    pub fn cache_lock(&self) -> PathBuf {
        self.root.join("cache.lock")
    }

    /// Transaction journal (sole transaction source of truth).
    pub fn journal(&self) -> PathBuf {
        self.root.join("journal.sqlite")
    }

    /// Request-idempotency ledger (p0-contracts §3.1, §5.1).
    pub fn requests(&self) -> PathBuf {
        self.root.join("requests.sqlite")
    }

    // --- directories -------------------------------------------------------

    /// `<uuid>` transaction recovery jobs (protected; p0-contracts §3.2).
    pub fn jobs_dir(&self) -> PathBuf {
        self.root.join("jobs")
    }

    /// `<uuid>` **transient** task runtime directories (`task.json`,
    /// `revoked`, `owner.lock`). Reclaimed on terminal state and
    /// swept by [`super::cache::CacheService`] after a crash. Deliberately
    /// **not** part of [`Self::cleanable_dirs`] (unlocked deletes are unsafe)
    /// and **not** protected by [`Self::is_protected`].
    pub fn tasks_dir(&self) -> PathBuf {
        self.root.join("tasks")
    }

    /// Normalized ZIP / payload cache (cleanable).
    pub fn content_dir(&self) -> PathBuf {
        self.root.join("content")
    }

    /// Success receipts keyed by `instance_hash` (protected).
    pub fn receipts_dir(&self) -> PathBuf {
        self.root.join("receipts")
    }

    /// Dynamic download cache (`<sha256>.zip` / `.partial`; cleanable).
    pub fn downloads_dir(&self) -> PathBuf {
        self.root.join("downloads")
    }

    /// Analysis reports keyed by `content_id` (cleanable / archival).
    pub fn reports_dir(&self) -> PathBuf {
        self.root.join("reports")
    }

    // --- resolved child paths ---------------------------------------------

    /// `content/<content_id>/` (cleanable).
    pub fn content(&self, content_id: &str) -> Result<PathBuf> {
        self.child(&self.content_dir(), content_id)
    }

    /// `receipts/<instance_hash>/` (protected).
    pub fn receipts(&self, instance_hash: &str) -> Result<PathBuf> {
        self.child(&self.receipts_dir(), instance_hash)
    }

    /// `reports/<content_id>/` (cleanable).
    pub fn reports(&self, content_id: &str) -> Result<PathBuf> {
        self.child(&self.reports_dir(), content_id)
    }

    /// `jobs/<plan_id>/` transaction recovery/authorization material
    /// (protected; p0-contracts §3.2).
    pub fn job(&self, plan_id: &str) -> Result<PathBuf> {
        self.child(&self.jobs_dir(), plan_id)
    }

    /// `tasks/<id>/` transient task runtime (reclaimed; not protected).
    pub fn task(&self, id: &str) -> Result<PathBuf> {
        self.child(&self.tasks_dir(), id)
    }

    /// `jobs/<plan_id>/authorization.json` (protected; design §5.2).
    pub fn authorization(&self, plan_id: &str) -> Result<PathBuf> {
        Ok(self.job(plan_id)?.join("authorization.json"))
    }

    fn child(&self, parent: &Path, id: &str) -> Result<PathBuf> {
        safe_component(id)?;
        let path = parent.join(id);
        safe_path(&path)?;
        Ok(path)
    }

    // --- cache boundary ----------------------------------------------------

    /// Roots that cache cleanup may delete from (p0-contracts §3.2). Never
    /// include `jobs/`, `receipts/`, the journal or `requests.sqlite`.
    pub fn cleanable_dirs(&self) -> Vec<PathBuf> {
        vec![self.content_dir(), self.downloads_dir(), self.reports_dir()]
    }

    /// Enumerates the removable entries under the cleanable roots.
    pub fn list_cache_entries(&self) -> Result<Vec<PathBuf>> {
        let mut entries = Vec::new();
        for dir in self.cleanable_dirs() {
            if !dir.exists() {
                continue;
            }
            for entry in fs::read_dir(&dir)? {
                entries.push(entry?.path());
            }
        }
        Ok(entries)
    }

    /// Whether `path` belongs to a protected region that cleanup must never
    /// remove: journal, `requests.sqlite`, `jobs/` (including `revoked` /
    /// `.stage` / `.backup` material), `receipts/` and the schema marker.
    pub fn is_protected(&self, path: &Path) -> bool {
        let relative = path.strip_prefix(self.root()).unwrap_or(path);
        let components: Vec<String> = relative
            .components()
            .filter_map(|component| match component {
                Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        let Some(first) = components.first() else {
            return false;
        };
        if matches!(
            first.as_str(),
            "schema.json"
                | "cache.lock"
                | "journal.lock"
                | "journal.sqlite"
                | "journal.sqlite-wal"
                | "journal.sqlite-shm"
                | "journal.sqlite-journal"
                | "requests.sqlite"
                | "jobs"
                | "receipts"
        ) {
            return true;
        }
        // Defensive: recognize recovery material even when addressed relatively.
        components
            .iter()
            .any(|name| name == "revoked" || name.ends_with(".stage") || name.ends_with(".backup"))
    }
}

/// `instance_hash = sha256(instance_id)` first 16 hex chars (p0-contracts §3.1).
pub fn instance_hash(instance_id: &str) -> String {
    let digest = Sha256::digest(instance_id.as_bytes());
    let mut hex = String::with_capacity(16);
    for byte in &digest[..8] {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Atomically initializes `installer/rust-v1/` (p0-contracts §3.3).
///
/// Idempotent and interrupt-safe: a present `schema.json` short-circuits; a
/// half-written root (staging leftover, missing marker) is republished from a
/// fresh staging tree, and the marker is written **last**.
pub fn init_atomic(paths: &InstallerPaths) -> Result<()> {
    if paths.schema().exists() {
        return Ok(());
    }

    ensure_directory(paths.root())?;
    remove_staging_leftovers(paths.root())?;

    let staging = paths
        .root()
        .join(format!("{INIT_STAGING_PREFIX}{}", Uuid::new_v4().simple()));
    ensure_directory(&staging)?;
    for dir in BASE_DIRS {
        ensure_directory(&staging.join(dir))?;
    }

    // Base files. journal.sqlite starts empty; `install::Store` owns its schema.
    touch_file(&staging.join("journal.lock"))?;
    touch_file(&staging.join("journal.sqlite"))?;
    super::requests::create_requests_db(&staging.join("requests.sqlite"))?;

    sync_tree(&staging)?;
    publish_tree(&staging, paths.root())?;
    let _ = fs::remove_dir_all(&staging);
    sync_dir(paths.root());

    // Completion marker strictly last.
    let marker = SchemaMarker {
        schema_version: STORE_SCHEMA_VERSION,
        created_at: now_unix(),
    };
    let bytes = serde_json::to_vec(&marker).map_err(|error| invalid(error.to_string()))?;
    write_atomic(&paths.schema(), &bytes, MAX_MARKER_BYTES)
}

// ---- atomic primitives ----------------------------------------------------

/// Same-directory `create_new` tmp + `sync_all` + rename, bounded Windows
/// overwrite retry. Never remove-then-write (p0-contracts §3.3).
pub(crate) fn write_atomic(path: &Path, bytes: &[u8], max: u64) -> Result<()> {
    if bytes.len() as u64 > max {
        return Err(invalid("file_too_large"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("linked_installer_state"))?;
    ensure_directory(parent)?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if linked(&metadata) || !metadata.is_file() {
            return Err(invalid("linked_installer_state"));
        }
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state".to_string());
    let tmp = parent.join(format!(".{name}.{}.tmp", Uuid::new_v4().simple()));
    {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
        }
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    match fs::rename(&tmp, path) {
        Ok(()) => {
            sync_dir(parent);
            Ok(())
        }
        Err(_) => {
            for _ in 0..WRITE_RETRIES {
                std::thread::sleep(WRITE_RETRY_DELAY);
                if fs::rename(&tmp, path).is_ok() {
                    sync_dir(parent);
                    return Ok(());
                }
            }
            let _ = fs::remove_file(&tmp);
            Err(invalid("installer_state_busy"))
        }
    }
}

/// Moves every entry from `staging` into `root`, creating directories as needed.
fn publish_tree(staging: &Path, root: &Path) -> Result<()> {
    for entry in fs::read_dir(staging)? {
        let entry = entry?;
        let from = entry.path();
        let name = entry.file_name();
        let to = root.join(&name);
        if from.is_dir() {
            ensure_directory(&to)?;
            publish_tree(&from, &to)?;
        } else {
            ensure_directory(root)?;
            match fs::rename(&from, &to) {
                Ok(()) => {}
                Err(_) => {
                    let bytes = fs::read(&from)?;
                    write_atomic(&to, &bytes, MAX_INIT_FILE_BYTES)?;
                    let _ = fs::remove_file(&from);
                }
            }
        }
    }
    Ok(())
}

fn touch_file(path: &Path) -> Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    file.sync_all()?;
    Ok(())
}

fn remove_staging_leftovers(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(INIT_STAGING_PREFIX) && entry.path().is_dir() {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    Ok(())
}

fn sync_tree(dir: &Path) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            sync_tree(&path)?;
        } else {
            // `sync_all` (FlushFileBuffers) needs write access on Windows, so a
            // read-only handle is insufficient.
            let file = OpenOptions::new().read(true).write(true).open(&path)?;
            file.sync_all()?;
        }
    }
    sync_dir(dir);
    Ok(())
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

fn ensure_directory(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    for ancestor in dir.ancestors() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        if linked(&fs::symlink_metadata(ancestor)?) {
            return Err(invalid("linked_installer_state"));
        }
    }
    Ok(())
}

fn safe_component(value: &str) -> Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains(['/', '\\'])
        || value.contains('\0')
    {
        return Err(invalid("invalid_installer_path"));
    }
    Ok(())
}

/// Rejects paths that escape or resolve outside the installer root.
pub(crate) fn safe_path(path: &Path) -> Result<()> {
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            return Err(invalid("invalid_installer_path"));
        }
    }
    Ok(())
}

fn sync_dir(dir: &Path) {
    // Directory fsync is unix-only; on Windows opening a directory handle is
    // not supported and the call is effectively a no-op.
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(temp: &tempfile::TempDir) -> InstallerPaths {
        InstallerPaths::new(temp.path())
    }

    #[test]
    fn init_is_idempotent_and_publishes_base_tree() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);

        init_atomic(&paths).unwrap();
        assert!(paths.schema().exists());
        for dir in [
            paths.jobs_dir(),
            paths.content_dir(),
            paths.receipts_dir(),
            paths.downloads_dir(),
            paths.reports_dir(),
        ] {
            assert!(dir.is_dir(), "missing {}", dir.display());
        }
        assert!(paths.journal_lock().exists());
        assert!(paths.journal().exists());
        assert!(paths.requests().exists());

        let marker: SchemaMarker =
            serde_json::from_slice(&fs::read(paths.schema()).unwrap()).unwrap();
        assert_eq!(marker.schema_version, STORE_SCHEMA_VERSION);

        // Repeated init must not error or mutate the completed root.
        let before = fs::read(paths.schema()).unwrap();
        init_atomic(&paths).unwrap();
        assert_eq!(fs::read(paths.schema()).unwrap(), before);
    }

    #[test]
    fn init_retries_after_interrupted_attempt() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        init_atomic(&paths).unwrap();

        // Simulate a crash after publishing but before the marker, with staging
        // leftovers still present.
        fs::remove_file(paths.schema()).unwrap();
        let staging = paths.root().join(format!("{INIT_STAGING_PREFIX}leftover"));
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("partial"), b"junk").unwrap();

        init_atomic(&paths).unwrap();
        assert!(paths.schema().exists());
        assert!(!staging.exists(), "staging leftover should be removed");
    }

    #[test]
    fn directory_boundaries_and_protection() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        init_atomic(&paths).unwrap();

        assert_eq!(
            paths.content("exe-content-abc-100").unwrap(),
            paths.content_dir().join("exe-content-abc-100")
        );
        assert_eq!(
            paths.receipts(&instance_hash("instance")).unwrap(),
            paths.receipts_dir().join(instance_hash("instance"))
        );
        assert_eq!(
            paths.reports("exe-content-abc-100").unwrap(),
            paths.reports_dir().join("exe-content-abc-100")
        );

        // Path traversal / separators are rejected.
        assert!(paths.content("../escape").is_err());
        assert!(paths.content("a/b").is_err());
        assert!(paths.receipts("").is_err());

        // Protected regions.
        assert!(paths.is_protected(&paths.cache_lock()));
        assert!(paths.is_protected(Path::new("cache.lock")));
        assert!(paths.is_protected(&paths.journal_lock()));
        assert!(paths.is_protected(&paths.journal()));
        assert!(paths.is_protected(&paths.requests()));
        assert!(paths.is_protected(&paths.schema()));
        assert!(paths.is_protected(&paths.jobs_dir().join("uuid").join("revoked")));
        assert!(paths.is_protected(&paths.jobs_dir().join("uuid").join("0.stage")));
        assert!(paths.is_protected(&paths.jobs_dir().join("uuid").join("0.backup")));
        assert!(paths.is_protected(&paths.receipts_dir().join("abc")));

        // Cleanable regions.
        assert!(!paths.is_protected(&paths.content_dir().join("content-id")));
        assert!(!paths.is_protected(&paths.downloads_dir().join("abc.zip")));
        assert!(!paths.is_protected(&paths.reports_dir().join("content-id")));
        assert_eq!(paths.cleanable_dirs().len(), 3);
    }

    #[test]
    fn list_cache_entries_only_sees_cleanable_roots() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        init_atomic(&paths).unwrap();

        fs::create_dir_all(paths.content("c1").unwrap()).unwrap();
        fs::write(paths.downloads_dir().join("a.zip"), b"x").unwrap();
        fs::create_dir_all(paths.reports("r1").unwrap()).unwrap();
        fs::create_dir_all(paths.jobs_dir().join("job1")).unwrap();

        let mut names: Vec<String> = paths
            .list_cache_entries()
            .unwrap()
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.zip", "c1", "r1"]);
    }

    #[test]
    fn instance_hash_is_sha256_prefix_16_hex() {
        let hash = instance_hash("steam-76561198000000000");
        assert_eq!(hash.len(), 16);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hash, instance_hash("steam-76561198000000000"));
        assert_ne!(hash, instance_hash("steam-76561198000000001"));

        // Known value: sha256("instance") = 5e8c03a9... (first 8 bytes).
        assert_eq!(instance_hash("instance"), "5e8c03a94c7835b8");
    }

    #[test]
    fn transactional_store_can_point_at_installer_root() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        init_atomic(&paths).unwrap();

        // P1 new-root wiring: journal.lock/journal.sqlite/jobs land under
        // `installer/rust-v1/` (legacy default remains the state dir itself).
        let store = crate::install::Store::open_at(paths.root()).unwrap();
        drop(store);
        assert!(paths.journal_lock().exists());
        assert!(paths.jobs_dir().is_dir());
    }
}
