//! Persistent `rust-v1` store: atomic writes, short lock + monotonic revision,
//! schema-2 index/detail/match persistence and the one-time schema 1 → 2
//! migration (design §4.1–§4.6, §5, R6/R9/R12; p1-contract §2, §8).
//!
//! First open initializes the Rust default state directly; the legacy Python
//! `<state_dir>/sources/` namespace is never read (p0-contracts §1.3, §7.3).

use super::index::{from_cache, from_discovery_index, normalize_entries};
use super::model::{
    DetailRecord, Diagnostic, DiscoveryIndexRecord, DiscoveryRecord, FallbackMatchRecord,
    FallbackMatchStatus, GameMatchRecord, GameMatchStatus, RulesConfig, SettingsFile, SourceCache,
    SourceIndex, SourceMatchRecord, SourceMatchStatus, SourceRole, SourceStatus,
};
use super::r#match::{candidate_is_credible, derive_game_status};
use super::{
    canonical, invalid, Result, DEFAULT_RULES_JSON, GAME_MATCH_SCHEMA_VERSION,
    SOURCE_SETTINGS_SCHEMA_VERSION, STORE_SCHEMA_VERSION,
};
use fs2::FileExt;
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

    /// Published per-source index snapshot (schema 2).
    pub fn index(&self, source_id: &str) -> PathBuf {
        self.root.join("index").join(format!("{source_id}.json"))
    }

    /// Unpublished update progress for a source (same shape as [`Self::index`]);
    /// removed after a complete update publishes the official snapshot.
    pub fn index_progress(&self, source_id: &str) -> PathBuf {
        self.root
            .join("index-progress")
            .join(format!("{source_id}.json"))
    }

    /// Per-source detail cache directory.
    pub fn details(&self, source_id: &str) -> PathBuf {
        self.root.join("details").join(source_id)
    }

    /// Shared detail record for one canonical page (schema 2).
    pub fn detail(&self, source_id: &str, key: &str) -> PathBuf {
        self.details(source_id).join(format!("{key}.json"))
    }

    /// Persisted match record for one AppID (schema 2).
    pub fn matches(&self, appid: u32) -> PathBuf {
        self.root.join("matches").join(format!("{appid}.json"))
    }
}

/// `sha256(canonical_page)` truncated to 32 lowercase hex characters
/// (p1-contract §2). The page is expected to already be `canonical_url()`.
pub fn detail_key(canonical_page: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(canonical_page.as_bytes()));
    digest[..32].to_string()
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
    /// Diagnostics raised while migrating a v1 namespace (kept with the store so
    /// later stages can surface them; the conflict case must not be silent).
    migration_diagnostics: Vec<Diagnostic>,
}

impl SourceStore {
    /// Opens the store, migrating a schema-1 namespace to schema 2 on first use.
    /// The legacy Python `sources/` namespace is never read.
    pub fn open(state_dir: &Path) -> Result<Self> {
        let paths = StorePaths::new(state_dir);
        ensure_directory(paths.root())?;
        let mut store = Self {
            paths,
            settings: Mutex::new(default_settings()),
            lock: Mutex::new(()),
            migration_diagnostics: Vec::new(),
        };
        // Serialize initialization/format upgrades with normal settings edits.
        let _file = store.acquire_file_lock()?;
        let settings_present = store.paths.settings().exists();
        // The completion marker is not the source of truth for user state: a
        // missing/corrupt `schema.json` must never discard an intact
        // `settings.json` (F7). Reuse persisted settings whenever they exist.
        let mut upgrade_settings = false;
        if settings_present {
            let mut settings = store.load_settings()?;
            if !(1..=SOURCE_SETTINGS_SCHEMA_VERSION).contains(&settings.schema_version) {
                return Err(invalid("invalid_source_schema"));
            }
            if settings.schema_version != SOURCE_SETTINGS_SCHEMA_VERSION {
                // Additive settings upgrade; the schema-2 index/detail/match
                // marker is independent, so v2 files are not migrated again.
                settings.schema_version = SOURCE_SETTINGS_SCHEMA_VERSION;
                upgrade_settings = true;
            }
            *store.settings_guard()? = settings;
        } else {
            *store.settings_guard()? = default_settings();
        }
        let schema_current = store.schema_is_current()?;
        // The schema-2 marker is written last, after the migrated index/match
        // files and the settings upgrade, so a crash anywhere is re-runnable.
        if !schema_current {
            store.migrate_legacy()?;
        }
        if upgrade_settings || (!settings_present && !schema_current) {
            // First start also persists the defaults before the marker.
            store.save_settings()?;
        }
        if !schema_current {
            store.write_schema_marker()?;
        }
        Ok(store)
    }

