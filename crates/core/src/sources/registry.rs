//! In-memory merged rules, subscriptions and source-cache matching (design
//! §2.2, §4.4, §5, R6/R8/R9/R15).
//!
//! The registry owns the [`SourceStore`]; read paths never create state, and
//! mutations persist through the store's short lock + revision primitive.

use super::adapters;
use super::canonical;
use super::http::RequestContext;
use super::model::*;
use super::store::SourceStore;
use super::validate;
use super::{
    invalid, Clock, Result, SystemClock, BUILTIN_ADAPTERS, MAX_DISCOVERY_OUTPUT, MAX_SOURCES,
    PARSER_VERSION, QUERY_VERSION, SOURCES_ENCODING_VERSION,
};
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::AtomicBool;

/// One discovery outcome fed to [`SourceRegistry::commit`] (design §4.4).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiscoveryOutcome {
    pub status: String,
    pub candidates: Vec<SourceCandidate>,
    pub diagnostics: Vec<Diagnostic>,
    pub index: Option<DiscoveryIndex>,
    pub checkpoint: Option<serde_json::Value>,
    pub queries: Option<serde_json::Value>,
    pub query_cache: Option<serde_json::Value>,
}

/// True while `checked_at` is within `ttl`; a clock rollback expires
/// conservatively (design §5.3, R11).
pub fn is_fresh(checked_at: Option<u64>, ttl: u64, now: u64) -> bool {
    checked_at.is_some_and(|checked| checked <= now && now - checked < ttl)
}

fn empty_success() -> DiscoverySuccess {
    DiscoverySuccess {
        status: String::new(),
        candidates: Vec::new(),
        diagnostics: Vec::new(),
        checked_at: 0,
        cached_at: None,
        query_key: String::new(),
        parser_version: 0,
        query_version: 0,
        encoding_version: SOURCES_ENCODING_VERSION,
    }
}

fn matched_basis(basis: MatchBasis) -> bool {
    matches!(
        basis,
        MatchBasis::Appid | MatchBasis::VerifiedAlias | MatchBasis::OfficialTitle
    )
}

fn valid_language_code(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    match parts.as_slice() {
        [primary] => {
            (2..=3).contains(&primary.len()) && primary.bytes().all(|b| b.is_ascii_alphabetic())
        }
        [primary, region] => {
            (2..=3).contains(&primary.len())
                && primary.bytes().all(|b| b.is_ascii_alphabetic())
                && (2..=4).contains(&region.len())
                && region.bytes().all(|b| b.is_ascii_alphabetic())
        }
        _ => false,
    }
}

pub struct SourceRegistry {
    store: SourceStore,
    builtins: Vec<SourceRule>,
    rules: Vec<SourceRule>,
    overrides: Overrides,
    revision: u64,
    cache: BTreeMap<String, SourceCache>,
    status: BTreeMap<String, SourceStatus>,
    diagnostics: Vec<Diagnostic>,
    discovery: BTreeMap<(String, u32), DiscoveryRecord>,
    indexes: BTreeMap<String, DiscoveryIndexRecord>,
    clock: Box<dyn Clock>,
    frozen_snapshot: bool,
}

impl SourceRegistry {
    pub fn load(store: SourceStore, builtins: Vec<SourceRule>) -> Result<Self> {
        Self::load_with_clock(store, builtins, Box::new(SystemClock))
    }

    pub fn load_with_clock(
        store: SourceStore,
        builtins: Vec<SourceRule>,
        clock: Box<dyn Clock>,
    ) -> Result<Self> {
        let mut registry = Self {
            store,
            builtins,
            rules: Vec::new(),
            overrides: Overrides::default(),
            revision: 0,
            cache: BTreeMap::new(),
            status: BTreeMap::new(),
            diagnostics: Vec::new(),
            discovery: BTreeMap::new(),
            indexes: BTreeMap::new(),
            clock,
            frozen_snapshot: false,
        };
        registry.reload()?;
        Ok(registry)
    }

    /// Preloads only the requested games' discovery records for an immutable snapshot.
    /// Every present record is parsed and validated here so later registry reads never
    /// need to touch disk and corrupt state cannot silently become an empty discovery.
    pub(crate) fn preload_snapshot_games(&mut self, games: &[GameQuery]) -> Result<()> {
        let appids: HashSet<u32> = games.iter().map(|game| game.appid).collect();
        for appid in appids {
            for rule in self.effective_discovery_rules(appid) {
                let record = self.store.load_discovery(&rule.id, appid)?;
                if let Some(record) = &record {
                    if record.schema_version != 1
                        || record.appid != appid
                        || record.source_id != rule.id
                    {
                        return Err(invalid("invalid_discovery_record"));
                    }
                    if record.success.as_ref().is_some_and(|success| {
                        validate::validate_candidates(&rule, appid, &success.candidates).is_err()
                    }) || record.attempt.as_ref().is_some_and(|attempt| {
                        validate::validate_candidates(&rule, appid, &attempt.candidates).is_err()
                    }) {
                        return Err(invalid("invalid_discovery_record"));
                    }
                }
                // Store explicit absence too, preventing the later cached-discovery
                // path from probing this game on disk again.
                self.discovery
                    .insert((rule.id.clone(), appid), record.unwrap_or_default());
            }
        }
        for rule in &self.rules {
            if let Some(index) = self.store.load_discovery_index(&rule.id)? {
                if index.schema_version != 1 {
                    return Err(invalid("invalid_discovery_index"));
                }
                self.indexes.insert(rule.id.clone(), index);
            }
        }
        self.frozen_snapshot = true;
        Ok(())
    }
    pub fn store(&self) -> &SourceStore {
        &self.store
    }

