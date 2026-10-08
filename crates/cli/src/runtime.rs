use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
use steam_patch_core::{
    catalog::{self, Catalog, Platform, SupportLevel},
    download, install, installer, sources, steam,
    trust::{self, VerifiedCatalog},
};

#[derive(Debug, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Params {
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    pub catalog: Option<PathBuf>,
    pub platform: Option<Platform>,
    pub instance_id: Option<String>,
    /// AppID alternative to `instance_id` for game-scoped source methods
    /// (P5 §1; resolution priority `instance_id` → `appid`).
    pub appid: Option<u32>,
    pub release_id: Option<String>,
    pub signed_catalog: Option<PathBuf>,
    pub trust_keys: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub archive: Option<PathBuf>,
    pub plan_id: Option<String>,
    /// Dynamic content-route selection (`exe-selection-*`), mutually exclusive
    /// with `release_id` (p1-host-contract §1.1 A).
    pub selection_id: Option<String>,
    #[serde(default)]
    pub operation: install::InstallOperation,
    pub request_id: Option<String>,
    pub source_id: Option<String>,
    pub enabled: Option<bool>,
    pub subscribed: Option<bool>,
    /// Declarative rule for `add_source` (validated against `RawSourceRule`).
    pub rule: Option<Value>,
    #[serde(default)]
    pub name_hints: Vec<String>,
    pub preferred_language: Option<String>,
    #[serde(default)]
    pub force: bool,
    pub generation: Option<u64>,
    pub record_id: Option<String>,
}

/// v3 method whitelist (p1-host-contract §1.1): the 8 installer/task methods
/// plus the 8 source methods. Deleted v2 methods are intentionally absent.
pub const METHODS: &[&str] = &[
    // Installer/task face.
    "get_capabilities",
    "scan",
    "preview",
    "install_patch",
    "list_transactions",
    "recover",
    "discard_plan",
    "cleanup_cache",
    // Source face.
    "list_sources",
    "list_game_sources",
    "set_source_enabled",
    "set_game_source",
    "add_source",
    "refresh_sources",
    "scan_sources",
    "discover_game",
];

/// Installer/task methods advertised by `get_capabilities.installer`-adjacent
/// `methods` (p1-host-contract §1.1 A).
pub const INSTALLER_METHODS: &[&str] = &[
    "get_capabilities",
    "scan",
    "preview",
    "install_patch",
    "list_transactions",
    "recover",
    "discard_plan",
    "cleanup_cache",
];

/// Source methods advertised by `get_capabilities.sources.methods`
/// (p1-host-contract §1.1 B).
pub const SOURCE_METHODS: &[&str] = &[
    "list_sources",
    "list_game_sources",
    "set_source_enabled",
    "set_game_source",
    "add_source",
    "refresh_sources",
    "scan_sources",
    "discover_game",
];

/// Streaming source-task methods that emit `source_progress`/`source_update`.
pub const SOURCE_STREAM_METHODS: &[&str] = &["scan_sources", "refresh_sources"];

/// Per-task cursor that renders `operation::Progress` into the flat v3
/// `install_progress` event (p1-host-contract §2.2). The cursor owns the
/// per-task `sequence`, which starts at 0 and increments strictly per event.
pub struct ProgressCursor {
    pub protocol_version: u32,
    pub request_id: String,
    pub instance_id: Option<String>,
    pub selection_id: Option<String>,
    pub sequence: u64,
}

impl ProgressCursor {
    pub fn new(
        protocol_version: u32,
        request_id: String,
        instance_id: Option<String>,
        selection_id: Option<String>,
    ) -> Self {
        Self {
            protocol_version,
            request_id,
            instance_id,
            selection_id,
            sequence: 0,
        }
    }

    /// Renders one event and advances the monotonic sequence. Internal apply
    /// substates fold into the host-visible `applying` stage + diagnostic
    /// `substage` (p1-host-contract §2.1).
    pub fn event(&mut self, progress: &steam_patch_core::operation::Progress) -> Value {
        let (stage, substage) = match progress.stage.as_str() {
            "backing_up" | "writing" | "committing" => ("applying", Some(progress.stage.as_str())),
            other => (other, None),
        };
        let event = json!({
            "event": "install_progress",
            "protocol_version": self.protocol_version,
            "request_id": self.request_id,
            "sequence": self.sequence,
            "instance_id": self.instance_id,
            "selection_id": self.selection_id,
            "stage": stage,
            "substage": substage,
            "bytes_done": progress.bytes_done,
            "total_bytes": progress.total_bytes,
            "files_done": progress.files_done,
            "total_files": progress.total_files,
            "current_file": &progress.current_file,
            "archive_bytes": progress.archive_bytes,
        });
        self.sequence += 1;
        event
    }
}

/// `get_capabilities` v3 payload (p1-host-contract §3.1): installer and source
/// sub-objects merged with the full 16-method whitelist.
fn capabilities(_params: &Params) -> Value {
    let availability = exe_sandbox_availability();
    let mut methods: Vec<&str> = Vec::with_capacity(METHODS.len());
    methods.extend(INSTALLER_METHODS.iter().copied());
    methods.extend(SOURCE_METHODS.iter().copied());
    json!({
        "protocol_version": 3,
        "installer": {
            "schema_version": 1,
            "selection_version": 1,
            "content_version": 1,
            "adapter_version": 1,
            "policy_schema_version": 1,
            "report_schema_version": 1,
            "formats": { "zip": "supported", "exe": "supported" },
            "strategies": ["managed_zip", "publisher_zip", "publisher_exe"],
            "links": {
                "zip": { "available": installer::host_identity_blocker().is_none(),
                    "blockers": installer::host_identity_blocker().map(|code|json!([{"code":code}])).unwrap_or_else(||json!([])) },
                "exe": {"available": availability.available, "blockers": if availability.available {json!([])}else{
                    json!([{"code":"exe_sandbox_unavailable","detail":availability.reason}])}},
            },
        },
        "sources": {
            "discovery_schema_version": 1,
            "adapter_version": 1,
            "parser_version": sources::PARSER_VERSION,
            "query_version": sources::QUERY_VERSION,
            "methods": SOURCE_METHODS,
        },
        "methods": methods,
    })
}

/// The bundled worker next to the running host executable. Only the trusted
/// host fixes the executable location; RPC cannot set it.
fn bundled_worker() -> Option<PathBuf> {
    let host = std::env::current_exe().ok()?;
    Some(host.parent()?.join(if cfg!(windows) {
        "sf-worker.exe"
    } else {
        "sf-worker"
    }))
}

fn worker_runtime(stage: &Path) -> Result<steam_patch_core::installer::WorkerRuntime, String> {
    state_safe(stage)?;
    fs::create_dir_all(stage).map_err(err)?;
    state_safe(stage)?;
    let worker = bundled_worker().ok_or("bundled_worker_missing")?;
    installer::WorkerRuntime::new(worker, stage, None)
        .map_err(|e| failed("exe_sandbox_unavailable", e.to_string()))
}

/// Process-cached EXE sandbox availability for read-only callers
/// (`get_capabilities`/`scan`/`preview`). The probe really executes the bundled
/// worker, but in a private OS-temp staging root that is removed afterwards, so
/// read-only methods never create or write the installer state (p0-contracts
/// §4.1). The install route re-probes at runtime with its state-rooted staging.
fn exe_sandbox_availability() -> installer::SandboxAvailability {
    if let Some(reason) = installer::host_identity_blocker() {
        return installer::SandboxAvailability {
            available: false,
            reason: reason.into(),
        };
    }
    let Some(worker) = bundled_worker() else {
        return installer::SandboxAvailability {
            available: false,
            reason: "bundled_worker_missing".into(),
        };
    };
    let key = match fs::metadata(&worker) {
        Ok(metadata) => format!(
            "{}|{}|{}",
            worker.display(),
            metadata.len(),
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|age| age.as_nanos())
                .unwrap_or(0)
        ),
        Err(error) => {
            return installer::SandboxAvailability {
                available: false,
                reason: format!("bundled_worker_missing: {error}"),
            };
        }
    };
    type ProbeCache = std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, installer::SandboxAvailability>>,
    >;
    static CACHE: ProbeCache = std::sync::OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap();
    if let Some(hit) = cache.get(&key).cloned() {
        return hit;
    }
    let availability = probe_in_temp_staging(&worker);
    cache.insert(key, availability.clone());
    availability
}

/// One real probe in a private temp staging root (never the installer state).
fn probe_in_temp_staging(worker: &Path) -> installer::SandboxAvailability {
    let root = std::env::temp_dir().join(format!("steam-patch-probe-{}", std::process::id()));
    let availability = match fs::create_dir_all(&root).map_err(err).and_then(|_| {
        installer::WorkerRuntime::new(worker, &root, None)
            .map_err(|e| failed("exe_sandbox_unavailable", e.to_string()))
    }) {
        Ok(runtime) => runtime.probe(),
        Err(reason) => installer::SandboxAvailability {
            available: false,
            reason,
        },
    };
    let _ = fs::remove_dir_all(&root);
    availability
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn value(data: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(data).map_err(err)
}
fn host_platform() -> Platform {
    if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Steamos
    }
}
fn state(params: &Params) -> Result<PathBuf, String> {
    params
        .state_dir
        .as_ref()
        .map(|p| std::path::absolute(p).map_err(err))
        .unwrap_or_else(|| Err("state_dir_required".into()))
}

