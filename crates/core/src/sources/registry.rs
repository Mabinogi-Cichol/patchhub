//! In-memory merged rules, subscriptions and the schema-2 source read model
//! (design §3.3/§4.2; p1-contract §4/§5/§7).
//!
//! The registry owns the [`SourceStore`]; read paths never create state, and
//! mutations persist through the store's short lock + revision primitive.
//! Published [`SourceIndex`] snapshots and persisted [`GameMatchRecord`]s are
//! loaded lazily (or preloaded for an immutable snapshot); the legacy per-game
//! cache/discovery trees are only read by the one-time v1 → v2 migration.

use super::matching::{select_entry, EntrySelection};
use super::model::*;
use super::r#match::candidate_is_credible;
use super::store::SourceStore;
use super::validate;
use super::{canonical, index};
use super::{
    invalid, Clock, Result, SystemClock, BUILTIN_ADAPTERS, GAME_MATCH_SCHEMA_VERSION,
    SOURCE_INDEX_SCHEMA_VERSION,
};
use std::collections::{BTreeMap, HashSet};

fn stored_rule<'a>(
    builtins: &'a [SourceRule],
    settings: &'a SettingsFile,
    source_id: &str,
) -> Option<&'a SourceRule> {
    settings
        .added
        .iter()
        .find(|rule| rule.id == source_id)
        .or_else(|| builtins.iter().find(|rule| rule.id == source_id))
}

