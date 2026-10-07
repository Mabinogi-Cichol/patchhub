//! Persistent `rust-v1` store: atomic writes, short lock + monotonic revision,
//! cache/status/discovery/index persistence and the dual-engine checkpoint
//! model (design §4.1–§4.6, §5, R6/R9/R12).
//!
//! First open initializes the Rust default state directly; the legacy Python
//! `<state_dir>/sources/` namespace is never read (p0-contracts §1.3, §7.3).

use super::model::{
    DiscoveryIndexRecord, DiscoveryRecord, Overrides, SettingsFile, SourceCache, SourceStatus,
};
use super::{
    invalid, Result, MAX_CACHE_BYTES, MAX_DISCOVERY_OUTPUT, MAX_STATUS_BYTES, STORE_SCHEMA_VERSION,
};
use fs2::FileExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// `rust-v1/settings.json`; the store caps it well above the overrides cap so
/// 32 added rules plus per-game subscriptions fit.
const MAX_SETTINGS_BYTES: u64 = 4 * 1024 * 1024;
const SHORT_LOCK_TIMEOUT: Duration = Duration::from_secs(2);
const WRITE_RETRY_DELAY: Duration = Duration::from_millis(20);
const WRITE_RETRIES: u32 = 5;

/// Namespace layout (design §4.1).
#[derive(Debug, Clone)]
pub struct StorePaths {
    root: PathBuf,
}

impl StorePaths {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            root: state_dir.join("sources").join("rust-v1"),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn schema(&self) -> PathBuf {
        self.root.join("schema.json")
    }

    pub fn settings(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn settings_lock(&self) -> PathBuf {
        self.root.join("settings.lock")
    }

    pub fn engine_switch(&self) -> PathBuf {
        self.root.join("engine-switch.json")
    }

    pub fn conflicts_dir(&self) -> PathBuf {
        self.root.join("conflicts")
    }

    pub fn cache(&self, source_id: &str) -> PathBuf {
        self.root.join("cache").join(format!("{source_id}.json"))
    }

    pub fn status(&self, source_id: &str) -> PathBuf {
        self.root.join("status").join(format!("{source_id}.json"))
    }

    pub fn discovery(&self, source_id: &str, appid: u32) -> PathBuf {
        self.root
            .join("discovery")
            .join(source_id)
            .join(format!("{appid}.json"))
    }

    pub fn discovery_index(&self, source_id: &str) -> PathBuf {
        self.root
            .join("discovery-index")
            .join(format!("{source_id}.json"))
    }
}

/// `engine-switch.json` (design §4.6). Frozen data model; the reentry algorithm
/// is driven by the session layer in P3.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EngineSwitch {
    pub schema_version: u32,
    pub active_engine: String,
    pub switch_seq: u64,
    pub rust: EngineSwitchRust,
    pub python: EngineSwitchPython,
    pub last_common: EngineSwitchCommon,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EngineSwitchRust {
    pub revision: u64,
    pub settings_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EngineSwitchPython {
    pub settings_hash: String,
    pub captured_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EngineSwitchCommon {
    pub settings: Overrides,
    pub hash: String,
    pub captured_at: u64,
}

/// RAII guard releasing the advisory `settings.lock`.
struct FileLock(File);

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Owns `<state_dir>/sources/rust-v1` and the in-memory `settings.json`.
pub struct SourceStore {
    paths: StorePaths,
    settings: Mutex<SettingsFile>,
    /// In-process serialization; the file lock covers cross-process.
    lock: Mutex<()>,
}

impl SourceStore {
    /// Opens the store, initializing the Rust default state directly on first
    /// use. The legacy Python `sources/` namespace is never read.
    pub fn open(state_dir: &Path) -> Result<Self> {
        let paths = StorePaths::new(state_dir);
        ensure_directory(paths.root())?;
        let store = Self {
            paths,
            settings: Mutex::new(default_settings()),
            lock: Mutex::new(()),
        };
        // The completion marker is not the source of truth for user state: a
        // missing/corrupt `schema.json` must never discard an intact
        // `settings.json` (F7). Reuse persisted settings whenever they exist and
        // only write the Rust defaults when the whole namespace is empty.
        if store.paths.settings().exists() {
            let settings = store.load_settings()?;
            *store.settings_guard()? = settings;
            if !store.paths.schema().exists() {
                store.write_schema_marker()?;
            }
        } else if store.paths.schema().exists() {
            // Marker without settings: keep the in-memory defaults; do not write
            // a fresh settings file over an ambiguous namespace.
            let settings = default_settings();
            *store.settings_guard()? = settings;
        } else {
            // First start: persist the default settings and write the
            // `schema.json` completion marker last, so a crash mid-init simply
            // re-runs on the next open (idempotent).
            let settings = default_settings();
            write_json(&store.paths.settings(), &settings, MAX_SETTINGS_BYTES)?;
            store.write_schema_marker()?;
            *store.settings_guard()? = settings;
        }
        Ok(store)
    }

    /// Writes the `schema.json` completion marker (idempotent; never touches
    /// `settings.json`).
    fn write_schema_marker(&self) -> Result<()> {
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);
        write_atomic(
            &self.paths.schema(),
            format!("{{\"schema_version\":{STORE_SCHEMA_VERSION},\"created_at\":{created_at}}}")
                .as_bytes(),
            MAX_STATUS_BYTES,
        )
    }

    pub fn paths(&self) -> &StorePaths {
        &self.paths
    }

    pub fn revision(&self) -> u64 {
        self.settings_guard()
            .map(|guard| guard.revision)
            .unwrap_or(0)
    }

    pub fn settings(&self) -> SettingsFile {
        self.settings_guard()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| default_settings())
    }

    /// Reads `settings.json` from disk, falling back to the in-memory copy.
    pub fn load_settings(&self) -> Result<SettingsFile> {
        match read_json(&self.paths.settings(), MAX_SETTINGS_BYTES)? {
            Some(settings) => Ok(settings),
            None => Ok(self.settings()),
        }
    }

    /// Persists the in-memory settings atomically.
    pub fn save_settings(&self) -> Result<()> {
        let settings = self.settings();
        write_json(&self.paths.settings(), &settings, MAX_SETTINGS_BYTES)
    }

    /// Applies user overrides to the in-memory settings (persisted by the next
    /// [`Self::bump_revision`]).
    pub fn apply_overrides(&self, overrides: &Overrides) -> Result<()> {
        let mut guard = self.settings_guard()?;
        guard.enabled = overrides.enabled.clone();
        guard.added = overrides.added.clone();
        guard.games = overrides.games.clone();
        Ok(())
    }

    /// Runs `f` while holding the independent `rust-v1/settings.lock`.
    ///
    /// Never uses `journal.lock` (design §4.1). `try_lock` with a 2s bound →
    /// `source_state_busy`; no lock is held during network activity.
    pub fn with_short_lock<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let _process = self.lock.lock().map_err(|_| invalid("source_state_busy"))?;
        let _file = self.acquire_file_lock()?;
        f()
    }

