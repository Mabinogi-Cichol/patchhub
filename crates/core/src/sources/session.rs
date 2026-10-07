//! P3 core discovery session: per-game discovery, bounded batch scanning and
//! ZIP-index refresh. Pure orchestration over the registry, HTTP context and
//! adapters; the CLI wraps these with its transport/event layer.

use super::adapters::{self, DiscoverPrevious};
use super::canonical;
use super::http::RequestContext;
use super::identity;
use super::model::{
    ArtifactFormat, Diagnostic, GameDiscovery, GameQuery, IdentityEvidence, MatchBasis, Provenance,
    QueryContext, RootLayout, SourceCandidate, SourceRole, SourceRule, Variant,
};
use super::registry::{self, SourceRegistry};
use super::{
    invalid, official_subbudget, Result, OFFICIAL_SUBDEADLINE, PARSER_VERSION, QUERY_VERSION,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Capability negotiation payload (`get_capabilities`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Capabilities {
    pub methods: Vec<String>,
    pub discovery_schema_version: u32,
    pub adapter_version: u32,
    pub parser_version: u32,
    pub query_version: u32,
}

/// Host-only policy snapshot for a source (`get_source_policy`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SourcePolicy {
    pub rule: SourceRule,
    pub origin: String,
    pub fingerprint: String,
}

/// Resolved source for installation consumers (`resolve_source_record`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ResolvedSource {
    pub instance_id: String,
    pub record_id: String,
    pub release_id: Option<String>,
    pub source_id: String,
    pub root_layout: Option<RootLayout>,
    pub page_hosts: Vec<String>,
    pub download_hosts: Vec<String>,
}

/// Progressive batch-scan event; the CLI adds request_id/instance context.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub enum ScanEvent {
    Progress {
        completed: usize,
        total: usize,
        running: bool,
    },
    Update {
        appid: u32,
        index: usize,
        discovery: GameDiscovery,
    },
}

/// Discovery bound the session advertises (single-threaded orchestration; the
/// CLI may parallelize while keeping the same per-game contract).
pub const SCAN_CONCURRENCY: usize = 3;

const METHODS: &[&str] = &[
    "get_capabilities",
    "list_sources",
    "list_source_matches",
    "list_game_sources",
    "set_source_enabled",
    "set_game_source",
    "add_source",
    "get_source_state",
    "get_source_settings",
    "refresh_sources",
    "discover_game",
    "scan_sources",
    "get_source_policy",
    "resolve_source_record",
];

pub fn capabilities() -> Capabilities {
    Capabilities {
        methods: METHODS.iter().map(|name| (*name).to_string()).collect(),
        discovery_schema_version: 1,
        adapter_version: 1,
        parser_version: PARSER_VERSION,
        query_version: QUERY_VERSION,
    }
}