pub struct SourceRegistry {
    store: SourceStore,
    builtins: Vec<SourceRule>,
    rules: Vec<SourceRule>,
    overrides: Overrides,
    names: BTreeMap<String, String>,
    script_sources: BTreeMap<String, String>,
    revision: u64,
    status: BTreeMap<String, SourceStatus>,
    /// Published index snapshots, keyed by source id.
    indexes: BTreeMap<String, SourceIndex>,
    /// Persisted match records, keyed by AppID.
    matches: BTreeMap<u32, GameMatchRecord>,
    /// Diagnostics surfaced by the one-time v1 → v2 migration.
    diagnostics: Vec<Diagnostic>,
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
            names: BTreeMap::new(),
            script_sources: BTreeMap::new(),
            revision: 0,
            status: BTreeMap::new(),
            indexes: BTreeMap::new(),
            matches: BTreeMap::new(),
            diagnostics: Vec::new(),
            clock,
            frozen_snapshot: false,
        };
        registry.reload()?;
        Ok(registry)
    }

    /// Preloads the requested games' match records and every rule's published
    /// index for an immutable snapshot. Corrupt schema versions are reported
    /// here so later reads never silently turn them into an empty result.
    pub(crate) fn preload_snapshot_games(&mut self, games: &[GameQuery]) -> Result<()> {
        let appids: HashSet<u32> = games.iter().map(|game| game.appid).collect();
        for appid in appids {
            let Some(record) = self.store.load_match(appid)? else {
                continue;
            };
            if record.schema_version != GAME_MATCH_SCHEMA_VERSION || record.appid != appid {
                return Err(invalid("invalid_source_match"));
            }
            self.matches.insert(appid, record);
        }
        for rule in &self.rules {
            let Some(index) = self.store.load_index(&rule.id)? else {
                continue;
            };
            if index.schema_version != SOURCE_INDEX_SCHEMA_VERSION || index.source_id != rule.id {
                return Err(invalid("invalid_source_index"));
            }
            self.indexes.insert(rule.id.clone(), index);
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

    /// Diagnostics raised by the one-time legacy migration (empty otherwise).
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn rules(&self) -> &[SourceRule] {
        &self.rules
    }

    /// Effective per-game/global user overrides (enabled/added/games), as merged
    /// into `rules`. Matching reads subscriptions from here.
    pub fn overrides(&self) -> &Overrides {
        &self.overrides
    }

    pub fn now_unix(&self) -> u64 {
        self.clock.now_unix()
    }

    /// Port of `_reload` merge (design §2.2): user `added` shadows builtins with
    /// the same id and the `enabled` overlay is applied.
    fn reload(&mut self) -> Result<()> {
        let settings = self.store.settings()?;
        self.revision = settings.revision;
        let mut overrides = Overrides {
            enabled: settings.enabled.clone(),
            added: settings.added.clone(),
            games: settings.games.clone(),
        };
        for rule in &mut overrides.added {
            super::builtin::bind(rule);
        }
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
        for rule in &mut rules {
            super::builtin::bind(rule);
        }
        rules.extend(overrides.added.iter().cloned());
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
        self.names = settings.names;
        self.script_sources = settings.script_sources;
        for rule in &self.rules {
            if let Some(code) = super::builtin::code(&rule.adapter) {
                self.script_sources
                    .entry(rule.id.clone())
                    .or_insert_with(|| code.to_owned());
            }
        }
        self.overrides = overrides;
        self.status.clear();
        self.indexes.clear();
        self.matches.clear();
        self.diagnostics = self.store.migration_diagnostics().to_vec();

        for rule in &self.rules {
            if let Ok(Some(status)) = self.store.load_status(&rule.id) {
                self.status.insert(rule.id.clone(), status);
            }
        }
        Ok(())
    }

    /// Published index snapshot for `source_id` (lazy read outside a snapshot).
    fn index(&self, source_id: &str) -> Option<SourceIndex> {
        if let Some(index) = self.indexes.get(source_id) {
            return self
                .rules
                .iter()
                .find(|rule| rule.id == source_id)
                .filter(|rule| index::compatible(index, rule))
                .map(|_| index.clone());
        }
        if self.frozen_snapshot {
            return None;
        }
        self.store
            .load_index(source_id)
            .ok()
            .flatten()
            .filter(|index| {
                self.rules
                    .iter()
                    .find(|rule| rule.id == source_id)
                    .is_some_and(|rule| index::compatible(index, rule))
            })
    }

    /// Persisted match record for `appid` (lazy read outside a snapshot).
    fn match_record(&self, appid: u32) -> Option<GameMatchRecord> {
        if let Some(record) = self.matches.get(&appid) {
            return Some(self.current_record(record.clone()));
        }
        if self.frozen_snapshot {
            return None;
        }
        self.store
            .load_match(appid)
            .ok()
            .flatten()
            .filter(|record| {
                record.schema_version == GAME_MATCH_SCHEMA_VERSION && record.appid == appid
            })
            .map(|record| self.current_record(record))
    }

    fn current_record(&self, record: GameMatchRecord) -> GameMatchRecord {
        super::matching::current_record(record, &self.rules, &self.overrides, |rule| {
            self.index(&rule.id)
        })
    }

    pub fn list_sources(&self) -> Vec<SourceSummary> {
        let intervals = self
            .store
            .settings()
            .map(|s| s.intervals)
            .unwrap_or_default();
        self.rules
            .iter()
            .map(|rule| {
                let index = self.index(&rule.id);
                let status = self
                    .status
                    .get(&rule.id)
                    .filter(|_| rule.script.is_none() || index.is_some());
                let code = status.map(|status| status.code.clone()).unwrap_or_else(|| {
                    if index.is_some() {
                        "cached".to_string()
                    } else {
                        "not_refreshed".to_string()
                    }
                });
                let entry_count = index
                    .as_ref()
                    .map(|index| index.entries.len() as u64)
                    .or_else(|| status.map(|status| status.entry_count))
                    .unwrap_or(0);
                let cached_at = index
                    .as_ref()
                    .map(|index| index.updated_at)
                    .or_else(|| status.and_then(|status| status.cached_at));
                let checked_at = status
                    .map(|status| status.checked_at)
                    .or_else(|| index.as_ref().map(|index| index.checked_at));
                let interval_hours = intervals.get(&rule.id).copied().unwrap_or(0);
                let next_due_at = super::schedule::next_due_at(
                    interval_hours,
                    status.map(|status| status.checked_at).unwrap_or(0),
                );
                let mut display_rule = rule.clone();
                if let Some(name) = self.names.get(&rule.id) {
                    display_rule.name = name.clone();
                }
                SourceSummary {
                    rule: display_rule,
                    origin: self.source_origin(&rule.id).unwrap_or("added").to_string(),
                    status: code,
                    entry_count,
                    cached_at,
                    checked_at,
                    revision: index.as_ref().map(|index| index.revision).unwrap_or(0),
                    complete: index.as_ref().is_some_and(|index| index.complete),
                    interval_hours,
                    next_due_at,
                }
            })
            .collect()
    }

    pub fn list_game_sources(&self, game: &GameQuery) -> Vec<GameSourceSummary> {
        let record = self.match_record(game.appid);
        let mut matched: HashSet<String> = HashSet::new();
        if let Some(record) = &record {
            for (source_id, source) in &record.sources {
                if source.status == SourceMatchStatus::Matched {
                    matched.insert(source_id.clone());
                }
            }
            if let Some(fallback) = record
                .fallback
                .as_ref()
                .filter(|fallback| fallback.status == FallbackMatchStatus::Matched)
            {
                for candidate in &fallback.candidates {
                    if candidate_is_credible(candidate) {
                        matched.insert(candidate.source_id.clone());
                    }
                }
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

    /// Publisher source matches for installation/display.
    ///
    /// Uses the persisted match record; before the first match task (no record)
    /// it falls back to local `kind = "entry"` index matching so installer
    /// resolution keeps working immediately after a source index update.
    pub fn matches(&self, game: &GameQuery, include_disabled: bool) -> Vec<SourceMatch> {
        if game.appid == 0 {
            return Vec::new();
        }
        match self.match_record(game.appid) {
            Some(record) => self.matches_from_record(game, &record, include_disabled),
            None => self.index_matches(game, include_disabled),
        }
    }

    fn matches_from_record(
        &self,
        game: &GameQuery,
        record: &GameMatchRecord,
        include_disabled: bool,
    ) -> Vec<SourceMatch> {
        let mut result = Vec::new();
        for (source_id, source) in &record.sources {
            let Some(rule) = self.rules.iter().find(|rule| rule.id == *source_id) else {
                continue;
            };
            if !include_disabled && (!rule.enabled || !self.game_subscribed(game.appid, &rule.id)) {
                continue;
            }
            let Some(candidate) = source
                .candidates
                .iter()
                .find(|candidate| candidate_is_credible(candidate))
                .or_else(|| source.candidates.first())
            else {
                continue;
            };
            if let Some(matched) = source_match_from_candidate(rule, candidate, game.appid) {
                result.push(matched);
            }
        }
        result
    }

    fn index_matches(&self, game: &GameQuery, include_disabled: bool) -> Vec<SourceMatch> {
        let mut result = Vec::new();
        for rule in &self.rules {
            if rule.role != SourceRole::Publisher {
                continue;
            }
            if !include_disabled && (!rule.enabled || !self.game_subscribed(game.appid, &rule.id)) {
                continue;
            }
            let Some(index) = self.index(&rule.id) else {
                continue;
            };
            let EntrySelection::Selected(entry, url) = select_entry(&index, game) else {
                continue;
            };
            if entry.date.as_deref() == Some("COMING SOON") {
                continue;
            }
            result.push(source_match_from_entry(rule, entry, url, game.appid));
        }
        result
    }

    /// Local `kind = "entry"` index matching projected as dynamic ZIP candidates
    /// (`candidate_id`/`fingerprint` identical to the P3 matching engine).
    pub fn index_candidates(&self, game: &GameQuery) -> Vec<SourceCandidate> {
        let mut result = Vec::new();
        for rule in &self.rules {
            if rule.role != SourceRole::Publisher || !rule.enabled {
                continue;
            }
            if !self.game_subscribed(game.appid, &rule.id) {
                continue;
            }
            let Some(index) = self.index(&rule.id) else {
                continue;
            };
            let EntrySelection::Selected(entry, url) = select_entry(&index, game) else {
                continue;
            };
            // A "COMING SOON" entry has no artifact yet: never project it.
            if entry.date.as_deref() == Some("COMING SOON") {
                continue;
            }
            let mut candidate = candidate_from_index_match(
                &rule.id,
                game.appid,
                entry.appid,
                &entry.title,
                &entry.aliases,
                entry.language.as_deref().unwrap_or(""),
                &entry.page,
                url,
                index.checked_at,
            );
            index::bind_candidate(rule, &mut candidate);
            result.push(candidate);
        }
        result
    }

    /// Dynamic ZIP candidate view: match-record publisher candidates first,
    /// then deterministic `kind = "entry"` index projections, deduped by
    /// `candidate_id`.
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

    pub fn set_enabled(&mut self, source_id: &str, enabled: bool) -> Result<()> {
        if !self.rules.iter().any(|rule| rule.id == source_id) {
            return Err(invalid("unknown_source"));
        }
        self.store.update_settings(|settings| {
            settings.enabled.insert(source_id.to_string(), enabled);
            Ok(())
        })?;
        self.reload()
    }

    pub fn set_name(&mut self, source_id: &str, name: &str) -> Result<String> {
        if !self.rules.iter().any(|rule| rule.id == source_id) {
            return Err(invalid("unknown_source"));
        }
        let name = name.trim();
        if name.is_empty() {
            return Err(invalid("invalid_source_name"));
        }
        self.store.set_name(source_id, name)?;
        self.reload()?;
        Ok(name.to_string())
    }

    pub fn add_source(&mut self, raw: &RawSourceRule) -> Result<SourceRule> {
        let rule = validate::validate_rule(raw)?;
        if !BUILTIN_ADAPTERS.contains(&rule.adapter.as_str()) {
            return Err(invalid("source_exists_or_adapter_unavailable"));
        }
        self.store.update_settings(|settings| {
            if stored_rule(&self.builtins, settings, &rule.id).is_some() {
                return Err(invalid("source_exists_or_adapter_unavailable"));
            }
            settings.added.push(rule.clone());
            Ok(())
        })?;
        self.reload()?;
        Ok(rule)
    }

    /// Compile only, then atomically store rule, raw UTF-8 source and digest.
    pub fn import_script_source(
        &mut self,
        raw: &RawSourceRule,
        api_version: u32,
        code: &str,
        replace: bool,
    ) -> Result<ImportedScriptSource> {
        let mut rule = validate::validate_rule(raw)?;
        if rule.adapter != "lua" {
            return Err(invalid("source_not_script"));
        }
        rule.script = Some(super::script::validate_code(
            &rule.id,
            api_version,
            code.as_bytes(),
        )?);
        let revision = self.store.update_settings(|settings| {
            let existing = stored_rule(&self.builtins, settings, &rule.id);
            match (replace, existing) {
                (false, Some(_)) => return Err(invalid("script_source_exists")),
                (true, None) => return Err(invalid("unknown_source")),
                (true, Some(existing)) => {
                    if existing.adapter != "lua"
                        && super::builtin::code(&existing.adapter).is_none()
                    {
                        return Err(invalid("source_not_script"));
                    }
                    rule.enabled = settings
                        .enabled
                        .get(&rule.id)
                        .copied()
                        .unwrap_or(existing.enabled);
                }
                (false, None) => {}
            }
            settings.added.retain(|existing| existing.id != rule.id);
            settings.added.push(rule.clone());
            settings
                .script_sources
                .insert(rule.id.clone(), code.to_string());
            Ok(settings.revision)
        })?;
        self.reload()?;
        Ok(ImportedScriptSource {
            rule,
            settings_revision: revision,
        })
    }

    pub fn get_script_source(&self, source_id: &str) -> Result<ScriptSourceDocument> {
        let rule = self
            .rules
            .iter()
            .find(|rule| rule.id == source_id)
            .ok_or_else(|| invalid("unknown_source"))?;
        super::script::document(rule, self.script_sources.get(source_id).map(String::as_str))
    }

    pub fn set_game_source(&mut self, appid: u32, source_id: &str, subscribed: bool) -> Result<()> {
        if appid == 0 {
            return Err(invalid("invalid_appid"));
        }
        if !self.rules.iter().any(|rule| rule.id == source_id) {
            return Err(invalid("unknown_source"));
        }
        self.store.update_settings(|settings| {
            settings
                .games
                .entry(appid.to_string())
                .or_default()
                .insert(source_id.to_string(), subscribed);
            Ok(())
        })?;
        self.reload()
    }

    /// Sets a source's automatic update interval in hours (`0` = off). The
    /// fallback indexer is queried per game and never site-wide scheduled.
    pub fn set_update_interval(&mut self, source_id: &str, interval_hours: u64) -> Result<()> {
        self.store.update_settings(|settings| {
            let rule = stored_rule(&self.builtins, settings, source_id)
                .ok_or_else(|| invalid("unknown_source"))?;
            if rule.role == SourceRole::FallbackIndexer {
                return Err(invalid("source_not_indexable"));
            }
            settings
                .intervals
                .insert(source_id.to_string(), interval_hours);
            Ok(())
        })?;
        self.reload()
    }

    /// Candidates persisted by the last match task: publisher source candidates
    /// first; when no publisher source confirmed a match, the positive fallback
    /// (HGamer) result is appended.
    pub fn cached_candidates(&self, game: &GameQuery) -> Vec<SourceCandidate> {
        let Some(record) = self.match_record(game.appid) else {
            return Vec::new();
        };
        let mut candidates = Vec::new();
        let mut publisher_matched = false;
        for (source_id, source) in &record.sources {
            let Some(rule) = self.rules.iter().find(|rule| rule.id == *source_id) else {
                continue;
            };
            if !rule.enabled || !self.game_subscribed(game.appid, &rule.id) {
                continue;
            }
            if source.status == SourceMatchStatus::Matched {
                publisher_matched = true;
            }
            candidates.extend(source.candidates.iter().cloned());
        }
        if !publisher_matched {
            if let Some(fallback) = &record.fallback {
                let fallback_active = self.rules.iter().any(|rule| {
                    rule.role == SourceRole::FallbackIndexer
                        && rule.enabled
                        && self.game_subscribed(game.appid, &rule.id)
                });
                if matches!(
                    fallback.status,
                    FallbackMatchStatus::Matched | FallbackMatchStatus::Incomplete
                ) && fallback_active
                {
                    candidates.extend(fallback.candidates.iter().cloned());
                }
            }
        }
        let mut seen = HashSet::new();
        candidates
            .into_iter()
            .filter(|candidate| seen.insert(candidate.candidate_id.clone()))
            .collect()
    }

    /// Persisted match state projected as the wire `GameDiscovery` (six states).
    pub fn cached_discovery(&self, game: &GameQuery) -> GameDiscovery {
        let Some(record) = self.match_record(game.appid) else {
            return GameDiscovery {
                status: "pending".to_string(),
                candidates: Vec::new(),
                diagnostics: Vec::new(),
            };
        };
        let mut diagnostics: Vec<Diagnostic> = record
            .sources
            .values()
            .flat_map(|source| source.diagnostics.iter().cloned())
            .collect();
        if let Some(fallback) = &record.fallback {
            diagnostics.extend(fallback.diagnostics.iter().cloned());
        }
        GameDiscovery {
            status: match_status_name(record.status).to_string(),
            candidates: self.cached_candidates(game),
            diagnostics,
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

    /// Match-task query bounds (design §2.6): AppID/name/hint limits.
    pub fn validate_query(game: &GameQuery, name_hints: &[String]) -> Result<()> {
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
        Ok(())
    }
}

fn match_status_name(status: GameMatchStatus) -> &'static str {
    match status {
        GameMatchStatus::Pending => "pending",
        GameMatchStatus::PublisherMatch => "publisher_match",
        GameMatchStatus::FallbackMatch => "fallback_match",
        GameMatchStatus::NoMatch => "no_match",
        GameMatchStatus::Incomplete => "incomplete",
        GameMatchStatus::Failed => "failed",
    }
}

fn source_match_from_candidate(
    rule: &SourceRule,
    candidate: &SourceCandidate,
    appid: u32,
) -> Option<SourceMatch> {
    let url = candidate
        .mirrors
        .iter()
        .find(|mirror| matches!(mirror.format.as_str(), "zip" | "exe"))
        .map(|mirror| mirror.url.clone())?;
    Some(SourceMatch {
        title: candidate.patch_title.clone(),
        aliases: candidate.aliases.clone(),
        url,
        source_page: candidate.source_page.clone(),
        language: candidate.languages.first().cloned().unwrap_or_default(),
        date: candidate.published_at.clone(),
        appid: candidate.game_appid,
        source_id: rule.id.clone(),
        release_id: release_id_for(&rule.id, appid),
        root_layout: rule.root_layout,
    })
}

fn source_match_from_entry(
    rule: &SourceRule,
    entry: &SourceIndexEntry,
    url: &str,
    appid: u32,
) -> SourceMatch {
    SourceMatch {
        title: entry.title.clone(),
        aliases: entry.aliases.clone(),
        url: url.to_string(),
        source_page: entry.page.clone(),
        language: entry.language.clone().unwrap_or_default(),
        date: entry.date.clone(),
        appid: entry.appid,
        source_id: rule.id.clone(),
        release_id: release_id_for(&rule.id, appid),
        root_layout: rule.root_layout,
    }
}

/// Release identity shared with `installer::resolve::source_release_id`.
fn release_id_for(source_id: &str, appid: u32) -> String {
    if source_id == "patchmoe" {
        format!("patchmoe-{appid}")
    } else {
        format!("source-{source_id}-{appid}")
    }
}

/// Projects one validated index match into the shared dynamic ZIP candidate
/// shape (`P4 B2`). Extracted so the P3 matching engine derives identical
/// `candidate_id`/`fingerprint` values from a `SourceIndexEntry`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn candidate_from_index_match(
    source_id: &str,
    appid: u32,
    entry_appid: Option<u32>,
    title: &str,
    aliases: &[String],
    language: &str,
    source_page: &str,
    url: &str,
    observed_at_unix: u64,
) -> SourceCandidate {
    let language = language.to_string();
    let version_identity = "page-unknown".to_string();
    let identity = serde_json::Value::Array(vec![
        serde_json::Value::String(source_id.to_string()),
        serde_json::Value::String(canonical::canonical_url(source_page)),
        serde_json::Value::String("base".to_string()),
        serde_json::Value::Array(vec![serde_json::Value::String(language.clone())]),
        serde_json::Value::String(version_identity.clone()),
    ]);
    let candidate_id = canonical::candidate_id(&identity);
    let mut candidate = SourceCandidate {
        parser_fingerprint: None,
        candidate_id,
        source_id: source_id.to_string(),
        source_record_id: canonical::canonical_url(source_page),
        game_appid: Some(entry_appid.unwrap_or(appid)),
        parent_appid: None,
        aliases: aliases.to_vec(),
        game_title: title.to_string(),
        patch_title: title.to_string(),
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
        source_page: source_page.to_string(),
        upstream_page: None,
        mirrors: vec![Mirror {
            url: url.to_string(),
            label: None,
            format: "zip".to_string(),
        }],
        format_reported: ArtifactFormat::Zip,
        format_observed: ArtifactFormat::Zip,
        capability: Capability::DiscoverOnly,
        identity_evidence: Vec::new(),
        provenance: Vec::new(),
        match_basis: if entry_appid.is_some() {
            MatchBasis::Appid
        } else {
            MatchBasis::Title
        },
        observed_at: iso8601_utc(observed_at_unix),
        cache_status: CacheStatus::Cached,
        warnings: Vec::new(),
        fingerprint: String::new(),
        related_index_pages: Vec::new(),
    };
    if let Ok(value) = serde_json::to_value(&candidate) {
        candidate.fingerprint = canonical::fingerprint(&value);
    }
    candidate
}

/// ISO8601 UTC rendering of a Unix second count. Used only for
/// `SourceCandidate::observed_at` on index projections, which the fingerprint
/// deliberately excludes; determinism comes from the source index `checked_at`.
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
    use super::*;
    use crate::sources::store::SourceStore;
    use crate::sources::{validate, FakeClock, DEFAULT_RULES_JSON};
    use std::path::Path;

    fn builtins() -> Vec<SourceRule> {
        let config: RulesConfig = serde_json::from_str(DEFAULT_RULES_JSON).unwrap();
        config
            .sources
            .iter()
            .map(|raw| validate::validate_rule(raw).unwrap())
            .collect()
    }

    fn registry_at(dir: &Path) -> SourceRegistry {
        let store = SourceStore::open(dir).unwrap();
        SourceRegistry::load_with_clock(store, builtins(), Box::new(FakeClock::new(1_700_000_000)))
            .unwrap()
    }

    #[test]
    fn display_names_persist_without_changing_rules_or_subscriptions() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path());
        assert_eq!(registry.list_sources()[0].rule.name, "Shiravune");
        let mut raw: RawSourceRule = serde_json::from_value(serde_json::json!({
            "id": "custom", "name": "Custom", "adapter": "patchmoe", "enabled": true,
            "pages": ["https://www.patch.moe/"], "page_hosts": ["www.patch.moe"],
            "download_hosts": ["download.patch.moe"], "root_layout": "game_root"
        }))
        .unwrap();
        registry.add_source(&raw).unwrap();
        registry.set_enabled("patchmoe", false).unwrap();
        registry.set_game_source(42, "patchmoe", false).unwrap();
        registry.set_update_interval("patchmoe", 24).unwrap();
        let rules = registry.rules().to_vec();
        let names = [
            ("patchmoe", "我的发行商".repeat(80)),
            ("custom", "自定义来源".to_string()),
            ("hgamer", "补查来源".to_string()),
        ];
        for (id, name) in &names {
            assert_eq!(
                registry.set_name(id, &format!("  {name}  ")).unwrap(),
                *name
            );
        }
        assert_eq!(registry.rules(), rules);
        let revision = registry.settings_revision();
        drop(registry);
        // Simulate an app upgrade: compiled rules still update under a display name.
        raw.pages = vec!["https://www.patch.moe/updated".to_string()];
        let mut updated_builtins = builtins();
        updated_builtins[0].pages = raw.pages;
        let registry =
            SourceRegistry::load(SourceStore::open(temp.path()).unwrap(), updated_builtins)
                .unwrap();
        assert_eq!(registry.settings_revision(), revision);
        assert_eq!(registry.rules()[0].pages, ["https://www.patch.moe/updated"]);
        for (id, name) in &names {
            let source = registry
                .list_game_sources(&game(42))
                .into_iter()
                .find(|source| &source.source.rule.id == id)
                .unwrap();
            assert_eq!(&source.source.rule.name, name);
        }
        assert!(!registry.game_subscribed(42, "patchmoe"));
        let source = registry
            .list_sources()
            .into_iter()
            .find(|s| s.rule.id == "patchmoe")
            .unwrap();
        assert!(!source.rule.enabled);
        assert_eq!(source.interval_hours, 24);
    }

    #[test]
    fn invalid_display_name_does_not_write_settings_and_old_settings_load() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path());
        let before = std::fs::read(registry.store().paths().settings()).unwrap();
        assert_eq!(
            registry
                .set_name("missing", "Name")
                .unwrap_err()
                .to_string(),
            "unknown_source"
        );
        assert_eq!(
            registry
                .set_name("patchmoe", " \t\n ")
                .unwrap_err()
                .to_string(),
            "invalid_source_name"
        );
        assert_eq!(
            std::fs::read(registry.store().paths().settings()).unwrap(),
            before
        );
        let mut old: serde_json::Value = serde_json::from_slice(&before).unwrap();
        old.as_object_mut().unwrap().remove("names");
        std::fs::write(
            registry.store().paths().settings(),
            serde_json::to_vec(&old).unwrap(),
        )
        .unwrap();
        drop(registry);
        assert_eq!(
            registry_at(temp.path()).list_sources()[0].rule.name,
            "Shiravune"
        );
    }

    fn game(appid: u32) -> GameQuery {
        GameQuery {
            appid,
            name: "Game".to_string(),
            localized_name: None,
            instance_id: None,
        }
    }

    fn entry(title: &str, url: &str, appid: Option<u32>) -> SourceIndexEntry {
        SourceIndexEntry {
            title: title.to_string(),
            aliases: Vec::new(),
            page: "https://www.patch.moe/".to_string(),
            kind: "entry".to_string(),
            appid,
            url: Some(url.to_string()),
            language: Some("zh-CN".to_string()),
            date: None,
        }
    }

    fn index(entries: Vec<SourceIndexEntry>) -> SourceIndex {
        SourceIndex {
            parser_fingerprint: super::super::index::parser_fingerprint(
                &super::super::test_fixtures::default_rule("patchmoe"),
            ),
            schema_version: SOURCE_INDEX_SCHEMA_VERSION,
            source_id: "patchmoe".to_string(),
            revision: 3,
            complete: true,
            checked_at: 1_000,
            updated_at: 1_000,
            etag: None,
            modified: None,
            entries,
            pending: Vec::new(),
        }
    }

    fn candidate(source_id: &str, appid: u32) -> SourceCandidate {
        candidate_from_index_match(
            source_id,
            appid,
            Some(appid),
            "Game",
            &[],
            "zh-CN",
            "https://www.patch.moe/",
            "https://download.patch.moe/g.zip",
            1_000,
        )
    }

    fn save_match(
        registry: &SourceRegistry,
        appid: u32,
        sources: &[(&str, SourceMatchStatus, Vec<SourceCandidate>)],
        fallback: Option<FallbackMatchStatus>,
        status: GameMatchStatus,
    ) {
        let record = GameMatchRecord {
            schema_version: GAME_MATCH_SCHEMA_VERSION,
            appid,
            name: "Game".to_string(),
            localized_name: None,
            input_key: "key".to_string(),
            status,
            sources: sources
                .iter()
                .map(|(source_id, status, candidates)| {
                    (
                        (*source_id).to_string(),
                        SourceMatchRecord {
                            index_revision: 3,
                            status: *status,
                            candidates: candidates.clone(),
                            detail_pages: Vec::new(),
                            checked_at: 1_000,
                            diagnostics: Vec::new(),
                        },
                    )
                })
                .collect(),
            fallback: fallback.map(|status| FallbackMatchRecord {
                sources_fingerprint: Some(super::super::matching::fallback_sources_fingerprint(
                    registry.rules(),
                    registry.overrides(),
                    appid,
                )),
                query_key: "query".to_string(),
                status,
                candidates: if status == FallbackMatchStatus::Matched {
                    vec![candidate("hgamer", appid)]
                } else {
                    Vec::new()
                },
                checked_at: 1_000,
                diagnostics: Vec::new(),
            }),
            updated_at: 1_000,
        };
        super::super::test_fixtures::save_match(registry, record).unwrap();
    }

    #[test]
    fn list_sources_reads_index_status_and_schedule() {
        let temp = tempfile::tempdir().unwrap();
        let registry = registry_at(temp.path());
        registry
            .store()
            .save_index(&index(vec![entry(
                "Game",
                "https://download.patch.moe/g.zip",
                Some(42),
            )]))
            .unwrap();
        let summary = registry
            .list_sources()
            .into_iter()
            .find(|source| source.rule.id == "patchmoe")
            .unwrap();
        assert_eq!(summary.revision, 3);
        assert!(summary.complete);
        assert_eq!(summary.entry_count, 1);
        assert_eq!(summary.cached_at, Some(1_000));
        assert_eq!(summary.status, "cached");
        assert_eq!(summary.interval_hours, 0);
        assert_eq!(summary.next_due_at, None);

        let summary = registry
            .list_sources()
            .into_iter()
            .find(|source| source.rule.id == "kagura-cn")
            .unwrap();
        assert_eq!(summary.revision, 0);
        assert!(!summary.complete);
        assert_eq!(summary.status, "not_refreshed");
    }

    #[test]
    fn matches_fall_back_to_entry_index_before_the_first_match_task() {
        let temp = tempfile::tempdir().unwrap();
        let registry = registry_at(temp.path());
        registry
            .store()
            .save_index(&index(vec![entry(
                "Game",
                "https://download.patch.moe/g.zip",
                Some(42),
            )]))
            .unwrap();
        let matches = registry.matches(&game(42), false);
        assert_eq!(matches.len(), 1);
        let matched = &matches[0];
        assert_eq!(matched.source_id, "patchmoe");
        assert_eq!(matched.release_id, "patchmoe-42");
        assert_eq!(matched.url, "https://download.patch.moe/g.zip");
        assert_eq!(matched.language, "zh-CN");
        assert!(registry.matches(&game(43), false).is_empty());

        // A persisted match record wins over the index fallback.
        save_match(
            &registry,
            42,
            &[(
                "patchmoe",
                SourceMatchStatus::Matched,
                vec![candidate("patchmoe", 42)],
            )],
            None,
            GameMatchStatus::PublisherMatch,
        );
        let matches = registry.matches(&game(42), false);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].release_id, "patchmoe-42");
    }

    #[test]
    fn game_sources_and_discovery_read_the_persisted_match() {
        let temp = tempfile::tempdir().unwrap();
        let registry = registry_at(temp.path());
        save_match(
            &registry,
            42,
            &[(
                "kagura-cn",
                SourceMatchStatus::Matched,
                vec![candidate("kagura-cn", 42)],
            )],
            Some(FallbackMatchStatus::Matched),
            GameMatchStatus::PublisherMatch,
        );
        let sources = registry.list_game_sources(&game(42));
        let kagura = sources
            .iter()
            .find(|source| source.source.rule.id == "kagura-cn")
            .unwrap();
        assert!(kagura.matched);
        let hgamer = sources
            .iter()
            .find(|source| source.source.rule.id == "hgamer")
            .unwrap();
        assert!(hgamer.matched);
        let patchmoe = sources
            .iter()
            .find(|source| source.source.rule.id == "patchmoe")
            .unwrap();
        assert!(!patchmoe.matched);

        let discovery = registry.cached_discovery(&game(42));
        assert_eq!(discovery.status, "publisher_match");
        assert_eq!(discovery.candidates.len(), 1);
        assert_eq!(
            registry.cached_candidates(&game(42))[0].source_id,
            "kagura-cn"
        );
        // The positive fallback is not appended once a publisher confirmed.
        save_match(
            &registry,
            43,
            &[],
            Some(FallbackMatchStatus::Matched),
            GameMatchStatus::FallbackMatch,
        );
        assert_eq!(
            registry.cached_discovery(&game(43)).status,
            "fallback_match"
        );
    }

    #[test]
    fn index_candidates_project_entry_rows_only() {
        let temp = tempfile::tempdir().unwrap();
        let registry = registry_at(temp.path());
        registry
            .store()
            .save_index(&index(vec![
                entry("Game", "https://download.patch.moe/g.zip", Some(42)),
                SourceIndexEntry {
                    title: "Category".to_string(),
                    aliases: Vec::new(),
                    page: "https://www.patch.moe/category/".to_string(),
                    kind: "category".to_string(),
                    appid: None,
                    url: None,
                    language: None,
                    date: None,
                },
            ]))
            .unwrap();
        let candidates = registry.index_candidates(&game(42));
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].mirrors[0].url,
            "https://download.patch.moe/g.zip"
        );
        assert_eq!(candidates[0].game_appid, Some(42));
        assert!(registry.index_candidates(&game(43)).is_empty());
    }

    #[test]
    fn settings_mutations_bump_the_revision_and_reload_rules() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path());
        assert_eq!(registry.settings_revision(), 1);
        registry.set_enabled("kagura-cn", false).unwrap();
        assert_eq!(registry.settings_revision(), 2);
        assert!(
            !registry
                .rules()
                .iter()
                .find(|rule| rule.id == "kagura-cn")
                .unwrap()
                .enabled
        );
        registry.set_game_source(42, "kagura-cn", false).unwrap();
        assert!(!registry.game_subscribed(42, "kagura-cn"));
        registry.set_update_interval("patchmoe", 24).unwrap();
        assert_eq!(
            registry
                .list_sources()
                .into_iter()
                .find(|source| source.rule.id == "patchmoe")
                .unwrap()
                .interval_hours,
            24
        );
        assert_eq!(
            registry
                .set_update_interval("hgamer", 24)
                .unwrap_err()
                .to_string(),
            "source_not_indexable"
        );
    }

    #[test]
    fn preload_snapshot_rejects_corrupt_schema_versions() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = registry_at(temp.path());
        registry
            .store()
            .save_match(&GameMatchRecord {
                schema_version: 99,
                appid: 42,
                name: "Game".to_string(),
                localized_name: None,
                input_key: String::new(),
                status: GameMatchStatus::Pending,
                sources: BTreeMap::new(),
                fallback: None,
                updated_at: 0,
            })
            .unwrap();
        assert_eq!(
            registry
                .preload_snapshot_games(&[game(42)])
                .unwrap_err()
                .to_string(),
            "invalid_source_match"
        );
    }
}