    /// Monotonic, persistent revision bump (R6). `load → +1 → save` under the
    /// short lock.
    pub fn bump_revision(&self) -> Result<u64> {
        let _process = self.lock.lock().map_err(|_| invalid("source_state_busy"))?;
        let _file = self.acquire_file_lock()?;
        let revision = {
            let mut guard = self.settings_guard()?;
            let next = guard.revision.saturating_add(1);
            guard.revision = next;
            next
        };
        self.save_settings()?;
        Ok(revision)
    }

    /// Publish-check primitive (design §4.3): runs `f` only if the captured
    /// revision is still current. Returns whether the write happened.
    pub fn run_if_revision(&self, captured: u64, f: impl FnOnce() -> Result<()>) -> Result<bool> {
        self.with_short_lock(|| {
            if self.revision() != captured {
                return Ok(false);
            }
            f()?;
            Ok(true)
        })
    }

    // --- cache / status / discovery / index -------------------------------

    pub fn load_cache(&self, source_id: &str) -> Result<Option<SourceCache>> {
        read_json(&self.paths.cache(source_id), MAX_CACHE_BYTES)
    }

    pub fn save_cache(&self, source_id: &str, cache: &SourceCache) -> Result<()> {
        write_json(&self.paths.cache(source_id), cache, MAX_CACHE_BYTES)
    }

    pub fn load_status(&self, source_id: &str) -> Result<Option<SourceStatus>> {
        read_json(&self.paths.status(source_id), MAX_STATUS_BYTES)
    }

    pub fn save_status(&self, source_id: &str, status: &SourceStatus) -> Result<()> {
        write_json(&self.paths.status(source_id), status, MAX_STATUS_BYTES)
    }

    pub fn load_discovery(&self, source_id: &str, appid: u32) -> Result<Option<DiscoveryRecord>> {
        read_json(
            &self.paths.discovery(source_id, appid),
            MAX_DISCOVERY_OUTPUT,
        )
    }