/// `discover_game`: validate, resolve effective rules, short-circuit fresh cache,
/// then dispatch each pending rule through one shared budget/deadline and
/// compare-and-publish the commit.
pub fn discover_game(
    registry: &mut SourceRegistry,
    ctx: &mut RequestContext,
    game: &GameQuery,
    name_hints: &[String],
    preferred_language: Option<&str>,
    force: bool,
    cancel: &AtomicBool,
) -> Result<GameDiscovery> {
    SourceRegistry::validate_query(game, name_hints, preferred_language)?;

    // §4.2: read the manifest name and a non-empty localized name from the
    // trusted local instance, deduped in a deterministic order; frontend hints
    // stay within the same ≤3 / ≤256 budget.
    let merged_hints = merged_name_hints(game, name_hints);

    let rules = registry.effective_discovery_rules(game.appid);
    let legacy_matches = registry.matches(game, false);
    let legacy_match =
        !SourceRegistry::filter_by_language(&legacy_matches, preferred_language).is_empty();
    let source_context: Vec<String> = rules.iter().map(|rule| rule.id.clone()).collect();
    let query = QueryContext {
        appid: game.appid,
        name: game.name.clone(),
        name_hints: merged_hints,
        preferred_language: preferred_language.map(str::to_string),
        legacy_match,
        force,
        source_context,
        parser_version: PARSER_VERSION,
        query_version: QUERY_VERSION,
        settings_revision: registry.settings_revision(),
        generation: None,
    };
    let key = query_key(&query);

    let pending: Vec<SourceRule> = rules
        .iter()
        .filter(|rule| registry.needs_refresh(rule, game.appid, &key, force))
        .cloned()
        .collect();
    if pending.is_empty() {
        return Ok(registry.cached_discovery(game));
    }

    let captured = registry.settings_revision();
    let mut stale = false;
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let official_rules: Vec<SourceRule> = rules
        .iter()
        .filter(|rule| rule.role == SourceRole::Publisher)
        .cloned()
        .collect();
    let fallback_rules: Vec<SourceRule> = rules
        .iter()
        .filter(|rule| rule.role == SourceRole::FallbackIndexer)
        .cloned()
        .collect();

    // --- first pass: publisher rules among the pending set (Python `officials`)
    let mut results: Vec<(SourceRule, adapters::DiscoverOutcome)> = Vec::new();
    for rule in pending
        .iter()
        .filter(|rule| rule.role == SourceRole::Publisher)
    {
        if cancel.load(Ordering::Relaxed) || crate::operation::check().is_err() {
            return Err(invalid("task_cancelled"));
        }
        let input = discover_previous(registry, rule, Vec::new());
        // Keep most of the single-game deadline/budget for the fallback pass
        // (design §0 `OFFICIAL_SUBDEADLINE` / `OFFICIAL_SUBBUDGET`).
        let deadline = ctx.deadline;
        let budget = ctx.budget;
        ctx.deadline = deadline.min(Instant::now() + OFFICIAL_SUBDEADLINE);
        ctx.budget = official_subbudget(budget, ctx.requests);
        let outcome = adapters::discover(rule, &query, ctx, &input);
        ctx.deadline = deadline;
        ctx.budget = budget;
        match outcome {
            Ok(outcome) => results.push((rule.clone(), outcome)),
            Err(error) => diagnostics.push(Diagnostic {
                source_id: Some(rule.id.clone()),
                code: error.to_string(),
                detail: None,
            }),
        }
    }

    // The fallback gate sees the publisher candidates collected so far plus the
    // last verified success for publishers that were not re-queried.
    let mut all_candidates = registry.previous_success_candidates(game.appid);
    for (rule, outcome) in &results {
        if rule.role == SourceRole::Publisher {
            all_candidates.retain(|candidate| candidate.source_id != rule.id);
            all_candidates.extend(outcome.candidates.iter().cloned());
        }
    }
    let official_failure = !diagnostics.is_empty()
        || results
            .iter()
            .any(|(_, outcome)| matches!(outcome.status.as_str(), "failed" | "partial"));
    let run_fallback = query.force
        || ((identity::needs_fallback(&all_candidates, &query) || official_failure)
            && !query.legacy_match);

    if run_fallback {
        for rule in pending
            .iter()
            .filter(|rule| rule.role == SourceRole::FallbackIndexer)
        {
            if cancel.load(Ordering::Relaxed) || crate::operation::check().is_err() {
                return Err(invalid("task_cancelled"));
            }
            let input = discover_previous(registry, rule, Vec::new());
            let outcome = match adapters::discover(rule, &query, ctx, &input) {
                Ok(outcome) => outcome,
                Err(error) => {
                    diagnostics.push(Diagnostic {
                        source_id: Some(rule.id.clone()),
                        code: error.to_string(),
                        detail: None,
                    });
                    continue;
                }
            };
            let identities: Vec<identity::Identity> = outcome
                .identities
                .iter()
                .filter(|entry| entry.appid == query.appid)
                .cloned()
                .collect();
            let fallback_candidates = outcome.candidates.clone();
            upsert_result(&mut results, rule, outcome);

            let has_upstream = fallback_candidates
                .iter()
                .any(|candidate| candidate.upstream_page.is_some());
            if identities.is_empty() && !has_upstream {
                continue;
            }

            // Second pass over every effective publisher: direct-revalidate the
            // confirmed upstream pages and enrich title-only candidates with the
            // fallback identities (Python `_discovery_sync` §L723-745).
            for official in &official_rules {
                let pages: Vec<String> = fallback_candidates
                    .iter()
                    .filter_map(|candidate| candidate.upstream_page.as_ref())
                    .filter(|page| adapters::hostname_in(page, &official.page_hosts))
                    .cloned()
                    .collect();
                let mut supplemented = query.clone();
                let mut hints = supplemented.name_hints.clone();
                for entry in &identities {
                    for alias in &entry.aliases {
                        if hints.len() >= 12 {
                            break;
                        }
                        if !hints.iter().any(|existing| existing == alias) {
                            hints.push(alias.clone());
                        }
                    }
                }
                supplemented.name_hints = hints;
                let input = discover_previous(registry, official, pages);
                match adapters::discover(official, &supplemented, ctx, &input) {
                    Ok(enhanced) => {
                        let prior_candidates: Vec<SourceCandidate> = match results
                            .iter()
                            .find(|(existing, _)| existing.id == official.id)
                        {
                            Some((_, prior)) => prior.candidates.clone(),
                            None => registry
                                .previous_record(official, game.appid)
                                .success
                                .map(|success| success.candidates)
                                .unwrap_or_default(),
                        };
                        let mut combined = prior_candidates;
                        combined.extend(enhanced.candidates.iter().cloned());
                        let merged = identity::merge_candidates(&combined);
                        let enriched = identity::enrich_candidates(&merged, &identities);
                        let mut outcome = enhanced;
                        outcome.candidates = enriched;
                        outcome.identities = identities.clone();
                        upsert_result(&mut results, official, outcome);
                    }
                    Err(error) => diagnostics.push(Diagnostic {
                        source_id: Some(official.id.clone()),
                        code: error.to_string(),
                        detail: None,
                    }),
                }
            }
        }
    } else {
        for rule in pending
            .iter()
            .filter(|rule| rule.role == SourceRole::FallbackIndexer)
        {
            let outcome = adapters::DiscoverOutcome {
                status: "skipped".to_string(),
                ..adapters::DiscoverOutcome::default()
            };
            upsert_result(&mut results, rule, outcome);
        }
    }

    // Confirmed shared upstream pages carry index provenance back onto the
    // publisher candidate (Python `_discovery_sync` §L750-781).
    link_index_provenance(&mut results, &official_rules, &fallback_rules, query.appid);

    for (rule, outcome) in &results {
        if cancel.load(Ordering::Relaxed) || crate::operation::check().is_err() {
            return Err(invalid("task_cancelled"));
        }
        let previous = registry.previous_record(rule, game.appid);
        let result = registry::DiscoveryOutcome {
            status: outcome.status.clone(),
            candidates: outcome.candidates.clone(),
            diagnostics: outcome.diagnostics.clone(),
            index: outcome.index.clone(),
            checkpoint: outcome.checkpoint.clone(),
            queries: None,
            query_cache: None,
        };
        match registry.commit_if_revision(rule, game.appid, result, &previous, &key, captured) {
            Ok(true) => {}
            Ok(false) => {
                stale = true;
                break;
            }
            Err(error) => diagnostics.push(Diagnostic {
                source_id: Some(rule.id.clone()),
                code: error.to_string(),
                detail: None,
            }),
        }
    }

    let mut discovery = registry.cached_discovery(game);
    if stale {
        discovery.status = "stale".to_string();
    } else {
        discovery.status = match discovery.status.as_str() {
            "cache_complete" => "complete",
            "cache_partial" => "partial",
            other => other,
        }
        .to_string();
    }
    discovery.diagnostics.extend(diagnostics);
    Ok(discovery)
}

