//! Authoritative version 5 JSONL wire contract. Core is deliberately independent.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
pub use steam_patch_core::{
    catalog::{PatchMatch, Platform, SupportLevel},
    install::{InstallOperation, Transaction},
    installer::{
        backup::{BackupCleanupResult, BackupInventory},
        outcome::{CleanupResult, InstallerOutcome},
        service::CleanupPolicy,
    },
    projection::{DomainError, InstanceProjection, PreviewReport, ScanReport},
    sources::{
        GameMatch, GameSourceSummary, RawSourceRule, SourceCandidate, SourceRule, SourceStatus,
        SourceSummary,
    },
};
use ts_rs::TS;
pub const PROTOCOL_VERSION: u32 = 5;
pub const MAX_REQUEST_ID_BYTES: usize = 128;
pub const MAX_ID_BYTES: usize = 256;
pub const MAX_INSTANCE_ID_BYTES: usize =
    steam_patch_core::installer::service::MAX_INSTANCE_ID_BYTES;
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ProtocolError {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub detail: Option<String>,
    #[serde(skip)]
    #[ts(skip)]
    pub request_id: Option<String>,
}
impl ProtocolError {
    pub fn code(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: None,
            request_id: None,
        }
    }
}
impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code)
    }
}
impl std::error::Error for ProtocolError {}
impl From<DomainError> for ProtocolError {
    fn from(e: DomainError) -> Self {
        Self {
            code: e.code,
            detail: e.detail,
            request_id: None,
        }
    }
}
pub fn validate_id(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    GetCapabilities,
    Scan,
    ProjectInstances,
    Preview,
    InstallPatch,
    ListTransactions,
    Recover,
    DiscardPlan,
    CleanupCache,
    BackupStatus,
    CleanupBackups,
    ListSources,
    ListGameSources,
    SetSourceEnabled,
    SetGameSource,
    AddSource,
    SetSourceUpdateInterval,
    UpdateSource,
    MatchGames,
    RematchGame,
}
pub const ALL_METHODS: [Method; 20] = [
    Method::GetCapabilities,
    Method::Scan,
    Method::ProjectInstances,
    Method::Preview,
    Method::InstallPatch,
    Method::ListTransactions,
    Method::Recover,
    Method::DiscardPlan,
    Method::CleanupCache,
    Method::BackupStatus,
    Method::CleanupBackups,
    Method::ListSources,
    Method::ListGameSources,
    Method::SetSourceEnabled,
    Method::SetGameSource,
    Method::AddSource,
    Method::SetSourceUpdateInterval,
    Method::UpdateSource,
    Method::MatchGames,
    Method::RematchGame,
];
pub const METHODS: [&str; 20] = [
    "get_capabilities",
    "scan",
    "project_instances",
    "preview",
    "install_patch",
    "list_transactions",
    "recover",
    "discard_plan",
    "cleanup_cache",
    "backup_status",
    "cleanup_backups",
    "list_sources",
    "list_game_sources",
    "set_source_enabled",
    "set_game_source",
    "add_source",
    "set_source_update_interval",
    "update_source",
    "match_games",
    "rematch_game",
];
pub const SOURCE_METHODS: [&str; 9] = [
    "list_sources",
    "list_game_sources",
    "set_source_enabled",
    "set_game_source",
    "add_source",
    "set_source_update_interval",
    "update_source",
    "match_games",
    "rematch_game",
];
impl Method {
    pub fn as_str(self) -> &'static str {
        METHODS[ALL_METHODS.iter().position(|m| *m == self).unwrap()]
    }
    pub fn is_task(self) -> bool {
        matches!(
            self,
            Self::InstallPatch | Self::UpdateSource | Self::MatchGames | Self::RematchGame
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct HostInputs {
    pub roots: Vec<PathBuf>,
    pub catalog: Option<PathBuf>,
    pub signed_catalog: Option<PathBuf>,
    pub trust_keys: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub platform: Platform,
}
macro_rules! params { ($name:ident {$($fields:tt)*}) => { #[derive(Debug,Clone,Serialize,Deserialize,JsonSchema,TS)] #[serde(deny_unknown_fields)] pub struct $name { pub host:HostInputs,$($fields)* } }; }
params!(HostParams {});
params!(ProjectInstancesParams{pub instance_ids:Vec<String>,});
params!(PreviewParams{pub instance_id:String,#[ts(optional = nullable)]pub release_id:Option<String>,#[ts(optional = nullable)]pub selection_id:Option<String>,});
params!(InstallPatchParams{pub instance_id:String,#[ts(optional = nullable)]pub release_id:Option<String>,#[ts(optional = nullable)]pub selection_id:Option<String>,#[ts(optional = nullable)]pub archive:Option<PathBuf>,pub operation:InstallOperation,pub cleanup_policy:CleanupPolicy,});
params!(DiscardPlanParams{pub plan_id:String,});
params!(ListGameSourcesParams{#[ts(optional = nullable)]pub instance_id:Option<String>,#[ts(optional = nullable)]pub appid:Option<u32>,});
params!(SetSourceEnabledParams{pub source_id:String,pub enabled:bool,});
params!(SetGameSourceParams{pub source_id:String,pub subscribed:bool,#[ts(optional = nullable)]pub instance_id:Option<String>,#[ts(optional = nullable)]pub appid:Option<u32>,});
params!(AddSourceParams{pub rule:RawSourceRule,});
params!(SetSourceUpdateIntervalParams{pub source_id:String,#[ts(type="number")]pub interval_hours:u64,});
params!(UpdateSourceParams{pub source_id:String,#[serde(default)]#[ts(as="Option<bool>", optional)]pub force:bool,});
params!(MatchGamesParams{#[serde(default)]#[ts(as="Option<Vec<String>>", optional)]pub instances:Vec<String>,#[serde(default)]#[ts(as="Option<Vec<String>>", optional)]pub name_hints:Vec<String>,#[ts(optional = nullable)]pub preferred_language:Option<String>,#[serde(default)]#[ts(as="Option<bool>", optional)]pub force_details:bool,});
params!(RematchGameParams{#[ts(optional = nullable)]pub instance_id:Option<String>,#[ts(optional = nullable)]pub appid:Option<u32>,#[serde(default)]#[ts(as="Option<Vec<String>>", optional)]pub name_hints:Vec<String>,#[ts(optional = nullable)]pub preferred_language:Option<String>,});
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum RequestParams {
    GetCapabilities(HostParams),
    Scan(HostParams),
    ProjectInstances(ProjectInstancesParams),
    Preview(PreviewParams),
    InstallPatch(InstallPatchParams),
    ListTransactions(HostParams),
    Recover(HostParams),
    DiscardPlan(DiscardPlanParams),
    CleanupCache(HostParams),
    BackupStatus(HostParams),
    CleanupBackups(HostParams),
    ListSources(HostParams),
    ListGameSources(ListGameSourcesParams),
    SetSourceEnabled(SetSourceEnabledParams),
    SetGameSource(SetGameSourceParams),
    AddSource(AddSourceParams),
    SetSourceUpdateInterval(SetSourceUpdateIntervalParams),
    UpdateSource(UpdateSourceParams),
    MatchGames(MatchGamesParams),
    RematchGame(RematchGameParams),
}
impl RequestParams {
    pub fn host(&self) -> &HostInputs {
        match self {
            Self::GetCapabilities(p)
            | Self::Scan(p)
            | Self::ListTransactions(p)
            | Self::Recover(p)
            | Self::CleanupCache(p)
            | Self::BackupStatus(p)
            | Self::CleanupBackups(p)
            | Self::ListSources(p) => &p.host,
            Self::ProjectInstances(p) => &p.host,
            Self::Preview(p) => &p.host,
            Self::InstallPatch(p) => &p.host,
            Self::DiscardPlan(p) => &p.host,
            Self::ListGameSources(p) => &p.host,
            Self::SetSourceEnabled(p) => &p.host,
            Self::SetGameSource(p) => &p.host,
            Self::AddSource(p) => &p.host,
            Self::SetSourceUpdateInterval(p) => &p.host,
            Self::UpdateSource(p) => &p.host,
            Self::MatchGames(p) => &p.host,
            Self::RematchGame(p) => &p.host,
        }
    }
    pub fn params_value(&self) -> Value {
        serde_json::to_value(self).unwrap()["params"].clone()
    }
}
#[derive(Debug, Clone)]
pub struct ValidatedRequest {
    pub request_id: String,
    pub method: Method,
    pub params: RequestParams,
}
impl ValidatedRequest {
    pub fn host(&self) -> &HostInputs {
        self.params.host()
    }
    pub fn params_value(&self) -> Value {
        self.params.params_value()
    }
}
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Request {
    #[ts(type = "5")]
    pub protocol_version: u32,
    pub request_id: String,
    #[serde(flatten)]
    pub call: RequestParams,
}

pub fn decode_request(bytes: &[u8], task_channel: bool) -> Result<ValidatedRequest, ProtocolError> {
    let value: Value = serde_json::from_slice::<StrictValue>(bytes)
        .map_err(|_| ProtocolError::code("invalid_request"))?
        .0;
    validate_request(&value, task_channel)
}
pub fn validate_request(
    value: &Value,
    task_channel: bool,
) -> Result<ValidatedRequest, ProtocolError> {
    // Recover only a trustworthy envelope ID; version rejection must not inspect params.
    let request_id = value
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|s| validate_id(s, MAX_REQUEST_ID_BYTES))
        .map(str::to_owned);
    // Version and method are examined before method-specific params errors.
    if value.get("protocol_version").and_then(Value::as_u64) != Some(PROTOCOL_VERSION as u64) {
        let mut error = ProtocolError::code("unsupported_protocol");
        error.request_id = request_id;
        return Err(error);
    }
    let id = request_id.ok_or_else(|| ProtocolError::code("invalid_request_id"))?;
    let run = || -> Result<ValidatedRequest, ProtocolError> {
        let method: Method =
            serde_json::from_value(value.get("method").cloned().unwrap_or(Value::Null))
                .map_err(|_| ProtocolError::code("unsupported_method"))?;
        if method.is_task() && !task_channel {
            return Err(ProtocolError::code("task_required"));
        }
        let obj = value
            .as_object()
            .ok_or_else(|| ProtocolError::code("invalid_request"))?;
        if obj.len() != 4
            || !["protocol_version", "request_id", "method", "params"]
                .iter()
                .all(|k| obj.contains_key(*k))
        {
            return Err(ProtocolError::code("invalid_request"));
        }
        let p = value
            .get("params")
            .and_then(Value::as_object)
            .ok_or_else(|| ProtocolError::code("invalid_request"))?;
        let h = p
            .get("host")
            .and_then(Value::as_object)
            .ok_or_else(|| ProtocolError::code("invalid_request"))?;
        if h.len() != 6
            || ![
                "roots",
                "catalog",
                "signed_catalog",
                "trust_keys",
                "state_dir",
                "platform",
            ]
            .iter()
            .all(|k| h.contains_key(*k))
        {
            return Err(ProtocolError::code("invalid_request"));
        }
        if p.get("operation")
            .is_some_and(|v| !matches!(v.as_str(), Some("install" | "reinstall" | "overwrite")))
        {
            return Err(ProtocolError::code("invalid_operation"));
        }
        let params: RequestParams =
            serde_json::from_value(serde_json::json!({"method":method,"params":p}))
                .map_err(|_| ProtocolError::code("invalid_request"))?;
        validate_numbers(value).map_err(|_| ProtocolError::code("invalid_request"))?;
        if params.host().roots.len() > 32 {
            return Err(ProtocolError::code("invalid_request"));
        }
        for key in [
            "instance_id",
            "release_id",
            "selection_id",
            "plan_id",
            "source_id",
        ] {
            if let Some(v) = p.get(key).filter(|v| !v.is_null()) {
                if !v.as_str().is_some_and(|s| {
                    validate_id(
                        s,
                        if key == "instance_id" {
                            MAX_INSTANCE_ID_BYTES
                        } else {
                            MAX_ID_BYTES
                        },
                    )
                }) {
                    return Err(ProtocolError::code("invalid_id"));
                }
            }
        }
        if matches!(method, Method::Preview | Method::InstallPatch) {
            let release = p.get("release_id").and_then(Value::as_str);
            let selection = p.get("selection_id").and_then(Value::as_str);
            if release.is_some() && selection.is_some() {
                return Err(ProtocolError::code("invalid_request"));
            }
            if release.is_none() && selection.is_none() {
                return Err(ProtocolError::code("selection_or_release_required"));
            }
            if let Some(s) = selection {
                if steam_patch_core::installer::service::ZipSelectionId::new(s).is_err()
                    && steam_patch_core::installer::service::ExeSelectionId::new(s).is_err()
                {
                    return Err(ProtocolError::code("invalid_selection_id"));
                }
                if p.get("archive").is_some_and(|a| !a.is_null()) {
                    return Err(ProtocolError::code("archive_not_allowed_for_selection"));
                }
            }
        }
        if matches!(
            method,
            Method::ListGameSources | Method::SetGameSource | Method::RematchGame
        ) && p.get("instance_id").and_then(Value::as_str).is_none()
            && p.get("appid").and_then(Value::as_u64).is_none()
        {
            return Err(ProtocolError::code("instance_required"));
        }
        for key in ["instance_ids", "instances"] {
            let Some(values) = p.get(key).and_then(Value::as_array) else {
                continue;
            };
            if values.iter().any(|value| {
                !value
                    .as_str()
                    .is_some_and(|s| validate_id(s, MAX_INSTANCE_ID_BYTES))
            }) {
                return Err(ProtocolError::code("invalid_id"));
            }
        }
        Ok(ValidatedRequest {
            request_id: id.clone(),
            method,
            params,
        })
    };
    run().map_err(|mut e| {
        e.request_id = Some(id);
        e
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ContractVersions {
    pub projection_version: u32,
    pub actions_version: u32,
    pub outcome_version: u32,
    pub request_validation_version: u32,
}
impl Default for ContractVersions {
    fn default() -> Self {
        Self {
            projection_version: 1,
            actions_version: 1,
            outcome_version: 1,
            request_validation_version: 1,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct WireLinkCapability {
    pub available: bool,
    pub blockers: Vec<DomainError>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct InstallerLinks {
    pub zip: WireLinkCapability,
    pub exe: WireLinkCapability,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct InstallerFormats {
    pub zip: String,
    pub exe: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct InstallerCapabilities {
    pub schema_version: u32,
    pub selection_version: u32,
    pub content_version: u32,
    pub adapter_version: u32,
    pub policy_schema_version: u32,
    pub report_schema_version: u32,
    pub formats: InstallerFormats,
    pub strategies: Vec<String>,
    pub links: InstallerLinks,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SourceCapabilities {
    pub index_schema_version: u32,
    pub detail_schema_version: u32,
    pub match_schema_version: u32,
    pub schedule_schema_version: u32,
    pub adapter_version: u32,
    pub parser_version: u32,
    pub encoding_version: u32,
    pub methods: Vec<Method>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    #[ts(type = "5")]
    pub protocol_version: u32,
    pub methods: Vec<Method>,
    pub installer: InstallerCapabilities,
    pub sources: SourceCapabilities,
    pub contracts: ContractVersions,
}
pub fn validate_capabilities(v: &Value) -> Result<Capabilities, ProtocolError> {
    if v["protocol_version"].as_u64() != Some(PROTOCOL_VERSION as u64) {
        return Err(ProtocolError::code("unsupported_protocol"));
    }
    let c: Capabilities = serde_json::from_value(v.clone())
        .map_err(|_| ProtocolError::code("capability_mismatch"))?;
    if c.methods != ALL_METHODS
        || c.sources
            .methods
            .iter()
            .map(|m| m.as_str())
            .collect::<Vec<_>>()
            != SOURCE_METHODS
        || serde_json::to_value(&c.contracts).unwrap()
            != serde_json::to_value(ContractVersions::default()).unwrap()
    {
        return Err(ProtocolError::code("capability_mismatch"));
    }
    if [
        c.installer.schema_version,
        c.installer.selection_version,
        c.installer.content_version,
        c.installer.adapter_version,
        c.installer.policy_schema_version,
        c.installer.report_schema_version,
        c.sources.index_schema_version,
        c.sources.detail_schema_version,
        c.sources.match_schema_version,
        c.sources.schedule_schema_version,
        c.sources.adapter_version,
        c.sources.encoding_version,
    ]
    .iter()
    .any(|v| *v != 1)
        || c.sources.parser_version != steam_patch_core::sources::PARSER_VERSION
    {
        return Err(ProtocolError::code("installer_schema_unsupported"));
    }
    if c.installer.formats.zip != "supported"
        || c.installer.formats.exe != "supported"
        || c.installer.strategies != ["managed_zip", "publisher_zip", "publisher_exe"]
    {
        return Err(ProtocolError::code("capability_mismatch"));
    }
    for link in [&c.installer.links.zip, &c.installer.links.exe] {
        if link.available != link.blockers.is_empty() {
            return Err(ProtocolError::code("capability_mismatch"));
        }
    }
    validate_bounds(v)?;
    Ok(c)
}
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SourceEnabledResult {
    #[ts(type = "number")]
    pub settings_revision: u64,
    pub source_id: String,
    pub enabled: bool,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct GameSourceResult {
    #[ts(type = "number")]
    pub settings_revision: u64,
    pub source_id: String,
    pub subscribed: bool,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct AddSourceResult {
    #[ts(type = "number")]
    pub settings_revision: u64,
    pub rule: SourceRule,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SetSourceIntervalResult {
    #[ts(type = "number")]
    pub settings_revision: u64,
    pub source_id: String,
    #[ts(type = "number")]
    pub interval_hours: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SourceUpdateResult {
    pub source_id: String,
    #[ts(type = "number")]
    pub settings_revision: u64,
    pub status: SourceStatus,
    #[ts(type = "number")]
    pub revision: u64,
    pub complete: bool,
    pub affected_appids: Vec<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct MatchGamesResult {
    #[ts(type = "number")]
    pub settings_revision: u64,
    #[ts(type = "Record<string, number>")]
    pub source_revisions: BTreeMap<String, u64>,
    pub results: Vec<GameMatch>,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectInstancesResult {
    #[ts(type = "number | null")]
    pub settings_revision: Option<u64>,
    pub instances: Vec<InstanceProjection>,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(untagged)]
pub enum MethodResult {
    Capabilities(Capabilities),
    Scan(ScanReport),
    ProjectInstances(ProjectInstancesResult),
    Preview(PreviewReport),
    Install(InstallerOutcome),
    Transactions(Vec<Transaction>),
    Transaction(Transaction),
    BackupInventory(BackupInventory),
    BackupCleanup(BackupCleanupResult),
    Cleanup(CleanupResult),
    Sources(Vec<SourceSummary>),
    GameSources(Vec<GameSourceSummary>),
    Enabled(SourceEnabledResult),
    GameSource(GameSourceResult),
    Added(AddSourceResult),
    SetInterval(SetSourceIntervalResult),
    SourceUpdated(SourceUpdateResult),
    Matched(MatchGamesResult),
}
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SuccessResponse {
    #[ts(type = "5")]
    pub protocol_version: u32,
    pub request_id: String,
    #[ts(type = "true")]
    pub ok: bool,
    pub result: MethodResult,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    #[ts(type = "5")]
    pub protocol_version: u32,
    #[serde(deserialize_with = "deserialize_required_request_id")]
    pub request_id: Option<String>,
    #[ts(type = "false")]
    pub ok: bool,
    pub error: DomainError,
}
// `deserialize_with` keeps a nullable ID required when deserializing the envelope.
fn deserialize_required_request_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(untagged)]
pub enum Response {
    Success(Box<SuccessResponse>),
    Error(ErrorResponse),
}
pub fn validate_response(
    v: &Value,
    request_id: &str,
    method: Option<Method>,
) -> Result<(), ProtocolError> {
    if v["protocol_version"].as_u64() != Some(PROTOCOL_VERSION as u64) {
        return Err(ProtocolError::code("unsupported_protocol"));
    }
    if v["request_id"].as_str() != Some(request_id) || !validate_id(request_id, 128) {
        return Err(ProtocolError::code("invalid_core_response"));
    }
    validate_bounds(v)?;
    let obj = v
        .as_object()
        .ok_or_else(|| ProtocolError::code("invalid_core_response"))?;
    if obj.len() != 4 {
        return Err(ProtocolError::code("invalid_core_response"));
    }
    if v["ok"].as_bool() == Some(false) {
        serde_json::from_value::<ErrorResponse>(v.clone())
            .map_err(|_| ProtocolError::code("invalid_core_response"))?;
        return Ok(());
    }
    if v["ok"].as_bool() != Some(true) || v.get("error").is_some() {
        return Err(ProtocolError::code("invalid_core_response"));
    }
    let r = v
        .get("result")
        .ok_or_else(|| ProtocolError::code("invalid_core_response"))?;
    let schema = output_schema();
    let name = method.map(result_schema_name).unwrap_or("MethodResult");
    if method != Some(Method::GetCapabilities) && !matches_schema(r, &schema["$defs"][name], schema)
    {
        return Err(ProtocolError::code("invalid_core_response"));
    }
    macro_rules! check {
        ($t:ty) => {
            serde_json::from_value::<$t>(r.clone())
                .map_err(|_| ProtocolError::code("invalid_core_response"))?;
        };
    }
    match method {
        Some(Method::GetCapabilities) => {
            validate_capabilities(r)?;
        }
        Some(Method::Scan) => {
            check!(ScanReport);
            validate_projection(r)?;
        }
        Some(Method::ProjectInstances) => {
            check!(ProjectInstancesResult);
        }
        Some(Method::Preview) => {
            required_fields(
                r,
                &[
                    "instance_id",
                    "release_id",
                    "selection_id",
                    "catalog_revision",
                    "executable",
                    "blockers",
                    "changes",
                    "diagnostics",
                ],
            )?;
            check!(PreviewReport);
            for c in r["changes"].as_array().unwrap() {
                required_fields(c, &["path", "action", "baseline_matches", "actual_sha256"])?;
            }
        }
        Some(Method::InstallPatch) => {
            required_fields(
                r,
                &[
                    "transaction_id",
                    "plan_id",
                    "instance_id",
                    "selection_id",
                    "content_id",
                    "release_id",
                    "state",
                    "error",
                    "created_at",
                    "updated_at",
                    "has_replacements",
                    "disposition",
                    "cause",
                    "recovery",
                    "cleanup",
                ],
            )?;
            check!(InstallerOutcome);
            validate_outcome(r)?;
        }
        Some(Method::ListTransactions | Method::Recover) => {
            check!(Vec<Transaction>);
            validate_transactions(r)?;
        }
        Some(Method::DiscardPlan) => {
            check!(Transaction);
            validate_transaction(r)?;
        }
        Some(Method::BackupStatus) => {
            check!(BackupInventory);
        }
        Some(Method::CleanupBackups) => {
            check!(BackupCleanupResult);
        }
        Some(Method::CleanupCache) => {
            check!(CleanupResult);
        }
        Some(Method::ListSources) => {
            check!(Vec<SourceSummary>);
        }
        Some(Method::ListGameSources) => {
            check!(Vec<GameSourceSummary>);
        }
        Some(Method::SetSourceEnabled) => {
            check!(SourceEnabledResult);
        }
        Some(Method::SetGameSource) => {
            check!(GameSourceResult);
        }
        Some(Method::AddSource) => {
            check!(AddSourceResult);
        }
        Some(Method::SetSourceUpdateInterval) => {
            check!(SetSourceIntervalResult);
        }
        Some(Method::UpdateSource) => {
            check!(SourceUpdateResult);
        }
        Some(Method::MatchGames | Method::RematchGame) => {
            check!(MatchGamesResult);
        }
        None => {
            check!(MethodResult);
        }
    }
    Ok(())
}
fn validate_transactions(v: &Value) -> Result<(), ProtocolError> {
    for t in v
        .as_array()
        .ok_or_else(|| ProtocolError::code("invalid_core_response"))?
    {
        validate_transaction(t)?;
    }
    Ok(())
}
fn validate_transaction(v: &Value) -> Result<(), ProtocolError> {
    if !TRANSACTION_STATES.contains(&v["state"].as_str().unwrap_or("")) {
        return Err(ProtocolError::code("installer_schema_unsupported"));
    }
    Ok(())
}
pub const TRANSACTION_STATES: [&str; 11] = [
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
];
fn validate_outcome(v: &Value) -> Result<(), ProtocolError> {
    let valid = match v["state"].as_str() {
        Some("committed") => v["disposition"] == "completed" && v["recovery"].is_null(),
        Some("needs_recovery") => v["disposition"] == "failed" && v["recovery"]["required"] == true,
        Some("rolled_back") => matches!(v["disposition"].as_str(), Some("cancelled" | "failed")),
        Some("failed") => matches!(v["disposition"].as_str(), Some("cancelled" | "failed")),
        _ => false,
    };
    if !valid {
        return Err(ProtocolError::code("invalid_core_response"));
    }
    Ok(())
}
fn validate_projection(v: &Value) -> Result<(), ProtocolError> {
    for row in v["installs"]
        .as_array()
        .ok_or_else(|| ProtocolError::code("invalid_core_response"))?
    {
        for p in row["patches"]
            .as_array()
            .ok_or_else(|| ProtocolError::code("invalid_core_response"))?
        {
            if ![
                "unknown",
                "not_detected",
                "partial_or_conflict",
                "installed_external",
                "installed_managed",
            ]
            .contains(&p["detection"]["status"].as_str().unwrap_or(""))
            {
                return Err(ProtocolError::code("invalid_core_response"));
            }
            for f in p["detection"]["files"]
                .as_array()
                .ok_or_else(|| ProtocolError::code("invalid_core_response"))?
            {
                if !["match", "mismatch", "missing", "unreadable_or_unsafe"]
                    .contains(&f["status"].as_str().unwrap_or(""))
                {
                    return Err(ProtocolError::code("invalid_core_response"));
                }
            }
            if p["actions"]["primary"]["operation"] != "overwrite" {
                return Err(ProtocolError::code("invalid_core_response"));
            }
            for a in ["automatic", "local_archive", "reinstall", "overwrite"] {
                let action = &p["actions"][a];
                if action["available"].as_bool()
                    != action["blockers"].as_array().map(|b| b.is_empty())
                {
                    return Err(ProtocolError::code("invalid_core_response"));
                }
            }
        }
    }
    Ok(())
}
/// Encodes one JSON Lines frame: the value followed by a single LF. JSON Lines
/// framing has no size limit; callers own their transport framing.
pub fn json_line(v: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(v).expect("a serde_json::Value always serializes");
    bytes.push(b'\n');
    bytes
}
fn validate_bounds(v: &Value) -> Result<(), ProtocolError> {
    match v {
        Value::Number(n) => {
            if n.as_u64().is_some_and(|n| n > MAX_SAFE_INTEGER)
                || n.as_i64()
                    .is_some_and(|n| n.unsigned_abs() > MAX_SAFE_INTEGER)
                || (!n.is_u64() && !n.is_i64())
            {
                return Err(ProtocolError::code("invalid_core_response"));
            }
        }
        Value::Array(a) => {
            for v in a {
                validate_bounds(v)?;
            }
        }
        Value::Object(o) => {
            for (k, v) in o {
                if k == "code"
                    && v.as_str().is_some_and(|s| {
                        s.is_empty()
                            || s.len() > 128
                            || !s
                                .bytes()
                                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                    })
                {
                    return Err(ProtocolError::code("invalid_core_response"));
                }
                if [
                    "id",
                    "request_id",
                    "instance_id",
                    "release_id",
                    "selection_id",
                    "plan_id",
                    "source_id",
                    "transaction_id",
                    "content_id",
                    "candidate_id",
                ]
                .contains(&k.as_str())
                    && v.as_str().is_some_and(|s| {
                        !validate_id(
                            s,
                            if k == "request_id" {
                                MAX_REQUEST_ID_BYTES
                            } else if k == "instance_id" {
                                MAX_INSTANCE_ID_BYTES
                            } else {
                                MAX_ID_BYTES
                            },
                        )
                    })
                {
                    return Err(ProtocolError::code("invalid_core_response"));
                }
                validate_bounds(v)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum InstallStage {
    Connecting,
    Downloading,
    Validating,
    AnalyzingInstaller,
    Extracting,
    Planning,
    Applying,
    RollingBack,
    Cleanup,
}
pub const INSTALL_STAGES: [&str; 9] = [
    "connecting",
    "downloading",
    "validating",
    "analyzing_installer",
    "extracting",
    "planning",
    "applying",
    "rolling_back",
    "cleanup",
];
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum InstallSubstage {
    BackingUp,
    Writing,
    Committing,
}
macro_rules! event_kind {
    ($name:ident,$kind:ident) => {
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $kind,
        }
    };
}
event_kind!(InstallProgressKind, InstallProgress);
event_kind!(SourceProgressKind, SourceProgress);
event_kind!(SourceUpdateKind, SourceUpdate);
event_kind!(MatchProgressKind, MatchProgress);
event_kind!(MatchUpdateKind, MatchUpdate);
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct InstallProgressEvent {
    pub event: InstallProgressKind,
    #[ts(type = "5")]
    pub protocol_version: u32,
    pub request_id: String,
    #[ts(type = "number")]
    pub sequence: u64,
    pub instance_id: Option<String>,
    pub selection_id: Option<String>,
    pub stage: InstallStage,
    pub substage: Option<InstallSubstage>,
    #[ts(type = "number")]
    pub bytes_done: u64,
    #[ts(type = "number | null")]
    pub total_bytes: Option<u64>,
    #[ts(type = "number")]
    pub files_done: u64,
    #[ts(type = "number | null")]
    pub total_files: Option<u64>,
    pub current_file: Option<String>,
    #[ts(type = "number | null")]
    pub archive_bytes: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SourceProgressEvent {
    pub event: SourceProgressKind,
    #[ts(type = "5")]
    pub protocol_version: u32,
    pub request_id: String,
    #[ts(type = "number")]
    pub sequence: u64,
    pub source_id: String,
    /// `"index" | "details" | "matched"`.
    pub phase: String,
    #[ts(type = "number")]
    pub completed: u64,
    #[ts(type = "number")]
    pub total: u64,
    pub running: bool,
    #[ts(type = "number")]
    pub settings_revision: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SourceUpdateEvent {
    pub event: SourceUpdateKind,
    #[ts(type = "5")]
    pub protocol_version: u32,
    pub request_id: String,
    #[ts(type = "number")]
    pub sequence: u64,
    pub source_id: String,
    #[ts(type = "number")]
    pub settings_revision: u64,
    pub status: SourceStatus,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct MatchProgressEvent {
    pub event: MatchProgressKind,
    #[ts(type = "5")]
    pub protocol_version: u32,
    pub request_id: String,
    #[ts(type = "number")]
    pub sequence: u64,
    /// `"inventory" | "index" | "details" | "fallback"`.
    pub phase: String,
    #[ts(type = "number")]
    pub completed: u64,
    #[ts(type = "number")]
    pub total: u64,
    pub running: bool,
    #[ts(type = "number")]
    pub settings_revision: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct MatchUpdateEvent {
    pub event: MatchUpdateKind,
    #[ts(type = "5")]
    pub protocol_version: u32,
    pub request_id: String,
    #[ts(type = "number")]
    pub sequence: u64,
    pub instance_id: Option<String>,
    pub appid: u32,
    /// One of the six `GameMatchStatus` values (snake_case).
    pub status: String,
    pub candidates: Vec<SourceCandidate>,
    pub diagnostics: Vec<steam_patch_core::sources::Diagnostic>,
    #[ts(type = "number")]
    pub settings_revision: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(untagged)]
pub enum Event {
    Install(InstallProgressEvent),
    Progress(SourceProgressEvent),
    SourceUpdate(SourceUpdateEvent),
    MatchProgress(MatchProgressEvent),
    MatchUpdate(MatchUpdateEvent),
}
pub type WireEvent = Event;
impl Event {
    pub fn sequence(&self) -> u64 {
        match self {
            Self::Install(e) => e.sequence,
            Self::Progress(e) => e.sequence,
            Self::SourceUpdate(e) => e.sequence,
            Self::MatchProgress(e) => e.sequence,
            Self::MatchUpdate(e) => e.sequence,
        }
    }
}
pub fn validate_event(v: &Value, request_id: &str) -> Result<Event, ProtocolError> {
    if v["protocol_version"].as_u64() != Some(PROTOCOL_VERSION as u64) {
        return Err(ProtocolError::code("unsupported_protocol"));
    }
    if v["request_id"].as_str() != Some(request_id) || !validate_id(request_id, 128) {
        return Err(ProtocolError::code("invalid_core_response"));
    }
    validate_bounds(v)?;
    let required = match v["event"].as_str() {
        Some("install_progress") => &[
            "event",
            "protocol_version",
            "request_id",
            "sequence",
            "instance_id",
            "selection_id",
            "stage",
            "substage",
            "bytes_done",
            "total_bytes",
            "files_done",
            "total_files",
            "current_file",
            "archive_bytes",
        ][..],
        Some("source_progress") => &[
            "event",
            "protocol_version",
            "request_id",
            "sequence",
            "source_id",
            "phase",
            "completed",
            "total",
            "running",
            "settings_revision",
        ][..],
        Some("source_update") => &[
            "event",
            "protocol_version",
            "request_id",
            "sequence",
            "source_id",
            "settings_revision",
            "status",
        ][..],
        Some("match_progress") => &[
            "event",
            "protocol_version",
            "request_id",
            "sequence",
            "phase",
            "completed",
            "total",
            "running",
            "settings_revision",
        ][..],
        Some("match_update") => &[
            "event",
            "protocol_version",
            "request_id",
            "sequence",
            "instance_id",
            "appid",
            "status",
            "candidates",
            "diagnostics",
            "settings_revision",
        ][..],
        _ => return Err(ProtocolError::code("invalid_core_response")),
    };
    required_fields(v, required)?;
    let schema = output_schema();
    if !matches_schema(v, &schema["$defs"]["Event"], schema) {
        return Err(ProtocolError::code("invalid_core_response"));
    }
    let event: Event = serde_json::from_value(v.clone())
        .map_err(|_| ProtocolError::code("invalid_core_response"))?;
    if let Event::Install(ref e) = event {
        if e.substage.is_some() && e.stage != InstallStage::Applying {
            return Err(ProtocolError::code("invalid_core_response"));
        }
    }
    Ok(event)
}
#[derive(Debug)]
pub struct EventSequence {
    request_id: String,
    next: u64,
    terminal: bool,
}
impl EventSequence {
    pub fn new(request_id: String) -> Self {
        Self {
            request_id,
            next: 0,
            terminal: false,
        }
    }
    pub fn accept(&mut self, v: &Value) -> Result<bool, ProtocolError> {
        let e = validate_event(v, &self.request_id)?;
        if self.terminal {
            return Ok(false);
        }
        let n = e.sequence();
        if n < self.next {
            return Ok(false);
        }
        if n != self.next {
            return Err(ProtocolError::code("invalid_core_response"));
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| ProtocolError::code("invalid_core_response"))?;
        Ok(true)
    }
    pub fn finish(&mut self) {
        self.terminal = true;
    }
}
/// All output types share one schema graph; no host maintains a DTO table.
#[derive(JsonSchema)]
pub struct ContractSchema {
    pub request: Request,
    pub response: Response,
    pub event: Event,
    pub capabilities: Capabilities,
    pub results: MethodResult,
}
pub fn schema() -> Value {
    let mut v = serde_json::to_value(schemars::schema_for!(ContractSchema)).unwrap();
    let defs = v["$defs"].as_object_mut().unwrap();
    defs.get_mut("HostInputs").unwrap()["required"] = serde_json::json!([
        "roots",
        "catalog",
        "signed_catalog",
        "trust_keys",
        "state_dir",
        "platform"
    ]);
    defs.get_mut("HostInputs").unwrap()["properties"]["roots"]["maxItems"] = serde_json::json!(32);
    for name in [
        "SuccessResponse",
        "ErrorResponse",
        "Capabilities",
        "InstallProgressEvent",
        "SourceProgressEvent",
        "SourceUpdateEvent",
        "MatchProgressEvent",
        "MatchUpdateEvent",
    ] {
        if let Some(d) = defs.get_mut(name) {
            d["properties"]["protocol_version"] = serde_json::json!({"const":PROTOCOL_VERSION});
        }
    }
    defs.get_mut("SuccessResponse").unwrap()["properties"]["ok"] =
        serde_json::json!({"const":true});
    defs.get_mut("ErrorResponse").unwrap()["properties"]["ok"] = serde_json::json!({"const":false});
    for name in ["PreviewParams", "InstallPatchParams"] {
        let d = defs.get_mut(name).unwrap();
        d["oneOf"] = serde_json::json!([
        {"required":["release_id"],"properties":{"release_id":{"type":"string"},"selection_id":{"type":"null"}}},
        {"required":["selection_id"],"properties":{"selection_id":{"type":"string","pattern":"^(zip|exe)-selection-[0-9a-f]{24}-[1-9][0-9]*$"},"release_id":{"type":"null"}}}
        ]);
        if name == "InstallPatchParams" {
            d["allOf"] = serde_json::json!([{"anyOf":[{"properties":{"selection_id":{"type":"null"}}},{"properties":{"archive":{"type":"null"}}}]}]);
        }
    }
    for name in [
        "ListGameSourcesParams",
        "SetGameSourceParams",
        "RematchGameParams",
    ] {
        defs.get_mut(name).unwrap()["anyOf"] = serde_json::json!([{"required":["instance_id"],"properties":{"instance_id":{"type":"string"}}},{"required":["appid"],"properties":{"appid":{"type":"integer","minimum":0}}}]);
    }
    // Strict event and outcome wire DTOs require their nullable facts explicitly.
    for name in [
        "ErrorResponse",
        "InstallProgressEvent",
        "SourceProgressEvent",
        "SourceUpdateEvent",
        "MatchProgressEvent",
        "MatchUpdateEvent",
        "InstallerOutcome",
        "ProposedChange",
        "PreviewReport",
        "Capabilities",
    ] {
        let d = defs.get_mut(name).unwrap();
        let required: Vec<_> = d["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        d["required"] = serde_json::json!(required);
    }
    for name in ["SourceSummary", "GameSourceSummary"] {
        defs.get_mut(name).unwrap()["additionalProperties"] = serde_json::json!(false);
    }
    defs.get_mut("DomainError").unwrap()["properties"]["code"] =
        serde_json::json!({"type":"string","minLength":1,"maxLength":128,"pattern":"^[a-z0-9_]+$"});
    defs.get_mut("Detection").unwrap()["properties"]["status"] = serde_json::json!({"enum":["unknown","not_detected","partial_or_conflict","installed_external","installed_managed"]});
    defs.get_mut("FileEvidence").unwrap()["properties"]["status"] =
        serde_json::json!({"enum":["match","mismatch","missing","unreadable_or_unsafe"]});
    defs.get_mut("Transaction").unwrap()["properties"]["state"] =
        serde_json::json!({"enum":TRANSACTION_STATES});
    defs.get_mut("PrimaryAction").unwrap()["properties"]["operation"] =
        serde_json::json!({"const":"overwrite"});
    defs.get_mut("ActionAvailability").unwrap()["oneOf"] = serde_json::json!([{"properties":{"available":{"const":true},"blockers":{"maxItems":0}}},{"properties":{"available":{"const":false},"blockers":{"minItems":1}}}]);
    defs.get_mut("InstallProgressEvent").unwrap()["anyOf"] = serde_json::json!([{"properties":{"stage":{"const":"applying"}}},{"properties":{"substage":{"type":"null"}}}]);
    defs.get_mut("InstallerOutcome").unwrap()["oneOf"] = serde_json::json!([{"properties":{"state":{"const":"committed"},"disposition":{"const":"completed"},"recovery":{"type":"null"}}},{"properties":{"state":{"const":"needs_recovery"},"disposition":{"const":"failed"},"recovery":{"type":"object","required":["required"],"properties":{"required":{"const":true}}}}},{"properties":{"state":{"const":"rolled_back"},"disposition":{"enum":["cancelled","failed"]}}}]);
    defs.get_mut("InstallerFormats").unwrap()["properties"]["zip"] =
        serde_json::json!({"const":"supported"});
    defs.get_mut("InstallerFormats").unwrap()["properties"]["exe"] =
        serde_json::json!({"const":"supported"});
    defs.get_mut("InstallerCapabilities").unwrap()["properties"]["strategies"] =
        serde_json::json!({"const":["managed_zip","publisher_zip","publisher_exe"]});
    defs.get_mut("WireLinkCapability").unwrap()["oneOf"] = serde_json::json!([{"properties":{"available":{"const":true},"blockers":{"maxItems":0}}},{"properties":{"available":{"const":false},"blockers":{"minItems":1}}}]);
    defs.get_mut("PreviewReport").unwrap()["properties"]["executable"] =
        serde_json::json!({"const":false});
    let variants: Vec<Value> = ALL_METHODS.iter().map(|method| {
        let name = match method {
            Method::GetCapabilities | Method::Scan | Method::ListTransactions | Method::Recover
                | Method::CleanupCache | Method::BackupStatus | Method::CleanupBackups | Method::ListSources => "HostParams",
            Method::Preview => "PreviewParams",
            Method::InstallPatch => "InstallPatchParams",
            Method::DiscardPlan => "DiscardPlanParams",
            Method::ProjectInstances => "ProjectInstancesParams",
            Method::ListGameSources => "ListGameSourcesParams",
            Method::SetSourceEnabled => "SetSourceEnabledParams",
            Method::SetGameSource => "SetGameSourceParams",
            Method::AddSource => "AddSourceParams",
            Method::SetSourceUpdateInterval => "SetSourceUpdateIntervalParams",
            Method::UpdateSource => "UpdateSourceParams",
            Method::MatchGames => "MatchGamesParams",
            Method::RematchGame => "RematchGameParams",
        };
        serde_json::json!({"type":"object","additionalProperties":false,"required":["protocol_version","request_id","method","params"],"properties":{"protocol_version":{"const":PROTOCOL_VERSION},"request_id":{"type":"string"},"method":{"const":method.as_str()},"params":{"$ref":format!("#/$defs/{name}")}}})
    }).collect();
    defs.insert("Request".to_owned(), serde_json::json!({"oneOf":variants}));
    for (name, item) in [
        ("Transactions", "Transaction"),
        ("Sources", "SourceSummary"),
        ("GameSources", "GameSourceSummary"),
    ] {
        defs.insert(
            name.to_owned(),
            serde_json::json!({"type":"array","items":{"$ref":format!("#/$defs/{item}")}}),
        );
    }
    for d in defs.values_mut() {
        if let Some(property) = d
            .get_mut("properties")
            .and_then(|p| p.get_mut("instance_id"))
        {
            property["x-max-utf8-bytes"] = serde_json::json!(MAX_INSTANCE_ID_BYTES);
        }
    }
    v
}

pub mod fixtures;

fn required_fields(v: &Value, fields: &[&str]) -> Result<(), ProtocolError> {
    if !fields.iter().all(|k| v.get(*k).is_some()) {
        return Err(ProtocolError::code("invalid_core_response"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum DetectionStatus {
    Unknown,
    NotDetected,
    PartialOrConflict,
    InstalledExternal,
    InstalledManaged,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Match,
    Mismatch,
    Missing,
    UnreadableOrUnsafe,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum TransactionState {
    Planned,
    BackingUp,
    Applying,
    Committed,
    Superseded,
    Uninstalling,
    RollingBack,
    RolledBack,
    Uninstalled,
    NeedsRecovery,
    Failed,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeState {
    Committed,
    Failed,
    RolledBack,
    NeedsRecovery,
}

fn validate_numbers(v: &Value) -> Result<(), ProtocolError> {
    match v {
        Value::Number(n) => {
            if n.as_u64().is_some_and(|n| n > MAX_SAFE_INTEGER)
                || n.as_i64()
                    .is_some_and(|n| n.unsigned_abs() > MAX_SAFE_INTEGER)
                || (!n.is_u64() && !n.is_i64())
            {
                return Err(ProtocolError::code("invalid_request"));
            }
        }
        Value::Array(a) => {
            for v in a {
                validate_numbers(v)?;
            }
        }
        Value::Object(o) => {
            for v in o.values() {
                validate_numbers(v)?;
            }
        }
        _ => {}
    }
    Ok(())
}
/// Reject ambiguous duplicate JSON keys recursively before DTO decoding.
struct StrictValue(Value);
impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = StrictValue;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("unambiguous JSON")
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<StrictValue, E> {
                Ok(StrictValue(Value::Bool(v)))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<StrictValue, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<StrictValue, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<StrictValue, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| StrictValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("invalid number"))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<StrictValue, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<StrictValue, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<StrictValue, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> Result<StrictValue, A::Error> {
                let mut values = Vec::new();
                while let Some(v) = a.next_element::<StrictValue>()? {
                    values.push(v.0);
                }
                Ok(StrictValue(Value::Array(values)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut a: A,
            ) -> Result<StrictValue, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if values.contains_key(&k) {
                        return Err(serde::de::Error::custom("duplicate key"));
                    }
                    values.insert(k, a.next_value::<StrictValue>()?.0);
                }
                Ok(StrictValue(Value::Object(values)))
            }
        }
        d.deserialize_any(V)
    }
}

pub fn result_schema_name(method: Method) -> &'static str {
    match method {
        Method::GetCapabilities => "Capabilities",
        Method::Scan => "ScanReport",
        Method::ProjectInstances => "ProjectInstancesResult",
        Method::Preview => "PreviewReport",
        Method::InstallPatch => "InstallerOutcome",
        Method::ListTransactions | Method::Recover => "Transactions",
        Method::DiscardPlan => "Transaction",
        Method::CleanupCache => "CleanupResult",
        Method::BackupStatus => "BackupInventory",
        Method::CleanupBackups => "BackupCleanupResult",
        Method::ListSources => "Sources",
        Method::ListGameSources => "GameSources",
        Method::SetSourceEnabled => "SourceEnabledResult",
        Method::SetGameSource => "GameSourceResult",
        Method::AddSource => "AddSourceResult",
        Method::SetSourceUpdateInterval => "SetSourceIntervalResult",
        Method::UpdateSource => "SourceUpdateResult",
        Method::MatchGames | Method::RematchGame => "MatchGamesResult",
    }
}
fn output_schema() -> &'static Value {
    static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(schema)
}
/// Structural checks use the same enriched graph consumed by generated hosts.
fn matches_schema(v: &Value, s: &Value, root: &Value) -> bool {
    if let Some(reference) = s.get("$ref").and_then(Value::as_str) {
        return root
            .pointer(reference.strip_prefix('#').unwrap_or(reference))
            .is_some_and(|s| matches_schema(v, s, root));
    }
    if let Some(c) = s.get("const") {
        if v != c {
            return false;
        }
    }
    if let Some(values) = s.get("enum").and_then(Value::as_array) {
        if !values.contains(v) {
            return false;
        }
    }
    for (key, exact) in [("anyOf", false), ("oneOf", true)] {
        if let Some(a) = s.get(key).and_then(Value::as_array) {
            let count = a.iter().filter(|s| matches_schema(v, s, root)).count();
            if count == 0 || (exact && count != 1) {
                return false;
            }
        }
    }
    if let Some(a) = s.get("allOf").and_then(Value::as_array) {
        if !a.iter().all(|s| matches_schema(v, s, root)) {
            return false;
        }
    }
    if let Some(t) = s.get("type") {
        let valid = |t: &str| match t {
            "object" => v.is_object(),
            "array" => v.is_array(),
            "string" => v.is_string(),
            "boolean" => v.is_boolean(),
            "null" => v.is_null(),
            "integer" => v.as_i64().is_some() || v.as_u64().is_some(),
            "number" => v.is_number(),
            _ => false,
        };
        if !t.as_str().is_some_and(valid)
            && !t
                .as_array()
                .is_some_and(|a| a.iter().any(|t| t.as_str().is_some_and(valid)))
        {
            return false;
        }
    }
    if let Some(value) = v.as_str() {
        if s.get("x-max-utf8-bytes")
            .and_then(Value::as_u64)
            .is_some_and(|limit| value.len() as u64 > limit)
        {
            return false;
        }
    }
    if let Some(obj) = v.as_object() {
        if s.get("required")
            .and_then(Value::as_array)
            .is_some_and(|a| {
                a.iter()
                    .any(|k| !obj.contains_key(k.as_str().unwrap_or("")))
            })
        {
            return false;
        }
        for (k, v) in obj {
            if let Some(rule) = s.get("properties").and_then(|p| p.get(k)) {
                if !matches_schema(v, rule, root) {
                    return false;
                }
            } else if s.get("additionalProperties") == Some(&Value::Bool(false)) {
                return false;
            }
        }
    }
    if let Some(a) = v.as_array() {
        if s.get("minItems")
            .and_then(Value::as_u64)
            .is_some_and(|n| a.len() < (n as usize))
            || s.get("maxItems")
                .and_then(Value::as_u64)
                .is_some_and(|n| a.len() > (n as usize))
        {
            return false;
        }
        if let Some(rule) = s.get("items") {
            if !a.iter().all(|v| matches_schema(v, rule, root)) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod early_error_tests {
    use super::*;

    #[test]
    fn early_errors_have_schema_valid_required_nullable_envelopes() {
        let cases: Value = serde_json::from_str(include_str!(
            "../../../packages/contracts/fixtures/v5/cases.json"
        ))
        .unwrap();
        let schema = schema();
        for case in cases["early_errors"].as_array().unwrap() {
            let bytes = case["json"]
                .as_str()
                .map_or_else(|| vec![255], |raw| raw.as_bytes().to_vec());
            let error =
                decode_request(&bytes, case["task_channel"].as_bool().unwrap()).unwrap_err();
            let mut envelope = serde_json::json!({
                "protocol_version": PROTOCOL_VERSION,
                "request_id": error.request_id,
                "ok": false,
                "error": {"code": error.code},
            });
            if let Some(detail) = error.detail {
                envelope["error"]["detail"] = serde_json::json!(detail);
            }
            assert_eq!(envelope, case["response"], "producer {}", case["name"]);
            assert!(
                matches_schema(&envelope, &schema["$defs"]["Response"], &schema),
                "schema {}",
                case["name"]
            );
            let parsed = serde_json::from_value::<ErrorResponse>(envelope.clone()).unwrap();
            if parsed.request_id.as_deref() == Some("fixture") {
                assert!(validate_response(&envelope, "fixture", Some(Method::Scan)).is_ok());
            } else {
                assert_eq!(
                    validate_response(&envelope, "fixture", Some(Method::Scan))
                        .unwrap_err()
                        .code,
                    "invalid_core_response"
                );
            }
            let mut missing = envelope;
            missing.as_object_mut().unwrap().remove("request_id");
            assert!(!matches_schema(
                &missing,
                &schema["$defs"]["Response"],
                &schema
            ));
            assert!(serde_json::from_value::<ErrorResponse>(missing).is_err());
        }
        let mut success = serde_json::json!({"protocol_version":5,"request_id":null,"ok":true,"result":fixtures::scan()});
        assert!(!matches_schema(
            &success,
            &schema["$defs"]["Response"],
            &schema
        ));
        assert!(serde_json::from_value::<SuccessResponse>(success.clone()).is_err());
        success.as_object_mut().unwrap().remove("request_id");
        assert!(!matches_schema(
            &success,
            &schema["$defs"]["Response"],
            &schema
        ));
    }
}

#[test]
fn schema_request_branches_accept_every_declared_method() {
    let schema = schema();
    assert_eq!(
        schema["$defs"]["Request"]["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        ALL_METHODS.len()
    );
    for method in ALL_METHODS {
        assert!(
            matches_schema(
                &fixtures::request(method),
                &schema["$defs"]["Request"],
                &schema
            ),
            "{}",
            method.as_str()
        );
    }
}
