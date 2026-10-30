#[cfg(test)]
use patchhub_core::catalog::SupportLevel;
use patchhub_core::projection::DomainError;
use patchhub_core::{
    catalog::{self, Catalog, Platform},
    install, installer, projection, sources, steam,
    trust::{self, VerifiedCatalog},
};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Debug, Default, Clone, Serialize)]
pub struct Params {
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
    pub manual_download: Option<installer::service::ManualDownloadInput>,
    pub operation: install::InstallOperation,
    pub request_id: Option<String>,
    pub cleanup_policy: installer::service::CleanupPolicy,
    pub source_id: Option<String>,
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub subscribed: Option<bool>,
    /// Automatic update interval for `set_source_update_interval` (0 = off).
    pub interval_hours: Option<u64>,
    /// `match_games` instance filter (empty = every installed game).
    pub instances: Vec<String>,
    /// `project_instances` requested instance ids.
    pub instance_ids: Vec<String>,
    /// Declarative rule for `add_source` (validated against `RawSourceRule`).
    pub rule: Option<Value>,
    pub name_hints: Vec<String>,

    pub force: bool,
    pub force_details: Option<bool>,
}

impl Params {
    pub fn from_wire(request: &patchhub_protocol::ValidatedRequest) -> Self {
        let h = request.params.host();
        let v = request.params.params_value();
        let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
        Self {
            roots: h.roots.clone(),
            catalog: h.catalog.clone(),
            signed_catalog: h.signed_catalog.clone(),
            trust_keys: h.trust_keys.clone(),
            state_dir: h.state_dir.clone(),
            platform: Some(h.platform),
            request_id: Some(request.request_id.clone()),
            instance_id: text("instance_id"),
            appid: v.get("appid").and_then(Value::as_u64).map(|n| n as u32),
            release_id: text("release_id"),
            selection_id: text("selection_id"),
            manual_download: v.get("manual_download").filter(|v| !v.is_null()).map(|v| {
                serde_json::from_value(v.clone()).expect("validated manual download input")
            }),
            plan_id: text("plan_id"),
            archive: text("archive").map(PathBuf::from),
            source_id: text("source_id"),
            name: text("name"),
            operation: match text("operation").as_deref() {
                Some("reinstall") => install::InstallOperation::Reinstall,
                Some("overwrite") => install::InstallOperation::Overwrite,
                _ => install::InstallOperation::Install,
            },
            cleanup_policy: match text("cleanup_policy").as_deref() {
                Some("after_commit") => installer::service::CleanupPolicy::AfterCommit,
                _ => installer::service::CleanupPolicy::Disabled,
            },
            enabled: v.get("enabled").and_then(Value::as_bool),
            subscribed: v.get("subscribed").and_then(Value::as_bool),
            interval_hours: v.get("interval_hours").and_then(Value::as_u64),
            instances: string_list(&v, "instances"),
            instance_ids: string_list(&v, "instance_ids"),
            rule: v.get("rule").cloned(),
            name_hints: string_list(&v, "name_hints"),

            force: v.get("force").and_then(Value::as_bool).unwrap_or(false),
            force_details: v.get("force_details").and_then(Value::as_bool),
        }
    }
}

fn string_list(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Formal methods are owned by the protocol crate.
pub const METHODS: &[&str] = &patchhub_protocol::METHODS;
pub const SOURCE_METHODS: &[&str] = &patchhub_protocol::SOURCE_METHODS;

/// Streaming source-task methods that emit `source_progress`/`source_update`/
/// `match_progress`/`match_update` lines before the single terminal response.
pub const SOURCE_STREAM_METHODS: &[&str] = &["update_source", "match_games", "rematch_game"];

/// Per-task cursor that renders `operation::Progress` into the flat v4
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
    pub fn event(&mut self, progress: &patchhub_core::operation::Progress) -> Value {
        let (stage, substage) = match progress.stage.as_str() {
            // Archive telemetry can precede an explicit stage in signed/local ZIP
            // routes. Protocol v4 never exposes the internal empty default.
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

/// `get_capabilities` v5 payload (p1-host-contract §3.1, p1-contract §9.2):
/// installer and source sub-objects merged with the full 22-method whitelist.
fn capabilities(_params: &Params) -> Value {
    let mut methods: Vec<&str> = Vec::with_capacity(METHODS.len());
    methods.extend(METHODS.iter().copied());
    json!({
        "protocol_version": patchhub_protocol::PROTOCOL_VERSION,
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
                "exe": {"available": installer::host_identity_blocker().is_none(),
                    "blockers": installer::host_identity_blocker().map(|code|json!([{"code":code}])).unwrap_or_else(||json!([]))},
            },
        },
        "sources": {
            "index_schema_version": 1,
            "detail_schema_version": 1,
            "match_schema_version": 1,
            "schedule_schema_version": 1,
            "adapter_version": 1,
            "parser_version": sources::PARSER_VERSION,
            "encoding_version": sources::SOURCES_ENCODING_VERSION,
            "methods": SOURCE_METHODS,
        },
        "methods": methods,
        "contracts": {"projection_version":2,"actions_version":1,"outcome_version":1,"request_validation_version":2},
    })
}

/// The bundled worker next to the running host executable. Only the trusted
/// host fixes the executable location; RPC cannot set it.
fn bundled_worker() -> Option<PathBuf> {
    let host = std::env::current_exe().ok()?;
    Some(host.parent()?.join(if cfg!(windows) {
        "patchhub-worker.exe"
    } else {
        "patchhub-worker"
    }))
}

fn worker_runtime() -> Result<patchhub_core::installer::WorkerRuntime, DomainError> {
    let worker = bundled_worker().ok_or_else(|| DomainError::code("bundled_worker_missing"))?;
    installer::WorkerRuntime::new(worker).map_err(|e| failed("worker_start_failed", e.to_string()))
}

/// Preserve exact machine codes. Only established diagnostic prefixes are split;
/// free-form OS/database text remains detail and can never become a wire code.
fn err(error: impl std::fmt::Display) -> DomainError {
    let text = error.to_string();
    if !text.is_empty()
        && text.len() <= 128
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return DomainError::code(text);
    }
    for prefix in [
        "script_source_syntax_error",
        "script_source_runtime_error",
        "script_source_contract_error",
        "script_source_digest_mismatch",
        "unsupported_script_api_version",
        "journal_error",
        "invalid_zip",
        "zip_member",
        "file_changed",
        "recovery_conflict",
        "baseline_conflict",
    ] {
        if let Some(detail) = text.strip_prefix(prefix).and_then(|s| s.strip_prefix(':')) {
            return DomainError::new(prefix, detail);
        }
    }
    DomainError::new("io_error", text)
}
fn value(data: impl Serialize) -> Result<Value, DomainError> {
    serde_json::to_value(data).map_err(err)
}