fn state_safe(path: &Path) -> Result<(), String> {
    for component in path.ancestors() {
        if component.as_os_str().is_empty() {
            continue;
        }
        match fs::symlink_metadata(component) {
            Ok(meta) => {
                #[cfg(windows)]
                let linked = {
                    use std::os::windows::fs::MetadataExt;
                    meta.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let linked = meta.file_type().is_symlink();
                if linked {
                    return Err("linked_state_path".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(err(e)),
        }
    }
    Ok(())
}

fn highest_revision(state: Option<&Path>) -> Result<u64, String> {
    let Some(state) = state else {
        return Ok(0);
    };
    let path = state.join("trust.sqlite");
    state_safe(&path)?;
    if !path.exists() {
        return Ok(0);
    }
    let conn =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(err)?;
    conn.query_row(
        "SELECT revision FROM catalog_revision WHERE id=1",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map_err(err)
    .and_then(|r| u64::try_from(r).map_err(err))
}

fn remember_revision(state: Option<&Path>, revision: u64) -> Result<(), String> {
    let Some(state) = state else {
        return Ok(());
    };
    state_safe(state)?;
    fs::create_dir_all(state).map_err(err)?;
    for name in ["trust.sqlite", "trust.sqlite-journal"] {
        state_safe(&state.join(name))?;
    }
    let mut conn = rusqlite::Connection::open(state.join("trust.sqlite")).map_err(err)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(err)?;
    conn.execute_batch("PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS catalog_revision(id INTEGER PRIMARY KEY, revision INTEGER NOT NULL);").map_err(err)?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let current: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(revision),0) FROM catalog_revision",
            [],
            |row| row.get(0),
        )
        .map_err(err)?;
    let revision = i64::try_from(revision).map_err(err)?;
    if revision < current {
        return Err("catalog_rollback".into());
    }
    tx.execute("INSERT INTO catalog_revision VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision", [revision]).map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(())
}

fn signed(params: &Params) -> Result<VerifiedCatalog, String> {
    let path = params
        .signed_catalog
        .as_ref()
        .ok_or("signed_catalog_required")?;
    let keys = match &params.trust_keys {
        Some(path) => trust::load_keys(path).map_err(err)?,
        None => {
            serde_json::from_str(include_str!("../../../catalog/trust-keys.json")).map_err(err)?
        }
    };
    let catalog = VerifiedCatalog::load(
        path,
        &keys,
        trust::now().map_err(err)?,
        highest_revision(params.state_dir.as_deref())?,
    )
    .map_err(err)?;
    remember_revision(params.state_dir.as_deref(), catalog.catalog().revision)?;
    Ok(catalog)
}

fn ready_release<'a>(
    catalog: &'a VerifiedCatalog,
    release_id: &str,
    game: &steam::GameInstall,
    platform: Platform,
) -> Result<&'a catalog::PatchRelease, String> {
    catalog
        .ensure_active(release_id, trust::now().map_err(err)?)
        .map_err(err)?;
    let release = catalog
        .catalog()
        .releases
        .iter()
        .find(|r| r.release_id == release_id && r.appid == game.appid)
        .ok_or("release_not_matched")?;
    if !release.platforms.contains(&platform) {
        return Err("platform_not_verified".into());
    }
    if game.availability != "installed" {
        return Err("game_not_ready".into());
    }
    if !game
        .buildid
        .as_ref()
        .is_some_and(|b| release.verified_buildids.contains(b))
    {
        return Err("build_not_verified".into());
    }
    if release.support_level != SupportLevel::ManagedZip {
        return Err("source_requires_guidance".into());
    }
    Ok(release)
}

fn canonical_future(path: &Path) -> Result<PathBuf, String> {
    let absolute = std::path::absolute(path).map_err(err)?;
    let mut base = absolute.as_path();
    let mut suffix = Vec::new();
    while !base.exists() {
        suffix.push(base.file_name().ok_or("invalid_state_path")?.to_owned());
        base = base.parent().ok_or("invalid_state_path")?;
    }
    let mut result = fs::canonicalize(base).map_err(err)?;
    for part in suffix.into_iter().rev() {
        result.push(part);
    }
    Ok(result)
}

fn validate_state_boundary(state: &Path, installs: &[steam::GameInstall]) -> Result<(), String> {
    state_safe(state)?;
    let state = canonical_future(state)?;
    for game in installs {
        let game = canonical_future(&game.install_path)?;
        if state.starts_with(&game) || game.starts_with(&state) {
            return Err("state_overlaps_game".into());
        }
    }
    Ok(())
}

pub fn execute(method: &str, params: Params) -> Result<Value, String> {
    let cancel = AtomicBool::new(false);
    execute_controlled(method, params, &cancel, &always_active, &no_lease_deadline)
}

fn always_active(_now: u64) -> bool {
    true
}

/// No task channel: no persisted foreground lease deadline.
fn no_lease_deadline() -> u64 {
    u64::MAX
}

/// Encode a stable `code` plus optional `detail` for the protocol envelope.
/// `main.rs::respond` splits on the separator and fills `error.detail`.
const DETAIL_SEPARATOR: char = '\u{1}';

pub fn failed(code: impl Into<String>, detail: impl Into<String>) -> String {
    let code = code.into();
    let detail = detail.into();
    if detail.is_empty() {
        code
    } else {
        format!("{code}{DETAIL_SEPARATOR}{detail}")
    }
}

/// Executes one method with the task control channel available to
/// `install_patch` (p1-zip-orchestration §3.10/§4.1).
pub fn execute_controlled<L: Fn(u64) -> bool, D: Fn() -> u64 + Sync>(
    method: &str,
    params: Params,
    cancel: &AtomicBool,
    lease_active: &L,
    lease_deadline_ms: &D,
) -> Result<Value, String> {
    if !METHODS.contains(&method) {
        return Err("unsupported_method".into());
    }
    if params.roots.len() > 32 {
        return Err("too_many_roots".into());
    }
    if method == "scan" && (params.instance_id.is_some() || params.release_id.is_some()) {
        return Err("unexpected_preview_arguments".into());
    }
    if let Some(code) = installer::host_identity_blocker() {
        if [
            "install_patch",
            "apply",
            "recover",
            "discard_plan",
            "migrate_state",
            "cleanup_cache",
        ]
        .contains(&method)
        {
            return Err(code.into());
        }
    }
    if method == "get_capabilities" {
        return Ok(capabilities(&params));
    }
    if method == "cleanup_cache" {
        return cleanup_cache(&params);
    }
    let roots = if params.roots.is_empty() {
        steam::discover_roots()
    } else {
        params.roots.clone()
    };
    let report = steam::scan(&roots);
    if let Some(state) = &params.state_dir {
        validate_state_boundary(state, &report.installs)?;
    }
    if SOURCE_METHODS.contains(&method) {
        return execute_source_method(method, &params, &report);
    }
    if ["discard_plan", "list_transactions", "recover"].contains(&method) {
        let state = state(&params)?;
        state_safe(&state)?;
        let paths = installer::InstallerPaths::new(&state);
        installer::init_atomic(&paths).map_err(err)?;
        return match method {
            "discard_plan" => value(
                install::discard_plan_at(
                    paths.root(),
                    params.plan_id.as_deref().ok_or("plan_required")?,
                )
                .map_err(err)?,
            ),
            "list_transactions" => value(install::list_transactions_at(paths.root()).map_err(err)?),
            _ => value(install::recover_at(paths.root()).map_err(err)?),
        };
    }
    let platform = params.platform.unwrap_or_else(host_platform);
    if method == "install_patch" {
        return install_patch(
            &params,
            &report,
            platform,
            cancel,
            lease_active,
            lease_deadline_ms,
        );
    }
    let mut catalog_error = None;
    let verified = if params.signed_catalog.is_some() {
        match signed(&params) {
            Ok(c) => Some(c),
            Err(e) => {
                catalog_error = Some(e);
                None
            }
        }
    } else {
        None
    };
    let catalog = match verified.as_ref() {
        Some(c) => c.catalog().clone(),
        None => match params.catalog.as_ref() {
            Some(path) => match catalog::load(path) {
                Ok(c) => c,
                Err(e) => {
                    catalog_error = Some(err(e));
                    Catalog {
                        schema_version: 1,
                        revision: 0,
                        releases: vec![],
                    }
                }
            },
            None => Catalog {
                schema_version: 1,
                revision: 0,
                releases: vec![],
            },
        },
    };
    if method == "preview" {
        let game = report
            .installs
            .iter()
            .find(|i| Some(&i.instance_id) == params.instance_id.as_ref())
            .ok_or("instance_not_found")?;
        return preview_route(&params, game, &catalog, platform);
    }
    let transactions = params
        .state_dir
        .as_ref()
        .map(|state| install::list_transactions_at(installer::InstallerPaths::new(state).root()))
        .transpose()
        .map_err(err)?
        .unwrap_or_default();
    let source_handle = params
        .state_dir
        .as_ref()
        .filter(|state| state.join("sources").join("rust-v1").exists())
        .and_then(|state| sources::Sources::open(state).ok());
    let rows: Vec<_> = report.installs.iter().map(|game| {
        let mut patches = catalog::matches(game, &catalog, platform);
        let signed_ids: HashSet<String> =
            patches.iter().map(|patch| patch.release_id.clone()).collect();
        for patch in &mut patches {
            if verified.as_ref().is_some_and(|c| c.is_revoked(&patch.release_id)) { patch.blockers.push("release_revoked".into()); }
            let owned = transactions.iter().any(|t| t.instance_id == game.instance_id
                && t.release_id == patch.release_id && t.state == "committed");
            if owned && patch.detection.status == "installed_external" {
                patch.detection.status = "installed_managed".into();
            } else if owned && patch.detection.status == "not_detected" {
                patch.detection.status = "partial_or_conflict".into();
            }
        }
        if let Some(handle) = &source_handle {
            if let Some(state) = &params.state_dir {
                let root = installer::InstallerPaths::new(state).root().to_path_buf();
                for dynamic in dynamic_patches(
                    handle.registry(),
                    game,
                    &signed_ids,
                    &transactions,
                    &root,
                ) {
                    patches.push(dynamic);
                }
            }
        }
        let mut row = json!({"game":game,"match_status":if patches.is_empty(){"no_catalog_match"}else{"matched"},"patches":patches});
        if let Some(handle) = &source_handle {
            let query = game_query(game);
            if let Ok(summaries) = value(handle.registry().list_game_sources(&query)) {
                row["sources"] = summaries;
            }
            if let Ok(matches) = value(handle.registry().matches(&query, false)) {
                row["matches"] = matches;
            }
            if let Ok(discovery) = value(handle.registry().cached_discovery(&query)) {
                row["discovery"] = discovery;
            }
        }
        row
    }).collect();
    let mut result = json!({"catalog_revision":catalog.revision,"catalog_trust":if verified.is_some(){"signed"}else{"unsigned_development"},
        "catalog_error":catalog_error,"platform":platform,"roots":roots,"installs":rows,"diagnostics":report.diagnostics});
    if let Some(handle) = &source_handle {
        result["settings_revision"] = json!(handle.settings_revision());
        if let Ok(summaries) = value(handle.registry().list_sources()) {
            result["sources"] = summaries;
        }
    }
    Ok(result)
}

/// P1 `cleanup_cache`: sweeps only the cleanable installer roots (`content/`,
/// `downloads/`, `reports/`), never an active lease/receipt/journal,
/// `.stage`/`.backup` material or a `revoked` marker
/// (p0-contracts §3.2; p1-host-contract §1.1 A / §7.3).
fn cleanup_cache(params: &Params) -> Result<Value, String> {
    let state = state(params)?;
    state_safe(&state)?;
    let paths = installer::InstallerPaths::new(&state);
    installer::init_atomic(&paths).map_err(err)?;
    installer::task::cleanup_stale_tasks(&paths).map_err(|_| "cleanup_failed".to_string())?;
    let mut removed_bytes = 0_u64;
    for entry in paths.list_cache_entries().map_err(err)? {
        let metadata = fs::symlink_metadata(&entry).map_err(|_| "cleanup_failed".to_string())?;
        if metadata.is_dir() {
            removed_bytes += directory_size(&entry);
            fs::remove_dir_all(&entry).map_err(|_| "cleanup_failed".to_string())?;
        } else if metadata.file_type().is_file() {
            removed_bytes += metadata.len();
            fs::remove_file(&entry).map_err(|_| "cleanup_failed".to_string())?;
        }
    }
    Ok(json!({ "performed": true, "removed_bytes": removed_bytes }))
}

fn directory_size(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0_u64;
    for entry in entries.flatten() {
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if metadata.is_dir() {
            total = total.saturating_add(directory_size(&entry.path()));
        } else if metadata.file_type().is_file() {
            total = total.saturating_add(metadata.len());
        }
    }
    total
}

fn game_query(install: &steam::GameInstall) -> sources::GameQuery {
    sources::GameQuery {
        appid: install.appid,
        name: install.name.clone(),
        localized_name: install.localized_name.clone(),
        instance_id: Some(install.instance_id.clone()),
    }
}

fn install_for<'a>(
    report: &'a steam::ScanReport,
    instance_id: &str,
) -> Result<&'a steam::GameInstall, String> {
    report
        .installs
        .iter()
        .find(|game| game.instance_id == instance_id)
        .ok_or_else(|| "instance_not_found".to_string())
}

