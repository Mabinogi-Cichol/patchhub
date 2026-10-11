use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
#[cfg(test)]
use steam_patch_core::catalog::SupportLevel;
use steam_patch_core::{
    catalog::{self, Catalog, Platform},
    install, installer, projection, sources, steam,
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
            // Archive telemetry can precede an explicit stage in signed/local ZIP
            // routes. Protocol v3 never exposes the internal empty default.
            "" => ("connecting", None),
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
fn projection_error(error: projection::DomainError) -> String {
    failed(error.code, error.detail.unwrap_or_default())
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

struct CatalogFiles<'a>(&'a Params);
impl trust::revision::TrustedCatalogProvider for CatalogFiles<'_> {
    fn load(&self) -> Result<trust::revision::TrustedCatalogMaterial, projection::DomainError> {
        let path = self
            .0
            .signed_catalog
            .as_ref()
            .ok_or_else(|| projection::DomainError::code("signed_catalog_required"))?;
        let keys = match &self.0.trust_keys {
            Some(path) => trust::load_keys(path)?,
            None => serde_json::from_str(include_str!("../../../catalog/trust-keys.json"))
                .map_err(|e| projection::DomainError::new("invalid_trust_keys", e.to_string()))?,
        };
        Ok(trust::revision::TrustedCatalogMaterial {
            envelope: trust::read_bounded(path, catalog::MAX_CATALOG_BYTES * 2)?,
            keys,
        })
    }
}
struct ReadOnlyRevisionFloor;
impl trust::revision::TrustRevisionStore for ReadOnlyRevisionFloor {
    fn highest_revision(&self) -> Result<u64, projection::DomainError> {
        Ok(0)
    }
    fn remember_verified_revision(
        &self,
        _: &VerifiedCatalog,
    ) -> Result<(), projection::DomainError> {
        Err(projection::DomainError::code("state_dir_required"))
    }
}
fn verify_signed(params: &Params, now: u64) -> Result<VerifiedCatalog, String> {
    let provider = CatalogFiles(params);
    match &params.state_dir {
        Some(state) => {
            let games = steam::scan(&params.roots);
            let store =
                trust::revision::SqliteTrustRevisionStore::new_for_games(state, &games.installs)
                    .map_err(projection_error)?;
            trust::revision::verify_catalog(&provider, &store, now).map_err(projection_error)
        }
        None => trust::revision::verify_catalog(&provider, &ReadOnlyRevisionFloor, now)
            .map_err(projection_error),
    }
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
pub fn execute_controlled<L: Fn(u64) -> bool + Sync, D: Fn() -> u64 + Sync>(
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
        return match method {
            "list_transactions" => value(install::list_transactions_at(paths.root()).map_err(err)?),
            _ => {
                let host = installer::service::ValidatedHostContext::new(
                    &state,
                    &roots,
                    params.platform.unwrap_or_else(host_platform),
                )
                .map_err(projection_error)?;
                if method == "discard_plan" {
                    let id = installer::service::PlanId::new(
                        params.plan_id.as_deref().ok_or("plan_required")?,
                    )
                    .map_err(projection_error)?;
                    value(installer::service::discard_plan(&host, &id).map_err(projection_error)?)
                } else {
                    value(installer::service::recover(&host).map_err(projection_error)?)
                }
            }
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
    let projection_now = trust::now().map_err(err)?;
    let mut catalog_error = None;
    let verified = if params.signed_catalog.is_some() {
        match verify_signed(&params, projection_now) {
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
    let transactions = params
        .state_dir
        .as_ref()
        .map(|state| install::TransactionSnapshot::load_ro(state, &report))
        .transpose()
        .map_err(err)?
        .unwrap_or_default();
    let queries: Vec<_> = report.installs.iter().map(game_query).collect();
    let source_snapshot = params
        .state_dir
        .as_ref()
        .map(|state| sources::snapshot::SourceSnapshot::load(state, &queries, projection_now))
        .transpose()
        .map_err(err)?
        .flatten();
    let availability = exe_sandbox_availability();
    let capabilities = projection::HostCapabilities {
        zip: projection::LinkCapability {
            available: installer::host_identity_blocker().is_none(),
            blockers: installer::host_identity_blocker()
                .map(projection::DomainError::code)
                .into_iter()
                .collect(),
        },
        exe: projection::LinkCapability {
            available: availability.available,
            blockers: if availability.available {
                vec![]
            } else {
                vec![projection::DomainError::new(
                    "exe_sandbox_unavailable",
                    availability.reason,
                )]
            },
        },
    };
    let inputs = projection::ProjectionInputs {
        steam: &report,
        catalog: match verified.as_ref() {
            Some(catalog) => projection::CatalogView::Verified(catalog),
            None => projection::CatalogView::Development(&catalog),
        },
        sources: source_snapshot.as_ref(),
        transactions: &transactions,
        roots: &roots,
        now: projection_now,
        platform,
        capabilities: &capabilities,
        cancel,
    };
    if method == "preview" {
        if params.selection_id.is_some() && params.state_dir.is_none() {
            return Err("state_dir_required".into());
        }
        // v3 wire exclusivity remains here until the shared protocol lands in P5.
        let target = match (params.release_id, params.selection_id) {
            (Some(_), Some(_)) => return Err("invalid_request".into()),
            (None, None) => return Err("selection_or_release_required".into()),
            (Some(id), None) => projection::PreviewTarget::SignedRelease(id),
            (None, Some(id)) if id.starts_with("exe-selection-") => {
                projection::PreviewTarget::PublisherExeSelection(id)
            }
            (None, Some(id)) if id.starts_with("zip-selection-") => {
                projection::PreviewTarget::PublisherZipSelection(id)
            }
            _ => return Err("invalid_selection_id".into()),
        };
        return value(
            projection::preview(
                &inputs,
                projection::PreviewRequest {
                    instance_id: params.instance_id.ok_or("instance_not_found")?,
                    target,
                },
            )
            .map_err(projection_error)?,
        );
    }
    let mut result = value(projection::scan(inputs).map_err(projection_error)?)?;
    // Transitional v3 wire error is a string; core reports typed errors. P5
    // replaces this serialization adapter with the generated v4 contract.
    result["catalog_error"] = json!(catalog_error);
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
struct HostLease<'a> {
    cancel: &'a AtomicBool,
    active: &'a (dyn Fn(u64) -> bool + Sync),
    deadline: &'a (dyn Fn() -> u64 + Sync),
}
impl installer::service::TaskLease for HostLease<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Acquire)
    }
    fn lease_active(&self, now: u64) -> bool {
        (self.active)(now)
    }
    fn deadline_ms(&self) -> u64 {
        (self.deadline)()
    }
    fn cancellation_cause(&self) -> Option<installer::outcome::CancellationCause> {
        if self.cancelled() {
            Some(installer::outcome::CancellationCause::UserCancelled)
        } else if !self.lease_active(trust::now().unwrap_or(u64::MAX)) {
            Some(installer::outcome::CancellationCause::LeaseExpired)
        } else {
            None
        }
    }
}
fn install_patch<L: Fn(u64) -> bool + Sync, D: Fn() -> u64 + Sync>(
    params: &Params,
    _report: &steam::ScanReport,
    platform: Platform,
    cancel: &AtomicBool,
    lease_active: &L,
    deadline: &D,
) -> Result<Value, String> {
    use installer::service::*;
    let request = RequestId::new(params.request_id.as_deref().ok_or("request_id_required")?)
        .map_err(projection_error)?;
    let instance = InstanceId::new(params.instance_id.as_deref().ok_or("instance_required")?)
        .map_err(projection_error)?;
    let target = match (params.release_id.as_deref(), params.selection_id.as_deref()) {
        (Some(_), Some(_)) => return Err("invalid_request".into()),
        (None, None) => return Err("selection_or_release_required".into()),
        (Some(release), None) => InstallTarget::SignedRelease {
            release_id: ReleaseId::new(release).map_err(projection_error)?,
            archive: params
                .archive
                .as_deref()
                .map(LocalArchive::new)
                .transpose()
                .map_err(projection_error)?,
        },
        (None, Some(selection)) => {
            if params.archive.is_some() {
                return Err("archive_not_allowed_for_selection".into());
            }
            if selection.starts_with("zip-selection-") {
                InstallTarget::PublisherZipSelection {
                    selection_id: ZipSelectionId::new(selection).map_err(projection_error)?,
                }
            } else {
                InstallTarget::PublisherExeSelection {
                    selection_id: ExeSelectionId::new(selection).map_err(projection_error)?,
                }
            }
        }
    };
    let state = state(params)?;
    let host =
        ValidatedHostContext::new(&state, &params.roots, platform).map_err(projection_error)?;
    let games = steam::scan(&params.roots);
    let revision =
        trust::revision::SqliteTrustRevisionStore::new_for_games(&state, &games.installs)
            .map_err(projection_error)?;
    let transport = installer::https_transport().map_err(err)?;
    let runtime_result = if matches!(target, InstallTarget::PublisherExeSelection { .. }) {
        Some(worker_runtime(
            &host.installer_paths().root().join("runtime"),
        ))
    } else {
        None
    };
    let lease = HostLease {
        cancel,
        active: lease_active,
        deadline,
    };
    let result = installer::service::install(
        InstallRequest {
            request_id: request,
            instance_id: instance,
            operation: params.operation,
            target,
            cleanup_policy: CleanupPolicy::Disabled,
        },
        InstallerDeps {
            host: &host,
            catalog: &CatalogFiles(params),
            revision_store: &revision,
            transport: transport.as_ref(),
            worker: runtime_result
                .as_ref()
                .and_then(|result| result.as_ref().ok()),
            control: &lease,
            clock: &SystemClock,
            emit: &mut |_| {},
        },
    )
    .map_err(|error| {
        if error.code == "exe_sandbox_unavailable" {
            runtime_result
                .as_ref()
                .and_then(|result| result.as_ref().err())
                .cloned()
                .unwrap_or_else(|| projection_error(error))
        } else {
            projection_error(error)
        }
    })?;
    value(result)
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
    fn legacy_marker_does_not_block_scan_or_install_routes() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        seed(&state);
        let roots = vec![fixture_root(temp.path())];
        let result = execute(
            "scan",
            Params {
                roots: roots.clone(),
                state_dir: Some(state.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let instance = result["installs"][0]["game"]["instance_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let paths = installer::InstallerPaths::new(&state);
        fs::create_dir_all(paths.root()).unwrap();
        for marker in [
            serde_json::to_vec(&json!({"schema_version":1,"instances":["old-library-identity"],"appids":[result["installs"][0]["game"]["appid"]]})).unwrap(),
            b"{broken".to_vec(),
        ] {
            fs::write(paths.root().join("preexisting-installations.json"), marker).unwrap();
            let result = execute(
                "scan",
                Params { roots: roots.clone(), state_dir: Some(state.clone()), ..Default::default() },
            ).unwrap();
            assert!(result["installs"][0].get("preexisting_installation").is_none());
            assert!(result["installs"][0]["patches"].as_array().unwrap().iter().all(|patch| {
                !patch["blockers"].as_array().unwrap().iter().any(|blocker|
                    blocker == "preexisting_installation_requires_review")
            }));
            for (release_id, selection_id) in [
                (Some("signed".into()), None),
                (None, Some("zip-selection-test".into())),
                (None, Some("exe-selection-test".into())),
            ] {
                let error = execute(
                    "install_patch",
                    Params {
                        roots: roots.clone(), state_dir: Some(state.clone()),
                        instance_id: Some(instance.clone()), release_id, selection_id,
                        request_id: Some("new-user-action".into()),
                        operation: install::InstallOperation::Overwrite,
                        ..Default::default()
                    },
                ).unwrap_err();
                assert_ne!(error, "preexisting_installation_requires_review");
                assert_ne!(error, "preexisting_protection_invalid");
            }
        }
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
    fn scan_projects_index_source_candidate_as_publisher_zip() {
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
        let patches = result["installs"][0]["patches"].as_array().unwrap();
        let zip = patches
            .iter()
            .find(|patch| patch["support_level"] == "publisher_zip")
            .expect("index candidate projected as publisher_zip");
        assert_eq!(zip["release_id"], "patchmoe-42");
        assert!(!zip["selection_id"].as_str().unwrap().is_empty());
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
    fn signed_envelope_above_four_mib_preserves_existing_eight_mib_limit() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        use ed25519_dalek::{Signer, SigningKey};
        let (temp, params, game) = fixture();
        let path = params.signed_catalog.as_ref().unwrap();
        let mut envelope: trust::SignedEnvelope =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let mut payload = STANDARD.decode(&envelope.payload).unwrap();
        payload.resize(3 * 1024 * 1024 + 128 * 1024, b' ');
        let secret: [u8; 32] = STANDARD
            .decode(fs::read(temp.path().join("private")).unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        envelope.payload = STANDARD.encode(&payload);
        envelope.signature =
            STANDARD.encode(SigningKey::from_bytes(&secret).sign(&payload).to_bytes());
        let bytes = serde_json::to_vec(&envelope).unwrap();
        assert!(bytes.len() > catalog::MAX_CATALOG_BYTES as usize);
        assert!(bytes.len() < (catalog::MAX_CATALOG_BYTES * 2) as usize);
        fs::write(path, bytes).unwrap();
        verify_signed(&params, trust::now().unwrap()).unwrap();
        execute("preview", copy_params(&params)).unwrap();
        assert!(!params.state_dir.as_ref().unwrap().exists());
        assert!(!game.join("patch.dat").exists());
        assert_eq!(
            execute("install_patch", params).unwrap()["state"],
            "committed"
        );
    }

    #[test]
    fn signed_scan_and_preview_do_not_create_or_advance_trust_state() {
        let (_temp, params, game) = fixture();
        let state = params.state_dir.as_ref().unwrap();
        assert!(!state.exists());
        let mut scan = copy_params(&params);
        scan.instance_id = None;
        scan.release_id = None;
        scan.archive = None;
        let result = execute("scan", copy_params(&scan)).unwrap();
        assert_eq!(result["catalog_trust"], "signed");
        assert!(!state.exists(), "scan must not initialize any state");
        execute("preview", copy_params(&params)).unwrap();
        assert!(!state.exists(), "preview must not initialize any state");
        assert!(!game.join("patch.dat").exists());

        fs::create_dir_all(state).unwrap();
        rusqlite::Connection::open(state.join("trust.sqlite")).unwrap().execute_batch("CREATE TABLE catalog_revision(id INTEGER PRIMARY KEY, revision INTEGER NOT NULL); INSERT INTO catalog_revision VALUES(1,0);").unwrap();
        let trust_before = fs::read(state.join("trust.sqlite")).unwrap();
        execute("scan", scan).unwrap();
        execute("preview", copy_params(&params)).unwrap();
        assert_eq!(
            trust::revision::TrustRevisionStore::highest_revision(
                &trust::revision::SqliteTrustRevisionStore::new(state).unwrap()
            )
            .unwrap(),
            0
        );
        assert_eq!(fs::read(state.join("trust.sqlite")).unwrap(), trust_before);
        assert!(!state.join("installer").exists());
        assert!(!state.join("sources").exists());
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
    use sha2::{Digest, Sha256};
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
        use base64::{engine::general_purpose::STANDARD, Engine};
        use ed25519_dalek::{Signer, SigningKey};
        let key = SigningKey::from_bytes(&[9; 32]);
        let now = trust::now().unwrap();
        let payload = serde_json::to_vec(&trust::CatalogPayload {
            catalog: Catalog {
                schema_version: 1,
                revision: 1,
                releases: vec![release],
            },
            generated_at: now - 1,
            expires_at: now + 3600,
            revoked_release_ids: vec![],
        })
        .unwrap();
        let envelope = trust::SignedEnvelope {
            schema_version: 1,
            key_id: "fixture".into(),
            payload: STANDARD.encode(&payload),
            signature: STANDARD.encode(key.sign(&payload).to_bytes()),
        };
        let signed = state.join("fixture-signed.json");
        let keys = state.join("fixture-keys.json");
        fs::write(&signed, serde_json::to_vec(&envelope).unwrap()).unwrap();
        fs::write(
            &keys,
            serde_json::to_vec(&trust::TrustKeys {
                schema_version: 1,
                keys: vec![trust::TrustKey {
                    key_id: "fixture".into(),
                    public_key: STANDARD.encode(key.verifying_key().to_bytes()),
                    revoked: false,
                }],
            })
            .unwrap(),
        )
        .unwrap();
        let value = execute(
            "install_patch",
            Params {
                roots: vec![game.library_path.clone()],
                state_dir: Some(state.to_owned()),
                instance_id: Some(game.instance_id.clone()),
                release_id: Some(release_id.to_owned()),
                signed_catalog: Some(signed),
                trust_keys: Some(keys),
                archive: Some(archive.to_owned()),
                request_id: Some(format!("fixture-{}", release_id)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(value["state"], "committed");
        value["plan_id"].as_str().unwrap().to_owned()
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
        // Same-named second whitelisted mirror (the real kagura-cn v4 shape):
        // resolution must deterministically pick the first, not block.
        let mut second = candidate.mirrors[0].clone();
        second.url = "https://kaguragamespatch.com/a.exe".into();
        candidate.mirrors.push(second);
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
        let blockers = dynamic["blockers"].as_array().unwrap();
        assert!(
            !blockers
                .iter()
                .any(|blocker| blocker == "exe_download_ambiguous"),
            "homogeneous mirrors must not block resolution: {blockers:?}"
        );
        // Resolution succeeded, so availability is only ever held back by the
        // environment's EXE sandbox probe (`bundled_worker_missing` in this
        // harness, where the unit-test binary sits in `target/debug/deps`
        // without the worker) — never by a download-availability error.
        let sandbox_blocked = blockers
            .iter()
            .any(|blocker| blocker == "exe_sandbox_unavailable");
        assert_eq!(dynamic["download_available"], !sandbox_blocked);
    }

    #[test]
    fn scan_blocks_exe_demo_variant_as_a_clean_blocker() {
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
        candidate.variant = steam_patch_core::sources::model::Variant::Demo;
        candidate.mirrors[0].format = "exe".into();
        candidate.mirrors[0].url = "https://kaguraserver.com/a.exe".into();
        candidate.format_reported = ArtifactFormat::Exe;
        candidate.format_observed = ArtifactFormat::Exe;
        // The demo candidate must still be projected (with its own selection id);
        // only resolution rejects it, which the UI surfaces as a clean blocker.
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
            .expect("exe demo selection projected");
        assert_eq!(dynamic["release_id"], "source-kagura-cn-42");
        assert_eq!(dynamic["download_available"], false);
        assert!(dynamic["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker == "exe_candidate_variant_unsupported"));
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
            selection_id: None,
            content_id: None,
        };
        let value = serde_json::to_value(installer::outcome::InstallerOutcome::from_transaction(
            &transaction,
            Some("zip-selection-000000000000000000000000-42"),
            None,
        ))
        .unwrap();
        assert_eq!(value["state"], "needs_recovery");
        assert_eq!(value["recovery"]["required"], true);
        assert_eq!(value["content_id"], Value::Null);
        assert_eq!(
            value["selection_id"],
            "zip-selection-000000000000000000000000-42"
        );
    }
}