    pub fn save_discovery(
        &self,
        source_id: &str,
        appid: u32,
        record: &DiscoveryRecord,
    ) -> Result<()> {
        write_json(
            &self.paths.discovery(source_id, appid),
            record,
            MAX_DISCOVERY_OUTPUT,
        )
    }

    pub fn load_discovery_index(&self, source_id: &str) -> Result<Option<DiscoveryIndexRecord>> {
        read_json(&self.paths.discovery_index(source_id), MAX_DISCOVERY_OUTPUT)
    }

    pub fn save_discovery_index(
        &self,
        source_id: &str,
        record: &DiscoveryIndexRecord,
    ) -> Result<()> {
        write_json(
            &self.paths.discovery_index(source_id),
            record,
            MAX_DISCOVERY_OUTPUT,
        )
    }

    // --- engine switch checkpoint -----------------------------------------

    pub fn load_engine_switch(&self) -> Result<Option<EngineSwitch>> {
        read_json(&self.paths.engine_switch(), MAX_SETTINGS_BYTES)
    }

    pub fn save_engine_switch(&self, value: &EngineSwitch) -> Result<()> {
        write_json(&self.paths.engine_switch(), value, MAX_SETTINGS_BYTES)
    }

    // --- internals --------------------------------------------------------

    fn settings_guard(&self) -> Result<MutexGuard<'_, SettingsFile>> {
        self.settings
            .lock()
            .map_err(|_| invalid("source_state_busy"))
    }

    fn acquire_file_lock(&self) -> Result<FileLock> {
        ensure_directory(self.paths.root())?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.paths.settings_lock())?;
        let deadline = Instant::now() + SHORT_LOCK_TIMEOUT;
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(FileLock(file)),
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(WRITE_RETRY_DELAY);
                }
                Err(_) => return Err(invalid("source_state_busy")),
            }
        }
    }
}

fn default_settings() -> SettingsFile {
    SettingsFile {
        schema_version: STORE_SCHEMA_VERSION,
        revision: 1,
        enabled: BTreeMap::new(),
        added: Vec::new(),
        games: BTreeMap::new(),
    }
}

// ---- atomic primitives (design §4.2) -------------------------------------

/// Writes `bytes` to `path` atomically: `> max` → `file_too_large`, parent chain
/// link-checked, same-directory `create_new` tmp + `sync_all` + `rename`, bounded
/// Windows-overwrite retry → `source_state_busy` (never remove-then-write).
pub fn write_atomic(path: &Path, bytes: &[u8], max: u64) -> Result<()> {
    if bytes.len() as u64 > max {
        return Err(invalid("file_too_large"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("linked_source_state"))?;
    ensure_directory(parent)?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if linked(&metadata) || !metadata.is_file() {
            return Err(invalid("linked_source_state"));
        }
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state".to_string());
    let tmp = parent.join(format!(".{}.{}.tmp", name, uuid::Uuid::new_v4().simple()));
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
            #[cfg(unix)]
            sync_dir(parent);
            Ok(())
        }
        Err(_) => {
            for _ in 0..WRITE_RETRIES {
                std::thread::sleep(WRITE_RETRY_DELAY);
                if fs::rename(&tmp, path).is_ok() {
                    #[cfg(unix)]
                    sync_dir(parent);
                    return Ok(());
                }
            }
            let _ = fs::remove_file(&tmp);
            Err(invalid("source_state_busy"))
        }
    }
}

/// Reads at most `max` bytes; missing → `None`, linked/non-regular →
/// `linked_source_state`, too large → `file_too_large`.
pub fn read_bounded(path: &Path, max: u64) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
        Ok(metadata) => {
            if linked(&metadata) || !metadata.is_file() {
                return Err(invalid("linked_source_state"));
            }
            if metadata.len() > max {
                return Err(invalid("file_too_large"));
            }
        }
    }
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(invalid("file_too_large"));
    }
    Ok(Some(bytes))
}

fn read_json<T: DeserializeOwned>(path: &Path, max: u64) -> Result<Option<T>> {
    match read_bounded(path, max)? {
        Some(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| invalid(error.to_string())),
        None => Ok(None),
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T, max: u64) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| invalid(error.to_string()))?;
    write_atomic(path, &bytes, max)
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