/// Builds a [`sources::GameQuery`] from `instance_id` (preferred) or `appid`.
///
/// For an `appid`-only request the trusted scan is consulted for the install
/// name; when the game is not installed the query carries only the AppID and
/// matching falls back to cache entries that carry it (P5 §1).
fn game_from_params(
    params: &Params,
    report: &steam::ScanReport,
) -> Result<sources::GameQuery, String> {
    if let Some(instance_id) = params.instance_id.as_deref() {
        return Ok(game_query(install_for(report, instance_id)?));
    }
    if let Some(appid) = params.appid {
        if let Some(install) = report.installs.iter().find(|game| game.appid == appid) {
            return Ok(game_query(install));
        }
        return Ok(sources::GameQuery {
            appid,
            name: String::new(),
            localized_name: None,
            instance_id: None,
        });
    }
    Err("instance_required".into())
}

/// Resolves the AppID from `instance_id` (preferred) or the numeric `appid`.
fn resolve_appid(params: &Params, report: &steam::ScanReport) -> Result<u32, String> {
    if let Some(instance_id) = params.instance_id.as_deref() {
        return Ok(install_for(report, instance_id)?.appid);
    }
    params.appid.ok_or_else(|| "instance_required".into())
}

fn open_sources(params: &Params) -> Result<sources::Sources, String> {
    let state = state(params)?;
    state_safe(&state)?;
    sources::Sources::open(&state).map_err(err)
}

fn discover_result(
    install: &steam::GameInstall,
    generation: Option<u64>,
    revision: u64,
    discovery: sources::GameDiscovery,
) -> Value {
    json!({
        "instance_id": install.instance_id.clone(),
        "appid": install.appid,
        "generation": generation,
        "settings_revision": revision,
        "status": discovery.status,
        "candidates": discovery.candidates,
        "diagnostics": discovery.diagnostics,
    })
}

fn execute_source_method(
    method: &str,
    params: &Params,
    report: &steam::ScanReport,
) -> Result<Value, String> {
    match method {
        "list_sources" => value(open_sources(params)?.registry().list_sources()),
        "list_game_sources" => {
            let game = game_from_params(params, report)?;
            value(open_sources(params)?.registry().list_game_sources(&game))
        }
        "set_source_enabled" => {
            let source_id = params.source_id.as_deref().ok_or("source_id_required")?;
            let enabled = params.enabled.ok_or("enabled_required")?;
            let mut handle = open_sources(params)?;
            handle
                .registry_mut()
                .set_enabled(source_id, enabled)
                .map_err(err)?;
            Ok(json!({
                "settings_revision": handle.settings_revision(),
                "source_id": source_id,
                "enabled": enabled,
            }))
        }
        "set_game_source" => {
            let appid = resolve_appid(params, report)?;
            let source_id = params.source_id.as_deref().ok_or("source_id_required")?;
            let subscribed = params.subscribed.ok_or("subscribed_required")?;
            let mut handle = open_sources(params)?;
            handle
                .registry_mut()
                .set_game_source(appid, source_id, subscribed)
                .map_err(err)?;
            Ok(json!({
                "settings_revision": handle.settings_revision(),
                "source_id": source_id,
                "subscribed": subscribed,
            }))
        }
        "add_source" => {
            let rule_value = params.rule.clone().ok_or("rule_required")?;
            let raw: sources::RawSourceRule = serde_json::from_value(rule_value).map_err(err)?;
            let mut handle = open_sources(params)?;
            let rule = handle.registry_mut().add_source(&raw).map_err(err)?;
            Ok(json!({
                "settings_revision": handle.settings_revision(),
                "rule": rule,
            }))
        }
        "refresh_sources" => refresh_sources_impl(params, &AtomicBool::new(false), &mut |_| ()),
        "discover_game" => {
            let install = install_for(
                report,
                params.instance_id.as_deref().ok_or("instance_required")?,
            )?;
            let mut handle = open_sources(params)?;
            let cancel = AtomicBool::new(false);
            let discovery = handle
                .discover_game(
                    &game_query(install),
                    &params.name_hints,
                    params.preferred_language.as_deref(),
                    params.force,
                    &cancel,
                )
                .map_err(err)?;
            Ok(discover_result(
                install,
                params.generation,
                handle.settings_revision(),
                discovery,
            ))
        }
        "scan_sources" => scan_sources_impl(params, &AtomicBool::new(false), &mut |_| ()),
        _ => Err("unsupported_method".into()),
    }
}

/// Streaming dispatch for `serve-task`: `scan_sources`/`refresh_sources` emit
/// source events through `emit` and then return the single final result.
pub fn execute_streaming(
    method: &str,
    params: Params,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Value),
) -> Result<Value, String> {
    match method {
        "scan_sources" => scan_sources_impl(&params, cancel, emit),
        "refresh_sources" => refresh_sources_impl(&params, cancel, emit),
        _ => execute(method, params),
    }
}

fn scan_sources_impl(
    params: &Params,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Value),
) -> Result<Value, String> {
    let state = state(params)?;
    state_safe(&state)?;
    let roots = if params.roots.is_empty() {
        steam::discover_roots()
    } else {
        params.roots.clone()
    };
    let report = steam::scan(&roots);
    validate_state_boundary(&state, &report.installs)?;
    let mut handle = sources::Sources::open(&state).map_err(err)?;
    let revision = handle.settings_revision();

    let mut seen = HashSet::new();
    let mut entries: Vec<(sources::GameQuery, String)> = Vec::new();
    for install in report
        .installs
        .iter()
        .filter(|game| game.availability == "installed")
    {
        if seen.insert(install.appid) {
            entries.push((game_query(install), install.instance_id.clone()));
        }
    }
    let games: Vec<sources::GameQuery> = entries.iter().map(|(query, _)| query.clone()).collect();
    let discoveries = handle
        .scan_sources(&games, params.force, cancel, |event| match event {
            sources::session::ScanEvent::Progress {
                completed,
                total,
                running,
            } => emit(json!({
                "event": "source_progress",
                "completed": completed,
                "total": total,
                "running": running,
                "settings_revision": revision,
            })),
            sources::session::ScanEvent::Update {
                appid,
                index,
                discovery,
            } => {
                let instance_id = entries
                    .iter()
                    .find(|(query, _)| query.appid == *appid)
                    .map(|(_, id)| id.clone());
                emit(json!({
                    "event": "source_update",
                    "appid": appid,
                    "index": index,
                    "instance_id": instance_id,
                    "settings_revision": revision,
                    "discovery": discovery.clone(),
                }));
            }
        })
        .map_err(err)?;
    let results: Vec<Value> = discoveries
        .iter()
        .enumerate()
        .map(|(index, discovery)| {
            json!({
                "instance_id": entries.get(index).map(|(_, id)| id.clone()),
                "appid": entries.get(index).map(|(query, _)| query.appid),
                "status": discovery.status,
                "candidates": discovery.candidates,
                "diagnostics": discovery.diagnostics,
            })
        })
        .collect();
    Ok(json!({"settings_revision": revision, "results": results}))
}