/// Builds the `DiscoverPrevious` input: the fresh shared index plus any direct
/// upstream pages from a confirmed fallback candidate.
fn discover_previous(
    registry: &SourceRegistry,
    rule: &SourceRule,
    upstream_pages: Vec<String>,
) -> DiscoverPrevious {
    DiscoverPrevious {
        index: registry.shared_index(&rule.id),
        upstream_pages,
    }
}

/// Replaces an existing rule outcome in place (preserving result order) or
/// appends it, mirroring Python `results[results.index(prior)] = enhanced`.
fn upsert_result(
    results: &mut Vec<(SourceRule, adapters::DiscoverOutcome)>,
    rule: &SourceRule,
    outcome: adapters::DiscoverOutcome,
) {
    match results
        .iter()
        .position(|(existing, _)| existing.id == rule.id)
    {
        Some(position) => results[position].1 = outcome,
        None => results.push((rule.clone(), outcome)),
    }
}

/// §4.2 trusted query names: manifest name stays `name`; a non-empty
/// `localized_name` precedes frontend hints, deduped and capped at 3 × 256.
fn merged_name_hints(game: &GameQuery, name_hints: &[String]) -> Vec<String> {
    let mut hints: Vec<String> = Vec::new();
    if let Some(localized) = &game.localized_name {
        push_hint(&mut hints, localized);
    }
    for hint in name_hints {
        push_hint(&mut hints, hint);
    }
    hints
}

