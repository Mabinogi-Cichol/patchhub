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

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum SourceRole {
    Publisher,
    FallbackIndexer,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum RootLayout {
    GameRoot,
}

/// Deserialization intermediate: loose field types so `validate_rule` can return
/// the exact Python error codes.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema, ts_rs::TS, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawSourceRule {
    pub id: String,
    pub name: String,
    pub adapter: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub role: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub discovery_only: Option<bool>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub priority: Option<i64>,
    pub enabled: bool,
    pub pages: Vec<String>,
    pub page_hosts: Vec<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub download_hosts: Option<Vec<String>>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub root_layout: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub columns: Option<Columns>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub table_id: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub language: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct ScriptDescriptor {
    pub api_version: u32,
    pub sha256: String,
}

/// Validated canonical rule; field names match the Python rule dict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
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
    #[ts(optional = nullable)]
    pub columns: Option<Columns>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub table_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub script: Option<ScriptDescriptor>,
}

/// Author-facing rule without the host-generated script descriptor.
impl From<&SourceRule> for RawSourceRule {
    fn from(rule: &SourceRule) -> Self {
        Self {
            id: rule.id.clone(),
            name: rule.name.clone(),
            adapter: rule.adapter.clone(),
            role: Some(
                match rule.role {
                    SourceRole::Publisher => "publisher",
                    SourceRole::FallbackIndexer => "fallback_indexer",
                }
                .into(),
            ),
            discovery_only: Some(rule.discovery_only),
            priority: Some(rule.priority.into()),
            enabled: rule.enabled,
            pages: rule.pages.clone(),
            page_hosts: rule.page_hosts.clone(),
            download_hosts: Some(rule.download_hosts.clone()),
            root_layout: rule.root_layout.map(|_| "game_root".into()),
            columns: rule.columns.clone(),
            table_id: rule.table_id.clone(),
            language: rule.language.clone(),
            url: rule.url.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct ScriptSourceDocument {
    pub rule: RawSourceRule,
    pub api_version: u32,
    pub code: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct ImportedScriptSource {
    pub rule: SourceRule,
    #[ts(type = "number")]
    pub settings_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
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
    /// `source_id -> automatic update interval in hours` (schema 2). Missing or
    /// `0` means automatic updates are off for that source.
    #[serde(default)]
    pub intervals: BTreeMap<String, u64>,
    /// User display names, independent of source identity and compiled rules.
    #[serde(default)]
    pub names: BTreeMap<String, String>,
    /// Stored together with rules/descriptors/revision (settings schema 3).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub script_sources: BTreeMap<String, String>,
}

/// Output-only summary; injected with a flattened [`SourceRule`].
#[derive(Debug, Clone, Serialize, schemars::JsonSchema, ts_rs::TS, serde::Deserialize)]
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
    /// Current published index revision (0 when no index snapshot exists).
    #[ts(type = "number")]
    pub revision: u64,
    /// Whether the current published index snapshot covers the whole source.
    pub complete: bool,
    /// Configured automatic update interval in hours (`0` = off).
    #[ts(type = "number")]
    pub interval_hours: u64,
    /// Next scheduled attempt derived from `checked_at + interval_hours * 3600`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub next_due_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema, ts_rs::TS, serde::Deserialize)]
pub struct GameSourceSummary {
    #[serde(flatten)]
    pub source: SourceSummary,
    pub subscribed: bool,
    pub matched: bool,
}

/// Aggregated discovery state for one game (matches TS `GameDiscovery`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct GameDiscovery {
    pub status: String,
    pub candidates: Vec<SourceCandidate>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Output of `matches()`: the matched source entry plus its rule/ release ids
/// (field names mirror the Python `{**selected, source_id, release_id,
/// root_layout}` dict and the TS `SourceSubscription` shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
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

// ---- §2.3 legacy cache / status ------------------------------------------
//
// `SourceEntry`/`SourceCache` are the v1 table-row shapes: the ZIP collectors
// still produce/consume them internally and the one-time v1 → v2 migration reads
// them. Nothing else reads the old `cache/` tree at runtime.

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
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

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    Base,
    Dlc,
    Demo,
    Unknown,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum MatchBasis {
    Appid,
    VerifiedAlias,
    OfficialTitle,
    Title,
    None,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    DiscoverOnly,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactFormat {
    Zip,
    Exe,
    Web,
    Unknown,
    Mixed,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum CacheStatus {
    Fresh,
    Cached,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct Mirror {
    pub url: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct IdentityEvidence {
    pub kind: String,
    #[serde(default)]
    pub appid: Option<u32>,
    #[serde(default)]
    pub page: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub source_id: String,
    pub page: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct SourceCandidate {
    /// Host-bound script identity, also binds install selections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub parser_fingerprint: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "DiscoveryDiagnostic")]
pub struct Diagnostic {
    #[serde(default)]
    pub source_id: Option<String>,
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub detail: Option<String>,
}

// ---- §2.5 legacy discovery records (migration-only) ----------------------
//
// These types are never written by the v5 flow; they exist only so the one-time
// v1 → v2 migration can read `discovery/` and `discovery-index/` and build
// `matches/` + `index/`. Old files stay on disk but are not read afterwards.

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
    /// `SOURCES_ENCODING_VERSION`; missing (old records) deserializes to `0`.
    #[serde(default)]
    pub encoding_version: u32,
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
    /// `SOURCES_ENCODING_VERSION`; missing (old records) deserializes to `0`.
    #[serde(default)]
    pub encoding_version: u32,
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
    /// `SOURCES_ENCODING_VERSION`; missing (old records) deserializes to `0`.
    #[serde(default)]
    pub encoding_version: u32,
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

// ---- schema-2 persistent records (store schema 2, p1-contract §5–§7) ------
//
// These are on-disk only: no `ts_rs`/`schemars` derives, so the wire contract
// (protocol v4) is untouched.

/// Status of a persisted game match, derived by [`super::r#match::derive_game_status`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum GameMatchStatus {
    Pending,
    PublisherMatch,
    FallbackMatch,
    NoMatch,
    Incomplete,
    Failed,
}

/// Per-source match state inside [`GameMatchRecord`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceMatchStatus {
    NoIndex,
    IndexIncomplete,
    Matched,
    NoCandidate,
    DetailIncomplete,
    DetailFailed,
}

/// HGamer (fallback indexer) query state inside [`GameMatchRecord`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackMatchStatus {
    NotQueried,
    Matched,
    NoMatch,
    Incomplete,
    Failed,
    Disabled,
    NotNeeded,
}

/// One list-page entry of a [`SourceIndex`] (no detail-page scraping).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIndexEntry {
    pub title: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Normalized detail/product/article/category URL; for table sources the
    /// entry's source page.
    pub page: String,
    /// `product|article|category|entry`; kept as a string so an adapter or a
    /// migrated v1 record never loses an unrecognized kind.
    pub kind: String,
    #[serde(default)]
    pub appid: Option<u32>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
}

/// Published per-source index snapshot (`index/<source_id>.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIndex {
    /// Script parser identity; absent for sources still using Rust adapters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parser_fingerprint: Option<String>,
    pub schema_version: u32,
    pub source_id: String,
    /// Content revision: parser identity or real entries/complete/pending changes.
    pub revision: u64,
    pub complete: bool,
    pub checked_at: u64,
    pub updated_at: u64,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub modified: Option<String>,
    #[serde(default)]
    pub entries: Vec<SourceIndexEntry>,
    /// Unfinished pagination queue; non-empty implies `complete = false`.
    #[serde(default)]
    pub pending: Vec<String>,
}

/// A link from a category listing to an article/detail page (P3 facts).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactLink {
    pub title: String,
    pub page: String,
}

/// Query-independent facts parsed from one source page (design §4.1, §9).
///
/// Built by [`super::adapters::SourceAdapter::resolve_page`] during a game match task,
/// never during a source index update, and persisted inside [`DetailRecord`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageFact {
    pub title: String,
    pub page: String,
    /// `article|product|category`.
    pub kind: String,
    #[serde(default)]
    pub steam_ids: Vec<u32>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub mirrors: Vec<Mirror>,
    /// Body text, used by `identity::build_candidates`.
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub published_at: Option<String>,
    /// Absolute URLs inside the body.
    #[serde(default)]
    pub body_links: Vec<String>,
    /// Category pages: inline article list.
    #[serde(default)]
    pub article_links: Vec<FactLink>,
    /// Category pages whose articles are rendered in full.
    #[serde(default)]
    pub article_facts: Vec<PageFact>,
    #[serde(default)]
    pub next_page: Option<String>,
}

/// Detail cache for one source page, shared across games
/// (`details/<source_id>/<key>.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetailRecord {
    pub schema_version: u32,
    pub source_id: String,
    pub page: String,
    pub index_revision: u64,
    pub checked_at: u64,
    pub complete: bool,
    #[serde(default)]
    pub fact: Option<PageFact>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

/// Per-source match result inside [`GameMatchRecord`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMatchRecord {
    pub index_revision: u64,
    pub status: SourceMatchStatus,
    #[serde(default)]
    pub candidates: Vec<SourceCandidate>,
    #[serde(default)]
    pub detail_pages: Vec<String>,
    pub checked_at: u64,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

/// HGamer fallback result inside [`GameMatchRecord`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FallbackMatchRecord {
    pub query_key: String,
    pub status: FallbackMatchStatus,
    #[serde(default)]
    pub candidates: Vec<SourceCandidate>,
    pub checked_at: u64,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

/// Persisted match state for one AppID (`matches/<appid>.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameMatchRecord {
    pub schema_version: u32,
    pub appid: u32,
    pub name: String,
    #[serde(default)]
    pub localized_name: Option<String>,
    /// Matching-input digest (see [`super::r#match::input_key`]); empty when the
    /// migration could not reconstruct it (forces a rematch).
    pub input_key: String,
    pub status: GameMatchStatus,
    #[serde(default)]
    pub sources: BTreeMap<String, SourceMatchRecord>,
    #[serde(default)]
    pub fallback: Option<FallbackMatchRecord>,
    pub updated_at: u64,
}

/// Wire-ready game match result (protocol v5 `GameMatch`, p1-contract §9).
///
/// P3 produces this shape; the protocol envelope only references it from P4 on,
/// so adding the derives here does not change the generated v4 contracts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct GameMatch {
    pub appid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub instance_id: Option<String>,
    pub status: GameMatchStatus,
    pub candidates: Vec<SourceCandidate>,
    pub diagnostics: Vec<Diagnostic>,
    pub source_revisions: BTreeMap<String, u64>,
}