    /// Opens an existing source namespace without creating, repairing, or writing files.
    /// A wholly absent namespace is handled by `SourceSnapshot::load` before this call.
    ///
    /// Schema 1 is accepted for projection before the first writing open runs the
    /// migration; corrupt, linked and unknown-version state is still rejected.
    pub(crate) fn open_readonly(state_dir: &Path) -> Result<Self> {
        let paths = StorePaths::new(state_dir);
        let metadata = fs::symlink_metadata(paths.root())?;
        if linked(&metadata) || !metadata.is_dir() {
            return Err(invalid("linked_source_state"));
        }
        let settings_bytes = read_bounded(&paths.settings())?;
        let schema_bytes = read_bounded(&paths.schema())?;
        if settings_bytes.is_none() && schema_bytes.is_none() {
            return Err(invalid("invalid_source_state"));
        }
        if let Some(bytes) = schema_bytes {
            let marker: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))?;
            let version = marker
                .get("schema_version")
                .and_then(serde_json::Value::as_u64);
            if version != Some(1) && version != Some(STORE_SCHEMA_VERSION as u64) {
                return Err(invalid("invalid_source_schema"));
            }
        }
        let settings = if let Some(bytes) = settings_bytes {
            serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))?
        } else {
            default_settings()
        };
        // v1 settings have no `intervals`; `#[serde(default)]` leaves them off.
        if !(1..=SOURCE_SETTINGS_SCHEMA_VERSION).contains(&settings.schema_version)
            || settings.revision == 0
        {
            return Err(invalid("invalid_source_state"));
        }
        Ok(Self {
            paths,
            settings: Mutex::new(settings),
            lock: Mutex::new(()),
            migration_diagnostics: Vec::new(),
        })
    }

    /// `schema.json` is current when its version is 2; a missing/corrupt marker
    /// re-runs the (idempotent) migration, a newer version is rejected.
    fn schema_is_current(&self) -> Result<bool> {
        let version = match read_bounded(&self.paths.schema())? {
            Some(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|marker| {
                    marker
                        .get("schema_version")
                        .and_then(serde_json::Value::as_u64)
                }),
            None => None,
        };
        match version {
            Some(version) if version > STORE_SCHEMA_VERSION as u64 => {
                Err(invalid("invalid_source_schema"))
            }
            Some(version) => Ok(version == STORE_SCHEMA_VERSION as u64),
            None => Ok(false),
        }
    }

    /// Diagnostics raised by the v1 → v2 migration (empty when nothing conflicted).
    pub fn migration_diagnostics(&self) -> &[Diagnostic] {
        &self.migration_diagnostics
    }

    /// One-time v1 → v2 migration of the legacy `cache/`, `discovery-index/` and
    /// `discovery/` trees. Old files are only read and never modified/deleted;
    /// every failure returns `source_migration_failed` and leaves them intact.
    fn migrate_legacy(&mut self) -> Result<()> {
        self.migrate_indexes()?;
        self.migrate_matches()?;
        Ok(())
    }

    /// `cache/<source>.json` and `discovery-index/<source>.json` → `index/<source>.json`.
    fn migrate_indexes(&mut self) -> Result<()> {
        let cache_files =
            json_files(&self.paths.root().join("cache")).map_err(|_| migration_failed())?;
        let mut cache_sources = BTreeSet::new();
        for path in &cache_files {
            let source_id = json_stem(path)?;
            let cache: SourceCache = migration(read_json(path))?.ok_or_else(migration_failed)?;
            let index = from_cache(&source_id, &cache);
            migration(self.save_index(&index))?;
            cache_sources.insert(source_id);
        }
        let discovery_index_files = json_files(&self.paths.root().join("discovery-index"))
            .map_err(|_| migration_failed())?;
        for path in &discovery_index_files {
            let source_id = json_stem(path)?;
            let record: DiscoveryIndexRecord =
                migration(read_json(path))?.ok_or_else(migration_failed)?;
            let discovered = from_discovery_index(&source_id, &record);
            let index = match migration(self.load_index(&source_id))? {
                Some(existing) => {
                    if cache_sources.contains(&source_id) {
                        // Both v1 representations exist for one source: keep the
                        // cache snapshot (complete coverage) and keep the
                        // discovery entries/pending instead of dropping them.
                        self.migration_diagnostics.push(Diagnostic {
                            source_id: Some(source_id.clone()),
                            code: "source_index_migration_conflict".to_string(),
                            detail: Some(
                                "cache and discovery-index both present; cache snapshot kept"
                                    .to_string(),
                            ),
                        });
                    }
                    combine_indexes(existing, discovered)
                }
                None => discovered,
            };
            migration(self.save_index(&index))?;
        }
        Ok(())
    }

    /// `discovery/<source>/<appid>.json` → `matches/<appid>.json`.
    ///
    /// The old per-game cache cannot claim full-source coverage: a source match
    /// without a matching index snapshot is `no_index` with revision 0.
    fn migrate_matches(&mut self) -> Result<()> {
        let fallback_sources = fallback_source_ids(&self.settings()?);
        let discovery_root = self.paths.root().join("discovery");
        let mut games: BTreeMap<u32, GameMatchRecord> = BTreeMap::new();
        let mut indexes: BTreeMap<String, Option<SourceIndex>> = BTreeMap::new();
        for (source_id, directory) in
            subdirectories(&discovery_root).map_err(|_| migration_failed())?
        {
            let index = match indexes.get(&source_id) {
                Some(index) => index.clone(),
                None => {
                    let index = migration(self.load_index(&source_id))?;
                    indexes.insert(source_id.clone(), index.clone());
                    index
                }
            };
            for path in json_files(&directory).map_err(|_| migration_failed())? {
                let appid = json_stem(&path)?
                    .parse::<u32>()
                    .map_err(|_| migration_failed())?;
                let legacy: DiscoveryRecord =
                    migration(read_json(&path))?.ok_or_else(migration_failed)?;
                if legacy.source_id != source_id || legacy.appid != appid {
                    return Err(migration_failed());
                }
                let game = games.entry(appid).or_insert_with(|| GameMatchRecord {
                    schema_version: GAME_MATCH_SCHEMA_VERSION,
                    appid,
                    name: String::new(),
                    localized_name: None,
                    // The v1 record has no name/localized-name/subscription
                    // snapshot, so a faithful key cannot be rebuilt; the empty
                    // key forces the next match pass to recompute it.
                    input_key: String::new(),
                    status: GameMatchStatus::Pending,
                    sources: BTreeMap::new(),
                    fallback: None,
                    updated_at: 0,
                });
                let is_fallback = fallback_sources.contains(&source_id);
                let checked_at =
                    merge_legacy_discovery(game, &source_id, &legacy, index.as_ref(), is_fallback);
                game.updated_at = game.updated_at.max(checked_at);
            }
        }
        for (appid, mut game) in games {
            if game.name.is_empty() {
                game.name = legacy_game_name(&game).unwrap_or_else(|| appid.to_string());
            }
            game.status = derive_game_status(&game);
            migration(self.save_match(&game))?;
        }
        Ok(())
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
        )
    }

    pub fn paths(&self) -> &StorePaths {
        &self.paths
    }

    pub fn revision(&self) -> Result<u64> {
        self.settings_guard().map(|guard| guard.revision)
    }

    pub fn settings(&self) -> Result<SettingsFile> {
        self.settings_guard().map(|guard| guard.clone())
    }

    /// Reads `settings.json` from disk, falling back to the in-memory copy.
    pub fn load_settings(&self) -> Result<SettingsFile> {
        match read_json(&self.paths.settings())? {
            Some(settings) => Ok(settings),
            None => self.settings(),
        }
    }

    /// Persists the in-memory settings atomically.
    fn save_settings(&self) -> Result<()> {
        let settings = self.settings()?;
        write_json(&self.paths.settings(), &settings)
    }

    /// Commits an edit to the latest on-disk settings under the process/file
    /// lock. The in-memory snapshot changes only after the entire file saves.
    pub fn update_settings<T>(
        &self,
        edit: impl FnOnce(&mut SettingsFile) -> Result<T>,
    ) -> Result<T> {
        self.update_settings_with(edit, write_json)
    }

    fn update_settings_with<T>(
        &self,
        edit: impl FnOnce(&mut SettingsFile) -> Result<T>,
        persist: impl FnOnce(&Path, &SettingsFile) -> Result<()>,
    ) -> Result<T> {
        self.with_short_lock(|| {
            let mut memory = self.settings_guard()?;
            let mut next: SettingsFile =
                read_json(&self.paths.settings())?.unwrap_or_else(|| memory.clone());
            if !(1..=SOURCE_SETTINGS_SCHEMA_VERSION).contains(&next.schema_version)
                || next.revision == 0
            {
                return Err(invalid("invalid_source_schema"));
            }
            next.schema_version = SOURCE_SETTINGS_SCHEMA_VERSION;
            next.revision = next.revision.saturating_add(1);
            let result = edit(&mut next)?;
            persist(&self.paths.settings(), &next)?;
            *memory = next;
            Ok(result)
        })
    }

    /// Persists one automatic update interval (hours, `0` = off).
    pub fn set_interval(&self, source_id: &str, interval_hours: u64) -> Result<()> {
        self.update_settings(|settings| {
            settings
                .intervals
                .insert(source_id.to_string(), interval_hours);
            Ok(())
        })
    }

    /// Persists a display-name override without changing parsing identity.
    pub fn set_name(&self, source_id: &str, name: &str) -> Result<()> {
        self.update_settings(|settings| {
            settings
                .names
                .insert(source_id.to_string(), name.to_string());
            Ok(())
        })
    }

    /// Runs `f` while holding the independent `rust-v1/settings.lock`.
    ///
    /// Never uses `journal.lock` (design §4.1). `try_lock` with a 2s bound →
    /// `source_state_busy`; no lock is held during network activity.
    fn with_short_lock<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let _process = self.lock.lock().map_err(|_| invalid("source_state_busy"))?;
        let _file = self.acquire_file_lock()?;
        f()
    }

    // --- status / index / detail / match (p1-contract §4–§7) --------------

    pub fn load_status(&self, source_id: &str) -> Result<Option<SourceStatus>> {
        read_json(&self.paths.status(source_id))
    }

    pub fn save_status(&self, source_id: &str, status: &SourceStatus) -> Result<()> {
        write_json(&self.paths.status(source_id), status)
    }

    pub fn load_index(&self, source_id: &str) -> Result<Option<SourceIndex>> {
        read_json(&self.paths.index(source_id))
    }

    pub fn save_index(&self, index: &SourceIndex) -> Result<()> {
        write_json(&self.paths.index(&index.source_id), index)
    }

    /// Reads the unpublished update progress for a source (if an earlier
    /// attempt failed or was cancelled before completing).
    pub fn load_index_progress(&self, source_id: &str) -> Result<Option<SourceIndex>> {
        read_json(&self.paths.index_progress(source_id))
    }

    /// Atomically saves partial update progress. The official `index/<source>.json`
    /// is never touched here.
    pub fn save_index_progress(&self, index: &SourceIndex) -> Result<()> {
        write_json(&self.paths.index_progress(&index.source_id), index)
    }

    /// Removes the progress file after the official snapshot is published.
    /// A missing file is not an error.
    pub fn clear_index_progress(&self, source_id: &str) -> Result<()> {
        match fs::remove_file(self.paths.index_progress(source_id)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Reads the shared detail record for a source page, keyed by
    /// `detail_key(canonical_url(page))` so every game reuses one file.
    pub fn load_detail(&self, source_id: &str, page: &str) -> Result<Option<DetailRecord>> {
        let key = detail_key(&canonical::canonical_url(page));
        read_json(&self.paths.detail(source_id, &key))
    }

    pub fn save_detail(&self, record: &DetailRecord) -> Result<()> {
        let key = detail_key(&canonical::canonical_url(&record.page));
        write_json(&self.paths.detail(&record.source_id, &key), record)
    }

    pub fn load_match(&self, appid: u32) -> Result<Option<GameMatchRecord>> {
        read_json(&self.paths.matches(appid))
    }

    pub fn save_match(&self, record: &GameMatchRecord) -> Result<()> {
        write_json(&self.paths.matches(record.appid), record)
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
        schema_version: SOURCE_SETTINGS_SCHEMA_VERSION,
        revision: 1,
        enabled: BTreeMap::new(),
        added: Vec::new(),
        games: BTreeMap::new(),
        intervals: BTreeMap::new(),
        names: BTreeMap::new(),
        script_sources: BTreeMap::new(),
    }
}

// ---- schema 1 -> 2 migration helpers (p1-contract §8) --------------------

fn migration_failed() -> std::io::Error {
    invalid("source_migration_failed")
}

/// Every migration step reports the same specific code; old files stay intact.
fn migration<T>(result: Result<T>) -> Result<T> {
    result.map_err(|_| migration_failed())
}

fn json_stem(path: &Path) -> Result<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
        .ok_or_else(migration_failed)
}