fn push_hint(hints: &mut Vec<String>, value: &str) {
    if hints.len() >= 3 {
        return;
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    let candidate: String = trimmed.chars().take(256).collect();
    if hints.iter().any(|existing| existing == &candidate) {
        return;
    }
    hints.push(candidate);
}

/// Port of the `_discovery_sync` provenance pass: an index candidate whose
/// confirmed upstream page canonicalizes to a publisher candidate's page can
/// donate its appid and link `related_index_pages`.
fn link_index_provenance(
    results: &mut [(SourceRule, adapters::DiscoverOutcome)],
    official_rules: &[SourceRule],
    fallback_rules: &[SourceRule],
    appid: u32,
) {
    let official_ids: HashSet<&str> = official_rules.iter().map(|rule| rule.id.as_str()).collect();
    let fallback_ids: HashSet<&str> = fallback_rules.iter().map(|rule| rule.id.as_str()).collect();
    let indexed_positions: Vec<(usize, usize)> = results
        .iter()
        .enumerate()
        .filter(|(_, (rule, _))| fallback_ids.contains(rule.id.as_str()))
        .flat_map(|(result_index, (_, outcome))| {
            (0..outcome.candidates.len())
                .map(move |candidate_index| (result_index, candidate_index))
        })
        .collect();
    let official_positions: Vec<(usize, usize)> = results
        .iter()
        .enumerate()
        .filter(|(_, (rule, _))| official_ids.contains(rule.id.as_str()))
        .flat_map(|(result_index, (_, outcome))| {
            (0..outcome.candidates.len())
                .map(move |candidate_index| (result_index, candidate_index))
        })
        .collect();

    for &(fallback_index, fallback_candidate) in &indexed_positions {
        let indexed = results[fallback_index].1.candidates[fallback_candidate].clone();
        let Some(upstream) = indexed.upstream_page.clone() else {
            continue;
        };
        if indexed.game_appid != Some(appid)
            || !matches!(
                indexed.match_basis,
                MatchBasis::Appid | MatchBasis::VerifiedAlias | MatchBasis::OfficialTitle
            )
            || !matches!(indexed.variant, Variant::Base | Variant::Dlc)
        {
            continue;
        }
        let upstream_canonical = canonical::canonical_url(&upstream);
        let mut indexed_conflict = false;
        for &(official_index, official_candidate) in &official_positions {
            let (
                source_page,
                variant,
                version_raw,
                languages,
                requires_dlc,
                excludes_dlc,
                patch_kind,
            ) = {
                let official = &results[official_index].1.candidates[official_candidate];
                (
                    official.source_page.clone(),
                    official.variant,
                    official.version_raw.clone(),
                    official.languages.clone(),
                    official.requires_dlc.clone(),
                    official.excludes_dlc.clone(),
                    official.patch_kind.clone(),
                )
            };
            if canonical::canonical_url(&source_page) != upstream_canonical
                || variant != indexed.variant
            {
                continue;
            }
            let official_languages: HashSet<&String> = languages.iter().collect();
            let indexed_languages: HashSet<&String> = indexed.languages.iter().collect();
            let same_version = version_raw.as_deref().is_some_and(|version| {
                !version.is_empty() && Some(version) == indexed.version_raw.as_deref()
            }) && !official_languages.is_empty()
                && official_languages == indexed_languages;
            let relation = Provenance {
                source_id: indexed.source_id.clone(),
                page: indexed.source_page.clone(),
            };
            let official = &mut results[official_index].1.candidates[official_candidate];
            if official.game_appid.is_none() {
                official.game_appid = indexed.game_appid;
                official.parent_appid = if official.variant == Variant::Dlc {
                    indexed.game_appid
                } else {
                    None
                };
                official.match_basis = MatchBasis::OfficialTitle;
                official.identity_evidence.push(IdentityEvidence {
                    kind: "indexed_upstream_page".to_string(),
                    appid: indexed.game_appid,
                    page: Some(indexed.source_page.clone()),
                });
            }
            if official.game_appid != indexed.game_appid {
                continue;
            }
            if !official.related_index_pages.contains(&relation) {
                official.related_index_pages.push(relation.clone());
            }
            if same_version
                && indexed.format_reported == ArtifactFormat::Zip
                && official.format_observed == ArtifactFormat::Exe
            {
                indexed_conflict = true;
                if !official
                    .warnings
                    .iter()
                    .any(|warning| warning == "artifact_format_conflict")
                {
                    official
                        .warnings
                        .push("artifact_format_conflict".to_string());
                }
            }
            if same_version
                && requires_dlc.is_some()
                && requires_dlc == indexed.requires_dlc
                && excludes_dlc.is_some()
                && excludes_dlc == indexed.excludes_dlc
                && patch_kind == indexed.patch_kind
                && !official.provenance.contains(&relation)
            {
                official.provenance.push(relation);
            }
        }
        if indexed_conflict {
            let indexed = &mut results[fallback_index].1.candidates[fallback_candidate];
            indexed.format_observed = ArtifactFormat::Exe;
            if !indexed
                .warnings
                .iter()
                .any(|warning| warning == "artifact_format_conflict")
            {
                indexed
                    .warnings
                    .push("artifact_format_conflict".to_string());
            }
        }
    }
}

/// `scan_sources`: dedupe by AppID (same game across instances is merged),
/// discover each game with a fresh context, and report progress. A cancelled
/// scan stops early and returns the results already produced.
pub fn scan_sources(
    registry: &mut SourceRegistry,
    mut make_ctx: impl FnMut() -> Result<RequestContext>,
    games: &[GameQuery],
    force: bool,
    cancel: &AtomicBool,
    mut on_event: impl FnMut(&ScanEvent),
) -> Result<Vec<GameDiscovery>> {
    let mut seen: HashSet<u32> = HashSet::new();
    let unique: Vec<GameQuery> = games
        .iter()
        .filter(|game| seen.insert(game.appid))
        .cloned()
        .collect();
    let total = unique.len();
    let mut results = Vec::new();
    on_event(&ScanEvent::Progress {
        completed: 0,
        total,
        running: true,
    });
    for (index, game) in unique.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        crate::operation::check()?;
        let mut ctx = make_ctx()?;
        let discovery = match discover_game(registry, &mut ctx, game, &[], None, force, cancel) {
            Ok(discovery) => discovery,
            Err(error) => GameDiscovery {
                status: "failed".to_string(),
                candidates: Vec::new(),
                diagnostics: vec![Diagnostic {
                    source_id: None,
                    code: "source_refresh_failed".to_string(),
                    detail: Some(error.to_string()),
                }],
            },
        };
        on_event(&ScanEvent::Update {
            appid: game.appid,
            index,
            discovery: discovery.clone(),
        });
        results.push(discovery);
        on_event(&ScanEvent::Progress {
            completed: index + 1,
            total,
            running: true,
        });
    }
    on_event(&ScanEvent::Progress {
        completed: results.len(),
        total,
        running: false,
    });
    Ok(results)
}