    pub fn settings_revision(&self) -> u64 {
        self.revision
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn rules(&self) -> &[SourceRule] {
        &self.rules
    }

    fn now(&self) -> u64 {
        self.clock.now_unix()
    }

    /// Clock reading for consumers that need the registry's notion of "now"
    /// (e.g. dynamic release-date checks in `installer::resolve`).
    pub fn now_unix(&self) -> u64 {
        self.clock.now_unix()
    }

    /// Port of `_reload` merge (design §2.2): user `added` shadows builtins with
    /// the same id, the `enabled` overlay is applied, and the result is capped.
    fn reload(&mut self) -> Result<()> {
        let settings = self.store.settings();
        self.revision = settings.revision;
        let overrides = Overrides {
            enabled: settings.enabled.clone(),
            added: settings.added.clone(),
            games: settings.games.clone(),
        };
        validate::validate_overrides(&overrides)?;

        let added_ids: HashSet<&str> = overrides
            .added
            .iter()
            .map(|rule| rule.id.as_str())
            .collect();
        let mut rules: Vec<SourceRule> = self
            .builtins
            .iter()
            .filter(|rule| !added_ids.contains(rule.id.as_str()))
            .cloned()
            .collect();
        rules.extend(overrides.added.iter().cloned());
        if rules.len() > MAX_SOURCES {
            return Err(invalid("invalid_source_overrides"));
        }
        let mut seen = HashSet::new();
        for rule in &rules {
            if !seen.insert(rule.id.clone()) {
                return Err(invalid("invalid_source_overrides"));
            }
        }
        for rule in &mut rules {
            if let Some(enabled) = overrides.enabled.get(&rule.id) {
                rule.enabled = *enabled;
            }
        }

        self.rules = rules;
        self.overrides = overrides;
        self.cache.clear();
        self.status.clear();
        self.discovery.clear();
        self.indexes.clear();
        self.diagnostics.clear();

        for rule in &self.rules {
            if let Ok(Some(status)) = self.store.load_status(&rule.id) {
                self.status.insert(rule.id.clone(), status);
            }
            if rule.discovery_only {
                continue;
            }
            match self.store.load_cache(&rule.id) {
                Ok(Some(cache)) => match validate::validate_entries(rule, &cache.entries) {
                    Ok(entries) => {
                        let clean = SourceCache {
                            entries,
                            etag: cache.etag,
                            modified: cache.modified,
                            cached_at: cache.cached_at,
                            checked_at: cache.checked_at,
                        };
                        self.cache.insert(rule.id.clone(), clean);
                    }
                    Err(_) => self.diagnostics.push(cache_diagnostic(&rule.id)),
                },
                Ok(None) => {}
                Err(_) => self.diagnostics.push(cache_diagnostic(&rule.id)),
            }
        }
        Ok(())
    }

    pub fn list_sources(&self) -> Vec<SourceSummary> {
        let diagnostic_codes: BTreeMap<&str, &str> = self
            .diagnostics
            .iter()
            .filter_map(|diagnostic| {
                diagnostic
                    .source_id
                    .as_deref()
                    .map(|id| (id, diagnostic.code.as_str()))
            })
            .collect();
        self.rules
            .iter()
            .map(|rule| {
                let status = self.status.get(&rule.id);
                let cache = self.cache.get(&rule.id);
                let code = diagnostic_codes
                    .get(rule.id.as_str())
                    .map(|code| (*code).to_string())
                    .or_else(|| status.map(|status| status.code.clone()))
                    .unwrap_or_else(|| {
                        if cache.is_some() {
                            "cached".to_string()
                        } else {
                            "not_refreshed".to_string()
                        }
                    });
                let entry_count = if rule.discovery_only {
                    status.map(|status| status.entry_count).unwrap_or(0)
                } else {
                    cache.map(|cache| cache.entries.len() as u64).unwrap_or(0)
                };
                let cached_at = if rule.discovery_only {
                    status.and_then(|status| status.cached_at)
                } else {
                    cache.and_then(|cache| cache.cached_at)
                };
                let checked_at = status
                    .map(|status| status.checked_at)
                    .or_else(|| cache.and_then(|cache| cache.checked_at));
                SourceSummary {
                    rule: rule.clone(),
                    origin: self.source_origin(&rule.id).unwrap_or("added").to_string(),
                    status: code,
                    entry_count,
                    cached_at,
                    checked_at,
                }
            })
            .collect()
    }

    pub fn list_game_sources(&self, game: &GameQuery) -> Vec<GameSourceSummary> {
        let mut matched: HashSet<String> = self
            .matches(game, true)
            .into_iter()
            .map(|entry| entry.source_id)
            .collect();
        for rule in &self.rules {
            if !rule.discovery_only {
                continue;
            }
            let record = self.discovery_record(rule, game.appid);
            let found = record
                .success
                .iter()
                .flat_map(|success| success.candidates.iter())
                .chain(
                    record
                        .attempt
                        .iter()
                        .flat_map(|attempt| attempt.candidates.iter()),
                )
                .any(|candidate| matched_basis(candidate.match_basis));
            if found {
                matched.insert(rule.id.clone());
            }
        }
        self.list_sources()
            .into_iter()
            .map(|source| {
                let subscribed = self.game_subscribed(game.appid, &source.rule.id);
                let matched = matched.contains(&source.rule.id);
                GameSourceSummary {
                    source,
                    subscribed,
                    matched,
                }
            })
            .collect()
    }

    /// Port of `matches()` (design §2.2/§5.2).
    ///
    /// Mirrors Python: only a non-positive AppID short-circuits. An empty name is
    /// accepted and still matches entries carrying the AppID, which lets
    /// `list_source_matches` resolve `appid` alone.
    pub fn matches(&self, game: &GameQuery, include_disabled: bool) -> Vec<SourceMatch> {
        if game.appid == 0 {
            return Vec::new();
        }
        let name = validate::normal(&game.name);
        let mut result = Vec::new();
        for rule in &self.rules {
            if rule.discovery_only {
                continue;
            }
            if !include_disabled && (!rule.enabled || !self.game_subscribed(game.appid, &rule.id)) {
                continue;
            }
            let Some(cache) = self.cache.get(&rule.id) else {
                continue;
            };
            let mut candidates: Vec<&SourceEntry> = Vec::new();
            for entry in &cache.entries {
                let names: HashSet<String> = std::iter::once(&entry.title)
                    .chain(entry.aliases.iter())
                    .flat_map(|title| validate::split_names(title))
                    .map(|title| validate::normal(&title))
                    .collect();
                if entry.appid == Some(game.appid)
                    || (entry.appid.is_none() && !name.is_empty() && names.contains(&name))
                {
                    candidates.push(entry);
                }
            }
            if candidates.is_empty() {
                continue;
            }
            let urls: HashSet<&str> = candidates.iter().map(|entry| entry.url.as_str()).collect();
            if urls.len() != 1 {
                continue;
            }
            let selected = candidates
                .iter()
                .find(|entry| matches!(entry.language.to_lowercase().as_str(), "zh-cn" | "zh_cn"))
                .copied()
                .unwrap_or(candidates[0]);
            let release_id = if rule.id == "patchmoe" {
                format!("patchmoe-{}", game.appid)
            } else {
                format!("source-{}-{}", rule.id, game.appid)
            };
            result.push(SourceMatch {
                title: selected.title.clone(),
                aliases: selected.aliases.clone(),
                url: selected.url.clone(),
                source_page: selected.source_page.clone(),
                language: selected.language.clone(),
                date: selected.date.clone(),
                appid: selected.appid,
                source_id: rule.id.clone(),
                release_id,
                root_layout: rule.root_layout,
            });
        }
        result
    }

    /// Deterministically projects the validated legacy index matches
    /// (`matches(game, false)`) into the dynamic ZIP [`SourceCandidate`] view
    /// (P4 B2 ruling).
    ///
    /// patchmoe carries no `discovery_only`, so its validated index entries never
    /// reached the dynamic ZIP pipeline; this pure projection makes them
    /// first-class ZIP candidates while keeping `discovery_only` semantics
    /// untouched. Identical cache state yields identical ids/fingerprints:
    /// `observed_at` derives only from the source's cached `checked_at`, and
    /// `candidate_id`/`fingerprint` follow the frozen canonical encoding.
    pub fn index_candidates(&self, game: &GameQuery) -> Vec<SourceCandidate> {
        let mut result = Vec::new();
        for source_match in self.matches(game, false) {
            // A "COMING SOON" entry has no artifact yet: never project it.
            if source_match.date.as_deref() == Some("COMING SOON") {
                continue;
            }
            let language = source_match.language.clone();
            let version_identity = "page-unknown".to_string();
            let identity = serde_json::Value::Array(vec![
                serde_json::Value::String(source_match.source_id.clone()),
                serde_json::Value::String(canonical::canonical_url(&source_match.source_page)),
                serde_json::Value::String("base".to_string()),
                serde_json::Value::Array(vec![serde_json::Value::String(language.clone())]),
                serde_json::Value::String(version_identity.clone()),
            ]);
            let candidate_id = canonical::candidate_id(&identity);
            let observed_at = iso8601_utc(
                self.cache
                    .get(&source_match.source_id)
                    .and_then(|cache| cache.checked_at)
                    .unwrap_or(0),
            );
            let mut candidate = SourceCandidate {
                candidate_id,
                source_id: source_match.source_id.clone(),
                source_record_id: canonical::canonical_url(&source_match.source_page),
                game_appid: Some(source_match.appid.unwrap_or(game.appid)),
                parent_appid: None,
                aliases: source_match.aliases.clone(),
                game_title: source_match.title.clone(),
                patch_title: source_match.title.clone(),
                variant: Variant::Base,
                requires_dlc: None,
                excludes_dlc: None,
                platforms: vec!["steam".to_string()],
                patch_kind: "adult_content".to_string(),
                languages: vec![language],
                version_raw: None,
                version_evidence: Vec::new(),
                version_identity,
                published_at: None,
                patch_updated_at: None,
                source_page: source_match.source_page.clone(),
                upstream_page: None,
                mirrors: vec![Mirror {
                    url: source_match.url.clone(),
                    label: None,
                    format: "zip".to_string(),
                }],
                format_reported: ArtifactFormat::Zip,
                format_observed: ArtifactFormat::Zip,
                capability: Capability::DiscoverOnly,
                identity_evidence: Vec::new(),
                provenance: Vec::new(),
                match_basis: if source_match.appid.is_some() {
                    MatchBasis::Appid
                } else {
                    MatchBasis::Title
                },
                observed_at,
                cache_status: CacheStatus::Cached,
                warnings: Vec::new(),
                fingerprint: String::new(),
                related_index_pages: Vec::new(),
            };
            if let Ok(value) = serde_json::to_value(&candidate) {
                candidate.fingerprint = canonical::fingerprint(&value);
            }
            result.push(candidate);
        }
        result
    }

    /// The dynamic ZIP candidate view: discovery-derived candidates
    /// ([`Self::cached_candidates`]) plus the validated index projection
    /// ([`Self::index_candidates`], P4 B2), deduplicated by `candidate_id`.
    ///
    /// Discovery candidates win a collision: they carry live freshness (their
    /// `cache_status` reflects schema compatibility) and richer evidence,
    /// while the index projection is a derived view of the same entries.
    pub fn dynamic_zip_candidates(&self, game: &GameQuery) -> Vec<SourceCandidate> {
        let mut candidates = self.cached_candidates(game);
        let mut seen: HashSet<String> = candidates
            .iter()
            .map(|candidate| candidate.candidate_id.clone())
            .collect();
        for candidate in self.index_candidates(game) {
            if seen.insert(candidate.candidate_id.clone()) {
                candidates.push(candidate);
            }
        }
        candidates
    }

    pub fn game_subscribed(&self, appid: u32, source_id: &str) -> bool {
        self.overrides
            .games
            .get(&appid.to_string())
            .and_then(|choices| choices.get(source_id))
            .copied()
            .unwrap_or(true)
    }

    /// Port of `_effective_discovery_rules`: enabled, subscribed, ordered by
    /// `(priority, id)`.
    pub fn effective_discovery_rules(&self, appid: u32) -> Vec<SourceRule> {
        let mut rules: Vec<SourceRule> = self
            .rules
            .iter()
            .filter(|rule| {
                rule.discovery_only && rule.enabled && self.game_subscribed(appid, &rule.id)
            })
            .cloned()
            .collect();
        rules.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.id.cmp(&b.id)));
        rules
    }

    pub fn set_enabled(&mut self, source_id: &str, enabled: bool) -> Result<()> {
        if !self.rules.iter().any(|rule| rule.id == source_id) {
            return Err(invalid("unknown_source"));
        }
        let mut overrides = self.overrides.clone();
        overrides.enabled.insert(source_id.to_string(), enabled);
        self.store.apply_overrides(&overrides)?;
        self.store.bump_revision()?;
        self.reload()
    }

    pub fn add_source(&mut self, raw: &RawSourceRule) -> Result<SourceRule> {
        let rule = validate::validate_rule(raw)?;
        if !BUILTIN_ADAPTERS.contains(&rule.adapter.as_str()) {
            return Err(invalid("source_exists_or_adapter_unavailable"));
        }
        if self.rules.iter().any(|existing| existing.id == rule.id) {
            return Err(invalid("source_exists_or_adapter_unavailable"));
        }
        if self.rules.len() >= MAX_SOURCES {
            return Err(invalid("source_limit"));
        }
        let mut overrides = self.overrides.clone();
        overrides.added.push(rule.clone());
        self.store.apply_overrides(&overrides)?;
        self.store.bump_revision()?;
        self.reload()?;
        Ok(rule)
    }

    pub fn set_game_source(&mut self, appid: u32, source_id: &str, subscribed: bool) -> Result<()> {
        if appid == 0 {
            return Err(invalid("invalid_appid"));
        }
        if !self.rules.iter().any(|rule| rule.id == source_id) {
            return Err(invalid("unknown_source"));
        }
        let mut overrides = self.overrides.clone();
        overrides
            .games
            .entry(appid.to_string())
            .or_default()
            .insert(source_id.to_string(), subscribed);
        if serde_json::to_vec(&overrides)
            .map_err(|error| invalid(error.to_string()))?
            .len()
            > 256 * 1024
        {
            return Err(invalid("source_limit"));
        }
        self.store.apply_overrides(&overrides)?;
        self.store.bump_revision()?;
        self.reload()
    }

    // --- discovery --------------------------------------------------------

    fn discovery_record(&self, rule: &SourceRule, appid: u32) -> DiscoveryRecord {
        if let Some(record) = self.discovery.get(&(rule.id.clone(), appid)) {
            return record.clone();
        }
        if self.frozen_snapshot {
            return DiscoveryRecord::default();
        }
        let Ok(Some(record)) = self.store.load_discovery(&rule.id, appid) else {
            return DiscoveryRecord::default();
        };
        if record.schema_version != 1 || record.appid != appid || record.source_id != rule.id {
            return DiscoveryRecord::default();
        }
        let valid = record
            .success
            .as_ref()
            .map(|success| validate::validate_candidates(rule, appid, &success.candidates).is_ok())
            .unwrap_or(true)
            && record
                .attempt
                .as_ref()
                .map(|attempt| {
                    validate::validate_candidates(rule, appid, &attempt.candidates).is_ok()
                })
                .unwrap_or(true);
        if !valid {
            return DiscoveryRecord::default();
        }
        record
    }

    fn discovery_compatible(&self, success: &DiscoverySuccess) -> bool {
        success.parser_version == PARSER_VERSION
            && success.query_version == QUERY_VERSION
            && success.encoding_version == SOURCES_ENCODING_VERSION
    }

    /// Port of `cached_candidates`: a partial attempt exposes new leads but never
    /// shadows the last complete record (design §4.4).
    pub fn cached_candidates(&self, game: &GameQuery) -> Vec<SourceCandidate> {
        let mut result = Vec::new();
        for rule in self.effective_discovery_rules(game.appid) {
            let record = self.discovery_record(&rule, game.appid);
            let mut found: BTreeMap<String, SourceCandidate> = BTreeMap::new();
            if let Some(attempt) = &record.attempt {
                if attempt.status == "partial" {
                    for candidate in &attempt.candidates {
                        found.insert(candidate.candidate_id.clone(), candidate.clone());
                    }
                }
            }
            if let Some(success) = &record.success {
                for candidate in &success.candidates {
                    // A current partial attempt must not be shadowed by an incompatible success.
                    if !self.discovery_compatible(success)
                        && found.contains_key(&candidate.candidate_id)
                    {
                        continue;
                    }
                    found.insert(candidate.candidate_id.clone(), candidate.clone());
                }
            }
            let fresh = record
                .success
                .as_ref()
                .is_some_and(|success| self.discovery_compatible(success));
            for (_, mut candidate) in found {
                let compatible_attempt = record.attempt.as_ref().is_some_and(|attempt| {
                    attempt.parser_version == PARSER_VERSION
                        && attempt.query_version == QUERY_VERSION
                        && attempt.encoding_version == SOURCES_ENCODING_VERSION
                        && attempt
                            .candidates
                            .iter()
                            .any(|c| c.candidate_id == candidate.candidate_id)
                });
                candidate.cache_status = if fresh || compatible_attempt {
                    CacheStatus::Cached
                } else {
                    CacheStatus::Stale
                };
                result.push(candidate);
            }
        }
        result
    }

    /// Port of `cached_discovery` status precedence (R15).
    pub fn cached_discovery(&self, game: &GameQuery) -> GameDiscovery {
        let rules = self.effective_discovery_rules(game.appid);
        let records: Vec<DiscoveryRecord> = rules
            .iter()
            .map(|rule| self.discovery_record(rule, game.appid))
            .collect();
        let candidates = self.cached_candidates(game);
        let statuses: Vec<String> = records
            .iter()
            .map(|record| {
                record
                    .attempt
                    .as_ref()
                    .map(|attempt| attempt.status.clone())
                    .unwrap_or_else(|| "not_queried".to_string())
            })
            .collect();
        let status = if statuses.iter().any(|status| status == "failed") {
            if candidates.is_empty() {
                "source_refresh_failed"
            } else {
                "source_offline_cached"
            }
        } else if statuses.iter().any(|status| status == "partial") {
            "cache_partial"
        } else if statuses.is_empty() || statuses.iter().any(|status| status == "not_queried") {
            "not_queried"
        } else {
            "cache_complete"
        };
        let diagnostics: Vec<Diagnostic> = records
            .iter()
            .flat_map(|record| record.attempt.iter())
            .flat_map(|attempt| attempt.diagnostics.iter().cloned())
            .collect();
        GameDiscovery {
            status: status.to_string(),
            candidates,
            diagnostics,
        }
    }

    /// Pending-refresh decision from `_run_discovery`: a changed query invalidates
    /// only a non-confirmed (negative/empty) cache.
    pub fn needs_refresh(&self, rule: &SourceRule, appid: u32, key: &str, force: bool) -> bool {
        let record = self.discovery_record(rule, appid);
        if record.attempt.as_ref().is_some_and(|attempt| {
            attempt.status == "partial"
                && attempt.query_key == key
                && attempt.parser_version == PARSER_VERSION
                && attempt.encoding_version == super::SOURCES_ENCODING_VERSION
                && (attempt.checkpoint.is_some()
                    || attempt
                        .index
                        .as_ref()
                        .is_some_and(|i| !i.pending.is_empty())
                    || self
                        .shared_index(&rule.id)
                        .is_some_and(|i| !i.pending.is_empty()))
        }) {
            return true;
        }
        let old = record.success.unwrap_or_else(empty_success);
        // A title-only candidate is still a useful positive cache for manual selection.
        let confirmed = !old.candidates.is_empty();
        force || !self.discovery_compatible(&old) || (!confirmed && old.query_key != key)
    }

    /// Shared indexes persist until explicitly refreshed or incompatible with the parser
    /// (design §4.3/§5.3).
    pub fn shared_index(&self, source_id: &str) -> Option<DiscoveryIndex> {
        let record = match self.indexes.get(source_id) {
            Some(record) => record.clone(),
            None if self.frozen_snapshot => return None,
            None => self.store.load_discovery_index(source_id).ok().flatten()?,
        };
        if record.schema_version != 1 {
            return None;
        }
        if record.index.parser_version != PARSER_VERSION
            || record.index.encoding_version != SOURCES_ENCODING_VERSION
        {
            return None;
        }
        Some(record.index)
    }

    /// Port of the layered commit (design §4.4). Only a complete success replaces
    /// `success`; partial/failed update `attempt` and preserve the previous
    /// success/checkpoint. Oversized records are not written.
    pub fn commit(
        &mut self,
        rule: &SourceRule,
        appid: u32,
        result: DiscoveryOutcome,
        previous: &DiscoveryRecord,
        key: &str,
    ) -> Result<DiscoveryRecord> {
        let now = self.now();
        let prev_success = previous.success.clone();
        let mut attempt = DiscoveryAttempt {
            status: result.status.clone(),
            candidates: result.candidates.clone(),
            diagnostics: result.diagnostics.clone(),
            checked_at: now,
            cached_at: if result.status == "complete" {
                Some(now)
            } else {
                prev_success.as_ref().and_then(|success| success.cached_at)
            },
            query_key: key.to_string(),
            parser_version: PARSER_VERSION,
            query_version: QUERY_VERSION,
            encoding_version: SOURCES_ENCODING_VERSION,
            index: result.index.clone().or_else(|| {
                previous
                    .attempt
                    .as_ref()
                    .and_then(|attempt| attempt.index.clone())
            }),
            checkpoint: result.checkpoint.clone().or_else(|| {
                if result.status == "complete"
                    && matches!(
                        rule.adapter.as_str(),
                        "kagura_wordpress" | "kagura_woocommerce"
                    )
                {
                    return None;
                }
                previous
                    .attempt
                    .as_ref()
                    .and_then(|attempt| attempt.checkpoint.clone())
            }),
            queries: result.queries.clone().or_else(|| {
                previous
                    .attempt
                    .as_ref()
                    .and_then(|attempt| attempt.queries.clone())
            }),
            query_cache: result.query_cache.clone().or_else(|| {
                previous
                    .attempt
                    .as_ref()
                    .and_then(|attempt| attempt.query_cache.clone())
            }),
        };

        let success = if result.status == "complete"
            || (result.status == "skipped" && prev_success.is_none())
        {
            Some(DiscoverySuccess {
                status: attempt.status.clone(),
                candidates: attempt.candidates.clone(),
                diagnostics: attempt.diagnostics.clone(),
                checked_at: attempt.checked_at,
                cached_at: attempt.cached_at,
                query_key: attempt.query_key.clone(),
                parser_version: attempt.parser_version,
                query_version: attempt.query_version,
                encoding_version: attempt.encoding_version,
            })
        } else {
            prev_success
        };
        // `attempt.candidates` is retained only for a partial pass.
        if result.status != "partial" {
            attempt.candidates = Vec::new();
        }

        let mut record = DiscoveryRecord {
            schema_version: 1,
            source_id: rule.id.clone(),
            appid,
            success: success.clone(),
            attempt: Some(attempt),
        };
        // A shared index must not consume the per-game result envelope twice.
        // Save the full bounded index first; large indexes are referenced through
        // the shared store instead of copied into every game's attempt.
        if let Some(index) = &result.index {
            let existing_checked = self.indexes.get(&rule.id).map(|record| record.checked_at);
            let envelope_checked = if index.checked_at != 0 {
                index.checked_at
            } else {
                existing_checked.unwrap_or_else(|| {
                    record
                        .attempt
                        .as_ref()
                        .map(|attempt| attempt.checked_at)
                        .unwrap_or(now)
                })
            };
            // Normalize the encoding version on the persisted envelope so an
            // adapter can never publish an index the current build would reject.
            let mut stored_index = index.clone();
            stored_index.encoding_version = SOURCES_ENCODING_VERSION;
            let index_record = DiscoveryIndexRecord {
                schema_version: 1,
                checked_at: envelope_checked,
                index: stored_index,
            };
            let index_blob =
                serde_json::to_vec(&index_record).map_err(|error| invalid(error.to_string()))?;
            if index_blob.len() as u64 <= super::MAX_DISCOVERY_INDEX_BYTES {
                self.store.save_discovery_index(&rule.id, &index_record)?;
                self.indexes.insert(rule.id.clone(), index_record);
            } else {
                return Err(invalid("discovery_index_resource_limit"));
            }
        }
        let mut blob = serde_json::to_vec(&record).map_err(|error| invalid(error.to_string()))?;
        if blob.len() as u64 > MAX_DISCOVERY_OUTPUT {
            if let Some(attempt) = record.attempt.as_mut() {
                attempt.index = None;
            }
            blob = serde_json::to_vec(&record).map_err(|error| invalid(error.to_string()))?;
        }
        if blob.len() as u64 > MAX_DISCOVERY_OUTPUT {
            return Err(invalid("discovery_output_resource_limit"));
        }
        self.store.save_discovery(&rule.id, appid, &record)?;
        self.discovery
            .insert((rule.id.clone(), appid), record.clone());

        let success_candidates = success
            .as_ref()
            .map(|success| success.candidates.len())
            .unwrap_or(0);
        let code = if result.status == "complete" {
            "source_refreshed"
        } else if result.status == "partial" {
            "source_partial"
        } else if success_candidates > 0 {
            "source_offline_cached"
        } else {
            "source_refresh_failed"
        };
        let status = SourceStatus {
            checked_at: record
                .attempt
                .as_ref()
                .map(|attempt| attempt.checked_at)
                .unwrap_or(now),
            cached_at: success.as_ref().and_then(|success| success.cached_at),
            entry_count: success_candidates as u64,
            code: code.to_string(),
        };
        self.store.save_status(&rule.id, &status)?;
        self.status.insert(rule.id.clone(), status);
        Ok(record)
    }

    /// Previous per-game record (success + last attempt) for the commit layer.
    pub fn previous_record(&self, rule: &SourceRule, appid: u32) -> DiscoveryRecord {
        self.discovery_record(rule, appid)
    }

    /// Python `_discovery_sync` `all_candidates`: publisher `success` candidates
    /// with display status reflecting record compatibility, not elapsed time.
    pub fn previous_success_candidates(&self, appid: u32) -> Vec<SourceCandidate> {
        let mut out = Vec::new();
        for rule in self.effective_discovery_rules(appid) {
            if rule.role != SourceRole::Publisher {
                continue;
            }
            let Some(success) = self.discovery_record(&rule, appid).success else {
                continue;
            };
            let fresh = self.discovery_compatible(&success);
            for mut candidate in success.candidates {
                candidate.cache_status = if fresh {
                    CacheStatus::Cached
                } else {
                    CacheStatus::Stale
                };
                out.push(candidate);
            }
        }
        out
    }

    /// Refreshes one ZIP-index source (design §5.3; Python `_refresh_one`).
    /// Existing cache short-circuits unless `force`; a failure preserves the
    /// existing cache bytes and returns an offline/failed status (never raises).
    pub fn refresh_source(
        &mut self,
        rule: &SourceRule,
        force: bool,
        ctx: &mut RequestContext,
        _cancel: &AtomicBool,
    ) -> Result<SourceStatus> {
        let old = self.cache.get(&rule.id).cloned().unwrap_or_default();
        if !force && old.checked_at.is_some() {
            return Ok(SourceStatus {
                checked_at: old.checked_at.unwrap_or(0),
                cached_at: old.cached_at,
                entry_count: old.entries.len() as u64,
                code: "source_cache_fresh".to_string(),
            });
        }
        let adapter = adapters::builtin_adapter(&rule.adapter)
            .ok_or_else(|| invalid("unknown_source_adapter"))?;
        let checked_at = self.now();
        match adapter.collect(rule, &old, ctx) {
            Ok(outcome) => {
                if outcome.not_modified {
                    let record = SourceCache {
                        entries: old.entries.clone(),
                        etag: old.etag.clone(),
                        modified: old.modified.clone(),
                        cached_at: old.cached_at,
                        checked_at: Some(checked_at),
                    };
                    self.store.save_cache(&rule.id, &record)?;
                    self.cache.insert(rule.id.clone(), record);
                    let status = SourceStatus {
                        checked_at,
                        cached_at: old.cached_at,
                        entry_count: old.entries.len() as u64,
                        code: "source_not_modified".to_string(),
                    };
                    self.store.save_status(&rule.id, &status)?;
                    self.status.insert(rule.id.clone(), status.clone());
                    Ok(status)
                } else {
                    let entry_count = outcome.entries.len() as u64;
                    let record = SourceCache {
                        entries: outcome.entries,
                        etag: outcome.etag,
                        modified: outcome.modified,
                        cached_at: Some(checked_at),
                        checked_at: Some(checked_at),
                    };
                    self.store.save_cache(&rule.id, &record)?;
                    self.cache.insert(rule.id.clone(), record);
                    let status = SourceStatus {
                        checked_at,
                        cached_at: Some(checked_at),
                        entry_count,
                        code: "source_refreshed".to_string(),
                    };
                    self.store.save_status(&rule.id, &status)?;
                    self.status.insert(rule.id.clone(), status.clone());
                    Ok(status)
                }
            }
            Err(_) => {
                // Failure must not touch existing cache bytes.
                let code = if old.entries.is_empty() {
                    "source_refresh_failed"
                } else {
                    "source_offline_cached"
                };
                let status = SourceStatus {
                    checked_at,
                    cached_at: old.cached_at,
                    entry_count: old.entries.len() as u64,
                    code: code.to_string(),
                };
                let _ = self.store.save_status(&rule.id, &status);
                self.status.insert(rule.id.clone(), status.clone());
                Ok(status)
            }
        }
    }

    /// Whether a source id came from the compiled-in defaults or user `added`.
    ///
    /// A user `added` rule that shadows a builtin id is reported as `"added"`,
    /// because that is the effective definition (P5 §1, §5).
    pub fn source_origin(&self, source_id: &str) -> Option<&'static str> {
        if self.overrides.added.iter().any(|rule| rule.id == source_id) {
            Some("added")
        } else if self.builtins.iter().any(|rule| rule.id == source_id) {
            Some("builtin")
        } else {
            None
        }
    }

    // --- query validation / language --------------------------------------

    /// Port of the `discover_game` input bounds (design §2.6).
    pub fn validate_query(
        game: &GameQuery,
        name_hints: &[String],
        preferred_language: Option<&str>,
    ) -> Result<()> {
        if game.appid == 0 || game.name.trim().is_empty() || game.name.len() > 1024 {
            return Err(invalid("invalid_discovery_game"));
        }
        if name_hints.len() > 3
            || name_hints
                .iter()
                .any(|hint| hint.trim().is_empty() || hint.len() > 256)
        {
            return Err(invalid("invalid_discovery_query"));
        }
        if let Some(language) = preferred_language {
            if !valid_language_code(language) {
                return Err(invalid("invalid_discovery_query"));
            }
        }
        Ok(())
    }

    /// Port of the legacy-match `preferred_language` filter (R10).
    pub fn filter_by_language(
        matches: &[SourceMatch],
        preferred_language: Option<&str>,
    ) -> Vec<SourceMatch> {
        let Some(preferred) = preferred_language else {
            return matches.to_vec();
        };
        let preferred = preferred.to_lowercase().replace('_', "-");
        matches
            .iter()
            .filter(|entry| {
                let language = entry.language.to_lowercase().replace('_', "-");
                language == preferred
                    || (!language.contains('-') && preferred.starts_with(&format!("{language}-")))
            })
            .cloned()
            .collect()
    }
}