/// Sorted `*.json` files directly inside `directory`; missing means empty.
fn json_files(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    match fs::read_dir(directory) {
        Ok(entries) => {
            for entry in entries {
                let path = entry?.path();
                if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
                    paths.push(path);
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    paths.sort();
    Ok(paths)
}

/// Sorted `(name, path)` for the immediate subdirectories of `directory`.
fn subdirectories(directory: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut found = Vec::new();
    match fs::read_dir(directory) {
        Ok(entries) => {
            for entry in entries {
                let path = entry?.path();
                if path.is_dir() {
                    let name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or_else(migration_failed)?
                        .to_string();
                    found.push((name, path));
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    found.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(found)
}

/// Merges a discovery-index view into an already-migrated cache view: the cache
/// snapshot keeps its coverage/time/validators, while discovery entries and the
/// continuation queue are preserved rather than dropped.
fn combine_indexes(mut cache: SourceIndex, discovered: SourceIndex) -> SourceIndex {
    for entry in discovered.entries {
        if !cache.entries.contains(&entry) {
            cache.entries.push(entry);
        }
    }
    cache.entries = normalize_entries(cache.entries);
    for page in discovered.pending {
        if !cache.pending.contains(&page) {
            cache.pending.push(page);
        }
    }
    cache.complete &= cache.pending.is_empty();
    cache.revision = 1;
    cache
}

/// Source ids whose configured rule is a fallback indexer (builtin + user added).
fn fallback_source_ids(settings: &SettingsFile) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    if let Ok(config) = serde_json::from_str::<RulesConfig>(DEFAULT_RULES_JSON) {
        for rule in config.sources {
            if rule.role.as_deref() == Some("fallback_indexer") {
                ids.insert(rule.id);
            }
        }
    }
    for rule in &settings.added {
        if rule.role == SourceRole::FallbackIndexer {
            ids.insert(rule.id.clone());
        }
    }
    ids
}

/// Folds one v1 per-game discovery record into the migrated game record and
/// returns the record's `checked_at` (for `updated_at`).
fn merge_legacy_discovery(
    game: &mut GameMatchRecord,
    source_id: &str,
    legacy: &DiscoveryRecord,
    index: Option<&SourceIndex>,
    is_fallback: bool,
) -> u64 {
    if is_fallback {
        let Some((checked_at, fallback)) = migrated_fallback(legacy) else {
            return 0;
        };
        if game
            .fallback
            .as_ref()
            .is_none_or(|existing| checked_at > existing.checked_at)
        {
            game.fallback = Some(fallback);
        }
        checked_at
    } else {
        let Some((checked_at, source)) = migrated_source_match(legacy, index) else {
            return 0;
        };
        game.sources.insert(source_id.to_string(), source);
        checked_at
    }
}

/// v1 discovery result → schema-2 source match. Candidates and fingerprints are
/// copied verbatim; the status only claims what the old data actually covered.
fn migrated_source_match(
    legacy: &DiscoveryRecord,
    index: Option<&SourceIndex>,
) -> Option<(u64, SourceMatchRecord)> {
    let success = legacy.success.as_ref();
    let attempt = legacy.attempt.as_ref();
    if success.is_none() && attempt.is_none() {
        return None;
    }
    let candidates = success
        .map(|success| success.candidates.clone())
        .or_else(|| attempt.map(|attempt| attempt.candidates.clone()))
        .unwrap_or_default();
    let diagnostics = success
        .map(|success| success.diagnostics.clone())
        .or_else(|| attempt.map(|attempt| attempt.diagnostics.clone()))
        .unwrap_or_default();
    let checked_at = success
        .map(|success| success.checked_at)
        .or_else(|| attempt.map(|attempt| attempt.checked_at))
        .unwrap_or(0);
    let status = match (success, index) {
        // No published index: the per-game cache cannot claim full coverage.
        (Some(_), None) => SourceMatchStatus::NoIndex,
        (Some(success), Some(index)) => {
            if candidates.iter().any(candidate_is_credible) {
                SourceMatchStatus::Matched
            } else if success.status == "complete" && index.complete {
                SourceMatchStatus::NoCandidate
            } else {
                SourceMatchStatus::DetailIncomplete
            }
        }
        (None, _) => match attempt.map(|attempt| attempt.status.as_str()) {
            Some("failed") => SourceMatchStatus::DetailFailed,
            _ => SourceMatchStatus::DetailIncomplete,
        },
    };
    let detail_pages = if success.is_some() {
        let mut pages: Vec<String> = Vec::new();
        for candidate in &candidates {
            let page = canonical::canonical_url(&candidate.source_page);
            if !pages.contains(&page) {
                pages.push(page);
            }
        }
        pages
    } else {
        Vec::new()
    };
    Some((
        checked_at,
        SourceMatchRecord {
            index_revision: index.map(|index| index.revision).unwrap_or(0),
            status,
            candidates,
            detail_pages,
            checked_at,
            diagnostics,
        },
    ))
}

/// v1 HGamer result → schema-2 fallback record (positive, completed-empty and
/// failed outcomes keep their meaning).
fn migrated_fallback(legacy: &DiscoveryRecord) -> Option<(u64, FallbackMatchRecord)> {
    let success = legacy.success.as_ref();
    let attempt = legacy.attempt.as_ref();
    let (query_key, candidates, diagnostics, checked_at, outcome) = match (success, attempt) {
        (Some(success), _) => (
            success.query_key.clone(),
            success.candidates.clone(),
            success.diagnostics.clone(),
            success.checked_at,
            success.status.as_str(),
        ),
        (None, Some(attempt)) => (
            attempt.query_key.clone(),
            attempt.candidates.clone(),
            attempt.diagnostics.clone(),
            attempt.checked_at,
            attempt.status.as_str(),
        ),
        (None, None) => return None,
    };
    let status = if candidates.iter().any(candidate_is_credible) {
        FallbackMatchStatus::Matched
    } else if outcome == "complete" {
        FallbackMatchStatus::NoMatch
    } else if outcome == "failed" {
        FallbackMatchStatus::Failed
    } else {
        FallbackMatchStatus::Incomplete
    };
    Some((
        checked_at,
        FallbackMatchRecord {
            query_key,
            status,
            candidates,
            checked_at,
            diagnostics,
        },
    ))
}

/// Best name evidence left by v1: a credible candidate's game title, else any
/// candidate's, else the AppID.
fn legacy_game_name(game: &GameMatchRecord) -> Option<String> {
    let mut fallback_title = None;
    let candidates = game
        .sources
        .values()
        .flat_map(|source| source.candidates.iter())
        .chain(
            game.fallback
                .iter()
                .flat_map(|fallback| fallback.candidates.iter()),
        );
    for candidate in candidates {
        if candidate_is_credible(candidate) {
            return Some(candidate.game_title.clone());
        }
        fallback_title = fallback_title.or_else(|| Some(candidate.game_title.clone()));
    }
    fallback_title
}

// ---- atomic primitives (design §4.2) -------------------------------------

/// Writes `bytes` to `path` atomically: parent chain link-checked,
/// same-directory `create_new` tmp + `sync_all` + `rename`, bounded
/// Windows-overwrite retry → `source_state_busy` (never remove-then-write).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
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

/// Reads a state file; missing → `None`, linked/non-regular →
/// `linked_source_state`, other IO errors propagate.
pub fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>> {
    crate::detection::read_bounded(path).map_err(|error| match error.to_string().as_str() {
        "evidence_linked_path" | "evidence_not_regular_file" => invalid("linked_source_state"),
        "evidence_changed" => invalid("source_state_changed"),
        _ => error,
    })
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match read_bounded(path)? {
        Some(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| invalid(error.to_string())),
        None => Ok(None),
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| invalid(error.to_string()))?;
    write_atomic(path, &bytes)
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
        assert_eq!(store.revision().unwrap(), 1);
        let bumped = store
            .update_settings(|settings| Ok(settings.revision))
            .unwrap();
        assert_eq!(bumped, 2);
        drop(store);

        let reopened = SourceStore::open(temp.path()).unwrap();
        assert_eq!(reopened.revision().unwrap(), 2);
        assert_eq!(reopened.load_settings().unwrap().revision, 2);
    }

    #[test]
    fn first_open_initializes_rust_default_state() {
        let temp = tempfile::tempdir().unwrap();
        let store = SourceStore::open(temp.path()).unwrap();
        assert_eq!(store.revision().unwrap(), 1);
        assert!(store.paths().schema().exists());
        assert!(store.paths().settings().exists());
        let settings = store.load_settings().unwrap();
        assert_eq!(settings.revision, 1);
        assert_eq!(settings.schema_version, SOURCE_SETTINGS_SCHEMA_VERSION);
        assert!(settings.added.is_empty());
        assert!(settings.enabled.is_empty());
        assert!(settings.games.is_empty());
        drop(store);

        // Reopen reads the persisted settings (schema marker is present).
        let reopened = SourceStore::open(temp.path()).unwrap();
        assert_eq!(reopened.revision().unwrap(), 1);
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
        store
            .update_settings(|settings| {
                settings.enabled = overrides.enabled;
                settings.added = overrides.added;
                settings.games = overrides.games;
                Ok(())
            })
            .unwrap();
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
    fn lock_error_propagates_instead_of_default_settings() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(SourceStore::open(temp.path()).unwrap());
        let before = fs::read(store.paths().settings()).unwrap();

        // Poison the settings mutex: a thread panics while holding the guard.
        let poisoned = Arc::clone(&store);
        let _ = std::thread::spawn(move || {
            let _guard = poisoned.settings.lock().unwrap();
            panic!("poison the settings mutex");
        })
        .join();

        // A poisoned lock must surface the real error, not a fake version 0 or
        // a default settings snapshot.
        assert_eq!(
            store.revision().unwrap_err().to_string(),
            "source_state_busy"
        );
        assert_eq!(
            store.settings().unwrap_err().to_string(),
            "source_state_busy"
        );
        // Saving must not write the default settings over the intact file.
        assert_eq!(
            store.save_settings().unwrap_err().to_string(),
            "source_state_busy"
        );
        assert_eq!(fs::read(store.paths().settings()).unwrap(), before);
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
        let settings = store.settings().unwrap();
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
        assert!(store.load_status("patchmoe").unwrap().is_none());
        assert!(store.load_index("patchmoe").unwrap().is_none());
        assert!(store.load_match(42).unwrap().is_none());
        // Legacy read paths are not created in the Rust namespace either.
        assert!(!store.paths().cache("patchmoe").exists());
        assert!(!store.paths().discovery("patchmoe", 42).exists());
    }

    #[test]
    fn atomic_replace_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("value.json");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second".to_vec());
    }

    #[test]
    fn atomic_replace_writes_full_bytes_and_leaves_no_tmp() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("value.json");
        write_atomic(&path, b"original").unwrap();
        // No fixed byte cap: a large state file is written in full.
        let large = vec![0u8; 8 * 1024 * 1024];
        write_atomic(&path, &large).unwrap();
        assert_eq!(fs::read(&path).unwrap(), large);
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
            write_atomic(&dir_target, b"x").unwrap_err().to_string(),
            "linked_source_state"
        );
        assert_eq!(
            read_bounded(&dir_target).unwrap_err().to_string(),
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
                read_bounded(&link).unwrap_err().to_string(),
                "linked_source_state"
            );
            assert_eq!(
                write_atomic(&link, b"new").unwrap_err().to_string(),
                "linked_source_state"
            );
        }
    }

    #[test]
    fn validate_entries_accepts_large_lists_and_rejects_empty() {
        let rule = builtins().into_iter().find(|r| r.id == "patchmoe").unwrap();
        // No entry-count ceiling and no byte cap: a large but otherwise valid
        // entry list is accepted.
        let entries = vec![entry("G", "https://download.patch.moe/G.zip", Some(1)); 4097];
        assert_eq!(
            validate::validate_entries(&rule, &entries).unwrap().len(),
            4097
        );
        assert_eq!(
            validate::validate_entries(&rule, &[])
                .unwrap_err()
                .to_string(),
            "empty_or_oversized_source"
        );
    }

    #[test]
    fn short_lock_serializes_revision_bumps() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(SourceStore::open(temp.path()).unwrap());
        let start = store.revision().unwrap();
        let mut handles = Vec::new();
        for _ in 0..4 {
            let store = Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                store
                    .update_settings(|settings| Ok(settings.revision))
                    .unwrap()
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(store.revision().unwrap(), start + 4);
    }

    #[test]
    fn failed_full_settings_write_does_not_publish_partial_memory_or_disk() {
        let temp = tempfile::tempdir().unwrap();
        let store = SourceStore::open(temp.path()).unwrap();
        let before = store.settings().unwrap();
        let bytes = fs::read(store.paths().settings()).unwrap();
        let code = "return {}";
        let mut rule = builtins().remove(0);
        rule.id = "custom".into();
        rule.adapter = "lua".into();
        rule.script =
            Some(crate::sources::script::validate_code(&rule.id, 1, code.as_bytes()).unwrap());
        let expected = rule.clone();
        let error = store
            .update_settings_with(
                |settings| {
                    settings.added.push(rule);
                    settings.script_sources.insert("custom".into(), code.into());
                    Ok(())
                },
                |path, next| {
                    assert_eq!(path, store.paths().settings());
                    assert_eq!(next.added, [expected]);
                    assert_eq!(next.script_sources["custom"], code);
                    assert_eq!(next.revision, before.revision + 1);
                    assert_eq!(next.schema_version, SOURCE_SETTINGS_SCHEMA_VERSION);
                    Err(invalid("injected_settings_write_failure"))
                },
            )
            .unwrap_err();
        assert_eq!(error.to_string(), "injected_settings_write_failure");
        assert_eq!(store.settings().unwrap(), before);
        assert_eq!(fs::read(store.paths().settings()).unwrap(), bytes);
    }
}
