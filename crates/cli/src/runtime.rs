use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
use steam_patch_core::{
    catalog::{self, Catalog, Platform, SupportLevel},
    download, install, sources, steam,
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
fn capabilities() -> Value {
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
            // TODO(P1/P3/P4): platform isolation probes are injected by the
            // host/runtime (LPAC/Job Object on Windows, bwrap on SteamOS).
            // Conservative default until then; `formats` is not installability.
            "links": {
                "zip": { "available": true, "blockers": [] },
                "exe": {
                    "available": false,
                    "blockers": [{
                        "code": "exe_sandbox_unavailable",
                        "detail": "isolation probe not implemented (P3/P4)",
                    }],
                },
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
    if !METHODS.contains(&method) {
        return Err("unsupported_method".into());
    }
    if params.roots.len() > 32 {
        return Err("too_many_roots".into());
    }
    if method == "scan" && (params.instance_id.is_some() || params.release_id.is_some()) {
        return Err("unexpected_preview_arguments".into());
    }
    if method == "get_capabilities" {
        return Ok(capabilities());
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
        return match method {
            "discard_plan" => value(
                install::discard_plan(params.plan_id.as_deref().ok_or("plan_required")?, &state)
                    .map_err(err)?,
            ),
            "list_transactions" if !state.join("journal.sqlite").exists() => Ok(json!([])),
            "list_transactions" => value(install::list_transactions(&state).map_err(err)?),
            _ => value(install::recover(&state).map_err(err)?),
        };
    }
    let platform = params.platform.unwrap_or_else(host_platform);
    if method == "install_patch" {
        return install_patch(&params, &report, platform);
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
        return value(
            catalog::preview(
                game,
                &catalog,
                params.release_id.as_deref().ok_or("release_required")?,
                platform,
            )
            .map_err(err)?,
        );
    }
    let transactions = params
        .state_dir
        .as_ref()
        .filter(|s| s.join("journal.sqlite").exists())
        .map(|s| install::list_transactions(s).map_err(err))
        .transpose()?
        .unwrap_or_default();
    let source_handle = params
        .state_dir
        .as_ref()
        .filter(|state| state.join("sources").join("rust-v1").exists())
        .and_then(|state| sources::Sources::open(state).ok());
    let rows: Vec<_> = report.installs.iter().map(|game| {
        let mut patches = catalog::matches(game, &catalog, platform);
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

/// Minimal P1 `cleanup_cache`: sweeps only derived, non-recoverable cache.
///
/// TODO(P1 store integration): replace the `downloads/` sweep with the shared
/// cache manifest/`CacheStatus` once the installer store exposes clearable
/// paths (p0-contracts §3.2). It must never delete an active lease, receipt,
/// journal, `.stage`/`.backup` material, or `revoked` markers
/// (p1-host-contract §1.1 A / §7.3).
fn cleanup_cache(params: &Params) -> Result<Value, String> {
    let state = state(params)?;
    state_safe(&state)?;
    let downloads = state.join("downloads");
    let mut removed_bytes = 0_u64;
    match fs::read_dir(&downloads) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(|_| "cleanup_failed".to_string())?;
                let path = entry.path();
                let metadata =
                    fs::symlink_metadata(&path).map_err(|_| "cleanup_failed".to_string())?;
                if metadata.file_type().is_file() {
                    removed_bytes += metadata.len();
                    fs::remove_file(&path).map_err(|_| "cleanup_failed".to_string())?;
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("cleanup_failed".into()),
    }
    Ok(json!({ "performed": true, "removed_bytes": removed_bytes }))
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

/// Persistent request binding makes a lost RPC response safe to retry. A request
/// interrupted before preparation is diagnosed rather than silently rerun.
fn install_patch(
    params: &Params,
    report: &steam::ScanReport,
    platform: Platform,
) -> Result<Value, String> {
    let state = state(params)?;
    let request = params.request_id.as_deref().ok_or("request_id_required")?;
    if request.is_empty() || request.len() > 128 {
        return Err("invalid_request_id".into());
    }
    let instance = params.instance_id.as_deref().ok_or("instance_required")?;
    let release_id = params.release_id.as_deref().ok_or("release_required")?;
    state_safe(&state)?;
    fs::create_dir_all(&state).map_err(err)?;
    let request_file = state.join("install_requests.sqlite");
    state_safe(&request_file)?;
    state_safe(&state.join("install_requests.sqlite-journal"))?;
    let conn = rusqlite::Connection::open(request_file).map_err(err)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(err)?;
    conn.execute_batch("PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS requests(id TEXT PRIMARY KEY, binding TEXT NOT NULL, plan TEXT, error TEXT)").map_err(err)?;
    let binding = serde_json::to_string(&(instance, release_id, params.operation)).map_err(err)?;
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO requests(id,binding) VALUES(?1,?2)",
            rusqlite::params![request, binding],
        )
        .map_err(err)?;
    if inserted == 0 {
        let (old_binding, plan, error): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT binding,plan,error FROM requests WHERE id=?1",
                [request],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(err)?;
        if old_binding != binding {
            return Err("request_id_binding_conflict".into());
        }
        if let Some(error) = error {
            return Err(error);
        }
        if let Some(plan) = plan {
            let transactions = install::list_transactions(&state).map_err(err)?;
            let transaction = transactions
                .iter()
                .find(|t| t.plan_id == plan)
                .ok_or("request_transaction_missing")?;
            if ["committed", "superseded", "rolled_back", "needs_recovery"]
                .contains(&transaction.state.as_str())
            {
                return value(transaction);
            }
        }
        return Err("install_request_in_progress_or_interrupted".into());
    }
    let outcome = (|| {
        let game = report
            .installs
            .iter()
            .find(|g| g.instance_id == instance)
            .ok_or("instance_not_found")?;
        let catalog = signed(params)?;
        let release = ready_release(&catalog, release_id, game, platform)?;
        let archive = match &params.archive {
            Some(path) => path.clone(),
            None => download::fetch(
                release
                    .artifact
                    .as_ref()
                    .ok_or("artifact_requires_manual_download")?,
                &state.join("downloads"),
                &AtomicBool::new(false),
                |_, _| (),
            )
            .map_err(err)?,
        };
        let plan = install::prepare_with_operation(
            game,
            release,
            &archive,
            &state,
            catalog.catalog().revision,
            params.operation,
        )
        .map_err(err)?;
        conn.execute(
            "UPDATE requests SET plan=?2 WHERE id=?1",
            rusqlite::params![request, plan.plan_id],
        )
        .map_err(err)?;
        let transaction = install::apply(&plan.plan_id, &state).map_err(err)?;
        value(transaction)
    })();
    if let Err(error) = &outcome {
        conn.execute(
            "UPDATE requests SET error=?2 WHERE id=?1",
            rusqlite::params![request, error],
        )
        .map_err(err)?;
    }
    outcome
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
            "\"AppState\" { \"appid\" \"42\" \"name\" \"Test\" \"installdir\" \"Test\" \"buildid\" \"100\" \"StateFlags\" \"4\" }").unwrap();
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
}