/// `refresh_sources`: refresh every enabled non-discovery ZIP source.
pub fn refresh_sources(
    registry: &mut SourceRegistry,
    ctx: &mut RequestContext,
    force: bool,
    cancel: &AtomicBool,
) -> Result<Vec<super::model::SourceStatus>> {
    let rules: Vec<SourceRule> = registry
        .rules()
        .iter()
        .filter(|rule| !rule.discovery_only && rule.enabled)
        .cloned()
        .collect();
    let mut statuses = Vec::new();
    for rule in &rules {
        if cancel.load(Ordering::Relaxed) || crate::operation::check().is_err() {
            return Err(invalid("task_cancelled"));
        }
        statuses.push(registry.refresh_source(rule, force, ctx, cancel)?);
    }
    Ok(statuses)
}

/// `get_source_policy`: current effective rule + origin + fingerprint.
pub fn source_policy(registry: &SourceRegistry, source_id: &str) -> Result<SourcePolicy> {
    let rule = registry
        .rules()
        .iter()
        .find(|rule| rule.id == source_id)
        .cloned()
        .ok_or_else(|| invalid("unknown_source"))?;
    let origin = registry
        .source_origin(source_id)
        .unwrap_or("added")
        .to_string();
    let fingerprint = rule_fingerprint(&rule);
    Ok(SourcePolicy {
        rule,
        origin,
        fingerprint,
    })
}

/// `resolve_source_record`: map a legacy release/record id to the current source
/// and its domain rules. Never guesses on ambiguity.
pub fn resolve_source_record(
    registry: &SourceRegistry,
    instance_id: &str,
    record_id: &str,
) -> Result<ResolvedSource> {
    let matches: Vec<SourceRule> = registry
        .rules()
        .iter()
        .filter(|rule| !rule.discovery_only && release_prefix_match(rule, record_id))
        .cloned()
        .collect();
    match matches.as_slice() {
        [] => Err(invalid("not_found")),
        [rule] => Ok(ResolvedSource {
            instance_id: instance_id.to_string(),
            record_id: record_id.to_string(),
            release_id: Some(record_id.to_string()),
            source_id: rule.id.clone(),
            root_layout: rule.root_layout,
            page_hosts: rule.page_hosts.clone(),
            download_hosts: rule.download_hosts.clone(),
        }),
        _ => Err(invalid("ambiguous_source")),
    }
}