fn refresh_sources_impl(
    params: &Params,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Value),
) -> Result<Value, String> {
    let state = state(params)?;
    state_safe(&state)?;
    let mut handle = sources::Sources::open(&state).map_err(err)?;
    let revision = handle.settings_revision();
    let rule_ids: Vec<String> = handle
        .registry()
        .rules()
        .iter()
        .filter(|rule| !rule.discovery_only && rule.enabled)
        .map(|rule| rule.id.clone())
        .collect();
    let statuses = handle.refresh_sources(params.force, cancel).map_err(err)?;
    for (source_id, status) in rule_ids.iter().zip(statuses.iter()) {
        emit(json!({
            "event": "source_update",
            "source_id": source_id,
            "status": status.clone(),
            "settings_revision": revision,
        }));
    }
    Ok(json!({"settings_revision": revision, "statuses": statuses}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rollback_is_rechecked_inside_write_transaction() {
        let state = tempfile::tempdir().unwrap();
        remember_revision(Some(state.path()), 1).unwrap();
        assert_eq!(highest_revision(Some(state.path())).unwrap(), 1);
        remember_revision(Some(state.path()), 3).unwrap();
        assert_eq!(
            remember_revision(Some(state.path()), 2).unwrap_err(),
            "catalog_rollback"
        );
        assert_eq!(highest_revision(Some(state.path())).unwrap(), 3);
    }
    #[test]
    fn recover_and_plan_reject_game_state_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("steamapps/common/Test");
        fs::create_dir_all(&game).unwrap();
        fs::write(temp.path().join("steamapps/appmanifest_42.acf"),
            "\"AppState\" { \"appid\" \"42\" \"name\" \"Test\" \"installdir\" \"Test\" \"StateFlags\" \"4\" }").unwrap();
        for method in ["recover", "list_transactions"] {
            let result = execute(
                method,
                Params {
                    roots: vec![temp.path().into()],
                    state_dir: Some(game.clone()),
                    ..Default::default()
                },
            );
            assert_eq!(result.unwrap_err(), "state_overlaps_game");
            assert_eq!(fs::read_dir(&game).unwrap().count(), 0);
        }
    }
}

/// Single-task install entry point (p1-host-contract §1.1 A; p1-zip-orchestration
/// §4.1). Validates mutual exclusion and dispatches to the signed `release_id`
/// route (priority) or the dynamic `selection_id` prefix route.
fn install_patch<L: Fn(u64) -> bool, D: Fn() -> u64 + Sync>(
    params: &Params,
    report: &steam::ScanReport,
    platform: Platform,
    cancel: &AtomicBool,
    lease_active: &L,
    lease_deadline_ms: &D,
) -> Result<Value, String> {
    let request = params.request_id.as_deref().ok_or("request_id_required")?;
    if request.is_empty() || request.len() > 128 {
        return Err("invalid_request_id".into());
    }
    let instance = params.instance_id.as_deref().ok_or("instance_required")?;
    let release = params.release_id.as_deref();
    let selection = params.selection_id.as_deref();
    if release.is_some() && selection.is_some() {
        return Err("invalid_request".into());
    }
    if release.is_none() && selection.is_none() {
        return Err("selection_or_release_required".into());
    }
    let state = state(params)?;
    state_safe(&state)?;
    if let Some(selection_id) = selection {
        if params.archive.is_some() {
            return Err("archive_not_allowed_for_selection".into());
        }
        if selection_id.starts_with("zip-selection-") {
            return install_zip_route(
                params,
                report,
                state,
                instance,
                request,
                selection_id,
                cancel,
                lease_active,
                lease_deadline_ms,
            );
        }
        if selection_id.starts_with("exe-selection-") {
            return install_exe_route(
                params,
                report,
                &state,
                instance,
                request,
                selection_id,
                cancel,
                lease_deadline_ms,
            );
        }
        return Err("invalid_selection_id".into());
    }
    install_signed_route(
        params,
        report,
        platform,
        state,
        instance,
        request,
        release.expect("release route"),
        cancel,
        lease_active,
        lease_deadline_ms,
    )
}

fn zip_task_failure(error: installer::ZipTaskError) -> String {
    match error.detail {
        Some(detail) => failed(error.code, detail),
        None => error.code,
    }
}

/// InstallOutcome terminal shape (p1-host-contract §2.3).
fn outcome_json(transaction: &install::Transaction, selection_id: Option<&str>) -> Value {
    json!({
        "transaction_id": transaction.transaction_id,
        "plan_id": transaction.plan_id,
        "instance_id": transaction.instance_id,
        "selection_id": selection_id,
        "content_id": if transaction.release_id.starts_with("exe-content-"){json!(transaction.release_id)}else{Value::Null},
        "release_id": transaction.release_id,
        "state": transaction.state,
        "error": transaction.error,
        "created_at": transaction.created_at,
        "updated_at": transaction.updated_at,
        "has_replacements": transaction.has_replacements,
        "recovery": if transaction.state == "needs_recovery" {
            json!({ "required": true })
        } else {
            Value::Null
        },
        "cleanup": { "performed": transaction.state!="needs_recovery" },
    })
}

/// Dynamic ZIP route: resolve from cached sources, then run the orchestrator
/// with an injected transport (p1-zip-orchestration §2/§3).
#[allow(clippy::too_many_arguments)]
fn install_zip_route<L: Fn(u64) -> bool, D: Fn() -> u64 + Sync>(
    params: &Params,
    report: &steam::ScanReport,
    state: PathBuf,
    instance: &str,
    request: &str,
    selection_id: &str,
    cancel: &AtomicBool,
    lease_active: &L,
    lease_deadline_ms: &D,
) -> Result<Value, String> {
    let paths = installer::InstallerPaths::new(&state);
    installer::init_atomic(&paths).map_err(err)?;
    let game = install_for(report, instance)?;
    if game.availability != "installed" {
        return Err("game_not_ready".into());
    }
    if game.buildid.is_none() {
        return Err("build_not_verified".into());
    }
    let handle = sources::Sources::open(&state).map_err(err)?;
    let query = game_query(game);
    let selection = installer::resolve_zip(handle.registry(), &query, selection_id)
        .map_err(|error| error.code)?;
    let transport = installer::https_transport().map_err(err)?;
    let now = trust::now().map_err(err)?;
    let lease = |now: u64| lease_active(now);
    let lease_deadline = || lease_deadline_ms();
    let inputs = installer::ZipInstallInputs {
        game,
        selection: &selection,
        operation: params.operation,
        request_id: request,
        now,
        ttl_seconds: install::PLAN_TTL_SECONDS,
    };
    let deps = installer::ZipInstallDeps {
        paths: &paths,
        state_dir: &state,
        transport: transport.as_ref(),
        cancel,
        lease_active: &lease,
        lease_deadline_ms: &lease_deadline,
    };
    let transaction = installer::install_zip(inputs, deps).map_err(zip_task_failure)?;
    Ok(outcome_json(&transaction, Some(selection_id)))
}

#[allow(clippy::too_many_arguments)]
fn install_exe_route<D: Fn() -> u64 + Sync>(
    params: &Params,
    report: &steam::ScanReport,
    state: &Path,
    instance: &str,
    request: &str,
    selection_id: &str,
    cancel: &AtomicBool,
    deadline: &D,
) -> Result<Value, String> {
    let paths = installer::InstallerPaths::new(state);
    installer::init_atomic(&paths).map_err(err)?;
    let game = install_for(report, instance)?;
    if game.availability != "installed" {
        return Err("game_not_ready".into());
    }
    let sources = sources::Sources::open(state).map_err(err)?;
    let selection =
        installer::exe_resolve::resolve_exe(sources.registry(), &game_query(game), selection_id)
            .map_err(|e| e.code)?;
    let runtime = worker_runtime(&paths.root().join("runtime"))?;
    let probe = runtime.probe();
    if !probe.available {
        return Err(failed("exe_sandbox_unavailable", probe.reason));
    }
    let transport = installer::https_transport().map_err(err)?;
    let inputs = installer::exe_orchestrator::ExeInstallInputs {
        game,
        selection: &selection,
        operation: params.operation,
        request_id: request,
        now: trust::now().map_err(err)?,
        ttl_seconds: install::PLAN_TTL_SECONDS,
    };
    let deps = installer::exe_orchestrator::ExeInstallDeps {
        paths: &paths,
        state_dir: state,
        transport: transport.as_ref(),
        runtime: &runtime,
        cancel,
        lease_deadline_ms: deadline,
    };
    let transaction =
        installer::exe_orchestrator::install_exe(inputs, deps).map_err(zip_task_failure)?;
    Ok(outcome_json(&transaction, Some(selection_id)))
}

/// Signed catalog route: migrate to the P1 installer root, new ledger and the
/// internal authorization re-check (p1-zip-orchestration §4.1 step 2).
#[allow(clippy::too_many_arguments)]
fn install_signed_route<L: Fn(u64) -> bool, D: Fn() -> u64 + Sync>(
    params: &Params,
    report: &steam::ScanReport,
    platform: Platform,
    state: PathBuf,
    instance: &str,
    request: &str,
    release: &str,
    cancel: &AtomicBool,
    lease: &L,
    deadline: &D,
) -> Result<Value, String> {
    let paths = installer::InstallerPaths::new(&state);
    installer::init_atomic(&paths).map_err(err)?;
    let _claim = installer::requests::RequestClaim::acquire(&paths, request).map_err(err)?;
    let control = installer::task::TaskControl::new(&paths, request, instance, release, deadline())
        .map_err(err)?;
    control.run(cancel, deadline, || {
        install_signed_route_inner(
            params,
            report,
            platform,
            state,
            instance,
            request,
            release,
            &control.cancel,
            lease,
            &control,
        )
    })
}
#[allow(clippy::too_many_arguments)]
fn install_signed_route_inner<L: Fn(u64) -> bool>(
    params: &Params,
    report: &steam::ScanReport,
    platform: Platform,
    state: PathBuf,
    instance: &str,
    request: &str,
    release_id: &str,
    cancel: &AtomicBool,
    lease_active: &L,
    control: &installer::task::TaskControl,
) -> Result<Value, String> {
    let paths = installer::InstallerPaths::new(&state);
    installer::init_atomic(&paths).map_err(err)?;
    let game = install_for(report, instance)?;
    let catalog = signed(params)?;
    let release = ready_release(&catalog, release_id, game, platform)?;
    let binding =
        installer::request_binding(instance, None, Some(release_id), params.operation, None);
    if let Some(transaction) =
        installer::begin_request(&paths, request, &binding).map_err(zip_task_failure)?
    {
        return Ok(outcome_json(&transaction, None));
    }
    let archive = match &params.archive {
        Some(path) => path.clone(),
        None => download::fetch(
            release
                .artifact
                .as_ref()
                .ok_or("artifact_requires_manual_download")?,
            &paths.downloads_dir(),
            cancel,
            |_, _| (),
        )
        .map_err(err)?,
    };
    let identity = install::recipe_identity(release).map_err(err)?;
    if installer::content_conflict(&paths, request, &identity).map_err(zip_task_failure)? {
        return Err("request_id_binding_conflict".into());
    }
    let plan = install::prepare_with_operation_at(
        paths.root(),
        game,
        release,
        &archive,
        catalog.catalog().revision,
        params.operation,
    )
    .map_err(err)?;
    let outcome = (|| {
        if let Err(e) = control.attach_plan(&paths, &plan.plan_id) {
            steam_patch_core::operation::restoring();
            let _ = install::discard_plan_at(paths.root(), &plan.plan_id);
            return Err(err(e));
        }
        if !plan.blockers.is_empty() {
            let _ = install::discard_plan_at(paths.root(), &plan.plan_id);
            return Err(failed("plan_has_blockers", plan.blockers.join(",")));
        }
        installer::record_plan(&paths, request, &binding, &plan.plan_id, &identity)
            .map_err(zip_task_failure)?;

        let expectation = signed_expectation(release, game, cancel);
        let artifact_sha = release
            .recipe
            .as_ref()
            .ok_or("recipe_missing")?
            .artifact_sha256
            .clone();
        let artifact_size = archive_size(&archive)?;
        let plan_mappings = |plan_id: &str| -> Result<Vec<catalog::FileMapping>, String> {
            let stored = install::get_plan_at(paths.root(), plan_id).map_err(err)?;
            Ok(stored
                .changes
                .iter()
                .map(|change| catalog::FileMapping {
                    source: change.source.clone().unwrap_or_else(|| change.path.clone()),
                    destination: change.path.clone(),
                    original_sha256: change.original_sha256.clone(),
                    installed_sha256: change.installed_sha256.clone(),
                })
                .collect())
        };
        let facts = installer::game_facts(game, &expectation, cancel);
        if !facts.is_valid() {
            let _ = install::discard_plan_at(paths.root(), &plan.plan_id);
            return Err(match facts.failures().first() {
                Some(failure) => failed(failure.code.clone(), failure.detail.clone()),
                None => "game_not_ready".into(),
            });
        }
        let content = installer::ContentBinding::publisher_zip(
            &artifact_sha,
            artifact_size,
            &plan_mappings(&plan.plan_id)?,
        );
        let candidate_sha256 = identity.clone();
        let rule_fingerprint = release_fingerprint(release);
        let conditions_sha256 =
            steam_patch_core::sources::canonical::canonical_sha256(&expectation);
        let authorization = installer::authorize(installer::AuthorizeInput {
            release_id,
            selection_id: None,
            source_id: "signed",
            source_record_id: Some(&release.source_page),
            candidate_sha256: &candidate_sha256,
            rule_fingerprint: &rule_fingerprint,
            settings_revision: catalog.catalog().revision,
            conditions_sha256: &conditions_sha256,
            content,
            policy: installer::PolicyVersions::default(),
            plan_id: &plan.plan_id,
            task_id: Some(&control.id),
            request_id: Some(request),
            facts: facts.clone(),
            now: trust::now().map_err(err)?,
            ttl_seconds: install::PLAN_TTL_SECONDS,
        })
        .map_err(|invalidation| {
            let _ = install::discard_plan_at(paths.root(), &plan.plan_id);
            failed(invalidation.code.clone(), invalidation.to_string())
        })?;
        installer::write(&paths, &authorization).map_err(err)?;
        let facts_now = installer::game_facts(game, &expectation, cancel);
        let content_now = installer::ContentBinding::publisher_zip(
            &artifact_sha,
            artifact_size,
            &plan_mappings(&plan.plan_id)?,
        );
        let current = installer::CurrentState {
            now: trust::now().map_err(err)?,
            plan_id: plan.plan_id.clone(),
            instance_id: game.instance_id.clone(),
            revoked: installer::is_revoked(&paths, &plan.plan_id).map_err(err)?,
            lease_active: lease_active(trust::now().map_err(err)?),
            facts: facts_now,
            settings_revision: catalog.catalog().revision,
            policy: installer::PolicyVersions::default(),
            candidate_sha256: identity.clone(),
            rule_fingerprint,
            conditions_sha256,
            content: content_now,
        };
        if let Err(invalidation) = installer::validate(&authorization, &current) {
            // Lease loss / persistent revoke after the plan is a durable
            // cancellation: roll back and return the `InstallOutcome` rather than an
            // RPC failure (p1-host-contract §2.3). Other invalidations keep
            // `ok:false + error{code}` (§4.4).
            if matches!(
                invalidation.code.as_str(),
                "authorization_lease_inactive" | "authorization_revoked"
            ) {
                let transaction =
                    install::discard_plan_at(paths.root(), &plan.plan_id).map_err(err)?;
                installer::record_terminal(&paths, request, &binding, &identity, &transaction)
                    .map_err(zip_task_failure)?;
                return Ok(outcome_json(&transaction, None));
            }
            let _ = install::discard_plan_at(paths.root(), &plan.plan_id);
            return Err(failed(invalidation.code.clone(), invalidation.to_string()));
        }
        let transaction = install::apply_at(paths.root(), &plan.plan_id).map_err(err)?;
        installer::record_terminal(&paths, request, &binding, &identity, &transaction)
            .map_err(zip_task_failure)?;
        Ok(outcome_json(&transaction, None))
    })();
    match outcome {
        Ok(value) => Ok(value),
        Err(error) => {
            control
                .revoke()
                .map_err(|e| failed("needs_recovery", e.to_string()))?;
            steam_patch_core::operation::restoring();
            let transaction = installer::orchestrator::finish_failed_plan(&paths, &plan.plan_id)
                .map_err(|e| failed("needs_recovery", e.to_string()))?;
            installer::record_terminal(&paths, request, &binding, &identity, &transaction)
                .map_err(zip_task_failure)?;
            if transaction.state == "committed"
                || transaction.state == "needs_recovery"
                || error.contains("cancel")
            {
                Ok(outcome_json(&transaction, None))
            } else {
                Err(error)
            }
        }
    }
}

fn archive_size(path: &Path) -> Result<u64, String> {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(err)
}

/// Signed-route facts expectation: language follows the live manifest (fallback
/// to the release language) so a signed install is self-consistent, and the
/// release's audited builds are the verified set.
fn signed_expectation(
    release: &catalog::PatchRelease,
    game: &steam::GameInstall,
    cancel: &AtomicBool,
) -> installer::FactsExpectation {
    let language = installer::facts::current_language(game, cancel)
        .unwrap_or_else(|| release.language.clone());
    installer::FactsExpectation {
        appid: release.appid,
        language,
        verified_buildids: release.verified_buildids.clone(),
        requires_dlc: Vec::new(),
        excludes_dlc: Vec::new(),
        baselines: Vec::new(),
    }
}

fn release_fingerprint(release: &catalog::PatchRelease) -> String {
    steam_patch_core::sources::canonical::canonical_sha256(release)
}

fn dynamic_release_id(source_id: &str, appid: u32) -> String {
    if source_id == "patchmoe" {
        format!("patchmoe-{appid}")
    } else {
        format!("source-{source_id}-{appid}")
    }
}

/// Read-only dynamic projection appended to `scan.installs[].patches[]`
/// (p1-zip-orchestration §4.2). Signed releases with the same `release_id` win.
fn dynamic_patches(
    registry: &sources::registry::SourceRegistry,
    game: &steam::GameInstall,
    signed_ids: &HashSet<String>,
    transactions: &[install::Transaction],
    root: &Path,
) -> Vec<catalog::PatchMatch> {
    let query = game_query(game);
    let mut patches = Vec::new();
    for candidate in registry.cached_discovery(&query).candidates {
        let has_zip = candidate
            .mirrors
            .iter()
            .any(|mirror| mirror.format == "zip");
        let has_exe = candidate
            .mirrors
            .iter()
            .any(|mirror| mirror.format == "exe");
        let release_id = dynamic_release_id(&candidate.source_id, game.appid);
        if has_zip {
            if signed_ids.contains(&release_id) {
                continue;
            }
            let Some(selection_id) = installer::zip_selection_id(&candidate) else {
                continue;
            };
            let mut blockers = Vec::new();
            if game.availability != "installed" {
                blockers.push("game_not_ready".into());
            }
            if game.buildid.is_none() {
                blockers.push("build_not_verified".into());
            }
            match registry
                .rules()
                .iter()
                .find(|rule| rule.id == candidate.source_id)
            {
                Some(rule) => {
                    if !rule.enabled {
                        blockers.push("source_not_enabled".into());
                    }
                    if !registry.game_subscribed(game.appid, &rule.id) {
                        blockers.push("source_not_subscribed".into());
                    }
                }
                None => blockers.push("source_not_enabled".into()),
            }
            if candidate.cache_status == sources::model::CacheStatus::Stale {
                blockers.push("selection_stale".into());
            }
            let detection = dynamic_detection(game, &release_id, transactions, root);
            // Signature priority: never project a dynamic patch that collides.
            if patches
                .iter()
                .any(|patch: &catalog::PatchMatch| patch.release_id == release_id)
            {
                continue;
            }
            patches.push(catalog::PatchMatch {
                release_id,
                selection_id: Some(selection_id),
                name: candidate.patch_title.clone(),
                source_page: candidate.source_page.clone(),
                support_level: SupportLevel::PublisherZip,
                download_available: blockers.is_empty(),
                compatibility: if blockers.is_empty() {
                    "verified_metadata".into()
                } else {
                    "needs_review".into()
                },
                blockers,
                detection,
            });
        } else if has_exe {
            let Some(selection_id) = installer::exe_resolve::exe_selection_id(&candidate) else {
                continue;
            };
            // Selection identity must not masquerade as a release id
            // (p1-host-contract §4.1); the source-scoped dynamic release id is
            // the projection key, exactly as in the ZIP branch above.
            if patches
                .iter()
                .any(|patch: &catalog::PatchMatch| patch.release_id == release_id)
            {
                continue;
            }
            let availability = exe_sandbox_availability();
            let mut blockers = Vec::new();
            if let Err(e) = installer::exe_resolve::resolve_exe(registry, &query, &selection_id) {
                blockers.push(e.code);
            }
            if !availability.available {
                blockers.push("exe_sandbox_unavailable".into());
            }
            if game.availability != "installed" {
                blockers.push("game_not_ready".into());
            }
            patches.push(catalog::PatchMatch {
                release_id,
                selection_id: Some(selection_id),
                name: candidate.patch_title.clone(),
                source_page: candidate.source_page.clone(),
                support_level: SupportLevel::PublisherExe,
                download_available: blockers.is_empty(),
                compatibility: "check_required".into(),
                blockers,
                detection: catalog::Detection {
                    status: "unknown".into(),
                    files: Vec::new(),
                },
            });
        }
    }
    patches
}

/// Detection from the most recent committed transaction for the same instance
/// and release id, re-hashing the recorded plan destinations (p1-zip-orchestration
/// §4.2).
fn dynamic_detection(
    game: &steam::GameInstall,
    release_id: &str,
    transactions: &[install::Transaction],
    root: &Path,
) -> catalog::Detection {
    let unknown = catalog::Detection {
        status: "unknown".into(),
        files: Vec::new(),
    };
    let Some(transaction) = transactions
        .iter()
        .filter(|transaction| {
            transaction.instance_id == game.instance_id
                && transaction.release_id == release_id
                && transaction.state == "committed"
        })
        .max_by_key(|transaction| transaction.updated_at)
    else {
        return unknown;
    };
    let Ok(plan) = install::get_plan_ro(root, &transaction.plan_id) else {
        return unknown;
    };
    let files: Vec<catalog::FileEvidence> = plan
        .changes
        .iter()
        .map(|change| {
            let actual = hash_target(&game.install_path, &change.path);
            let status = if actual.as_deref() == Some(change.installed_sha256.as_str()) {
                "match"
            } else if actual.is_none() {
                "missing"
            } else {
                "mismatch"
            };
            catalog::FileEvidence {
                path: change.path.clone(),
                status: status.into(),
                actual_sha256: actual,
            }
        })
        .collect();
    let status = if files.iter().all(|file| file.status == "match") {
        "installed_managed"
    } else {
        "partial_or_conflict"
    };
    catalog::Detection {
        status: status.into(),
        files,
    }
}

fn hash_target(root: &Path, relative: &str) -> Option<String> {
    let path = relative
        .split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part));
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let count = file.read(&mut buffer).ok()?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

