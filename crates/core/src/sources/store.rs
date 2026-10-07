//! Persistent `rust-v1` store: atomic writes, short lock + monotonic revision,
//! cache/status/discovery/index persistence, one-shot Python import and the
//! dual-engine checkpoint model (design §4.1–§4.6, §5, R6/R9/R12).
//!
//! The store never writes to `<state_dir>/sources/` (the Python namespace); it
//! only reads it once during [`SourceStore::import_python_state`].

use super::model::{
    Diagnostic, DiscoveryIndexRecord, DiscoveryRecord, Overrides, RawSourceRule, SettingsFile,
    SourceCache, SourceRule, SourceStatus,
};
use super::validate;
use super::{
    invalid, Result, MAX_CACHE_BYTES, MAX_DISCOVERY_OUTPUT, MAX_OVERRIDES_BYTES, MAX_STATUS_BYTES,
    PARSER_VERSION, STORE_SCHEMA_VERSION,
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
/// Ceiling used when publishing staged files of any kind.
const MAX_STORE_FILE: u64 = 8 * 1024 * 1024;
/// Ceiling for the persisted first-open [`ImportReport`].
const MAX_IMPORT_REPORT_BYTES: u64 = 512 * 1024;
const SHORT_LOCK_TIMEOUT: Duration = Duration::from_secs(2);
const WRITE_RETRY_DELAY: Duration = Duration::from_millis(20);
const WRITE_RETRIES: u32 = 5;

/// Namespace layout (design §4.1).
#[derive(Debug, Clone)]
pub struct StorePaths {
    python_root: PathBuf,
    root: PathBuf,
}

impl StorePaths {
    pub fn new(state_dir: &Path) -> Self {
        let python_root = state_dir.join("sources");
        let root = python_root.join("rust-v1");
        Self { python_root, root }
    }

    pub fn python_root(&self) -> &Path {
        &self.python_root
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

    /// Persisted first-open import summary (P5 §1, `get_source_state`).
    pub fn import_report(&self) -> PathBuf {
        self.root.join("import-report.json")
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

    // --- read-only Python namespace ---------------------------------------

    pub fn python_overrides(&self) -> PathBuf {
        self.python_root.join("overrides.json")
    }

    pub fn python_cache(&self, source_id: &str) -> PathBuf {
        self.python_root
            .join("cache")
            .join(format!("{source_id}.json"))
    }

    pub fn python_status(&self, source_id: &str) -> PathBuf {
        self.python_root
            .join("status")
            .join(format!("{source_id}.json"))
    }

    pub fn python_discovery_dir(&self, source_id: &str) -> PathBuf {
        self.python_root.join("discovery").join(source_id)
    }

    pub fn python_discovery(&self, source_id: &str, appid: u32) -> PathBuf {
        self.python_discovery_dir(source_id)
            .join(format!("{appid}.json"))
    }

    pub fn python_discovery_index(&self, source_id: &str) -> PathBuf {
        self.python_root
            .join("discovery-index")
            .join(format!("{source_id}.json"))
    }
}

/// Result of one import pass (design §4.5).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportReport {
    pub imported_settings: bool,
    pub imported_cache: u64,
    pub imported_discovery: u64,
    pub imported_index: u64,
    pub diagnostics: Vec<Diagnostic>,
    pub revision: u64,
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

/// Raw shape of the Python `overrides.json` (added rules still need validation).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOverrides {
    #[serde(default)]
    enabled: BTreeMap<String, bool>,
    #[serde(default)]
    added: Vec<RawSourceRule>,
    #[serde(default)]
    games: BTreeMap<String, BTreeMap<String, bool>>,
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
    /// Opens (and on first use imports) the store. `builtins` are only needed for
    /// the one-shot Python import.
    pub fn open(state_dir: &Path, builtins: &[SourceRule]) -> Result<Self> {
        let paths = StorePaths::new(state_dir);
        ensure_directory(paths.root())?;
        let store = Self {
            paths,
            settings: Mutex::new(default_settings()),
            lock: Mutex::new(()),
        };
        if store.paths.schema().exists() {
            let settings = store.load_settings()?;
            *store.settings_guard()? = settings;
        } else {
            let report = store.import_python_state(builtins)?;
            let _ = report;
        }
        Ok(store)
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

    // --- first-open import report -----------------------------------------

    /// Reads the persisted first-open [`ImportReport`], or `None` when the file
    /// was never written (P5 §1; `get_source_state`).
    pub fn load_import_report(&self) -> Result<Option<ImportReport>> {
        read_json(&self.paths.import_report(), MAX_IMPORT_REPORT_BYTES)
    }

    /// Atomically persists the first-open [`ImportReport`].
    pub fn save_import_report(&self, report: &ImportReport) -> Result<()> {
        write_json(&self.paths.import_report(), report, MAX_IMPORT_REPORT_BYTES)
    }

    // --- import -----------------------------------------------------------

    /// One-shot import of the Python namespace (design §4.5). Idempotent via the
    /// `schema.json` completion marker; each published file is written with the
    /// atomic tmp+rename primitive and `schema.json` is written last.
    pub fn import_python_state(&self, builtins: &[SourceRule]) -> Result<ImportReport> {
        if self.paths.schema().exists() {
            return Ok(ImportReport {
                revision: self.revision(),
                ..ImportReport::default()
            });
        }
        let mut report = ImportReport::default();

        let mut enabled = BTreeMap::new();
        let mut added: Vec<SourceRule> = Vec::new();
        let mut games = BTreeMap::new();
        let overrides_path = self.paths.python_overrides();
        if overrides_path.exists() {
            let bytes = read_bounded(&overrides_path, MAX_OVERRIDES_BYTES)
                .map_err(|_| invalid("invalid_source_overrides"))?
                .ok_or_else(|| invalid("invalid_source_overrides"))?;
            let raw: RawOverrides =
                serde_json::from_slice(&bytes).map_err(|_| invalid("invalid_source_overrides"))?;
            enabled = raw.enabled;
            games = raw.games;
            for raw_rule in &raw.added {
                added.push(validate::validate_rule(raw_rule)?);
            }
            validate::validate_overrides(&Overrides {
                enabled: enabled.clone(),
                added: added.clone(),
                games: games.clone(),
            })?;
        }

        let settings = SettingsFile {
            schema_version: STORE_SCHEMA_VERSION,
            revision: 1,
            enabled,
            added,
            games,
        };

        let staging = self
            .paths
            .root()
            .join(format!(".import-{}", uuid::Uuid::new_v4().simple()));
        ensure_directory(&staging)?;
        write_json(
            &staging.join("settings.json"),
            &settings,
            MAX_SETTINGS_BYTES,
        )?;
        report.imported_settings = true;

        let mut rules: Vec<SourceRule> = builtins
            .iter()
            .filter(|rule| !settings.added.iter().any(|added| added.id == rule.id))
            .cloned()
            .collect();
        rules.extend(settings.added.iter().cloned());

        for rule in &rules {
            if rule.discovery_only {
                continue;
            }
            self.import_cache(rule, &staging, &mut report)?;
        }
        for rule in &rules {
            self.import_status(rule, &staging)?;
        }
        for rule in &rules {
            if rule.discovery_only {
                self.import_discovery(rule, &staging, &mut report)?;
            }
        }
        for rule in &rules {
            if rule.discovery_only {
                self.import_index(rule, &staging, &mut report)?;
            }
        }

        publish_tree(&staging, self.paths.root())?;
        let _ = fs::remove_dir_all(&staging);

        // Persist the report before the `schema.json` completion marker so a
        // crash between the two re-runs the import (idempotency unchanged).
        report.revision = 1;
        self.save_import_report(&report)?;

        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);
        write_atomic(
            &self.paths.schema(),
            format!("{{\"schema_version\":{STORE_SCHEMA_VERSION},\"created_at\":{created_at}}}")
                .as_bytes(),
            MAX_STATUS_BYTES,
        )?;

        *self.settings_guard()? = settings;
        Ok(report)
    }

    fn import_cache(
        &self,
        rule: &SourceRule,
        staging: &Path,
        report: &mut ImportReport,
    ) -> Result<()> {
        let path = self.paths.python_cache(&rule.id);
        let bytes = match read_bounded(&path, MAX_CACHE_BYTES) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Ok(()),
            Err(_) => {
                report.diagnostics.push(invalid_cache_diagnostic(&rule.id));
                return Ok(());
            }
        };
        let cache: SourceCache = match serde_json::from_slice(&bytes) {
            Ok(cache) => cache,
            Err(_) => {
                report.diagnostics.push(invalid_cache_diagnostic(&rule.id));
                return Ok(());
            }
        };
        match validate::validate_entries(rule, &cache.entries) {
            Ok(entries) => {
                let clean = SourceCache {
                    entries,
                    etag: cache.etag,
                    modified: cache.modified,
                    cached_at: cache.cached_at,
                    checked_at: cache.checked_at,
                };
                write_json(
                    &staging.join("cache").join(format!("{}.json", rule.id)),
                    &clean,
                    MAX_CACHE_BYTES,
                )?;
                report.imported_cache += 1;
            }
            Err(_) => report.diagnostics.push(invalid_cache_diagnostic(&rule.id)),
        }
        Ok(())
    }

    fn import_status(&self, rule: &SourceRule, staging: &Path) -> Result<()> {
        let path = self.paths.python_status(&rule.id);
        if let Ok(Some(bytes)) = read_bounded(&path, MAX_STATUS_BYTES) {
            if let Ok(status) = serde_json::from_slice::<SourceStatus>(&bytes) {
                write_json(
                    &staging.join("status").join(format!("{}.json", rule.id)),
                    &status,
                    MAX_STATUS_BYTES,
                )?;
            }
        }
        Ok(())
    }

    fn import_discovery(
        &self,
        rule: &SourceRule,
        staging: &Path,
        report: &mut ImportReport,
    ) -> Result<()> {
        let dir = self.paths.python_discovery_dir(&rule.id);
        let Ok(entries) = fs::read_dir(&dir) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Some(appid) = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.parse::<u32>().ok())
            else {
                continue;
            };
            let bytes = match read_bounded(&path, MAX_DISCOVERY_OUTPUT) {
                Ok(Some(bytes)) => bytes,
                _ => {
                    report
                        .diagnostics
                        .push(invalid_discovery_diagnostic(&rule.id));
                    continue;
                }
            };
            let mut record: DiscoveryRecord = match serde_json::from_slice(&bytes) {
                Ok(record) => record,
                Err(_) => {
                    report
                        .diagnostics
                        .push(invalid_discovery_diagnostic(&rule.id));
                    continue;
                }
            };
            if record.schema_version != 1 || record.appid != appid || record.source_id != rule.id {
                report
                    .diagnostics
                    .push(invalid_discovery_diagnostic(&rule.id));
                continue;
            }
            let valid = record
                .success
                .as_ref()
                .map(|success| {
                    validate::validate_candidates(rule, appid, &success.candidates).is_ok()
                })
                .unwrap_or(true)
                && record
                    .attempt
                    .as_ref()
                    .map(|attempt| {
                        validate::validate_candidates(rule, appid, &attempt.candidates).is_ok()
                    })
                    .unwrap_or(true);
            if !valid {
                report
                    .diagnostics
                    .push(invalid_discovery_diagnostic(&rule.id));
                continue;
            }
            // Imported success is a stale seed, never fresh (R9).
            downgrade_record(&mut record);
            write_json(
                &staging
                    .join("discovery")
                    .join(&rule.id)
                    .join(format!("{appid}.json")),
                &record,
                MAX_DISCOVERY_OUTPUT,
            )?;
            report.imported_discovery += 1;
        }
        Ok(())
    }

    fn import_index(
        &self,
        rule: &SourceRule,
        staging: &Path,
        report: &mut ImportReport,
    ) -> Result<()> {
        let path = self.paths.python_discovery_index(&rule.id);
        if let Ok(Some(bytes)) = read_bounded(&path, MAX_DISCOVERY_OUTPUT) {
            if let Ok(record) = serde_json::from_slice::<DiscoveryIndexRecord>(&bytes) {
                if record.schema_version == 1 && record.index.parser_version == PARSER_VERSION {
                    write_json(
                        &staging
                            .join("discovery-index")
                            .join(format!("{}.json", rule.id)),
                        &record,
                        MAX_DISCOVERY_OUTPUT,
                    )?;
                    report.imported_index += 1;
                }
            }
        }
        Ok(())
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

fn invalid_cache_diagnostic(source_id: &str) -> Diagnostic {
    Diagnostic {
        source_id: Some(source_id.to_string()),
        code: "invalid_source_cache".to_string(),
        detail: None,
    }
}

fn invalid_discovery_diagnostic(source_id: &str) -> Diagnostic {
    Diagnostic {
        source_id: Some(source_id.to_string()),
        code: "invalid_discovery_cache".to_string(),
        detail: None,
    }
}

fn downgrade_record(record: &mut DiscoveryRecord) {
    if let Some(success) = &mut record.success {
        success.checked_at = 0;
        success.cached_at = None;
    }
    if let Some(attempt) = &mut record.attempt {
        attempt.checked_at = 0;
    }
}

/// Moves every file from `staging` into `root`, creating directories as needed.
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
                    write_atomic(&to, &bytes, MAX_STORE_FILE)?;
                    let _ = fs::remove_file(&from);
                }
            }
        }
    }
    Ok(())
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
    use std::sync::Arc;

    fn builtins() -> Vec<SourceRule> {
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
        let store = SourceStore::open(temp.path(), &builtins()).unwrap();
        assert_eq!(store.revision(), 1);
        let bumped = store.bump_revision().unwrap();
        assert_eq!(bumped, 2);
        drop(store);

        let reopened = SourceStore::open(temp.path(), &builtins()).unwrap();
        assert_eq!(reopened.revision(), 2);
        assert_eq!(reopened.load_settings().unwrap().revision, 2);
    }

    #[test]
    fn import_report_persists_on_first_open_only() {
        let temp = tempfile::tempdir().unwrap();
        let store = SourceStore::open(temp.path(), &builtins()).unwrap();
        let report = store
            .load_import_report()
            .unwrap()
            .expect("first open persists the import report");
        assert!(report.imported_settings);
        assert_eq!(report.revision, 1);
        drop(store);

        // Reopen is idempotent (schema exists) and keeps the same report.
        let reopened = SourceStore::open(temp.path(), &builtins()).unwrap();
        assert_eq!(reopened.load_import_report().unwrap(), Some(report));
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
        let store = Arc::new(SourceStore::open(temp.path(), &builtins()).unwrap());
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
        let store = SourceStore::open(temp.path(), &builtins()).unwrap();
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