fn cache_diagnostic(source_id: &str) -> Diagnostic {
    Diagnostic {
        source_id: Some(source_id.to_string()),
        detail: None,
        code: "invalid_source_cache".to_string(),
    }
}

/// ISO8601 UTC rendering of a Unix second count. Used only for
/// `SourceCandidate::observed_at` on index projections, which the fingerprint
/// deliberately excludes; determinism comes from the cached `checked_at`.
fn iso8601_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let remainder = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00",
        year,
        month,
        day,
        remainder / 3600,
        (remainder % 3600) / 60,
        remainder % 60
    )
}

/// Howard Hinnant's `civil_from_days` (days since 1970-01-01).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::super::store::{read_bounded, write_atomic};
    use super::*;
    use std::fs;
    use std::path::Path;

    fn builtins() -> Vec<SourceRule> {
        let config: RulesConfig = serde_json::from_str(crate::sources::DEFAULT_RULES_JSON).unwrap();
        config
            .sources
            .iter()
            .map(|raw| validate::validate_rule(raw).unwrap())
            .collect()
    }

    fn registry_at(dir: &Path, now: u64) -> SourceRegistry {
        let store = SourceStore::open(dir).unwrap();
        SourceRegistry::load_with_clock(
            store,
            builtins(),
            Box::new(crate::sources::FakeClock::new(now)),
        )
        .unwrap()
    }

    fn game(appid: u32) -> GameQuery {
        GameQuery {
            appid,
            name: "Game".to_string(),
            localized_name: None,
            instance_id: None,
        }
    }

    fn entry(title: &str, url: &str, language: &str, appid: Option<u32>) -> SourceEntry {
        SourceEntry {
            title: title.to_string(),
            aliases: Vec::new(),
            url: url.to_string(),
            source_page: "https://www.patch.moe/".to_string(),
            language: language.to_string(),
            date: None,
            appid,
        }
    }

    fn candidate(hex: char, appid: u32, basis: MatchBasis) -> SourceCandidate {
        SourceCandidate {
            candidate_id: format!("candidate-{}", hex.to_string().repeat(24)),
            source_id: "hgamer".to_string(),
            source_record_id: "https://hgamer.tw/zh-CN/patches/game".to_string(),
            game_appid: Some(appid),
            parent_appid: None,
            aliases: Vec::new(),
            game_title: "Game".to_string(),
            patch_title: "Game".to_string(),
            variant: Variant::Base,
            requires_dlc: None,
            excludes_dlc: None,
            platforms: vec!["steam".to_string()],
            patch_kind: "adult_content".to_string(),
            languages: Vec::new(),
            version_raw: Some("1".to_string()),
            version_evidence: Vec::new(),
            version_identity: "1".to_string(),
            published_at: None,
            patch_updated_at: None,
            source_page: "https://hgamer.tw/zh-CN/patches/game".to_string(),
            upstream_page: None,
            mirrors: Vec::new(),
            format_reported: ArtifactFormat::Unknown,
            format_observed: ArtifactFormat::Unknown,
            capability: Capability::DiscoverOnly,
            identity_evidence: Vec::new(),
            provenance: Vec::new(),
            match_basis: basis,
            observed_at: "2026-01-01T00:00:00+00:00".to_string(),
            cache_status: CacheStatus::Fresh,
            warnings: Vec::new(),
            fingerprint: "fp".to_string(),
            related_index_pages: Vec::new(),
        }
    }

    fn index(checked_at: u64) -> DiscoveryIndex {
        DiscoveryIndex {
            parser_version: PARSER_VERSION,
            encoding_version: SOURCES_ENCODING_VERSION,
            records: Vec::new(),
            pending: Vec::new(),
            checked_at,
        }
    }

    fn complete(candidates: Vec<SourceCandidate>) -> DiscoveryOutcome {
        DiscoveryOutcome {
            status: "complete".to_string(),
            candidates,
            ..DiscoveryOutcome::default()
        }
    }

    fn rule(registry: &SourceRegistry, id: &str) -> SourceRule {
        registry
            .rules
            .iter()
            .find(|rule| rule.id == id)
            .unwrap()
            .clone()
    }

    fn snapshot(root: &Path) -> Vec<String> {
        fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
            let mut entries: Vec<_> = fs::read_dir(dir)
                .unwrap()
                .flatten()
                .map(|entry| entry.path())
                .collect();
            entries.sort();
            for path in entries {
                out.push(
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
                if path.is_dir() {
                    walk(&path, base, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(root, root, &mut out);
        out
    }

    #[test]
    fn registry_reads_seeded_cache_and_separates_sources() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        registry
            .store()
            .save_cache(
                "patchmoe",
                &SourceCache {
                    entries: vec![entry(
                        "Game",
                        "https://download.patch.moe/g.zip",
                        "zh-CN",
                        Some(42),
                    )],
                    etag: None,
                    modified: None,
                    cached_at: Some(900),
                    checked_at: Some(900),
                },
            )
            .unwrap();
        registry.reload().unwrap();
        assert_eq!(registry.matches(&game(42), false).len(), 1);
        assert!(registry.matches(&game(43), false).is_empty());
        let summary = registry
            .list_sources()
            .into_iter()
            .find(|source| source.rule.id == "patchmoe")
            .unwrap();
        assert_eq!(summary.status, "cached");
        assert_eq!(summary.entry_count, 1);
    }

    #[test]
    fn matches_refuses_multiple_distinct_urls() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        registry
            .store()
            .save_cache(
                "patchmoe",
                &SourceCache {
                    entries: vec![
                        entry("Game", "https://download.patch.moe/a.zip", "en", Some(42)),
                        entry("Game", "https://download.patch.moe/b.zip", "en", Some(42)),
                    ],
                    etag: None,
                    modified: None,
                    cached_at: None,
                    checked_at: None,
                },
            )
            .unwrap();
        registry.reload().unwrap();
        assert!(registry.matches(&game(42), false).is_empty());

        registry
            .store()
            .save_cache(
                "patchmoe",
                &SourceCache {
                    entries: vec![
                        entry("Game", "https://download.patch.moe/a.zip", "en", Some(42)),
                        entry(
                            "Game",
                            "https://download.patch.moe/a.zip",
                            "zh-CN",
                            Some(42),
                        ),
                    ],
                    etag: None,
                    modified: None,
                    cached_at: None,
                    checked_at: None,
                },
            )
            .unwrap();
        registry.reload().unwrap();
        let matches = registry.matches(&game(42), false);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].language, "zh-CN");
        assert_eq!(matches[0].release_id, "patchmoe-42");
    }

    #[test]
    fn matches_by_appid_without_name() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        registry
            .store()
            .save_cache(
                "patchmoe",
                &SourceCache {
                    entries: vec![entry(
                        "Game",
                        "https://download.patch.moe/g.zip",
                        "en",
                        Some(42),
                    )],
                    ..SourceCache::default()
                },
            )
            .unwrap();
        registry.reload().unwrap();
        // An empty name still matches entries that carry the AppID (P5 §1).
        let query = GameQuery {
            appid: 42,
            name: String::new(),
            localized_name: None,
            instance_id: None,
        };
        let matches = registry.matches(&query, false);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].source_id, "patchmoe");
        assert_eq!(matches[0].release_id, "patchmoe-42");
        // A cache entry bound to another AppID does not match.
        assert!(registry.matches(&game(43), false).is_empty());
    }

    #[test]
    fn list_sources_reports_origin_for_builtin_and_added() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let builtin = registry
            .list_sources()
            .into_iter()
            .find(|source| source.rule.id == "patchmoe")
            .unwrap();
        assert_eq!(builtin.origin, "builtin");

        let raw = RawSourceRule {
            id: "custom".to_string(),
            name: "Custom".to_string(),
            adapter: "patchmoe".to_string(),
            role: None,
            discovery_only: None,
            priority: None,
            enabled: true,
            pages: vec!["https://custom.example/".to_string()],
            page_hosts: vec!["custom.example".to_string()],
            download_hosts: Some(vec!["dl.custom.example".to_string()]),
            root_layout: Some("game_root".to_string()),
            columns: None,
            table_id: None,
            language: None,
            url: None,
        };
        registry.add_source(&raw).unwrap();
        let added = registry
            .list_sources()
            .into_iter()
            .find(|source| source.rule.id == "custom")
            .unwrap();
        assert_eq!(added.origin, "added");
        assert_eq!(registry.source_origin("patchmoe"), Some("builtin"));
        assert_eq!(registry.source_origin("custom"), Some("added"));
    }

    #[test]
    fn failed_refresh_preserves_cache_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let cache = SourceCache {
            entries: vec![entry(
                "Game",
                "https://download.patch.moe/g.zip",
                "en",
                Some(42),
            )],
            etag: Some("etag".to_string()),
            modified: None,
            cached_at: Some(900),
            checked_at: Some(900),
        };
        registry.store().save_cache("patchmoe", &cache).unwrap();
        let before = read_bounded(&registry.store().paths().cache("patchmoe"), 4 * 1024 * 1024)
            .unwrap()
            .unwrap();
        let rule = rule(&registry, "patchmoe");
        let failed = DiscoveryOutcome {
            status: "failed".to_string(),
            ..DiscoveryOutcome::default()
        };
        registry
            .commit(&rule, 42, failed, &DiscoveryRecord::default(), "k")
            .unwrap();
        let after = read_bounded(&registry.store().paths().cache("patchmoe"), 4 * 1024 * 1024)
            .unwrap()
            .unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn add_source_and_set_enabled_persist_across_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        registry.set_enabled("patchmoe", false).unwrap();
        let raw = RawSourceRule {
            id: "custom".to_string(),
            name: "Custom".to_string(),
            adapter: "patchmoe".to_string(),
            role: None,
            discovery_only: None,
            priority: None,
            enabled: true,
            pages: vec!["https://custom.example/".to_string()],
            page_hosts: vec!["custom.example".to_string()],
            download_hosts: Some(vec!["dl.custom.example".to_string()]),
            root_layout: Some("game_root".to_string()),
            columns: None,
            table_id: None,
            language: None,
            url: None,
        };
        registry.add_source(&raw).unwrap();
        drop(registry);

        let reopened = registry_at(temp.path(), 1000);
        assert!(reopened.rules.iter().any(|rule| rule.id == "custom"));
        assert!(
            !reopened
                .rules
                .iter()
                .find(|rule| rule.id == "patchmoe")
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn game_subscription_scoped_and_global_disable_wins() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        registry.set_game_source(42, "hgamer", false).unwrap();
        assert!(!registry.game_subscribed(42, "hgamer"));
        assert!(registry.game_subscribed(43, "hgamer"));
        assert!(registry
            .effective_discovery_rules(42)
            .iter()
            .all(|rule| rule.id != "hgamer"));
        assert!(registry
            .effective_discovery_rules(43)
            .iter()
            .any(|rule| rule.id == "hgamer"));

        registry.set_enabled("kagura-cn", false).unwrap();
        assert!(registry
            .effective_discovery_rules(42)
            .iter()
            .all(|rule| rule.id != "kagura-cn"));
    }

    #[test]
    fn game_subscription_input_boundaries_and_missing_games() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        // Missing `games` means all subscribed (R8).
        assert!(registry.game_subscribed(42, "hgamer"));
        assert_eq!(
            registry
                .set_game_source(0, "hgamer", true)
                .unwrap_err()
                .to_string(),
            "invalid_appid"
        );
        assert_eq!(
            registry
                .set_game_source(42, "missing", true)
                .unwrap_err()
                .to_string(),
            "unknown_source"
        );
        registry.set_game_source(42, "hgamer", true).unwrap();
        assert!(registry.game_subscribed(42, "hgamer"));
    }

    #[test]
    fn custom_rule_shadows_builtin_and_preserves_selection() {
        let temp = tempfile::tempdir().unwrap();
        let store = SourceStore::open(temp.path()).unwrap();
        let custom = validate::validate_rule(&RawSourceRule {
            id: "patchmoe".to_string(),
            name: "Custom patch".to_string(),
            adapter: "patchmoe".to_string(),
            role: None,
            discovery_only: None,
            priority: None,
            enabled: true,
            pages: vec!["https://www.patch.moe/?flag=1".to_string()],
            page_hosts: vec!["www.patch.moe".to_string()],
            download_hosts: Some(vec!["download.patch.moe".to_string()]),
            root_layout: Some("game_root".to_string()),
            columns: None,
            table_id: None,
            language: None,
            url: None,
        })
        .unwrap();
        let mut overrides = Overrides::default();
        overrides.added.push(custom);
        overrides.enabled.insert("patchmoe".to_string(), false);
        store.apply_overrides(&overrides).unwrap();
        store.bump_revision().unwrap();

        let registry = SourceRegistry::load(store, builtins()).unwrap();
        let patchmoe: Vec<_> = registry
            .rules
            .iter()
            .filter(|rule| rule.id == "patchmoe")
            .collect();
        assert_eq!(patchmoe.len(), 1);
        assert_eq!(patchmoe[0].name, "Custom patch");
        assert!(!patchmoe[0].enabled);
    }

    #[test]
    fn read_paths_create_no_state() {
        let temp = tempfile::tempdir().unwrap();
        let registry = registry_at(temp.path(), 1000);
        let before = snapshot(temp.path());
        let game = game(42);
        let _ = registry.list_sources();
        let _ = registry.matches(&game, false);
        let _ = registry.cached_candidates(&game);
        let _ = registry.cached_discovery(&game);
        let _ = registry.list_game_sources(&game);
        let _ = registry.shared_index("hgamer");
        let after = snapshot(temp.path());
        assert_eq!(before, after);
    }

    #[test]
    fn discovery_commit_layers_success_and_attempt() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let rule = rule(&registry, "hgamer");
        let record = registry
            .commit(
                &rule,
                42,
                complete(vec![candidate('a', 42, MatchBasis::Appid)]),
                &DiscoveryRecord::default(),
                "k1",
            )
            .unwrap();
        assert_eq!(record.success.as_ref().unwrap().candidates.len(), 1);
        assert!(record.attempt.as_ref().unwrap().candidates.is_empty());
        assert_eq!(
            registry.status.get("hgamer").unwrap().code,
            "source_refreshed"
        );
        assert_eq!(registry.cached_candidates(&game(42)).len(), 1);
    }

    #[test]
    fn failed_attempt_preserves_success_and_pending_index() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let rule = rule(&registry, "hgamer");
        let mut with_index = complete(vec![candidate('a', 42, MatchBasis::Appid)]);
        with_index.index = Some(index(500));
        let previous = registry
            .commit(&rule, 42, with_index, &DiscoveryRecord::default(), "k1")
            .unwrap();
        assert!(previous.attempt.as_ref().unwrap().index.is_some());

        let failed = DiscoveryOutcome {
            status: "failed".to_string(),
            ..DiscoveryOutcome::default()
        };
        let record = registry.commit(&rule, 42, failed, &previous, "k2").unwrap();
        assert_eq!(record.success.as_ref().unwrap().candidates.len(), 1);
        assert_eq!(record.attempt.as_ref().unwrap().status, "failed");
        assert!(record.attempt.as_ref().unwrap().index.is_some());
        assert_eq!(
            registry.status.get("hgamer").unwrap().code,
            "source_offline_cached"
        );
    }

    #[test]
    fn partial_attempt_does_not_shadow_success_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let rule = rule(&registry, "hgamer");
        let previous = registry
            .commit(
                &rule,
                42,
                complete(vec![candidate('a', 42, MatchBasis::Appid)]),
                &DiscoveryRecord::default(),
                "k1",
            )
            .unwrap();
        let partial = DiscoveryOutcome {
            status: "partial".to_string(),
            candidates: vec![candidate('b', 42, MatchBasis::Appid)],
            ..DiscoveryOutcome::default()
        };
        let record = registry
            .commit(&rule, 42, partial, &previous, "k2")
            .unwrap();
        assert_eq!(record.success.as_ref().unwrap().candidates.len(), 1);
        assert_eq!(record.attempt.as_ref().unwrap().candidates.len(), 1);
        let candidates = registry.cached_candidates(&game(42));
        assert_eq!(candidates.len(), 2);
        assert!(candidates
            .iter()
            .any(|c| c.candidate_id.starts_with("candidate-aaaa")));
        assert!(candidates
            .iter()
            .any(|c| c.candidate_id.starts_with("candidate-bbbb")));
    }

    #[test]
    fn current_partial_candidate_is_not_shadowed_by_incompatible_success() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let rule = rule(&registry, "hgamer");
        let old = candidate('a', 42, MatchBasis::Appid);
        let mut record = registry
            .commit(
                &rule,
                42,
                complete(vec![old.clone()]),
                &DiscoveryRecord::default(),
                "key",
            )
            .unwrap();
        record.success.as_mut().unwrap().parser_version = PARSER_VERSION - 1;
        let mut current = old;
        current.patch_title = "Current candidate".into();
        registry
            .commit(
                &rule,
                42,
                DiscoveryOutcome {
                    status: "partial".into(),
                    candidates: vec![current],
                    ..Default::default()
                },
                &record,
                "key",
            )
            .unwrap();
        let candidates = registry.cached_candidates(&game(42));
        assert_eq!(candidates[0].patch_title, "Current candidate");
        assert_eq!(candidates[0].cache_status, CacheStatus::Cached);
    }

    #[test]
    fn complete_kagura_pass_clears_finished_work_checkpoint() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let rule = rule(&registry, "kagura-cn");
        let partial = DiscoveryOutcome {
            status: "partial".into(),
            checkpoint: Some(serde_json::json!({"work":[{"page":"https://kagurafan.com/game/"}]})),
            ..Default::default()
        };
        let old = registry
            .commit(&rule, 42, partial, &DiscoveryRecord::default(), "key")
            .unwrap();
        let complete = registry
            .commit(&rule, 42, complete(Vec::new()), &old, "key")
            .unwrap();
        assert!(complete.attempt.unwrap().checkpoint.is_none());
    }

    #[test]
    fn title_only_positive_cache_survives_name_hint_changes_without_network_refresh() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let rule = rule(&registry, "hgamer");
        let mut clue = candidate('a', 42, MatchBasis::Title);
        clue.game_appid = None;
        registry
            .commit(
                &rule,
                42,
                complete(vec![clue]),
                &DiscoveryRecord::default(),
                "chinese-hints",
            )
            .unwrap();
        assert!(!registry.needs_refresh(&rule, 42, "english-only-scan", false));
        assert!(registry.needs_refresh(&rule, 42, "english-only-scan", true));
    }

    #[test]
    fn negative_cache_invalidated_by_query_key_change() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let rule = rule(&registry, "hgamer");
        registry
            .commit(
                &rule,
                42,
                complete(Vec::new()),
                &DiscoveryRecord::default(),
                "k1",
            )
            .unwrap();
        assert!(!registry.needs_refresh(&rule, 42, "k1", false));
        assert!(registry.needs_refresh(&rule, 42, "k2", false));
    }

    #[test]
    fn toggle_filters_candidates_and_bumps_revision() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let before = registry.settings_revision();
        registry.set_enabled("hgamer", false).unwrap();
        assert!(registry.settings_revision() > before);
        // Disabling one discovery source leaves the other enabled publishers.
        assert!(!registry
            .effective_discovery_rules(42)
            .iter()
            .any(|rule| rule.id == "hgamer"));
        registry.set_enabled("hgamer", true).unwrap();
        assert!(registry
            .effective_discovery_rules(42)
            .iter()
            .any(|rule| rule.id == "hgamer"));
    }

    #[test]
    fn stale_revision_never_publishes() {
        let temp = tempfile::tempdir().unwrap();
        let store = SourceStore::open(temp.path()).unwrap();
        let registry = SourceRegistry::load(store, builtins()).unwrap();
        let captured = registry.settings_revision();
        registry.store().bump_revision().unwrap();
        let target = temp.path().join("published.json");
        let published = registry
            .store()
            .run_if_revision(captured, || write_atomic(&target, b"x", 1024))
            .unwrap();
        assert!(!published);
        assert!(!target.exists());
        let published = registry
            .store()
            .run_if_revision(registry.store().revision(), || {
                write_atomic(&target, b"x", 1024)
            })
            .unwrap();
        assert!(published);
        assert!(target.exists());
    }

    #[test]
    fn shared_index_remains_usable_after_30_days() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let rule = rule(&registry, "hgamer");
        let mut outcome = complete(Vec::new());
        outcome.index = Some(index(1000));
        registry
            .commit(&rule, 42, outcome, &DiscoveryRecord::default(), "k1")
            .unwrap();
        assert!(registry.shared_index("hgamer").is_some());

        let fresh = registry_at(temp.path(), 1000 + 24 * 3600 - 1);
        assert!(fresh.shared_index("hgamer").is_some());
        let expired = registry_at(temp.path(), 1000 + 30 * 24 * 3600);
        assert!(expired.shared_index("hgamer").is_some());
    }

    #[test]
    fn parser_or_query_version_change_marks_stale() {
        let temp = tempfile::tempdir().unwrap();
        let registry = registry_at(temp.path(), 1000);
        let mut success = empty_success();
        success.parser_version = PARSER_VERSION;
        success.query_version = QUERY_VERSION;
        success.encoding_version = SOURCES_ENCODING_VERSION;
        success.checked_at = 1000;
        assert!(registry.discovery_compatible(&success));

        let mut old_parser = success.clone();
        old_parser.parser_version = PARSER_VERSION - 1;
        assert!(!registry.discovery_compatible(&old_parser));

        let mut old_query = success.clone();
        old_query.query_version = QUERY_VERSION + 1;
        assert!(!registry.discovery_compatible(&old_query));

        // An encoding-version mismatch invalidates confirmed discovery even when
        // parser/query versions and freshness match (contract §1.3).
        let mut old_encoding = success.clone();
        old_encoding.encoding_version = SOURCES_ENCODING_VERSION - 1;
        assert!(!registry.discovery_compatible(&old_encoding));
        let mut missing_encoding = success;
        missing_encoding.encoding_version = 0;
        assert!(!registry.discovery_compatible(&missing_encoding));
    }

    #[test]
    fn old_encoding_version_invalidates_stored_discovery_and_index() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let rule = rule(&registry, "hgamer");

        // A complete success plus a fresh index carrying the current encoding.
        let mut outcome = complete(vec![candidate('a', 42, MatchBasis::Appid)]);
        outcome.index = Some(index(1000));
        registry
            .commit(&rule, 42, outcome, &DiscoveryRecord::default(), "k1")
            .unwrap();
        assert!(!registry.needs_refresh(&rule, 42, "k1", false));
        assert!(registry.shared_index("hgamer").is_some());

        // Rewrite both records at the previous encoding version (missing field
        // in older files deserializes to `0`): both gates must reject them.
        let mut old_success = empty_success();
        old_success.status = "complete".to_string();
        old_success.checked_at = 1000;
        old_success.query_key = "k1".to_string();
        old_success.candidates = vec![candidate('a', 42, MatchBasis::Appid)];
        old_success.encoding_version = SOURCES_ENCODING_VERSION - 1;
        let old_index = DiscoveryIndex {
            encoding_version: SOURCES_ENCODING_VERSION - 1,
            ..index(1000)
        };
        let record = DiscoveryRecord {
            schema_version: 1,
            source_id: rule.id.clone(),
            appid: 42,
            success: Some(old_success),
            attempt: None,
        };
        let store = SourceStore::open(temp.path()).unwrap();
        store.save_discovery(&rule.id, 42, &record).unwrap();
        store
            .save_discovery_index(
                &rule.id,
                &DiscoveryIndexRecord {
                    schema_version: 1,
                    checked_at: 1000,
                    index: old_index,
                },
            )
            .unwrap();
        drop(store);

        let reopened = registry_at(temp.path(), 1000);
        assert!(reopened.shared_index("hgamer").is_none());
        assert!(reopened.needs_refresh(&rule, 42, "k1", false));
    }

    #[test]
    fn partial_status_not_hidden_by_unqueried() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let rule = rule(&registry, "kagura-cn");
        let partial = DiscoveryOutcome {
            status: "partial".to_string(),
            ..DiscoveryOutcome::default()
        };
        registry
            .commit(&rule, 42, partial, &DiscoveryRecord::default(), "k1")
            .unwrap();
        assert_eq!(registry.cached_discovery(&game(42)).status, "cache_partial");
    }

    #[test]
    fn clock_rollback_expires_conservatively() {
        assert!(!is_fresh(Some(1000), 24 * 3600, 999));
        assert!(is_fresh(Some(1000), 24 * 3600, 1000));
        assert!(is_fresh(Some(1000), 24 * 3600, 1000 + 24 * 3600 - 1));
        assert!(!is_fresh(Some(1000), 24 * 3600, 1000 + 24 * 3600));
        assert!(!is_fresh(None, 24 * 3600, 1000));
    }

    #[test]
    fn persistent_success_requires_only_explicit_refresh() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let rule = rule(&registry, "hgamer");
        registry
            .commit(
                &rule,
                42,
                complete(vec![candidate('a', 42, MatchBasis::Appid)]),
                &DiscoveryRecord::default(),
                "k1",
            )
            .unwrap();
        assert!(!registry.needs_refresh(&rule, 42, "k1", false));
        assert!(registry.needs_refresh(&rule, 42, "k1", true));
        assert!(is_fresh(Some(1000), 24 * 3600, 1000 + 3600));
    }

    #[test]
    fn query_context_boundaries() {
        let good = GameQuery {
            appid: 42,
            name: "Game".to_string(),
            localized_name: None,
            instance_id: None,
        };
        assert!(SourceRegistry::validate_query(&good, &[], None).is_ok());
        assert!(SourceRegistry::validate_query(&good, &[], Some("zh-CN")).is_ok());
        assert!(SourceRegistry::validate_query(&good, &[], Some("en")).is_ok());
        assert!(SourceRegistry::validate_query(&good, &[], Some("zh-cn-x")).is_err());

        let mut bad = good.clone();
        bad.appid = 0;
        assert_eq!(
            SourceRegistry::validate_query(&bad, &[], None)
                .unwrap_err()
                .to_string(),
            "invalid_discovery_game"
        );
        let mut bad = good.clone();
        bad.name = " ".to_string();
        assert!(SourceRegistry::validate_query(&bad, &[], None).is_err());
        let mut bad = good.clone();
        bad.name = "x".repeat(1025);
        assert!(SourceRegistry::validate_query(&bad, &[], None).is_err());

        let hints = vec!["a".to_string(); 4];
        assert_eq!(
            SourceRegistry::validate_query(&good, &hints, None)
                .unwrap_err()
                .to_string(),
            "invalid_discovery_query"
        );
        let hints = vec!["x".repeat(257)];
        assert!(SourceRegistry::validate_query(&good, &hints, None).is_err());
    }

    #[test]
    fn legacy_match_language_filter() {
        let match_a = SourceMatch {
            title: "A".to_string(),
            aliases: Vec::new(),
            url: "https://download.patch.moe/a.zip".to_string(),
            source_page: "https://www.patch.moe/".to_string(),
            language: "zh-CN".to_string(),
            date: None,
            appid: Some(42),
            source_id: "patchmoe".to_string(),
            release_id: "patchmoe-42".to_string(),
            root_layout: None,
        };
        let mut match_b = match_a.clone();
        match_b.language = "zh_cn".to_string();
        let mut match_c = match_a.clone();
        match_c.language = "ja".to_string();
        let matches = vec![match_a, match_b, match_c];
        assert_eq!(
            SourceRegistry::filter_by_language(&matches, Some("zh-CN")).len(),
            2
        );
        // Python: `zh` equals neither `zh-cn` nor `zh_cn`, and the no-dash
        // prefix branch requires the entry language to be dash-free (R10).
        assert_eq!(
            SourceRegistry::filter_by_language(&matches, Some("zh")).len(),
            0
        );
        assert_eq!(
            SourceRegistry::filter_by_language(&matches, Some("ja")).len(),
            1
        );
        assert_eq!(SourceRegistry::filter_by_language(&matches, None).len(), 3);

        // Entry language without a region ("zh") matches a regional preference.
        let mut plain = matches[0].clone();
        plain.language = "zh".to_string();
        assert_eq!(
            SourceRegistry::filter_by_language(&[plain, matches[1].clone()], Some("zh-CN")).len(),
            2
        );
    }

    #[test]
    fn missing_games_means_all_subscribed() {
        let temp = tempfile::tempdir().unwrap();
        let registry = registry_at(temp.path(), 1000);
        for id in ["patchmoe", "kagura-cn", "kagura-en", "hgamer"] {
            assert!(registry.game_subscribed(7, id), "{id}");
        }
    }

    fn seed_index_cache(registry: &mut SourceRegistry, entries: Vec<SourceEntry>, checked_at: u64) {
        registry
            .store()
            .save_cache(
                "patchmoe",
                &SourceCache {
                    entries,
                    etag: None,
                    modified: None,
                    cached_at: Some(checked_at),
                    checked_at: Some(checked_at),
                },
            )
            .unwrap();
        registry.reload().unwrap();
    }

    #[test]
    fn index_candidates_are_deterministic_and_zip_addressable() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        seed_index_cache(
            &mut registry,
            vec![entry(
                "Game",
                "https://download.patch.moe/g.zip",
                "zh-CN",
                Some(42),
            )],
            900,
        );
        let first = registry.index_candidates(&game(42));
        let second = registry.index_candidates(&game(42));
        assert_eq!(first.len(), 1);
        assert_eq!(first, second);
        let candidate = &first[0];
        assert_eq!(candidate.cache_status, CacheStatus::Cached);
        assert_eq!(candidate.match_basis, MatchBasis::Appid);
        assert_eq!(candidate.variant, Variant::Base);
        assert_eq!(candidate.format_reported, ArtifactFormat::Zip);
        // 900s after the epoch; derived from the cached `checked_at`.
        assert_eq!(candidate.observed_at, "1970-01-01T00:15:00+00:00");
        assert_eq!(
            crate::installer::zip_selection_id(candidate),
            crate::installer::zip_selection_id(&second[0])
        );
        // No discovery rule produces a candidate, so the union is the projection.
        assert_eq!(
            registry.dynamic_zip_candidates(&game(42)),
            registry.index_candidates(&game(42))
        );
    }

    #[test]
    fn index_candidate_identity_is_field_sensitive() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        seed_index_cache(
            &mut registry,
            vec![entry(
                "Game",
                "https://download.patch.moe/g.zip",
                "en",
                Some(42),
            )],
            900,
        );
        let base = registry.index_candidates(&game(42)).pop().unwrap();

        // Language is part of the candidate identity.
        seed_index_cache(
            &mut registry,
            vec![entry(
                "Game",
                "https://download.patch.moe/g.zip",
                "zh-CN",
                Some(42),
            )],
            900,
        );
        let language = registry.index_candidates(&game(42)).pop().unwrap();
        assert_ne!(language.candidate_id, base.candidate_id);

        // A different ZIP URL changes the fingerprint and the selection id.
        seed_index_cache(
            &mut registry,
            vec![entry(
                "Game",
                "https://download.patch.moe/other.zip",
                "en",
                Some(42),
            )],
            900,
        );
        let url = registry.index_candidates(&game(42)).pop().unwrap();
        assert_eq!(url.candidate_id, base.candidate_id);
        assert_ne!(url.fingerprint, base.fingerprint);
        assert_ne!(
            crate::installer::zip_selection_id(&url),
            crate::installer::zip_selection_id(&base)
        );

        // A different title changes the fingerprint.
        seed_index_cache(
            &mut registry,
            vec![entry(
                "Other",
                "https://download.patch.moe/g.zip",
                "en",
                Some(42),
            )],
            900,
        );
        let title = registry.index_candidates(&game(42)).pop().unwrap();
        assert_eq!(title.candidate_id, base.candidate_id);
        assert_ne!(title.fingerprint, base.fingerprint);
        assert_eq!(title.patch_title, "Other");
    }

    #[test]
    fn index_candidates_skip_coming_soon() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let mut soon = entry("Game", "https://download.patch.moe/g.zip", "en", Some(42));
        soon.date = Some("COMING SOON".to_string());
        seed_index_cache(&mut registry, vec![soon], 900);
        // `matches` still reports it, but the projection must drop it.
        assert_eq!(registry.matches(&game(42), false).len(), 1);
        assert!(registry.index_candidates(&game(42)).is_empty());
    }

    #[test]
    fn index_candidates_respect_disabled_and_subscription() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        seed_index_cache(
            &mut registry,
            vec![entry(
                "Game",
                "https://download.patch.moe/g.zip",
                "en",
                Some(42),
            )],
            900,
        );
        assert_eq!(registry.index_candidates(&game(42)).len(), 1);

        registry.set_game_source(42, "patchmoe", false).unwrap();
        assert!(registry.matches(&game(42), false).is_empty());
        assert!(registry.index_candidates(&game(42)).is_empty());

        registry.set_game_source(42, "patchmoe", true).unwrap();
        registry.set_enabled("patchmoe", false).unwrap();
        assert!(registry.index_candidates(&game(42)).is_empty());
    }

    #[test]
    fn index_candidates_preserve_aliases_for_title_match() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let mut kuppa = entry(
            "Kudo's Kuppa",
            "https://download.patch.moe/Kudos_Kuppa_patch.zip",
            "en",
            None,
        );
        kuppa.source_page = "https://www.patch.moe/Kudos-Kuppa".to_string();
        kuppa.aliases = vec!["Kudo’s Kuppa".to_string()];
        kuppa.date = Some("2026/6/23".to_string());
        seed_index_cache(&mut registry, vec![kuppa], 900);
        let query = GameQuery {
            appid: 42,
            name: "Kudo’s Kuppa".to_string(),
            localized_name: None,
            instance_id: None,
        };
        let candidates = registry.index_candidates(&query);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].aliases, vec!["Kudo’s Kuppa".to_string()]);
        assert_eq!(candidates[0].match_basis, MatchBasis::Title);
        // The game AppID supplies the missing entry AppID.
        assert_eq!(candidates[0].game_appid, Some(42));
    }
    #[test]
    fn shared_index_larger_than_game_envelope_is_not_silently_lost() {
        use crate::sources::model::IndexRecord;
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path(), 1000);
        let rule = rule(&registry, "hgamer");
        let mut large = index(1000);
        large.records = (0..6000)
            .map(|i| IndexRecord {
                title: format!("Game {i} {}", "title".repeat(12)),
                page: format!("https://hgamer.tw/zh-CN/patches/game-{i}"),
                kind: "article".into(),
            })
            .collect();
        assert!(serde_json::to_vec(&large).unwrap().len() as u64 > MAX_DISCOVERY_OUTPUT);
        registry
            .commit(
                &rule,
                42,
                DiscoveryOutcome {
                    status: "partial".into(),
                    index: Some(large),
                    ..Default::default()
                },
                &DiscoveryRecord::default(),
                "many",
            )
            .unwrap();
        let reopened = registry_at(temp.path(), 1001);
        assert_eq!(reopened.shared_index(&rule.id).unwrap().records.len(), 6000);
        assert_eq!(
            reopened.previous_record(&rule, 42).attempt.unwrap().status,
            "partial"
        );
    }
}