/// `preview` dispatch (p1-host-contract §1.1 A; p1-zip-orchestration §4.2).
fn preview_route(
    params: &Params,
    game: &steam::GameInstall,
    catalog: &Catalog,
    platform: Platform,
) -> Result<Value, String> {
    let release = params.release_id.as_deref();
    let selection = params.selection_id.as_deref();
    if release.is_some() && selection.is_some() {
        return Err("invalid_request".into());
    }
    if release.is_none() && selection.is_none() {
        return Err("selection_or_release_required".into());
    }
    let Some(selection_id) = selection else {
        return value(
            catalog::preview(game, catalog, release.expect("release route"), platform)
                .map_err(err)?,
        );
    };
    if selection_id.starts_with("exe-selection-") {
        let state = state(params)?;
        state_safe(&state)?;
        let handle = sources::Sources::open(&state).map_err(err)?;
        let selected =
            installer::exe_resolve::resolve_exe(handle.registry(), &game_query(game), selection_id)
                .map_err(|e| e.code)?;
        let availability = exe_sandbox_availability();
        let blockers = if availability.available {
            json!(["exe_check_required"])
        } else {
            json!(["exe_sandbox_unavailable"])
        };
        return Ok(
            json!({"instance_id":game.instance_id,"selection_id":selected.selection_id,"release_id":null,
            "executable":false,"blockers":blockers,"changes":[]}),
        );
    }
    if !selection_id.starts_with("zip-selection-") {
        return Err("invalid_selection_id".into());
    }
    let state = state(params)?;
    state_safe(&state)?;
    let handle = sources::Sources::open(&state).map_err(err)?;
    let query = game_query(game);
    match installer::resolve_zip(handle.registry(), &query, selection_id) {
        Ok(selection) => Ok(json!({
            "instance_id": game.instance_id,
            "selection_id": selection_id,
            "release_id": selection.release_id,
            "executable": false,
            "blockers": [],
            "changes": committed_changes(params, game, &selection.release_id)?,
        })),
        Err(error) => Ok(json!({
            "instance_id": game.instance_id,
            "selection_id": selection_id,
            "release_id": Value::Null,
            "executable": false,
            "blockers": [error.code],
            "changes": [],
        })),
    }
}