fn query_key(query: &QueryContext) -> String {
    let mut map = serde_json::Map::new();
    map.insert("appid".to_string(), serde_json::Value::from(query.appid));
    map.insert(
        "name".to_string(),
        serde_json::Value::String(query.name.clone()),
    );
    map.insert(
        "name_hints".to_string(),
        serde_json::Value::Array(
            query
                .name_hints
                .iter()
                .cloned()
                .map(serde_json::Value::String)
                .collect(),
        ),
    );
    map.insert(
        "preferred_language".to_string(),
        match &query.preferred_language {
            Some(language) => serde_json::Value::String(language.clone()),
            None => serde_json::Value::Null,
        },
    );
    map.insert(
        "legacy_match".to_string(),
        serde_json::Value::Bool(query.legacy_match),
    );
    map.insert(
        "source_context".to_string(),
        serde_json::Value::Array(
            query
                .source_context
                .iter()
                .cloned()
                .map(serde_json::Value::String)
                .collect(),
        ),
    );
    map.insert(
        "parser_version".to_string(),
        serde_json::Value::from(query.parser_version),
    );
    map.insert(
        "query_version".to_string(),
        serde_json::Value::from(query.query_version),
    );
    canonical::query_key(&serde_json::Value::Object(map))
}

fn rule_fingerprint(rule: &SourceRule) -> String {
    let value = serde_json::to_value(rule).unwrap_or(serde_json::Value::Null);
    let json = canonical::python_json(&value, true, false);
    format!("{:x}", Sha256::digest(json.as_bytes()))
}

fn release_prefix_match(rule: &SourceRule, release_id: &str) -> bool {
    let prefix = if rule.id == "patchmoe" {
        "patchmoe-".to_string()
    } else {
        format!("source-{}-", rule.id)
    };
    match release_id.strip_prefix(&prefix) {
        Some(rest) => valid_release_suffix(rest),
        None => false,
    }
}