fn ensure_directory(dir: &Path) -> std::io::Result<()> {
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
            return Err(invalid("linked_source_state"));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::model::RulesConfig;
    use crate::sources::validate;
    use std::sync::Arc;

    fn builtins() -> Vec<crate::sources::model::SourceRule> {
        let config: RulesConfig = serde_json::from_str(crate::sources::DEFAULT_RULES_JSON).unwrap();
        config
            .sources
            .iter()
            .map(|raw| validate::validate_rule(raw).unwrap())
            .collect()
    }

    fn entry(title: &str, url: &str, appid: Option<u32>) -> super::super::model::SourceEntry {
        super::super::model::SourceEntry {
            title: title.to_string(),
            aliases: Vec::new(),
            url: url.to_string(),
            source_page: "https://www.patch.moe/".to_string(),
            language: "en".to_string(),
            date: None,
            appid,
        }
    }

    #[test]
    fn settings_round_trips_and_revision_is_persistent() {
        let temp = tempfile::tempdir().unwrap();
        let store = SourceStore::open(temp.path()).unwrap();
        assert_eq!(store.revision(), 1);
        let bumped = store.bump_revision().unwrap();
        assert_eq!(bumped, 2);
        drop(store);

        let reopened = SourceStore::open(temp.path()).unwrap();
        assert_eq!(reopened.revision(), 2);
        assert_eq!(reopened.load_settings().unwrap().revision, 2);
    }

    #[test]
    fn first_open_initializes_rust_default_state() {
        let temp = tempfile::tempdir().unwrap();
        let store = SourceStore::open(temp.path()).unwrap();
        assert_eq!(store.revision(), 1);
        assert!(store.paths().schema().exists());
        assert!(store.paths().settings().exists());
        let settings = store.load_settings().unwrap();
        assert_eq!(settings.revision, 1);
        assert_eq!(settings.schema_version, STORE_SCHEMA_VERSION);
        assert!(settings.added.is_empty());
        assert!(settings.enabled.is_empty());
        assert!(settings.games.is_empty());
        drop(store);

        // Reopen reads the persisted settings (schema marker is present).
        let reopened = SourceStore::open(temp.path()).unwrap();
        assert_eq!(reopened.revision(), 1);
        assert_eq!(reopened.load_settings().unwrap(), settings);
    }

    #[test]
    fn missing_or_corrupt_schema_preserves_existing_settings() {
        let temp = tempfile::tempdir().unwrap();
        let store = SourceStore::open(temp.path()).unwrap();
        let rule = builtins()
            .into_iter()
            .find(|rule| rule.id == "kagura-cn")
            .unwrap();
        let overrides = super::super::model::Overrides {
            enabled: BTreeMap::from([("patchmoe".to_string(), false)]),
            added: vec![rule],
            games: BTreeMap::from([(
                "42".to_string(),
                BTreeMap::from([("kagura-cn".to_string(), true)]),
            )]),
        };
        store.apply_overrides(&overrides).unwrap();
        store.bump_revision().unwrap();
        let settings = store.load_settings().unwrap();
        assert!(!settings.added.is_empty());
        drop(store);

        // A missing `schema.json` must not overwrite the intact user settings.
        let paths = StorePaths::new(temp.path());
        fs::remove_file(paths.schema()).unwrap();
        let reopened = SourceStore::open(temp.path()).unwrap();
        let restored = reopened.load_settings().unwrap();
        assert_eq!(restored.added, settings.added);
        assert_eq!(restored.games, settings.games);
        assert_eq!(restored.enabled, settings.enabled);
        assert!(paths.schema().exists(), "completion marker is recreated");
        drop(reopened);

        // A corrupt marker is equally ignored.
        fs::write(paths.schema(), b"{not json").unwrap();
        let reopened = SourceStore::open(temp.path()).unwrap();
        assert_eq!(reopened.load_settings().unwrap().added, settings.added);
        assert_eq!(reopened.load_settings().unwrap().games, settings.games);
    }

    #[test]
    fn legacy_python_state_is_not_read_on_open() {
        let temp = tempfile::tempdir().unwrap();
        let python = temp.path().join("sources");
        fs::create_dir_all(python.join("cache")).unwrap();
        fs::create_dir_all(python.join("status")).unwrap();
        fs::create_dir_all(python.join("discovery").join("patchmoe")).unwrap();
        fs::create_dir_all(python.join("discovery-index")).unwrap();
        // Deliberately valid-looking legacy state that must never be imported.
        fs::write(
            python.join("overrides.json"),
            br#"{"enabled":{"patchmoe":false},"added":[{"id":"legacy","name":"Legacy","adapter":"patchmoe","enabled":true,"pages":["https://www.patch.moe/"],"page_hosts":["www.patch.moe"],"download_hosts":["download.patch.moe"],"root_layout":"game_root"}],"games":{"42":{"patchmoe":true}}}"#,
        )
        .unwrap();
        fs::write(python.join("cache").join("patchmoe.json"), b"{not json").unwrap();
        fs::write(python.join("status").join("patchmoe.json"), b"{not json").unwrap();
        fs::write(
            python.join("discovery").join("patchmoe").join("42.json"),
            b"{not json",
        )
        .unwrap();
        fs::write(
            python.join("discovery-index").join("patchmoe.json"),
            b"{not json",
        )
        .unwrap();

        let store = SourceStore::open(temp.path()).unwrap();
        let settings = store.settings();
        assert_eq!(settings.revision, 1);
        assert!(
            settings.added.is_empty(),
            "Python added rules must not import"
        );
        assert!(
            settings.enabled.is_empty(),
            "Python enabled flags must not import"
        );
        assert!(
            settings.games.is_empty(),
            "Python subscriptions must not import"
        );
        // The Rust namespace is initialized independently of the legacy files.
        assert!(store.paths().schema().exists());
        assert!(store.load_cache("patchmoe").unwrap().is_none());
        assert!(store.load_status("patchmoe").unwrap().is_none());
        assert!(store.load_discovery("patchmoe", 42).unwrap().is_none());
        assert!(store.load_discovery_index("patchmoe").unwrap().is_none());
    }

    #[test]
    fn atomic_replace_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("value.json");
        write_atomic(&path, b"first", 1024).unwrap();
        write_atomic(&path, b"second", 1024).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second".to_vec());
    }

    #[test]
    fn atomic_recovery_on_crash_leaves_original_and_no_tmp() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("value.json");
        write_atomic(&path, b"original", 1024).unwrap();
        assert_eq!(
            write_atomic(&path, &[0u8; 32], 8).unwrap_err().to_string(),
            "file_too_large"
        );
        assert_eq!(fs::read(&path).unwrap(), b"original".to_vec());
        let leftovers: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn reject_non_regular_and_linked_state() {
        let temp = tempfile::tempdir().unwrap();
        // A directory is not a regular file target.
        let dir_target = temp.path().join("state.json");
        fs::create_dir(&dir_target).unwrap();
        assert_eq!(
            write_atomic(&dir_target, b"x", 1024)
                .unwrap_err()
                .to_string(),
            "linked_source_state"
        );
        assert_eq!(
            read_bounded(&dir_target, 1024).unwrap_err().to_string(),
            "linked_source_state"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let real = temp.path().join("real.json");
            fs::write(&real, b"real").unwrap();
            let link = temp.path().join("link.json");
            symlink(&real, &link).unwrap();
            assert_eq!(
                read_bounded(&link, 1024).unwrap_err().to_string(),
                "linked_source_state"
            );
            assert_eq!(
                write_atomic(&link, b"new", 1024).unwrap_err().to_string(),
                "linked_source_state"
            );
        }
    }

    #[test]
    fn enforce_size_and_entry_caps() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("big.json");
        assert_eq!(
            write_atomic(&path, &vec![0u8; 300], 256)
                .unwrap_err()
                .to_string(),
            "file_too_large"
        );
        let rule = builtins().into_iter().find(|r| r.id == "patchmoe").unwrap();
        let entries = vec![entry("G", "https://download.patch.moe/G.zip", Some(1)); 4097];
        assert_eq!(
            validate::validate_entries(&rule, &entries)
                .unwrap_err()
                .to_string(),
            "empty_or_oversized_source"
        );
    }

    #[test]
    fn short_lock_serializes_revision_bumps() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(SourceStore::open(temp.path()).unwrap());
        let start = store.revision();
        let mut handles = Vec::new();
        for _ in 0..4 {
            let store = Arc::clone(&store);
            handles.push(std::thread::spawn(move || store.bump_revision().unwrap()));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(store.revision(), start + 4);
    }

    #[test]
    fn source_cache_round_trips() {
        let temp = tempfile::tempdir().unwrap();
        let store = SourceStore::open(temp.path()).unwrap();
        let cache = SourceCache {
            entries: vec![entry("Game", "https://download.patch.moe/g.zip", Some(42))],
            etag: Some("etag".to_string()),
            modified: None,
            cached_at: Some(100),
            checked_at: Some(100),
        };
        store.save_cache("patchmoe", &cache).unwrap();
        assert_eq!(store.load_cache("patchmoe").unwrap(), Some(cache));
        assert!(store.load_cache("missing").unwrap().is_none());
    }
}