fn committed_changes(
    params: &Params,
    game: &steam::GameInstall,
    release_id: &str,
) -> Result<Vec<Value>, String> {
    let Some(state) = &params.state_dir else {
        return Ok(Vec::new());
    };
    let root = installer::InstallerPaths::new(state).root().to_path_buf();
    let transactions = install::list_transactions_at(&root).map_err(err)?;
    let Some(transaction) = transactions
        .iter()
        .filter(|transaction| {
            transaction.instance_id == game.instance_id
                && transaction.release_id == release_id
                && transaction.state == "committed"
        })
        .max_by_key(|transaction| transaction.updated_at)
    else {
        return Ok(Vec::new());
    };
    let plan = install::get_plan_ro(&root, &transaction.plan_id).map_err(err)?;
    Ok(plan
        .changes
        .iter()
        .map(|change| json!({ "path": change.path, "action": change.action }))
        .collect())
}

#[cfg(test)]
mod sources_tests {
    use super::*;
    use steam_patch_core::sources::{
        canonical, model::DiscoveryRecord, registry::DiscoveryOutcome,
    };

    fn fixture_root(temp: &Path) -> PathBuf {
        let root = temp.join("Steam");
        std::fs::create_dir_all(root.join("steamapps/common/Test/game")).unwrap();
        std::fs::write(
            root.join("steamapps/appmanifest_42.acf"),
            "\"AppState\" { \"appid\" \"42\" \"name\" \"Synthetic\" \"installdir\" \"Test\" \"buildid\" \"100\" \"StateFlags\" \"4\" }",
        )
        .unwrap();
        root
    }

    fn key_for_42() -> String {
        canonical::query_key(&json!({
            "appid": 42,
            "name": "Synthetic",
            "name_hints": [],
            "preferred_language": null,
            "legacy_match": false,
            "source_context": ["kagura-cn"],
            "parser_version": 2,
            "query_version": 1,
            "encoding_version": steam_patch_core::sources::SOURCES_ENCODING_VERSION,
        }))
    }

    /// Seeds a fresh, confirmed-free complete record for AppID 42 and disables
    /// the other discovery rules so `discover_game` short-circuits offline.
    fn seed(state: &Path) {
        let mut handle = sources::Sources::open(state).unwrap();
        handle
            .registry_mut()
            .set_enabled("kagura-en", false)
            .unwrap();
        handle.registry_mut().set_enabled("hgamer", false).unwrap();
        let rule = handle
            .registry()
            .rules()
            .iter()
            .find(|rule| rule.id == "kagura-cn")
            .cloned()
            .unwrap();
        handle
            .registry_mut()
            .commit(
                &rule,
                42,
                DiscoveryOutcome {
                    status: "complete".to_string(),
                    ..DiscoveryOutcome::default()
                },
                &DiscoveryRecord::default(),
                &key_for_42(),
            )
            .unwrap();
    }

    #[test]
    fn capabilities_and_unknown_method() {
        let capabilities = execute("get_capabilities", Params::default()).unwrap();
        assert!(capabilities["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|method| method == "discover_game"));
        assert_eq!(capabilities["sources"]["parser_version"], 2);
        assert_eq!(capabilities["sources"]["query_version"], 1);
        assert_eq!(capabilities["installer"]["schema_version"], 1);
        assert_eq!(
            execute("bogus", Params::default()).unwrap_err(),
            "unsupported_method"
        );
    }

    #[test]
    fn read_only_capabilities_never_create_installer_state() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let first = Params {
            state_dir: Some(state.clone()),
            ..Default::default()
        };
        let capabilities = execute("get_capabilities", first).unwrap();
        assert_eq!(capabilities["installer"]["schema_version"], 1);
        assert!(
            !state.exists(),
            "read-only capabilities must not create the installer root"
        );
        // Repeated calls stay read-only through the process probe cache.
        let second = Params {
            state_dir: Some(state.clone()),
            ..Default::default()
        };
        execute("get_capabilities", second).unwrap();
        assert!(!state.exists(), "cached sandbox probe must stay read-only");
    }

    #[test]
    fn list_sources_returns_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let result = execute(
            "list_sources",
            Params {
                roots: vec![fixture_root(temp.path())],
                state_dir: Some(temp.path().join("state")),
                ..Default::default()
            },
        )
        .unwrap();
        let array = result.as_array().unwrap();
        assert_eq!(array.len(), 4);
        assert!(array.iter().any(|source| source["id"] == "patchmoe"));
    }

    #[test]
    fn set_source_enabled_returns_bumped_revision() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let result = execute(
            "set_source_enabled",
            Params {
                roots: vec![fixture_root(temp.path())],
                state_dir: Some(state.clone()),
                source_id: Some("kagura-cn".to_string()),
                enabled: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(result["settings_revision"].as_u64().unwrap() >= 2);
        assert_eq!(result["enabled"], false);

        let list = execute(
            "list_sources",
            Params {
                roots: vec![fixture_root(temp.path())],
                state_dir: Some(state),
                ..Default::default()
            },
        )
        .unwrap();
        let kagura = list
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["id"] == "kagura-cn")
            .unwrap();
        assert_eq!(kagura["enabled"], false);
    }

