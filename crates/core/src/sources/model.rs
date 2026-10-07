//! Serde + JSON-schema types for shared source discovery (design §2).
//!
//! Field names, `Option` null-serialization and `skip_serializing_if` rules are
//! copied verbatim from the Python JSON / `packages/ui/src/index.ts` DTOs. Types
//! used behind `#[serde(flatten)]` deliberately do not derive `Deserialize`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn d_role() -> SourceRole {
    SourceRole::Publisher
}

fn d_priority() -> u16 {
    100
}

// ---- §2.1 rules ----------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceRole {
    Publisher,
    FallbackIndexer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RootLayout {
    GameRoot,
}

/// Deserialization intermediate: loose field types so `validate_rule` can return
/// the exact Python error codes.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RawSourceRule {
    pub id: String,
    pub name: String,
    pub adapter: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub discovery_only: Option<bool>,
    #[serde(default)]
    pub priority: Option<i64>,
    pub enabled: bool,
    pub pages: Vec<String>,
    pub page_hosts: Vec<String>,
    #[serde(default)]
    pub download_hosts: Option<Vec<String>>,
    #[serde(default)]
    pub root_layout: Option<String>,
    #[serde(default)]
    pub columns: Option<Columns>,
    #[serde(default)]
    pub table_id: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// Validated canonical rule; field names match the Python rule dict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceRule {
    pub id: String,
    pub name: String,
    pub adapter: String,
    #[serde(default = "d_role")]
    pub role: SourceRole,
    #[serde(default)]
    pub discovery_only: bool,
    #[serde(default = "d_priority")]
    pub priority: u16,
    pub enabled: bool,
    pub pages: Vec<String>,
    pub page_hosts: Vec<String>,
    #[serde(default)]
    pub download_hosts: Vec<String>,
    #[serde(default)]
    pub root_layout: Option<RootLayout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<Columns>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Columns {
    pub title: u8,
    pub download: u8,
    #[serde(default)]
    pub date: Option<u8>,
    #[serde(default)]
    pub language: Option<u8>,
    #[serde(default)]
    pub appid: Option<u8>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RulesConfig {
    pub schema_version: u32,
    pub sources: Vec<RawSourceRule>,
}

// ---- §2.2 overrides / settings / summaries -------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Overrides {
    #[serde(default)]
    pub enabled: BTreeMap<String, bool>,
    #[serde(default)]
    pub added: Vec<SourceRule>,
    /// `"<appid>" -> source_id -> subscribed`.
    #[serde(default)]
    pub games: BTreeMap<String, BTreeMap<String, bool>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SettingsFile {
    pub schema_version: u32,
    pub revision: u64,
    pub enabled: BTreeMap<String, bool>,
    pub added: Vec<SourceRule>,
    pub games: BTreeMap<String, BTreeMap<String, bool>>,
}

/// Output-only summary; injected with a flattened [`SourceRule`].
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SourceSummary {
    #[serde(flatten)]
    pub rule: SourceRule,
    /// `"builtin"` (compiled default) or `"added"` (user rule, including one that
    /// shadows a builtin id).
    pub origin: String,
    pub status: String,
    pub entry_count: u64,
    pub cached_at: Option<u64>,
    pub checked_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct GameSourceSummary {
    #[serde(flatten)]
    pub source: SourceSummary,
    pub subscribed: bool,
    pub matched: bool,
}

/// Aggregated discovery state for one game (matches TS `GameDiscovery`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GameDiscovery {
    pub status: String,
    pub candidates: Vec<SourceCandidate>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Output of `matches()`: the matched source entry plus its rule/ release ids
/// (field names mirror the Python `{**selected, source_id, release_id,
/// root_layout}` dict and the TS `SourceSubscription` shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SourceMatch {
    pub title: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub url: String,
    pub source_page: String,
    pub language: String,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub appid: Option<u32>,
    pub source_id: String,
    pub release_id: String,
    pub root_layout: Option<RootLayout>,
}

// ---- §2.3 source cache / status ------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceEntry {
    pub title: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub url: String,
    pub source_page: String,
    pub language: String,
    /// `"YYYY/M/D"` or `"COMING SOON"`.
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub appid: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceCache {
    pub entries: Vec<SourceEntry>,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub modified: Option<String>,
    #[serde(default)]
    pub cached_at: Option<u64>,
    #[serde(default)]
    pub checked_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceStatus {
    pub checked_at: u64,
    #[serde(default)]
    pub cached_at: Option<u64>,
    #[serde(default)]
    pub entry_count: u64,
    pub code: String,
}

// ---- §2.4 candidates / records (aligned with the TS DTO) -----------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    Base,
    Dlc,
    Demo,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MatchBasis {
    Appid,
    VerifiedAlias,
    OfficialTitle,
    Title,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    DiscoverOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactFormat {
    Zip,
    Exe,
    Web,
    Unknown,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CacheStatus {
    Fresh,
    Cached,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Mirror {
    pub url: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdentityEvidence {
    pub kind: String,
    #[serde(default)]
    pub appid: Option<u32>,
    #[serde(default)]
    pub page: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub source_id: String,
    pub page: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceCandidate {
    /// `candidate-[0-9a-f]{24}`.
    pub candidate_id: String,
    pub source_id: String,
    /// `canonical_url(source_page)`.
    pub source_record_id: String,
    /// Always present, may be `null`.
    pub game_appid: Option<u32>,
    /// Always present, may be `null`.
    pub parent_appid: Option<u32>,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub game_title: String,
    pub patch_title: String,
    pub variant: Variant,
    /// Always present, may be `null`.
    pub requires_dlc: Option<Vec<u32>>,
    /// Always present, may be `null`.
    pub excludes_dlc: Option<Vec<u32>>,
    #[serde(default)]
    pub platforms: Vec<String>,
    /// `"adult_content"`.
    pub patch_kind: String,
    #[serde(default)]
    pub languages: Vec<String>,
    pub version_raw: Option<String>,
    #[serde(default)]
    pub version_evidence: Vec<String>,
    pub version_identity: String,
    pub published_at: Option<String>,
    pub patch_updated_at: Option<String>,
    pub source_page: String,
    pub upstream_page: Option<String>,
    #[serde(default)]
    pub mirrors: Vec<Mirror>,
    pub format_reported: ArtifactFormat,
    pub format_observed: ArtifactFormat,
    pub capability: Capability,
    #[serde(default)]
    pub identity_evidence: Vec<IdentityEvidence>,
    #[serde(default)]
    pub provenance: Vec<Provenance>,
    pub match_basis: MatchBasis,
    /// ISO8601 string.
    pub observed_at: String,
    pub cache_status: CacheStatus,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub fingerprint: String,
    /// Only present after sources.py merges index records.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_index_pages: Vec<Provenance>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    #[serde(default)]
    pub source_id: Option<String>,
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

// ---- §2.5 discovery records / index envelopes ----------------------------

/// `success` and `attempt` carry different field sets (design §2.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoverySuccess {
    /// `complete|skipped`.
    pub status: String,
    pub candidates: Vec<SourceCandidate>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    pub checked_at: u64,
    #[serde(default)]
    pub cached_at: Option<u64>,
    pub query_key: String,
    pub parser_version: u32,
    pub query_version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryAttempt {
    /// `complete|partial|failed|skipped`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<SourceCandidate>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    pub checked_at: u64,
    #[serde(default)]
    pub cached_at: Option<u64>,
    pub query_key: String,
    pub parser_version: u32,
    pub query_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<DiscoveryIndex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queries: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_cache: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryRecord {
    /// `1`.
    pub schema_version: u32,
    pub source_id: String,
    pub appid: u32,
    #[serde(default)]
    pub success: Option<DiscoverySuccess>,
    #[serde(default)]
    pub attempt: Option<DiscoveryAttempt>,
}

/// `kind` is `product|article|category`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexRecord {
    pub title: String,
    pub page: String,
    pub kind: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryIndex {
    pub parser_version: u32,
    #[serde(default)]
    pub records: Vec<IndexRecord>,
    #[serde(default)]
    pub pending: Vec<String>,
    #[serde(default)]
    pub checked_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryIndexRecord {
    pub schema_version: u32,
    pub checked_at: u64,
    pub index: DiscoveryIndex,
}

// ---- §2.6 query context / game input -------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GameQuery {
    pub appid: u32,
    pub name: String,
    #[serde(default)]
    pub localized_name: Option<String>,
    #[serde(default)]
    pub instance_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryContext {
    pub appid: u32,
    pub name: String,
    /// `<= 3` entries, each `<= 256`.
    #[serde(default)]
    pub name_hints: Vec<String>,
    #[serde(default)]
    pub preferred_language: Option<String>,
    pub legacy_match: bool,
    pub force: bool,
    #[serde(default)]
    pub source_context: Vec<String>,
    /// `2`.
    pub parser_version: u32,
    /// `1`.
    pub query_version: u32,
    pub settings_revision: u64,
    #[serde(default)]
    pub generation: Option<u64>,
}