fn valid_release_suffix(value: &str) -> bool {
    !value.is_empty() && !value.starts_with('0') && value.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::http::{FakeFetcher, FetchResponse, RequestContext};
    use crate::sources::model::{Columns, DiscoveryRecord, RulesConfig, SourceRole};
    use crate::sources::registry::SourceRegistry;
    use crate::sources::store::SourceStore;
    use crate::sources::{validate, FakeClock};
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    fn kagura_rule() -> SourceRule {
        SourceRule {
            id: "kagura-cn".to_string(),
            name: "Kagura".to_string(),
            adapter: "kagura_wordpress".to_string(),
            role: SourceRole::Publisher,
            discovery_only: true,
            priority: 10,
            enabled: true,
            pages: vec!["https://kagurafan.com/".to_string()],
            page_hosts: vec!["kagurafan.com".to_string()],
            download_hosts: Vec::new(),
            root_layout: None,
            columns: None,
            table_id: None,
            language: None,
            url: None,
        }
    }

    fn hgamer_rule() -> SourceRule {
        SourceRule {
            id: "hgamer".to_string(),
            name: "HGamer".to_string(),
            adapter: "hgamer_html".to_string(),
            role: SourceRole::FallbackIndexer,
            discovery_only: true,
            priority: 100,
            enabled: true,
            pages: vec!["https://hgamer.tw/zh-CN/patches".to_string()],
            page_hosts: vec!["hgamer.tw".to_string()],
            download_hosts: Vec::new(),
            root_layout: None,
            columns: None,
            table_id: None,
            language: None,
            url: None,
        }
    }

    fn table_rule() -> SourceRule {
        SourceRule {
            id: "example".to_string(),
            name: "Example".to_string(),
            adapter: "table_html".to_string(),
            role: SourceRole::Publisher,
            discovery_only: false,
            priority: 100,
            enabled: true,
            pages: vec!["https://example.com/list".to_string()],
            page_hosts: vec!["example.com".to_string()],
            download_hosts: vec!["download.example.com".to_string()],
            root_layout: Some(RootLayout::GameRoot),
            columns: Some(Columns {
                title: 0,
                download: 1,
                date: None,
                language: None,
                appid: None,
            }),
            table_id: None,
            language: None,
            url: None,
        }
    }

    fn default_rules() -> Vec<SourceRule> {
        let config: RulesConfig = serde_json::from_str(crate::sources::DEFAULT_RULES_JSON).unwrap();
        config
            .sources
            .iter()
            .map(|raw| validate::validate_rule(raw).unwrap())
            .collect()
    }

    fn registry_at(temp: &Path, rules: Vec<SourceRule>, now: u64) -> SourceRegistry {
        let store = SourceStore::open(temp, &rules).unwrap();
        SourceRegistry::load_with_clock(store, rules, Box::new(FakeClock::new(now))).unwrap()
    }

    fn context(fake: &Arc<FakeFetcher>, budget: u32) -> RequestContext {
        RequestContext::with_settings(
            Box::new(Arc::clone(fake)),
            budget,
            Duration::from_secs(30),
            Duration::ZERO,
        )
    }

    fn empty_index() -> FetchResponse {
        FetchResponse {
            status: 200,
            body: b"<html><body></body></html>".to_vec(),
            final_url: "https://kagurafan.com/".to_string(),
            ..FetchResponse::default()
        }
    }

    fn game(appid: u32, name: &str) -> GameQuery {
        GameQuery {
            appid,
            name: name.to_string(),
            localized_name: None,
            instance_id: None,
        }
    }

    fn complete_record() -> registry::DiscoveryOutcome {
        registry::DiscoveryOutcome {
            status: "complete".to_string(),
            ..registry::DiscoveryOutcome::default()
        }
    }

    /// Reconstructs the session query key so a seeded cache is treated as fresh.
    fn session_query(registry: &SourceRegistry, appid: u32, name: &str) -> String {
        let source_context = registry
            .effective_discovery_rules(appid)
            .iter()
            .map(|rule| rule.id.clone())
            .collect();
        let query = QueryContext {
            appid,
            name: name.to_string(),
            name_hints: Vec::new(),
            preferred_language: None,
            legacy_match: false,
            force: false,
            source_context,
            parser_version: PARSER_VERSION,
            query_version: QUERY_VERSION,
            settings_revision: registry.settings_revision(),
            generation: None,
        };
        super::query_key(&query)
    }

    #[test]
    fn fresh_cache_short_circuits_network() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), vec![kagura_rule()], 1000);
        let rule = registry.rules()[0].clone();
        let key = session_query(&registry, 42, "Game");
        registry
            .commit(
                &rule,
                42,
                complete_record(),
                &DiscoveryRecord::default(),
                &key,
            )
            .unwrap();

        let fake = Arc::new(FakeFetcher::new());
        let mut ctx = context(&fake, 24);
        let discovery = discover_game(
            &mut registry,
            &mut ctx,
            &game(42, "Game"),
            &[],
            None,
            false,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(fake.call_count(), 0);
        assert_eq!(discovery.status, "cache_complete");
    }

    #[test]
    fn force_bypasses_fresh_cache() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), vec![kagura_rule()], 1000);
        let rule = registry.rules()[0].clone();
        let key = session_query(&registry, 42, "Game");
        registry
            .commit(
                &rule,
                42,
                complete_record(),
                &DiscoveryRecord::default(),
                &key,
            )
            .unwrap();

        let fake = Arc::new(FakeFetcher::new());
        fake.push(empty_index());
        let mut ctx = context(&fake, 24);
        let discovery = discover_game(
            &mut registry,
            &mut ctx,
            &game(42, "Game"),
            &[],
            None,
            true,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(fake.call_count() >= 1);
        assert_ne!(discovery.status, "cache_complete");
    }

    #[test]
    fn budget_is_shared_across_rules() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), vec![kagura_rule(), hgamer_rule()], 1000);
        let fake = Arc::new(FakeFetcher::new());
        fake.push(empty_index());
        let mut ctx = context(&fake, 1);
        let discovery = discover_game(
            &mut registry,
            &mut ctx,
            &game(42, "Game"),
            &[],
            None,
            false,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(discovery
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "discovery_budget_exhausted"));
    }

    #[test]
    fn localized_name_reaches_discovery_query() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), vec![hgamer_rule()], 1000);
        let fake = Arc::new(FakeFetcher::new());
        let mut ctx = context(&fake, 24);
        let mut game = game(42, "Game");
        game.localized_name = Some("LocalizedName".to_string());
        discover_game(
            &mut registry,
            &mut ctx,
            &game,
            &[],
            None,
            true,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(
            fake.calls()
                .iter()
                .any(|call| call.url.contains("q=LocalizedName")),
            "localized_name must appear in the discovery search query"
        );
    }

    #[test]
    fn stale_revision_never_publishes() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), vec![kagura_rule()], 1000);
        let rule = registry.rules()[0].clone();
        let captured = registry.settings_revision();
        registry.store().bump_revision().unwrap();
        let published = registry
            .commit_if_revision(
                &rule,
                42,
                complete_record(),
                &DiscoveryRecord::default(),
                "k",
                captured,
            )
            .unwrap();
        assert!(!published);
        assert!(registry
            .store()
            .load_discovery(&rule.id, 42)
            .unwrap()
            .is_none());
    }

    #[test]
    fn cancellation_aborts_discovery() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), vec![kagura_rule()], 1000);
        let fake = Arc::new(FakeFetcher::new());
        let mut ctx = context(&fake, 24);
        let error = discover_game(
            &mut registry,
            &mut ctx,
            &game(42, "Game"),
            &[],
            None,
            false,
            &AtomicBool::new(true),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "task_cancelled");
    }

    #[test]
    fn batch_dedupes_appid_and_reports_progress() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), vec![kagura_rule()], 1000);
        let fake = Arc::new(FakeFetcher::new());
        fake.push(empty_index());
        fake.push(empty_index());
        let games = vec![game(42, "Game"), game(42, "Game"), game(43, "Other")];
        let mut events = Vec::new();
        let results = scan_sources(
            &mut registry,
            || Ok(context(&fake, 24)),
            &games,
            false,
            &AtomicBool::new(false),
            |event| events.push(event.clone()),
        )
        .unwrap();
        assert_eq!(results.len(), 2);
        assert!(events.iter().any(|event| matches!(
            event,
            ScanEvent::Progress {
                completed: 0,
                total: 2,
                running: true
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ScanEvent::Progress {
                completed: 2,
                total: 2,
                running: false
            }
        )));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, ScanEvent::Update { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn refresh_failure_preserves_cache() {
        let temp = tempfile::tempdir().unwrap();
        let rule = table_rule();
        let mut registry = registry_at(temp.path(), vec![rule.clone()], 1000);
        let fake = Arc::new(FakeFetcher::new());
        fake.push(FetchResponse {
            status: 200,
            body: b"<table><tr><td>Game</td><td><a href=\"https://download.example.com/g.zip\">dl</a></td></tr></table>".to_vec(),
            final_url: "https://example.com/list".to_string(),
            ..FetchResponse::default()
        });
        let mut ctx = context(&fake, 24);
        let status = registry
            .refresh_source(&rule, true, &mut ctx, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(status.code, "source_refreshed");
        assert_eq!(status.entry_count, 1);

        let failing = Arc::new(FakeFetcher::new());
        for _ in 0..2 {
            let mut response = FetchResponse {
                status: 503,
                ..FetchResponse::default()
            };
            response
                .headers
                .push(("Retry-After".to_string(), "0".to_string()));
            failing.push(response);
        }
        let mut ctx = context(&failing, 24);
        let status = registry
            .refresh_source(&rule, true, &mut ctx, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(status.code, "source_offline_cached");
        assert_eq!(status.entry_count, 1);

        let summary = registry
            .list_sources()
            .into_iter()
            .find(|source| source.rule.id == "example")
            .unwrap();
        assert_eq!(summary.entry_count, 1);
        // Cache bytes are preserved on failure.
        assert!(registry.store().load_cache("example").unwrap().is_some());
    }

    #[test]
    fn capabilities_shape() {
        let capabilities = capabilities();
        assert!(capabilities
            .methods
            .iter()
            .any(|method| method == "discover_game"));
        assert_eq!(capabilities.discovery_schema_version, 1);
        assert_eq!(capabilities.adapter_version, 1);
        assert_eq!(capabilities.parser_version, PARSER_VERSION);
        assert_eq!(capabilities.query_version, QUERY_VERSION);
    }

    #[test]
    fn source_policy_origin_and_fingerprint() {
        let temp = tempfile::tempdir().unwrap();
        let registry = registry_at(temp.path(), default_rules(), 1000);
        let policy = source_policy(&registry, "patchmoe").unwrap();
        assert_eq!(policy.origin, "builtin");
        assert_eq!(policy.fingerprint.len(), 64);
        assert_eq!(policy.rule.id, "patchmoe");
        assert!(source_policy(&registry, "missing").is_err());
    }

    #[test]
    fn resolve_source_record_maps_release() {
        let temp = tempfile::tempdir().unwrap();
        let registry = registry_at(temp.path(), default_rules(), 1000);
        let resolved = resolve_source_record(&registry, "inst", "patchmoe-42").unwrap();
        assert_eq!(resolved.source_id, "patchmoe");
        assert_eq!(resolved.release_id.as_deref(), Some("patchmoe-42"));
        assert!(resolve_source_record(&registry, "inst", "unknown-42").is_err());
    }
}