    #[test]
    fn discover_game_uses_sealed_cache_offline() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        seed(&state);
        let root = fixture_root(temp.path());
        let instance_id = steam::scan(std::slice::from_ref(&root))
            .installs
            .remove(0)
            .instance_id;
        let result = execute(
            "discover_game",
            Params {
                roots: vec![root],
                state_dir: Some(state),
                instance_id: Some(instance_id.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result["instance_id"], instance_id);
        assert_eq!(result["appid"], 42);
        assert_eq!(result["status"], "cache_complete");
        assert!(result["candidates"].as_array().unwrap().is_empty());
    }

    #[test]
    fn scan_projects_cached_sources() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        seed(&state);
        let result = execute(
            "scan",
            Params {
                roots: vec![fixture_root(temp.path())],
                state_dir: Some(state),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(result["settings_revision"].as_u64().unwrap() >= 1);
        assert!(result["sources"].as_array().is_some());
        let install = &result["installs"][0];
        assert_eq!(install["discovery"]["status"], "cache_complete");
        assert!(install["sources"].as_array().is_some());
    }

    /// Seeds a patchmoe cache entry that carries AppID 42 so matching is offline.
    fn seed_patchmoe(state: &Path) {
        let handle = sources::Sources::open(state).unwrap();
        handle
            .store()
            .save_cache(
                "patchmoe",
                &sources::SourceCache {
                    entries: vec![sources::SourceEntry {
                        title: "Synthetic".to_string(),
                        aliases: Vec::new(),
                        url: "https://download.patch.moe/synthetic.zip".to_string(),
                        source_page: "https://www.patch.moe/synthetic".to_string(),
                        language: "en".to_string(),
                        date: None,
                        appid: Some(42),
                    }],
                    etag: None,
                    modified: None,
                    cached_at: Some(1000),
                    checked_at: Some(1000),
                },
            )
            .unwrap();
    }

    #[test]
    fn deleted_source_methods_are_not_in_the_whitelist() {
        for method in [
            "list_source_matches",
            "get_source_state",
            "get_source_settings",
            "get_source_policy",
            "resolve_source_record",
        ] {
            assert_eq!(
                execute(method, Params::default()).unwrap_err(),
                "unsupported_method",
                "deleted source method {method} must be rejected"
            );
        }
    }

    #[test]
    fn list_sources_includes_origin() {
        let temp = tempfile::tempdir().unwrap();
        let result = execute(
            "list_sources",
            Params {
                roots: vec![fixture_root(temp.path())],
                state_dir: Some(temp.path().join("state")),
                ..Default::default()
            },
        )
        .unwrap();
        let array = result.as_array().unwrap();
        assert!(array
            .iter()
            .all(|source| matches!(source["origin"].as_str(), Some("builtin" | "added"))));
        let patchmoe = array
            .iter()
            .find(|source| source["id"] == "patchmoe")
            .unwrap();
        assert_eq!(patchmoe["origin"], "builtin");
    }

    #[test]
    fn scan_projects_matches_per_install() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        seed_patchmoe(&state);
        let result = execute(
            "scan",
            Params {
                roots: vec![fixture_root(temp.path())],
                state_dir: Some(state),
                ..Default::default()
            },
        )
        .unwrap();
        let matches = result["installs"][0]["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["release_id"], "patchmoe-42");
    }

    #[test]
    fn set_game_source_by_appid_reflects_in_subscriptions() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let root = fixture_root(temp.path());
        let write = execute(
            "set_game_source",
            Params {
                roots: vec![root.clone()],
                state_dir: Some(state.clone()),
                appid: Some(42),
                source_id: Some("kagura-cn".to_string()),
                subscribed: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(write["settings_revision"].as_u64().unwrap() >= 2);

        let subscriptions = execute(
            "list_game_sources",
            Params {
                roots: vec![root],
                state_dir: Some(state),
                appid: Some(42),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(subscriptions.is_array());
    }

    #[test]
    fn scan_sources_streams_progress_for_empty_scan() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let mut events = Vec::new();
        let result = execute_streaming(
            "scan_sources",
            Params {
                roots: vec![temp.path().to_path_buf()],
                state_dir: Some(state),
                ..Default::default()
            },
            &AtomicBool::new(false),
            &mut |event| events.push(event),
        )
        .unwrap();
        assert!(events
            .iter()
            .any(|event| event["event"] == "source_progress"));
        assert!(result["settings_revision"].is_number());
    }
}

#[cfg(test)]
mod install_flow_tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::Write;
    use zip::{write::SimpleFileOptions, ZipWriter};

    fn fixture() -> (tempfile::TempDir, Params, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Steam");
        let game = root.join("steamapps/common/Test");
        fs::create_dir_all(&game).unwrap();
        fs::write(root.join("steamapps/appmanifest_42.acf"),
            "\"AppState\" { \"appid\" \"42\" \"name\" \"Test\" \"installdir\" \"Test\" \"buildid\" \"100\" \"StateFlags\" \"4\" \"UserConfig\" { \"language\" \"schinese\" } \"MountedConfig\" { \"language\" \"schinese\" } }").unwrap();
        let archive = temp.path().join("patch.zip");
        let mut zip = ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.start_file(
            "patch.dat",
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
        zip.write_all(b"patch content").unwrap();
        zip.finish().unwrap();
        let digest = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
        let catalog = temp.path().join("catalog.json");
        fs::write(&catalog, json!({"schema_version":1,"revision":1,"releases":[{
            "patch_id":"test","release_id":"test-v1","appid":42,"name":"Test patch","version":"1",
            "language":"zh-CN","category":"content_restore","source_page":"https://example.invalid/patch",
            "support_level":"managed_zip","platforms":["windows","steamos"],"verified_buildids":["100"],
            "detection":[{"path":"patch.dat","sha256":digest(b"patch content")}],
            "recipe":{"artifact_sha256":digest(&fs::read(&archive).unwrap()),"files":[{
                "source":"patch.dat","destination":"patch.dat","original_sha256":null,"installed_sha256":digest(b"patch content")}]}
        }]}).to_string()).unwrap();
        let private = temp.path().join("private");
        let keys = temp.path().join("keys.json");
        let signed = temp.path().join("signed.json");
        crate::maintenance::run(
            "keygen",
            &[
                private.display().to_string(),
                keys.display().to_string(),
                "test-key".into(),
            ],
        )
        .unwrap();
        crate::maintenance::run(
            "sign-catalog",
            &[
                catalog.display().to_string(),
                private.display().to_string(),
                "test-key".into(),
                signed.display().to_string(),
                "3600".into(),
            ],
        )
        .unwrap();
        let instance_id = steam::scan(std::slice::from_ref(&root))
            .installs
            .remove(0)
            .instance_id;
        let params = Params {
            roots: vec![root],
            instance_id: Some(instance_id),
            release_id: Some("test-v1".into()),
            signed_catalog: Some(signed),
            trust_keys: Some(keys),
            state_dir: Some(temp.path().join("state")),
            archive: Some(archive),
            request_id: Some("request-1".into()),
            ..Default::default()
        };
        (temp, params, game)
    }

    fn copy_params(params: &Params) -> Params {
        serde_json::from_value(serde_json::to_value(params).unwrap()).unwrap()
    }

    #[test]
    fn one_step_signed_install_retry_and_reinstall_are_bound_and_idempotent() {
        let (_temp, params, game) = fixture();
        let first = execute("install_patch", copy_params(&params)).unwrap();
        assert_eq!(first["state"], "committed");
        let repeated = execute("install_patch", copy_params(&params)).unwrap();
        assert_eq!(first["transaction_id"], repeated["transaction_id"]);
        let mut conflict = copy_params(&params);
        conflict.operation = install::InstallOperation::Reinstall;
        assert_eq!(
            execute("install_patch", conflict).unwrap_err(),
            "request_id_binding_conflict"
        );
        fs::write(game.join("patch.dat"), b"damaged").unwrap();
        let mut reinstall = copy_params(&params);
        reinstall.request_id = Some("request-2".into());
        reinstall.operation = install::InstallOperation::Reinstall;
        assert_eq!(
            execute("install_patch", reinstall).unwrap()["state"],
            "committed"
        );
        assert_eq!(fs::read(game.join("patch.dat")).unwrap(), b"patch content");
    }

    /// End-to-end task channel: `install_patch` streams flat `install_progress`
    /// events with a strictly increasing sequence, followed by exactly one
    /// final response (terminal-once; p1-host-contract §2.2/§2.3).
    #[test]
    fn task_streams_install_progress_then_single_terminal_response() {
        use std::sync::{Arc, Mutex};
        let (_temp, params, _game) = fixture();
        let request = json!({
            "protocol_version": 3,
            "request_id": "request-1",
            "method": "install_patch",
            "params": serde_json::to_value(&params).unwrap(),
        })
        .to_string();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let events = Arc::clone(&events);
            move |value| events.lock().unwrap().push(value)
        };
        let response = crate::run_task(
            request.as_bytes(),
            false,
            Arc::new(AtomicBool::new(false)),
            sink,
        );
        let events = events.lock().unwrap();
        assert!(!events.is_empty(), "install_patch must stream progress");
        assert!(events
            .iter()
            .all(|event| event["event"] == "install_progress"));
        assert!(events
            .iter()
            .all(|event| event["request_id"] == "request-1"));
        assert!(events.iter().all(|event| event["instance_id"].is_string()));
        let sequences: Vec<u64> = events
            .iter()
            .map(|event| event["sequence"].as_u64().unwrap())
            .collect();
        assert_eq!(sequences, (0..sequences.len() as u64).collect::<Vec<_>>());
        // Terminal-once: the task returns a single final response after events.
        assert_eq!(response["ok"], true);
        assert_eq!(response["protocol_version"], 3);
        assert_eq!(response["result"]["state"], "committed");
    }

    #[test]
    fn install_patch_route_matrix() {
        let (_temp, params, _game) = fixture();

        let mut conflict = copy_params(&params);
        conflict.selection_id = Some("zip-selection-000000000000000000000000-42".into());
        assert_eq!(
            execute("install_patch", conflict).unwrap_err(),
            "invalid_request"
        );

        let mut neither = copy_params(&params);
        neither.release_id = None;
        assert_eq!(
            execute("install_patch", neither).unwrap_err(),
            "selection_or_release_required"
        );

        let mut invalid = copy_params(&params);
        invalid.release_id = None;
        invalid.archive = None;
        invalid.selection_id = Some("candidate-000000000000000000000000".into());
        assert_eq!(
            execute("install_patch", invalid).unwrap_err(),
            "invalid_selection_id"
        );

        let mut exe = copy_params(&params);
        exe.release_id = None;
        exe.archive = None;
        exe.selection_id = Some("exe-selection-000000000000000000000000-42".into());
        assert_eq!(
            execute("install_patch", exe).unwrap_err(),
            "selection_not_found"
        );

        let mut archive = copy_params(&params);
        archive.release_id = None;
        archive.selection_id = Some("zip-selection-000000000000000000000000-42".into());
        archive.archive = Some(PathBuf::from("unused.zip"));
        assert_eq!(
            execute("install_patch", archive).unwrap_err(),
            "archive_not_allowed_for_selection"
        );
    }

    #[test]
    fn signed_install_uses_the_installer_root_and_new_ledger() {
        let (temp, params, _game) = fixture();
        let result = execute("install_patch", copy_params(&params)).unwrap();
        assert_eq!(result["state"], "committed");
        assert_eq!(result["content_id"], Value::Null);
        let root = temp.path().join("state/installer/rust-v1");
        assert!(root.join("journal.sqlite").exists());
        assert!(root.join("requests.sqlite").exists());
        let transactions = execute(
            "list_transactions",
            Params {
                roots: params.roots.clone(),
                state_dir: params.state_dir.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(transactions.as_array().unwrap().len(), 1);
    }

    #[test]
    fn preview_selection_route_is_read_only() {
        let (_temp, params, _game) = fixture();
        let game = steam::scan(std::slice::from_ref(&params.roots[0]))
            .installs
            .remove(0);
        let mut preview = copy_params(&params);
        preview.release_id = None;
        preview.selection_id = Some("candidate-000000000000000000000000".into());
        assert_eq!(
            execute("preview", preview).unwrap_err(),
            "invalid_selection_id"
        );

        let mut exe = copy_params(&params);
        exe.release_id = None;
        exe.selection_id = Some("exe-selection-000000000000000000000000-42".into());
        assert_eq!(execute("preview", exe).unwrap_err(), "selection_not_found");

        let mut zip = copy_params(&params);
        zip.release_id = None;
        zip.instance_id = Some(game.instance_id);
        zip.selection_id = Some("zip-selection-000000000000000000000000-42".into());
        let result = execute("preview", zip).unwrap();
        assert_eq!(result["executable"], false);
        assert_eq!(result["blockers"][0], "selection_not_found");
        assert!(result["changes"].as_array().unwrap().is_empty());
    }
}

#[cfg(test)]
mod dynamic_projection_tests {
    use super::*;
    use steam_patch_core::sources::model::{
        ArtifactFormat, CacheStatus, Capability, DiscoveryRecord, MatchBasis, Mirror,
        SourceCandidate,
    };
    use steam_patch_core::sources::registry::DiscoveryOutcome;

    const APPID: u32 = 42;

    fn dynamic_candidate() -> SourceCandidate {
        SourceCandidate {
            candidate_id: "candidate-000000000000000000000000".to_string(),
            source_id: "kagura-cn".to_string(),
            source_record_id: "https://kagurafan.com/patches/game".to_string(),
            game_appid: Some(APPID),
            parent_appid: None,
            aliases: Vec::new(),
            game_title: "Test".to_string(),
            patch_title: "Test patch".to_string(),
            variant: steam_patch_core::sources::model::Variant::Base,
            requires_dlc: None,
            excludes_dlc: None,
            platforms: vec!["steam".to_string()],
            patch_kind: "adult_content".to_string(),
            languages: vec!["zh-CN".to_string()],
            version_raw: Some("1".to_string()),
            version_evidence: Vec::new(),
            version_identity: "1".to_string(),
            published_at: None,
            patch_updated_at: None,
            source_page: "https://kagurafan.com/patches/game".to_string(),
            upstream_page: None,
            mirrors: vec![Mirror {
                url: "https://kaguraserver.com/a.zip".to_string(),
                label: None,
                format: "zip".to_string(),
            }],
            format_reported: ArtifactFormat::Zip,
            format_observed: ArtifactFormat::Zip,
            capability: Capability::DiscoverOnly,
            identity_evidence: Vec::new(),
            provenance: Vec::new(),
            match_basis: MatchBasis::Appid,
            observed_at: "2026-01-01T00:00:00+00:00".to_string(),
            cache_status: CacheStatus::Cached,
            warnings: Vec::new(),
            fingerprint: "f".repeat(64),
            related_index_pages: Vec::new(),
        }
    }

    fn seed_dynamic(state: &Path) {
        let mut handle = sources::Sources::open(state).unwrap();
        let rule = handle
            .registry()
            .rules()
            .iter()
            .find(|rule| rule.id == "kagura-cn")
            .cloned()
            .unwrap();
        handle
            .registry_mut()
            .commit(
                &rule,
                APPID,
                DiscoveryOutcome {
                    status: "complete".to_string(),
                    candidates: vec![dynamic_candidate()],
                    ..DiscoveryOutcome::default()
                },
                &DiscoveryRecord::default(),
                "key",
            )
            .unwrap();
    }

    /// Synthetic installed game + seeded source candidate + dynamic archive.
    fn fixture() -> (
        tempfile::TempDir,
        Params,
        steam::GameInstall,
        PathBuf,
        String,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("Steam");
        let install = library.join("steamapps").join("common").join("Test");
        fs::create_dir_all(&install).unwrap();
        fs::write(
            library
                .join("steamapps")
                .join(format!("appmanifest_{APPID}.acf")),
            "\"AppState\" { \"appid\" \"42\" \"name\" \"Test\" \"installdir\" \"Test\" \
             \"buildid\" \"100\" \"StateFlags\" \"4\" \
             \"UserConfig\" { \"language\" \"schinese\" } \
             \"MountedConfig\" { \"language\" \"schinese\" } }",
        )
        .unwrap();
        let mut report = steam::scan(std::slice::from_ref(&library));
        let game = report.installs.remove(0);
        let archive = temp.path().join("dynamic.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.start_file(
            "data.txt",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
        use std::io::Write as _;
        zip.write_all(b"patched").unwrap();
        zip.finish().unwrap();
        let state = temp.path().join("state");
        seed_dynamic(&state);
        let selection_id = installer::zip_selection_id(&dynamic_candidate()).unwrap();
        let params = Params {
            roots: vec![library],
            state_dir: Some(state),
            instance_id: Some(game.instance_id.clone()),
            platform: Some(Platform::Windows),
            ..Default::default()
        };
        (temp, params, game, archive, selection_id)
    }

    fn clone_params(params: &Params) -> Params {
        serde_json::from_value(serde_json::to_value(params).unwrap()).unwrap()
    }

    fn dynamic_release(
        release_id: &str,
        archive: &Path,
        mappings: &[catalog::FileMapping],
    ) -> catalog::PatchRelease {
        let digest = format!("{:x}", Sha256::digest(fs::read(archive).unwrap()));
        catalog::PatchRelease {
            patch_id: release_id.to_string(),
            release_id: release_id.to_string(),
            appid: APPID,
            name: "Dynamic".to_string(),
            version: "0".to_string(),
            language: "zh-CN".to_string(),
            category: "publisher_zip".to_string(),
            source_page: "https://kagurafan.com/patches/game".to_string(),
            support_level: SupportLevel::ManagedZip,
            platforms: vec![Platform::Windows, Platform::Steamos],
            verified_buildids: vec!["100".to_string()],
            detection: mappings
                .iter()
                .map(|mapping| catalog::Fingerprint {
                    path: mapping.destination.clone(),
                    sha256: mapping.installed_sha256.clone(),
                })
                .collect(),
            recipe: Some(catalog::Recipe {
                artifact_sha256: digest,
                files: mappings.to_vec(),
            }),
            artifact: None,
        }
    }

    fn install_dynamic_committed(
        state: &Path,
        game: &steam::GameInstall,
        archive: &Path,
        release_id: &str,
    ) -> String {
        let paths = installer::InstallerPaths::new(state);
        installer::init_atomic(&paths).unwrap();
        let cancel = AtomicBool::new(false);
        let mappings = installer::build_recipe(archive, &game.install_path, &cancel).unwrap();
        let release = dynamic_release(release_id, archive, &mappings);
        let plan = install::prepare_with_operation_at(
            paths.root(),
            game,
            &release,
            archive,
            0,
            install::InstallOperation::Install,
        )
        .unwrap();
        let transaction = install::apply_at(paths.root(), &plan.plan_id).unwrap();
        assert_eq!(transaction.state, "committed");
        plan.plan_id
    }

    #[test]
    fn scan_projects_the_dynamic_selection_with_full_fields() {
        let (_temp, params, _game, _archive, selection_id) = fixture();
        let mut scan = clone_params(&params);
        scan.instance_id = None;
        let result = execute("scan", scan).unwrap();
        let patches = result["installs"][0]["patches"].as_array().unwrap();
        let dynamic = patches
            .iter()
            .find(|patch| patch["selection_id"] == selection_id.as_str())
            .expect("dynamic selection projected");
        assert_eq!(dynamic["release_id"], "source-kagura-cn-42");
        assert_eq!(dynamic["support_level"], "publisher_zip");
        assert_eq!(dynamic["download_available"], true);
        assert!(dynamic["blockers"].as_array().unwrap().is_empty());
        assert!(dynamic["detection"]["status"].is_string());
        assert!(dynamic["compatibility"].is_string());
    }

    #[test]
    fn scan_projects_exe_release_id_without_masquerading_selection() {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("Steam");
        let install = library.join("steamapps").join("common").join("Test");
        fs::create_dir_all(&install).unwrap();
        fs::write(
            library
                .join("steamapps")
                .join(format!("appmanifest_{APPID}.acf")),
            "\"AppState\" { \"appid\" \"42\" \"name\" \"Test\" \"installdir\" \"Test\" \
             \"buildid\" \"100\" \"StateFlags\" \"4\" \
             \"UserConfig\" { \"language\" \"schinese\" } \
             \"MountedConfig\" { \"language\" \"schinese\" } }",
        )
        .unwrap();
        let state = temp.path().join("state");
        let mut candidate = dynamic_candidate();
        candidate.mirrors[0].format = "exe".into();
        candidate.mirrors[0].url = "https://kaguraserver.com/a.exe".into();
        candidate.format_reported = ArtifactFormat::Exe;
        candidate.format_observed = ArtifactFormat::Exe;
        let selection_id = installer::exe_resolve::exe_selection_id(&candidate).unwrap();
        let mut handle = sources::Sources::open(&state).unwrap();
        let rule = handle
            .registry()
            .rules()
            .iter()
            .find(|rule| rule.id == "kagura-cn")
            .cloned()
            .unwrap();
        handle
            .registry_mut()
            .commit(
                &rule,
                APPID,
                DiscoveryOutcome {
                    status: "complete".to_string(),
                    candidates: vec![candidate],
                    ..DiscoveryOutcome::default()
                },
                &DiscoveryRecord::default(),
                "key",
            )
            .unwrap();
        let params = Params {
            roots: vec![library],
            state_dir: Some(state),
            platform: Some(Platform::Windows),
            ..Default::default()
        };
        let result = execute("scan", params).unwrap();
        let patches = result["installs"][0]["patches"].as_array().unwrap();
        let dynamic = patches
            .iter()
            .find(|patch| patch["selection_id"] == selection_id.as_str())
            .expect("exe selection projected");
        assert_eq!(dynamic["release_id"], "source-kagura-cn-42");
        assert_eq!(dynamic["support_level"], "publisher_exe");
    }

    #[test]
    fn scan_lets_a_signed_release_win_over_the_dynamic_selection() {
        let (temp, params, _game, _archive, selection_id) = fixture();
        let digest = format!("{:x}", Sha256::digest(b"patched"));
        let catalog_path = temp.path().join("catalog.json");
        fs::write(
            &catalog_path,
            json!({
                "schema_version": 1,
                "revision": 1,
                "releases": [{
                    "patch_id": "dynamic", "release_id": "source-kagura-cn-42", "appid": 42,
                    "name": "Dynamic", "version": "1", "language": "zh-CN", "category": "publisher_zip",
                    "source_page": "https://kagurafan.com/patches/game",
                    "support_level": "managed_zip", "platforms": ["windows", "steamos"],
                    "verified_buildids": ["100"],
                    "detection": [{ "path": "data.txt", "sha256": digest }],
                    "recipe": { "artifact_sha256": "0".repeat(64), "files": [
                        { "source": "data.txt", "destination": "data.txt",
                          "original_sha256": null, "installed_sha256": digest }] }
                }]
            })
            .to_string(),
        )
        .unwrap();
        let mut scan = clone_params(&params);
        scan.instance_id = None;
        scan.catalog = Some(catalog_path);
        let result = execute("scan", scan).unwrap();
        let patches = result["installs"][0]["patches"].as_array().unwrap();
        let matching: Vec<_> = patches
            .iter()
            .filter(|patch| patch["release_id"] == "source-kagura-cn-42")
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "signature priority must deduplicate: {patches:?}"
        );
        assert_eq!(matching[0]["support_level"], "managed_zip");
        assert!(patches
            .iter()
            .all(|patch| patch["selection_id"] != selection_id.as_str()));
    }

    #[test]
    fn preview_selection_is_read_only_and_reflects_a_committed_plan() {
        let (_temp, params, game, archive, selection_id) = fixture();
        let mut preview = clone_params(&params);
        preview.selection_id = Some(selection_id.clone());

        let read_only = execute("preview", clone_params(&preview)).unwrap();
        assert_eq!(read_only["executable"], false);
        assert_eq!(read_only["selection_id"], selection_id.as_str());
        assert_eq!(read_only["release_id"], "source-kagura-cn-42");
        assert!(read_only["blockers"].as_array().unwrap().is_empty());
        assert!(read_only["changes"].as_array().unwrap().is_empty());
        assert!(!game.install_path.join("data.txt").exists());

        let state = params.state_dir.clone().unwrap();
        install_dynamic_committed(&state, &game, &archive, "source-kagura-cn-42");

        let after = execute("preview", clone_params(&preview)).unwrap();
        let changes = after["changes"].as_array().unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["path"], "data.txt");
        assert_eq!(changes[0]["action"], "add");
    }

    #[test]
    fn outcome_reports_needs_recovery_explicitly() {
        let transaction = install::Transaction {
            transaction_id: "11111111-1111-1111-1111-111111111111".to_string(),
            plan_id: "11111111-1111-1111-1111-111111111111".to_string(),
            instance_id: "inst".to_string(),
            release_id: "source-kagura-cn-42".to_string(),
            state: "needs_recovery".to_string(),
            error: Some("recovery_conflict".to_string()),
            created_at: 1,
            updated_at: 2,
            has_replacements: true,
        };
        let value = super::outcome_json(
            &transaction,
            Some("zip-selection-000000000000000000000000-42"),
        );
        assert_eq!(value["state"], "needs_recovery");
        assert_eq!(value["recovery"]["required"], true);
        assert_eq!(value["content_id"], Value::Null);
        assert_eq!(
            value["selection_id"],
            "zip-selection-000000000000000000000000-42"
        );
    }
}