#[cfg(test)]
mod script_error_tests {
    #[test]
    fn script_error_codes_keep_the_complete_lua_detail() {
        for code in [
            "script_source_syntax_error",
            "script_source_runtime_error",
            "script_source_contract_error",
            "script_source_digest_mismatch",
            "unsupported_script_api_version",
        ] {
            let detail = format!(
                "source=custom, entry=resolve_page: source/custom.lua:7: {}",
                "中文".repeat(1200)
            );
            let domain = super::err(format!("{code}: {detail}"));
            assert_eq!(domain.code, code);
            assert!(domain.detail.unwrap().contains(&detail));
        }
    }
}
fn projection_error(error: DomainError) -> DomainError {
    error
}

fn host_platform() -> Platform {
    if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Steamos
    }
}
fn state(params: &Params) -> Result<PathBuf, DomainError> {
    params
        .state_dir
        .as_ref()
        .map(|p| std::path::absolute(p).map_err(err))
        .unwrap_or_else(|| Err(DomainError::code("state_dir_required")))
}

fn state_safe(path: &Path) -> Result<(), DomainError> {
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
                    return Err(DomainError::code("linked_state_path"));
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
fn verify_signed(params: &Params, now: u64) -> Result<VerifiedCatalog, DomainError> {
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

fn canonical_future(path: &Path) -> Result<PathBuf, DomainError> {
    let absolute = std::path::absolute(path).map_err(err)?;
    let mut base = absolute.as_path();
    let mut suffix = Vec::new();
    while !base.exists() {
        suffix.push(
            base.file_name()
                .ok_or_else(|| DomainError::code("invalid_state_path"))?
                .to_owned(),
        );
        base = base
            .parent()
            .ok_or_else(|| DomainError::code("invalid_state_path"))?;
    }
    let mut result = fs::canonicalize(base).map_err(err)?;
    for part in suffix.into_iter().rev() {
        result.push(part);
    }
    Ok(result)
}

fn validate_state_boundary(
    state: &Path,
    installs: &[steam::GameInstall],
) -> Result<(), DomainError> {
    state_safe(state)?;
    let state = canonical_future(state)?;
    for game in installs {
        let game = canonical_future(&game.install_path)?;
        if state.starts_with(&game) || game.starts_with(&state) {
            return Err(DomainError::code("state_overlaps_game"));
        }
    }
    Ok(())
}

pub fn execute(method: &str, params: Params) -> Result<Value, DomainError> {
    let cancel = Arc::new(AtomicBool::new(false));
    execute_controlled(method, params, &cancel)
}

pub fn failed(code: impl Into<String>, detail: impl Into<String>) -> DomainError {
    let detail = detail.into();
    if detail.is_empty() {
        DomainError::code(code)
    } else {
        DomainError::new(code, detail)
    }
}

/// Executes one method with the task control channel available to
/// `install_patch` (p1-zip-orchestration §3.10/§4.1).
pub fn execute_controlled(
    method: &str,
    params: Params,
    cancel: &Arc<AtomicBool>,
) -> Result<Value, DomainError> {
    if !METHODS.contains(&method) {
        return Err(DomainError::code("unsupported_method"));
    }
    if params.roots.len() > 32 {
        return Err(DomainError::code("too_many_roots"));
    }
    if method == "scan" && (params.instance_id.is_some() || params.release_id.is_some()) {
        return Err(DomainError::code("unexpected_preview_arguments"));
    }
    if let Some(code) = installer::host_identity_blocker() {
        if [
            "install_patch",
            "apply",
            "recover",
            "discard_plan",
            "cleanup_cache",
            "cleanup_backups",
        ]
        .contains(&method)
        {
            return Err(DomainError::code(code));
        }
    }
    if method == "get_capabilities" {
        return Ok(capabilities(&params));
    }
    if method == "backup_status" || method == "cleanup_backups" {
        return backups(&params, method == "cleanup_backups");
    }
    if method == "cache_status" || method == "cleanup_cache" {
        return cache(&params, method == "cleanup_cache");
    }
    // Source tasks stream progress; direct CLI invocations run the same
    // implementation with no event sink.
    if SOURCE_STREAM_METHODS.contains(&method) {
        return execute_streaming(method, params, cancel, &mut |_| {});
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
    if SOURCE_METHODS.contains(&method) && method != "project_instances" {
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
                        params
                            .plan_id
                            .as_deref()
                            .ok_or_else(|| DomainError::code("plan_required"))?,
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
        return install_patch(&params, &report, platform, cancel);
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
    let capabilities = projection::HostCapabilities {
        zip: projection::LinkCapability {
            available: installer::host_identity_blocker().is_none(),
            blockers: installer::host_identity_blocker()
                .map(projection::DomainError::code)
                .into_iter()
                .collect(),
        },
        exe: projection::LinkCapability {
            available: installer::host_identity_blocker().is_none(),
            blockers: installer::host_identity_blocker()
                .map(projection::DomainError::code)
                .into_iter()
                .collect(),
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
    if method == "project_instances" {
        let instances =
            projection::project_batch(&inputs, &params.instance_ids).map_err(projection_error)?;
        let instances = attach_match_discovery(instances, params.state_dir.as_deref())?;
        return Ok(json!({
            "settings_revision": source_snapshot
                .as_ref()
                .map(|snapshot| snapshot.settings_revision()),
            "instances": instances,
        }));
    }
    if method == "preview" {
        if params.selection_id.is_some() && params.state_dir.is_none() {
            return Err(DomainError::code("state_dir_required"));
        }
        // Internal direct-call checks mirror the public protocol target boundary.
        let target = match (params.release_id, params.selection_id) {
            (Some(_), Some(_)) => return Err(DomainError::code("invalid_request")),
            (None, None) => return Err(DomainError::code("selection_or_release_required")),
            (Some(id), None) => projection::PreviewTarget::SignedRelease(id),
            (None, Some(id)) if id.starts_with("exe-selection-") => {
                projection::PreviewTarget::PublisherExeSelection(id)
            }
            (None, Some(id)) if id.starts_with("zip-selection-") => {
                projection::PreviewTarget::PublisherZipSelection(id)
            }
            _ => return Err(DomainError::code("invalid_selection_id")),
        };
        return value(
            projection::preview(
                &inputs,
                projection::PreviewRequest {
                    instance_id: params
                        .instance_id
                        .ok_or_else(|| DomainError::code("instance_not_found"))?,
                    target,
                },
            )
            .map_err(projection_error)?,
        );
    }
    let mut projected = projection::scan(inputs).map_err(projection_error)?;
    projected.installs = attach_match_discovery(projected.installs, params.state_dir.as_deref())?;
    let mut result = value(projected)?;
    // Catalog failures remain structured read-only diagnostics.
    result["catalog_error"] = json!(catalog_error);
    Ok(result)
}

/// Replaces each projection's discovery snapshot with the persisted match
/// record. This never touches the network or creates source state; a missing
/// record is `pending` with no candidates.
fn attach_match_discovery(
    mut installs: Vec<projection::InstanceProjection>,
    state_dir: Option<&Path>,
) -> Result<Vec<projection::InstanceProjection>, DomainError> {
    let Some(state_dir) = state_dir else {
        return Ok(installs);
    };
    for install in &mut installs {
        let summary =
            sources::matching::match_summary_at(state_dir, install.game.appid).map_err(err)?;
        install.discovery = Some(match summary {
            Some(match_result) => sources::GameDiscovery {
                status: match_status(&match_result.status),
                candidates: match_result.candidates,
                diagnostics: match_result.diagnostics,
            },
            None => sources::GameDiscovery {
                status: "pending".to_string(),
                candidates: Vec::new(),
                diagnostics: Vec::new(),
            },
        });
    }
    Ok(installs)
}

fn match_status(status: &sources::GameMatchStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "pending".to_string())
}

fn backups(params: &Params, delete: bool) -> Result<Value, DomainError> {
    let state = state(params)?;
    let host = installer::service::ValidatedHostContext::new(
        &state,
        &params.roots,
        params.platform.unwrap_or(if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Steamos
        }),
    )
    .map_err(projection_error)?;
    let service = installer::backup::BackupService::new(&host);
    if delete {
        value(service.cleanup().map_err(projection_error)?)
    } else {
        value(service.inspect().map_err(projection_error)?)
    }
}

/// Thin adapter to the validated core cache service.
fn cache(params: &Params, delete: bool) -> Result<Value, DomainError> {
    let state = state(params)?;
    let host = installer::service::ValidatedHostContext::new(
        &state,
        &params.roots,
        params.platform.unwrap_or(if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Steamos
        }),
    )
    .map_err(projection_error)?;
    let service = installer::cache::CacheService::new(&host);
    if delete {
        value(service.cleanup().map_err(projection_error)?)
    } else {
        value(service.inspect().map_err(projection_error)?)
    }
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
) -> Result<&'a steam::GameInstall, DomainError> {
    report
        .installs
        .iter()
        .find(|game| game.instance_id == instance_id)
        .ok_or_else(|| DomainError::code("instance_not_found"))
}

/// Builds a [`sources::GameQuery`] from `instance_id` (preferred) or `appid`.
///
/// For an `appid`-only request the trusted scan is consulted for the install
/// name; when the game is not installed the query carries only the AppID and
/// matching falls back to cache entries that carry it (P5 §1).
fn game_from_params(
    params: &Params,
    report: &steam::ScanReport,
) -> Result<sources::GameQuery, DomainError> {
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
    Err(DomainError::code("instance_required"))
}

/// Resolves the AppID from `instance_id` (preferred) or the numeric `appid`.
fn resolve_appid(params: &Params, report: &steam::ScanReport) -> Result<u32, DomainError> {
    if let Some(instance_id) = params.instance_id.as_deref() {
        return Ok(install_for(report, instance_id)?.appid);
    }
    params
        .appid
        .ok_or_else(|| DomainError::code("instance_required"))
}

fn open_sources(params: &Params) -> Result<sources::Sources, DomainError> {
    let state = state(params)?;
    state_safe(&state)?;
    sources::Sources::open(&state).map_err(err)
}

fn scan_roots(params: &Params) -> Vec<PathBuf> {
    if params.roots.is_empty() {
        steam::discover_roots()
    } else {
        params.roots.clone()
    }
}

/// Builds a fresh HTTP context for source update/match tasks.
fn source_http_context() -> Result<sources::http::RequestContext, DomainError> {
    sources::http::RequestContext::new().map_err(err)
}

fn match_update_value(result: &sources::GameMatch, revision: u64) -> Value {
    json!({
        "event": "match_update",
        "instance_id": result.instance_id,
        "appid": result.appid,
        "status": match_status(&result.status),
        "candidates": result.candidates,
        "diagnostics": result.diagnostics,
        "settings_revision": revision,
    })
}

fn source_progress_value(
    source_id: &str,
    phase: &str,
    completed: usize,
    total: usize,
    running: bool,
    revision: u64,
) -> Value {
    json!({
        "event": "source_progress",
        "source_id": source_id,
        "phase": phase,
        "completed": completed,
        "total": total,
        "running": running,
        "settings_revision": revision,
    })
}

fn match_progress_value(
    phase: &str,
    completed: usize,
    total: usize,
    running: bool,
    revision: u64,
) -> Value {
    json!({
        "event": "match_progress",
        "phase": phase,
        "completed": completed,
        "total": total,
        "running": running,
        "settings_revision": revision,
    })
}

fn match_games_result(
    revision: u64,
    source_revisions: std::collections::BTreeMap<String, u64>,
    results: Vec<sources::GameMatch>,
) -> Result<Value, DomainError> {
    value(json!({
        "settings_revision": revision,
        "source_revisions": source_revisions,
        "results": results,
    }))
}

fn execute_source_method(
    method: &str,
    params: &Params,
    report: &steam::ScanReport,
) -> Result<Value, DomainError> {
    match method {
        "list_sources" => value(open_sources(params)?.registry().list_sources()),
        "list_game_sources" => {
            let game = game_from_params(params, report)?;
            value(open_sources(params)?.registry().list_game_sources(&game))
        }
        "set_source_name" => {
            let source_id = params
                .source_id
                .as_deref()
                .ok_or_else(|| DomainError::code("source_id_required"))?;
            let name = params
                .name
                .as_deref()
                .ok_or_else(|| DomainError::code("name_required"))?;
            let mut handle = open_sources(params)?;
            let name = handle
                .registry_mut()
                .set_name(source_id, name)
                .map_err(err)?;
            Ok(json!({
                "settings_revision": handle.settings_revision(),
                "source_id": source_id,
                "name": name,
            }))
        }
        "set_source_enabled" => {
            let source_id = params
                .source_id
                .as_deref()
                .ok_or_else(|| DomainError::code("source_id_required"))?;
            let enabled = params
                .enabled
                .ok_or_else(|| DomainError::code("enabled_required"))?;
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
            let source_id = params
                .source_id
                .as_deref()
                .ok_or_else(|| DomainError::code("source_id_required"))?;
            let subscribed = params
                .subscribed
                .ok_or_else(|| DomainError::code("subscribed_required"))?;
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
            let rule_value = params
                .rule
                .clone()
                .ok_or_else(|| DomainError::code("rule_required"))?;
            let raw: sources::RawSourceRule = serde_json::from_value(rule_value).map_err(err)?;
            let mut handle = open_sources(params)?;
            let rule = handle.registry_mut().add_source(&raw).map_err(err)?;
            Ok(json!({
                "settings_revision": handle.settings_revision(),
                "rule": rule,
            }))
        }
        "set_source_update_interval" => {
            let source_id = params
                .source_id
                .as_deref()
                .ok_or_else(|| DomainError::code("source_id_required"))?;
            let interval_hours = params
                .interval_hours
                .ok_or_else(|| DomainError::code("interval_hours_required"))?;
            let mut handle = open_sources(params)?;
            handle
                .registry_mut()
                .set_update_interval(source_id, interval_hours)
                .map_err(err)?;
            Ok(json!({
                "settings_revision": handle.settings_revision(),
                "source_id": source_id,
                "interval_hours": interval_hours,
            }))
        }
        _ => Err(DomainError::code("unsupported_method")),
    }
}

/// Streaming dispatch for `serve-task`: the source and match tasks emit their
/// typed events through `emit` and then return the single final result.
pub fn execute_streaming(
    method: &str,
    params: Params,
    cancel: &Arc<AtomicBool>,
    emit: &mut dyn FnMut(Value),
) -> Result<Value, DomainError> {
    match method {
        "update_source" => update_source_impl(&params, cancel, emit),
        "match_games" => match_games_impl(&params, cancel, emit),
        "rematch_game" => rematch_game_impl(&params, cancel, emit),
        _ => execute(method, params),
    }
}

/// Installed games requested by `match_games`; an empty filter means every
/// installed game.
fn selected_games(
    params: &Params,
    report: &steam::ScanReport,
) -> Result<Vec<sources::GameQuery>, DomainError> {
    if params.instances.is_empty() {
        return Ok(report
            .installs
            .iter()
            .filter(|game| game.availability == "installed")
            .map(game_query)
            .collect());
    }
    params
        .instances
        .iter()
        .map(|instance_id| install_for(report, instance_id).map(game_query))
        .collect()
}

fn validate_match_query(games: &[sources::GameQuery], params: &Params) -> Result<(), DomainError> {
    for game in games {
        sources::registry::SourceRegistry::validate_query(game, &params.name_hints).map_err(err)?;
    }
    Ok(())
}

fn update_source_impl(
    params: &Params,
    cancel: &Arc<AtomicBool>,
    emit: &mut dyn FnMut(Value),
) -> Result<Value, DomainError> {
    let state = state(params)?;
    state_safe(&state)?;
    let source_id = params
        .source_id
        .as_deref()
        .ok_or_else(|| DomainError::code("source_id_required"))?;
    let handle = sources::Sources::open(&state).map_err(err)?;
    let rules = handle.registry().rules().to_vec();
    let rule = rules
        .iter()
        .find(|rule| rule.id == source_id)
        .cloned()
        .ok_or_else(|| DomainError::code("unknown_source"))?;
    let overrides = handle.registry().overrides().clone();
    let revision = handle.settings_revision();
    let now = handle.registry().now_unix();
    let store = handle.store();
    let previous_revision = store
        .load_index(source_id)
        .map_err(err)?
        .map(|index| index.revision)
        .unwrap_or(0);
    let mut ctx = source_http_context()?;

    emit(source_progress_value(
        source_id, "index", 0, 0, true, revision,
    ));
    let outcome =
        sources::update::update_source(store, &rule, params.force, &mut ctx, cancel, |_| {});
    emit(source_progress_value(
        source_id, "index", 0, 0, false, revision,
    ));
    let outcome = outcome.map_err(err)?;
    emit(json!({
        "event": "source_update",
        "source_id": source_id,
        "status": outcome.status,
        "settings_revision": revision,
    }));

    let mut affected_appids = Vec::new();
    // No index content change means there is nothing to re-derive.
    if outcome.revision != previous_revision {
        let report = steam::scan(&scan_roots(params));
        let games: Vec<sources::GameQuery> = report
            .installs
            .iter()
            .filter(|game| {
                game.availability == "installed"
                    && handle.registry().game_subscribed(game.appid, source_id)
            })
            .map(game_query)
            .collect();
        let total = games.len();
        emit(source_progress_value(
            source_id, "details", 0, total, true, revision,
        ));
        let mut completed = 0usize;
        let results = sources::matching::rematch_source(
            store,
            &rules,
            &overrides,
            &games,
            source_id,
            &params.name_hints,
            &mut ctx,
            cancel,
            now,
            |result| {
                emit(match_update_value(result, revision));
                completed += 1;
                emit(source_progress_value(
                    source_id, "matched", completed, total, true, revision,
                ));
            },
        )
        .map_err(err)?;
        emit(source_progress_value(
            source_id, "matched", completed, total, false, revision,
        ));
        affected_appids = results.iter().map(|result| result.appid).collect();
        affected_appids.sort_unstable();
        affected_appids.dedup();
    }

    Ok(json!({
        "source_id": source_id,
        "settings_revision": revision,
        "status": outcome.status,
        "revision": outcome.revision,
        "complete": outcome.complete,
        "affected_appids": affected_appids,
    }))
}

fn match_games_impl(
    params: &Params,
    cancel: &Arc<AtomicBool>,
    emit: &mut dyn FnMut(Value),
) -> Result<Value, DomainError> {
    let state = state(params)?;
    state_safe(&state)?;
    let report = steam::scan(&scan_roots(params));
    validate_state_boundary(&state, &report.installs)?;
    let games = selected_games(params, &report)?;
    let handle = sources::Sources::open(&state).map_err(err)?;
    let rules = handle.registry().rules().to_vec();
    let overrides = handle.registry().overrides().clone();
    let revision = handle.settings_revision();
    let now = handle.registry().now_unix();
    let store = handle.store();
    validate_match_query(&games, params)?;
    let mut ctx = source_http_context()?;
    let total = games.len();
    emit(match_progress_value("inventory", 0, total, true, revision));
    let sink = std::cell::RefCell::new(emit);
    let results = sources::matching::match_games(
        store,
        &rules,
        &overrides,
        &games,
        &params.name_hints,
        params.force_details.unwrap_or(false),
        &mut ctx,
        cancel,
        now,
        |progress| {
            let mut emit = sink.borrow_mut();
            emit(match_progress_value(
                &progress.phase,
                progress.completed,
                progress.total,
                progress.running,
                revision,
            ));
        },
        |result| {
            let mut emit = sink.borrow_mut();
            emit(match_update_value(result, revision));
        },
    )
    .map_err(err)?;
    match_games_result(
        revision,
        sources::matching::source_revisions(store, &rules),
        results,
    )
}

fn rematch_game_impl(
    params: &Params,
    cancel: &Arc<AtomicBool>,
    emit: &mut dyn FnMut(Value),
) -> Result<Value, DomainError> {
    let state = state(params)?;
    state_safe(&state)?;
    let report = steam::scan(&scan_roots(params));
    validate_state_boundary(&state, &report.installs)?;
    let game = game_from_params(params, &report)?;
    let handle = sources::Sources::open(&state).map_err(err)?;
    let rules = handle.registry().rules().to_vec();
    let overrides = handle.registry().overrides().clone();
    let revision = handle.settings_revision();
    let now = handle.registry().now_unix();
    let store = handle.store();
    validate_match_query(std::slice::from_ref(&game), params)?;
    let mut ctx = source_http_context()?;
    emit(match_progress_value("inventory", 0, 1, true, revision));
    let sink = std::cell::RefCell::new(emit);
    let result = sources::matching::rematch_game(
        store,
        &rules,
        &overrides,
        &game,
        &params.name_hints,
        &mut ctx,
        cancel,
        now,
        |result| {
            let mut emit = sink.borrow_mut();
            emit(match_update_value(result, revision));
        },
    )
    .map_err(err)?;
    sink.borrow_mut()(match_progress_value("inventory", 1, 1, false, revision));
    match_games_result(
        revision,
        sources::matching::source_revisions(store, &rules),
        vec![result],
    )
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
            assert_eq!(result.unwrap_err().code, "state_overlaps_game");
            assert_eq!(fs::read_dir(&game).unwrap().count(), 0);
        }
    }
}

/// Single-task install entry point (p1-host-contract §1.1 A; p1-zip-orchestration
/// §4.1). Validates mutual exclusion and dispatches to the signed `release_id`
/// route (priority) or the dynamic `selection_id` prefix route.
struct HostLease<'a> {
    cancel: &'a AtomicBool,
}
impl installer::service::TaskLease for HostLease<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Acquire)
    }
    fn cancellation_cause(&self) -> Option<installer::outcome::CancellationCause> {
        self.cancelled()
            .then_some(installer::outcome::CancellationCause::UserCancelled)
    }
}
fn install_patch(
    params: &Params,
    _report: &steam::ScanReport,
    platform: Platform,
    cancel: &Arc<AtomicBool>,
) -> Result<Value, DomainError> {
    use installer::service::*;
    let request = RequestId::new(
        params
            .request_id
            .as_deref()
            .ok_or_else(|| DomainError::code("request_id_required"))?,
    )
    .map_err(projection_error)?;
    let instance = InstanceId::new(
        params
            .instance_id
            .as_deref()
            .ok_or_else(|| DomainError::code("instance_required"))?,
    )
    .map_err(projection_error)?;
    let target = if let Some(input) = &params.manual_download {
        if params.release_id.is_some() || params.selection_id.is_some() || params.archive.is_some()
        {
            return Err(DomainError::code("invalid_request"));
        }
        InstallTarget::ManualDownload {
            input: input.clone(),
        }
    } else {
        match (params.release_id.as_deref(), params.selection_id.as_deref()) {
            (Some(_), Some(_)) => return Err(DomainError::code("invalid_request")),
            (None, None) => return Err(DomainError::code("selection_or_release_required")),
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
                    return Err(DomainError::code("archive_not_allowed_for_selection"));
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
    let runtime_result = if matches!(
        target,
        InstallTarget::PublisherExeSelection { .. } | InstallTarget::ManualDownload { .. }
    ) {
        Some(worker_runtime())
    } else {
        None
    };
    let lease = HostLease { cancel };
    let result = installer::service::install(
        InstallRequest {
            request_id: request,
            instance_id: instance,
            operation: params.operation,
            target,
            cleanup_policy: params.cleanup_policy,
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
        if error.code == "bundled_worker_missing" {
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
    use patchhub_core::sources::model::{SourceIndex, SourceIndexEntry};

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

    /// Initializes the v5 source namespace (empty index/match state).
    fn seed(state: &Path) {
        sources::Sources::open(state).unwrap();
    }

    /// Recursive path -> bytes snapshot so a test can prove a read wrote nothing.
    fn state_files(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        fn walk(base: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
            let Ok(entries) = fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(base, &path, out);
                } else {
                    let relative = path
                        .strip_prefix(base)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned();
                    out.insert(relative, fs::read(&path).unwrap_or_default());
                }
            }
        }
        let mut files = std::collections::BTreeMap::new();
        walk(root, root, &mut files);
        files
    }

    #[test]
    fn capabilities_and_unknown_method() {
        let capabilities = execute("get_capabilities", Params::default()).unwrap();
        let methods: Vec<&str> = capabilities["methods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|method| method.as_str().unwrap())
            .collect();
        assert_eq!(methods.len(), 22);
        assert!(methods.contains(&"match_games"));
        assert!(methods.contains(&"project_instances"));
        assert_eq!(capabilities["sources"]["index_schema_version"], 1);
        assert_eq!(capabilities["sources"]["detail_schema_version"], 1);
        assert_eq!(capabilities["sources"]["match_schema_version"], 1);
        assert_eq!(capabilities["sources"]["schedule_schema_version"], 1);
        assert_eq!(capabilities["sources"]["parser_version"], 2);
        assert_eq!(capabilities["sources"]["encoding_version"], 1);
        assert_eq!(
            capabilities["sources"]["methods"].as_array().unwrap().len(),
            10
        );
        assert_eq!(capabilities["installer"]["schema_version"], 1);
        assert_eq!(
            execute("bogus", Params::default()).unwrap_err().code,
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
        // Repeated calls remain read-only.
        let second = Params {
            state_dir: Some(state.clone()),
            ..Default::default()
        };
        execute("get_capabilities", second).unwrap();
        assert!(!state.exists(), "capabilities must stay read-only");
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
        for source in array {
            assert_eq!(source["revision"], 0);
            assert_eq!(source["complete"], false);
            assert_eq!(source["interval_hours"], 0);
            assert!(source["next_due_at"].is_null());
        }
    }

    #[test]
    fn set_source_name_returns_saved_name_and_persists_across_calls() {
        let temp = tempfile::tempdir().unwrap();
        let params = Params {
            roots: vec![fixture_root(temp.path())],
            state_dir: Some(temp.path().join("state")),
            source_id: Some("patchmoe".to_string()),
            name: Some("  自定义发行商  ".to_string()),
            ..Default::default()
        };
        let result = execute("set_source_name", params.clone()).unwrap();
        assert_eq!(result["name"], "自定义发行商");
        assert_eq!(result["source_id"], "patchmoe");
        assert_eq!(result["settings_revision"], 2);
        let sources = execute("list_sources", params.clone()).unwrap();
        assert_eq!(sources[0]["name"], "自定义发行商");
        assert_eq!(sources[0]["pages"][0], "https://www.patch.moe/?flag=1");
        assert_eq!(
            execute(
                "set_source_name",
                Params {
                    name: Some("  ".into()),
                    ..params
                }
            )
            .unwrap_err()
            .code,
            "invalid_source_name"
        );
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

    fn save_match_record(
        state: &Path,
        appid: u32,
        fallback_status: Option<sources::FallbackMatchStatus>,
    ) {
        let store = sources::store::SourceStore::open(state).unwrap();
        store
            .save_match(&sources::GameMatchRecord {
                schema_version: 1,
                appid,
                name: "Synthetic".to_string(),
                localized_name: None,
                input_key: String::new(),
                status: sources::GameMatchStatus::Pending,
                sources: Default::default(),
                fallback: fallback_status.map(|status| sources::FallbackMatchRecord {
                    sources_fingerprint: Some(sources::matching::fallback_sources_fingerprint(
                        sources::Sources::open(state).unwrap().registry().rules(),
                        &sources::model::Overrides::default(),
                        appid,
                    )),
                    query_key: "fixture".to_string(),
                    status,
                    candidates: Vec::new(),
                    checked_at: 1,
                    diagnostics: Vec::new(),
                }),
                updated_at: 1,
            })
            .unwrap();
    }

    #[test]
    fn scan_reads_the_persisted_match_summary() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        save_match_record(&state, 42, Some(sources::FallbackMatchStatus::NoMatch));
        let result = execute(
            "scan",
            Params {
                roots: vec![fixture_root(temp.path())],
                state_dir: Some(state),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result["installs"][0]["discovery"]["status"], "no_match");
        assert!(result["installs"][0]["discovery"]["candidates"].is_array());
        assert_eq!(result["sources"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn scan_without_state_reports_pending_discovery_without_creating_state() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("absent-state");
        let result = execute(
            "scan",
            Params {
                roots: vec![fixture_root(temp.path())],
                state_dir: Some(state.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result["installs"][0]["discovery"]["status"], "pending");
        assert!(result["installs"][0]["discovery"]["candidates"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(!state.exists(), "scan must stay read-only");
    }

    #[test]
    fn match_games_streams_inventory_for_an_empty_library() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let mut events = Vec::new();
        let result = execute_streaming(
            "match_games",
            Params {
                roots: vec![temp.path().to_path_buf()],
                state_dir: Some(state),
                ..Default::default()
            },
            &Arc::new(AtomicBool::new(false)),
            &mut |event| events.push(event),
        )
        .unwrap();
        assert!(events
            .iter()
            .any(|event| { event["event"] == "match_progress" && event["phase"] == "inventory" }));
        assert!(result["settings_revision"].is_number());
        assert!(result["source_revisions"].is_object());
        assert!(result["results"].as_array().unwrap().is_empty());
    }

    #[test]
    fn rematch_game_requires_an_instance_or_appid() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let error = execute_streaming(
            "rematch_game",
            Params {
                roots: vec![temp.path().to_path_buf()],
                state_dir: Some(state),
                ..Default::default()
            },
            &Arc::new(AtomicBool::new(false)),
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(error.code, "instance_required");
    }

    #[test]
    fn project_instances_projects_requested_rows_and_reports_missing_instances() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        save_match_record(&state, 42, Some(sources::FallbackMatchStatus::NoMatch));
        let root = fixture_root(temp.path());
        let instance_id = steam::scan(std::slice::from_ref(&root))
            .installs
            .remove(0)
            .instance_id;
        let result = execute(
            "project_instances",
            Params {
                roots: vec![root.clone()],
                state_dir: Some(state.clone()),
                instance_ids: vec![instance_id.clone()],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(result["settings_revision"].is_number());
        assert_eq!(result["instances"].as_array().unwrap().len(), 1);
        assert_eq!(
            result["instances"][0]["game"]["instance_id"],
            instance_id.as_str()
        );
        assert_eq!(result["instances"][0]["discovery"]["status"], "no_match");

        let missing = execute(
            "project_instances",
            Params {
                roots: vec![root],
                state_dir: Some(state),
                instance_ids: vec!["missing".to_string()],
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(missing.code, "instance_not_found");
    }

    #[test]
    fn project_instances_without_source_state_returns_no_settings_revision() {
        let temp = tempfile::tempdir().unwrap();
        let root = fixture_root(temp.path());
        let instance_id = steam::scan(std::slice::from_ref(&root))
            .installs
            .remove(0)
            .instance_id;
        let result = execute(
            "project_instances",
            Params {
                roots: vec![root],
                instance_ids: vec![instance_id],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(result["settings_revision"].is_null());
        assert!(result["instances"][0]["discovery"].is_null());
    }

    /// P6 matrix 9: `scan` / `project_instances` are read-only projections and
    /// never create or rewrite source state (and never touch the network).
    #[test]
    fn scan_and_project_instances_do_not_modify_existing_source_state() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        seed(&state);
        let store = sources::store::SourceStore::open(&state).unwrap();
        store
            .save_index(&SourceIndex {
                parser_fingerprint: None,
                schema_version: 1,
                source_id: "patchmoe".to_string(),
                revision: 3,
                complete: true,
                checked_at: 100,
                updated_at: 100,
                etag: None,
                modified: None,
                entries: vec![SourceIndexEntry {
                    title: "Synthetic".to_string(),
                    aliases: Vec::new(),
                    page: "https://www.patch.moe/zh".to_string(),
                    kind: "entry".to_string(),
                    appid: Some(42),
                    url: Some("https://download.patch.moe/synthetic.zip".to_string()),
                    language: Some("zh-CN".to_string()),
                    date: None,
                }],
                pending: Vec::new(),
            })
            .unwrap();
        drop(store);
        let root = fixture_root(temp.path());
        let instance_id = steam::scan(std::slice::from_ref(&root))
            .installs
            .remove(0)
            .instance_id;
        let before = state_files(&state);

        let scan = execute(
            "scan",
            Params {
                roots: vec![root.clone()],
                state_dir: Some(state.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let projected = execute(
            "project_instances",
            Params {
                roots: vec![root],
                state_dir: Some(state.clone()),
                instance_ids: vec![instance_id],
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(scan["installs"].as_array().unwrap().len(), 1);
        assert_eq!(projected["instances"].as_array().unwrap().len(), 1);
        assert_eq!(
            state_files(&state),
            before,
            "scan/project_instances must not write source state"
        );
        let patchmoe = scan["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["id"] == "patchmoe")
            .unwrap();
        assert_eq!(patchmoe["revision"], 3);
    }

    /// P6 matrix 8 (removed game): a persisted match record can never re-add a
    /// game row; projection is derived from the current Steam inventory.
    #[test]
    fn scan_does_not_reintroduce_a_removed_game_from_a_persisted_match() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let root = fixture_root(temp.path());
        seed(&state);
        save_match_record(&state, 42, Some(sources::FallbackMatchStatus::NoMatch));

        let before_removal = execute(
            "scan",
            Params {
                roots: vec![root.clone()],
                state_dir: Some(state.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(before_removal["installs"].as_array().unwrap().len(), 1);
        assert_eq!(
            before_removal["installs"][0]["discovery"]["status"],
            "no_match"
        );

        fs::remove_file(root.join("steamapps/appmanifest_42.acf")).unwrap();
        let after_removal = execute(
            "scan",
            Params {
                roots: vec![root],
                state_dir: Some(state),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            after_removal["installs"].as_array().unwrap().is_empty(),
            "a persisted match record must not re-add a removed game row"
        );
    }

    #[test]
    fn set_source_update_interval_persists_and_derives_next_due_at() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let result = execute(
            "set_source_update_interval",
            Params {
                roots: vec![fixture_root(temp.path())],
                state_dir: Some(state.clone()),
                source_id: Some("patchmoe".to_string()),
                interval_hours: Some(6),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result["interval_hours"], 6);
        assert!(result["settings_revision"].as_u64().unwrap() >= 2);

        let store = sources::store::SourceStore::open(&state).unwrap();
        store
            .save_status(
                "patchmoe",
                &sources::SourceStatus {
                    checked_at: 1_000,
                    cached_at: Some(1_000),
                    entry_count: 1,
                    code: "source_refreshed".to_string(),
                },
            )
            .unwrap();
        let list = execute(
            "list_sources",
            Params {
                roots: vec![fixture_root(temp.path())],
                state_dir: Some(state),
                ..Default::default()
            },
        )
        .unwrap();
        let patchmoe = list
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["id"] == "patchmoe")
            .unwrap();
        assert_eq!(patchmoe["interval_hours"], 6);
        assert_eq!(patchmoe["next_due_at"], 1_000 + 6 * 3_600);
    }

    #[test]
    fn set_source_update_interval_rejects_unknown_and_fallback_sources() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let unknown = execute(
            "set_source_update_interval",
            Params {
                roots: vec![fixture_root(temp.path())],
                state_dir: Some(state.clone()),
                source_id: Some("missing".to_string()),
                interval_hours: Some(6),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(unknown.code, "unknown_source");
        let fallback = execute(
            "set_source_update_interval",
            Params {
                roots: vec![fixture_root(temp.path())],
                state_dir: Some(state),
                source_id: Some("hgamer".to_string()),
                interval_hours: Some(6),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(fallback.code, "source_not_indexable");
    }

    #[test]
    fn update_source_rejects_unknown_and_fallback_sources_without_network() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let unknown = execute_streaming(
            "update_source",
            Params {
                roots: vec![fixture_root(temp.path())],
                state_dir: Some(state.clone()),
                source_id: Some("missing".to_string()),
                ..Default::default()
            },
            &Arc::new(AtomicBool::new(false)),
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(unknown.code, "unknown_source");
        let fallback = execute_streaming(
            "update_source",
            Params {
                roots: vec![fixture_root(temp.path())],
                state_dir: Some(state),
                source_id: Some("hgamer".to_string()),
                ..Default::default()
            },
            &Arc::new(AtomicBool::new(false)),
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(fallback.code, "source_not_indexable");
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
                ).unwrap_err().code;
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
        assert_eq!(install["discovery"]["status"], "pending");
        assert!(install["sources"].as_array().is_some());
    }

    /// Seeds a patchmoe index entry that carries AppID 42 so matching is offline.
    fn seed_patchmoe(state: &Path) {
        let handle = sources::Sources::open(state).unwrap();
        handle
            .store()
            .save_index(&SourceIndex {
                parser_fingerprint: None,
                schema_version: 1,
                source_id: "patchmoe".to_string(),
                revision: 1,
                complete: true,
                checked_at: 1000,
                updated_at: 1000,
                etag: None,
                modified: None,
                entries: vec![SourceIndexEntry {
                    title: "Synthetic".to_string(),
                    aliases: Vec::new(),
                    page: "https://www.patch.moe/synthetic".to_string(),
                    kind: "entry".to_string(),
                    appid: Some(42),
                    url: Some("https://download.patch.moe/synthetic.zip".to_string()),
                    language: Some("en".to_string()),
                    date: None,
                }],
                pending: Vec::new(),
            })
            .unwrap();
    }

    #[test]
    fn deleted_source_methods_are_not_in_the_whitelist() {
        for method in [
            "refresh_sources",
            "scan_sources",
            "discover_game",
            "list_source_matches",
            "get_source_state",
            "get_source_settings",
            "get_source_policy",
            "resolve_source_record",
        ] {
            assert_eq!(
                execute(method, Params::default()).unwrap_err().code,
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
    fn set_source_update_interval_streams_no_events_and_returns_the_setting() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let mut events = Vec::new();
        let result = execute_streaming(
            "set_source_update_interval",
            Params {
                roots: vec![temp.path().to_path_buf()],
                state_dir: Some(state),
                source_id: Some("patchmoe".to_string()),
                interval_hours: Some(24),
                ..Default::default()
            },
            &Arc::new(AtomicBool::new(false)),
            &mut |event| events.push(event),
        )
        .unwrap();
        assert!(events.is_empty());
        assert_eq!(result["interval_hours"], 24);
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
        params.clone()
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
            execute("install_patch", conflict).unwrap_err().code,
            "request_id_binding_conflict"
        );
        let mut reinstall = copy_params(&params);
        reinstall.request_id = Some("request-2".into());
        reinstall.operation = install::InstallOperation::Reinstall;
        assert_eq!(
            execute("install_patch", reinstall).unwrap()["state"],
            "committed"
        );
        fs::write(game.join("patch.dat"), b"damaged").unwrap();
        let mut damaged = copy_params(&params);
        damaged.request_id = Some("request-damaged".into());
        damaged.operation = install::InstallOperation::Reinstall;
        // Direct overwrite semantics: the explicit reinstall writes the selected
        // release; the projection keeps the reinstall action unavailable in the UI.
        assert_eq!(
            execute("install_patch", damaged).unwrap()["state"],
            "committed"
        );
        assert_eq!(fs::read(game.join("patch.dat")).unwrap(), b"patch content");
        let mut overwrite = copy_params(&params);
        overwrite.request_id = Some("request-overwrite".into());
        overwrite.operation = install::InstallOperation::Overwrite;
        assert_eq!(
            execute("install_patch", overwrite).unwrap()["state"],
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
            "protocol_version": 5,
            "request_id": "request-1",
            "method": "install_patch",
            "params": crate::tests::wire_params("install_patch", serde_json::to_value(&params).unwrap()),
        })
        .to_string();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let events = Arc::clone(&events);
            move |value| events.lock().unwrap().push(value)
        };
        let response = crate::run_task(request.as_bytes(), Arc::new(AtomicBool::new(false)), sink);
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
        assert_eq!(response["protocol_version"], 5);
        assert_eq!(response["result"]["state"], "committed");
    }

    #[test]
    fn install_patch_route_matrix() {
        let (_temp, params, _game) = fixture();

        let mut conflict = copy_params(&params);
        conflict.selection_id = Some("zip-selection-000000000000000000000000-42".into());
        assert_eq!(
            execute("install_patch", conflict).unwrap_err().code,
            "invalid_request"
        );

        let mut neither = copy_params(&params);
        neither.release_id = None;
        assert_eq!(
            execute("install_patch", neither).unwrap_err().code,
            "selection_or_release_required"
        );

        let mut invalid = copy_params(&params);
        invalid.release_id = None;
        invalid.archive = None;
        invalid.selection_id = Some("candidate-000000000000000000000000".into());
        assert_eq!(
            execute("install_patch", invalid).unwrap_err().code,
            "invalid_selection_id"
        );

        let mut exe = copy_params(&params);
        exe.release_id = None;
        exe.archive = None;
        exe.selection_id = Some("exe-selection-000000000000000000000000-42".into());
        assert_eq!(
            execute("install_patch", exe).unwrap_err().code,
            "selection_not_found"
        );

        let mut archive = copy_params(&params);
        archive.release_id = None;
        archive.selection_id = Some("zip-selection-000000000000000000000000-42".into());
        archive.archive = Some(PathBuf::from("unused.zip"));
        assert_eq!(
            execute("install_patch", archive).unwrap_err().code,
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
            execute("preview", preview).unwrap_err().code,
            "invalid_selection_id"
        );

        let mut exe = copy_params(&params);
        exe.release_id = None;
        exe.selection_id = Some("exe-selection-000000000000000000000000-42".into());
        assert_eq!(
            execute("preview", exe).unwrap_err().code,
            "selection_not_found"
        );

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
    use patchhub_core::sources::model::{
        ArtifactFormat, CacheStatus, Capability, GameMatchRecord, GameMatchStatus, MatchBasis,
        Mirror, SourceCandidate, SourceMatchRecord, SourceMatchStatus,
    };
    use sha2::{Digest, Sha256};

    const APPID: u32 = 42;

    fn dynamic_candidate() -> SourceCandidate {
        SourceCandidate {
            parser_fingerprint: None,
            candidate_id: "candidate-000000000000000000000000".to_string(),
            source_id: "kagura-cn".to_string(),
            source_record_id: "https://kagurafan.com/patches/game".to_string(),
            game_appid: Some(APPID),
            parent_appid: None,
            aliases: Vec::new(),
            game_title: "Test".to_string(),
            patch_title: "Test patch".to_string(),
            variant: patchhub_core::sources::model::Variant::Base,
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

    fn save_dynamic_match(sources: &sources::Sources, candidate: SourceCandidate) {
        let source_id = candidate.source_id.clone();
        sources
            .store()
            .save_match(&GameMatchRecord {
                schema_version: 1,
                appid: APPID,
                name: "Test".to_string(),
                localized_name: None,
                input_key: "key".to_string(),
                status: GameMatchStatus::PublisherMatch,
                sources: std::collections::BTreeMap::from([(
                    source_id,
                    SourceMatchRecord {
                        index_revision: 1,
                        status: SourceMatchStatus::Matched,
                        candidates: vec![candidate],
                        detail_pages: Vec::new(),
                        checked_at: 100,
                        diagnostics: Vec::new(),
                    },
                )]),
                fallback: None,
                updated_at: 100,
            })
            .unwrap();
    }

    fn seed_dynamic(state: &Path) {
        let handle = sources::Sources::open(state).unwrap();
        save_dynamic_match(&handle, dynamic_candidate());
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
        params.clone()
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
        let mappings = {
            let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
            let mut mappings = Vec::new();
            for index in 0..zip.len() {
                let mut member = zip.by_index(index).unwrap();
                let source = member.name().to_string();
                let mut hasher = Sha256::new();
                std::io::copy(&mut member, &mut hasher).unwrap();
                let destination = source.clone();
                let original_sha256 = fs::read(game.install_path.join(&destination))
                    .ok()
                    .map(|bytes| format!("{:x}", Sha256::digest(&bytes)));
                mappings.push(catalog::FileMapping {
                    source,
                    destination,
                    original_sha256,
                    installed_sha256: format!("{:x}", hasher.finalize()),
                });
            }
            mappings
        };
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
    fn stale_actions_cannot_bypass_source_or_subscription_changes() {
        for source_disabled in [true, false] {
            let (_temp, mut params, game, _archive, selection_id) = fixture();
            params.platform = Some(host_platform());
            let mut scan = clone_params(&params);
            scan.instance_id = None;
            let snapshot = execute("scan", clone_params(&scan)).unwrap();
            assert_eq!(
                snapshot["installs"][0]["patches"][0]["actions"]["automatic"]["available"],
                true
            );
            let mut setting = clone_params(&params);
            setting.source_id = Some("kagura-cn".into());
            if source_disabled {
                setting.enabled = Some(false);
                execute("set_source_enabled", setting).unwrap();
            } else {
                setting.subscribed = Some(false);
                execute("set_game_source", setting).unwrap();
            }
            params.selection_id = Some(selection_id);
            params.request_id = Some("stale-actions-source".into());
            params.operation = install::InstallOperation::Overwrite;
            assert_eq!(
                execute("install_patch", clone_params(&params))
                    .unwrap_err()
                    .code,
                // Disabled/unsubscribed discoveries are removed from the live
                // candidate set, so the old selection is no longer resolvable.
                "selection_not_found"
            );
            assert!(!game.install_path.join("data.txt").exists());
            let paths = installer::InstallerPaths::new(params.state_dir.as_ref().unwrap());
            assert!(install::list_transactions_at(paths.root())
                .unwrap()
                .is_empty());
            let fresh = execute("scan", scan).unwrap();
            assert!(!fresh["installs"][0]["patches"]
                .as_array()
                .unwrap()
                .iter()
                .any(|patch| patch["actions"]["automatic"]["available"] == true));
        }
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
        assert_eq!(dynamic["actions"]["automatic"]["available"], true);
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
        let handle = sources::Sources::open(&state).unwrap();
        save_dynamic_match(&handle, candidate);
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
        assert_eq!(dynamic["actions"]["automatic"]["available"], true);
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
        candidate.variant = patchhub_core::sources::model::Variant::Demo;
        candidate.mirrors[0].format = "exe".into();
        candidate.mirrors[0].url = "https://kaguraserver.com/a.exe".into();
        candidate.format_reported = ArtifactFormat::Exe;
        candidate.format_observed = ArtifactFormat::Exe;
        // The demo candidate must still be projected (with its own selection id);
        // only resolution rejects it, which the UI surfaces as a clean blocker.
        let selection_id = installer::exe_resolve::exe_selection_id(&candidate).unwrap();
        let handle = sources::Sources::open(&state).unwrap();
        save_dynamic_match(&handle, candidate);
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
        assert_eq!(dynamic["actions"]["automatic"]["available"], false);
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
            direct: false,
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
