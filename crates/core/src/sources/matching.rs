//! P3 incremental game matching (design §3.2/§3.3, §4.2; p1-contract §7).
//!
//! One match pass never traverses a source's list pages: it reads the published
//! [`SourceIndex`] snapshots, filters them locally, resolves only the detail
//! pages of games that actually hit the index, and reuses the shared
//! [`DetailRecord`] cache across games and runs.
//!
//! Matching is a single-task serial pass; per-page detail fetches are bounded by
//! the number of index hits, not by a global crawler. Progress is reported
//! through explicit phases so the host never claims a network task is offline.

use super::adapters::kagura::{self, resolve_page};
use super::adapters::{self, DiscoverPrevious};
use super::canonical;
use super::http::RequestContext;
use super::identity;
use super::model::{
    DetailRecord, Diagnostic, FactLink, FallbackMatchRecord, FallbackMatchStatus, GameMatch,
    GameMatchRecord, GameMatchStatus, GameQuery, IdentityEvidence, MatchBasis, Overrides, PageFact,
    QueryContext, SourceCandidate, SourceIndex, SourceIndexEntry, SourceMatchRecord,
    SourceMatchStatus, SourceRole, SourceRule, Variant,
};
use super::r#match::{candidate_is_credible, derive_game_status, input_key};
use super::registry::candidate_from_index_match;
use super::store::SourceStore;
use super::validate;
use super::{
    invalid, rule_fingerprint, Result, GAME_MATCH_SCHEMA_VERSION, PARSER_VERSION, QUERY_VERSION,
    SOURCES_ENCODING_VERSION, SOURCE_DETAIL_SCHEMA_VERSION,
};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Non-wire progress signal for the host's `match_progress` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchProgress {
    /// `index` | `details` | `fallback`.
    pub phase: String,
    pub completed: usize,
    pub total: usize,
    pub running: bool,
}

// ---- public entry points ------------------------------------------------

/// Matches a batch of installed games incrementally.
///
/// `name_hints`/`preferred_language` are the untrusted frontend query inputs
/// merged into the per-game [`QueryContext`] and the matching-input digest.
/// Games whose persisted record still matches the current matching inputs and
/// source revisions are returned from disk without any network request. The
/// same AppID under several instances is matched once; `instance_id` in the
/// result is the first input instance.
#[allow(clippy::too_many_arguments)]
pub fn match_games(
    store: &SourceStore,
    rules: &[SourceRule],
    overrides: &Overrides,
    games: &[GameQuery],
    name_hints: &[String],
    preferred_language: Option<&str>,
    force_details: bool,
    ctx: &mut RequestContext,
    cancel: &AtomicBool,
    now: u64,
    mut on_progress: impl FnMut(MatchProgress),
    mut on_update: impl FnMut(&GameMatch),
) -> Result<Vec<GameMatch>> {
    let games = dedupe_games(games);
    let total = games.len();
    emit(&mut on_progress, "index", 0, total, true);
    let mut results = Vec::with_capacity(total);
    for (index, game) in games.iter().enumerate() {
        if cancelled(cancel) {
            finish_progress(&mut on_progress, results.len(), total);
            return Err(cancelled_error());
        }
        emit(&mut on_progress, "index", index, total, true);
        let previous = store.load_match(game.appid)?;
        let reused = !force_details
            && match &previous {
                Some(record) => record_reusable(
                    store,
                    record,
                    game,
                    name_hints,
                    preferred_language,
                    rules,
                    overrides,
                )?,
                None => false,
            };
        if reused {
            let mut summary = summary_from_record(previous.as_ref().expect("reused record"));
            summary.instance_id = game.instance_id.clone();
            results.push(summary);
        } else {
            let record = {
                let mut on_phase = |phase: &str| emit(&mut on_progress, phase, index, total, true);
                match_one(
                    store,
                    rules,
                    overrides,
                    game,
                    name_hints,
                    preferred_language,
                    previous.as_ref(),
                    MatchOptions {
                        force_details,
                        force_fallback: false,
                        only_source: None,
                    },
                    ctx,
                    cancel,
                    now,
                    &mut on_phase,
                )
            };
            let record = match record {
                Ok(record) => record,
                Err(error) => {
                    finish_progress(&mut on_progress, results.len(), total);
                    return Err(error);
                }
            };
            let mut summary = summary_from_record(&record);
            summary.instance_id = game.instance_id.clone();
            on_update(&summary);
            results.push(summary);
        }
        emit(&mut on_progress, "index", index + 1, total, true);
        emit(&mut on_progress, "details", index + 1, total, true);
        emit(&mut on_progress, "fallback", index + 1, total, true);
    }
    finish_progress(&mut on_progress, total, total);
    Ok(results)
}

/// Re-matches one game: recomputes every publisher source, forces its hit
/// details to be refreshed, and actively re-queries HGamer when no credible
/// publisher match remains.
#[allow(clippy::too_many_arguments)]
pub fn rematch_game(
    store: &SourceStore,
    rules: &[SourceRule],
    overrides: &Overrides,
    game: &GameQuery,
    name_hints: &[String],
    preferred_language: Option<&str>,
    ctx: &mut RequestContext,
    cancel: &AtomicBool,
    now: u64,
    mut on_update: impl FnMut(&GameMatch),
) -> Result<GameMatch> {
    let previous = store.load_match(game.appid)?;
    let record = match_one(
        store,
        rules,
        overrides,
        game,
        name_hints,
        preferred_language,
        previous.as_ref(),
        MatchOptions {
            force_details: true,
            force_fallback: true,
            only_source: None,
        },
        ctx,
        cancel,
        now,
        &mut |_| {},
    )?;
    let mut summary = summary_from_record(&record);
    summary.instance_id = game.instance_id.clone();
    on_update(&summary);
    Ok(summary)
}

/// Re-matches only `source_id` after that source's index changed.
///
/// Other sources' records are preserved, the target source's hit details are
/// refreshed, and an existing terminal HGamer result is reused (only a source
/// without any result yet is queried). No source index is written here.
#[allow(clippy::too_many_arguments)]
pub fn rematch_source(
    store: &SourceStore,
    rules: &[SourceRule],
    overrides: &Overrides,
    games: &[GameQuery],
    source_id: &str,
    name_hints: &[String],
    preferred_language: Option<&str>,
    ctx: &mut RequestContext,
    cancel: &AtomicBool,
    now: u64,
    mut on_update: impl FnMut(&GameMatch),
) -> Result<Vec<GameMatch>> {
    let rule = rules
        .iter()
        .find(|rule| rule.id == source_id)
        .ok_or_else(|| invalid("unknown_source"))?;
    if rule.role == SourceRole::FallbackIndexer {
        return Err(invalid("unknown_source"));
    }
    let games = dedupe_games(games);
    let mut results = Vec::with_capacity(games.len());
    for game in &games {
        if cancelled(cancel) {
            return Err(cancelled_error());
        }
        let previous = store.load_match(game.appid)?;
        let record = match_one(
            store,
            rules,
            overrides,
            game,
            name_hints,
            preferred_language,
            previous.as_ref(),
            MatchOptions {
                force_details: true,
                force_fallback: false,
                only_source: Some(source_id),
            },
            ctx,
            cancel,
            now,
            &mut |_| {},
        )?;
        let mut summary = summary_from_record(&record);
        summary.instance_id = game.instance_id.clone();
        on_update(&summary);
        results.push(summary);
    }
    Ok(results)
}

/// Pure read of a persisted match record (`matches/<appid>.json`), no network
/// and no writes. A missing record is `pending` with empty candidates.
pub fn game_match_summary(store: &SourceStore, appid: u32) -> GameMatch {
    match store.load_match(appid) {
        Ok(Some(record)) => summary_from_record(&record),
        _ => GameMatch {
            appid,
            instance_id: None,
            status: GameMatchStatus::Pending,
            candidates: Vec::new(),
            diagnostics: Vec::new(),
            source_revisions: BTreeMap::new(),
        },
    }
}

/// Read-only persisted-match lookup that never creates or migrates source
/// state. Returns `None` when the source namespace does not exist yet, and a
/// `pending` summary when the namespace exists but the game was never matched.
pub fn match_summary_at(state_dir: &Path, appid: u32) -> Result<Option<GameMatch>> {
    let paths = super::store::StorePaths::new(state_dir);
    match std::fs::symlink_metadata(paths.root()) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    let store = SourceStore::open_readonly(state_dir)?;
    Ok(Some(game_match_summary(&store, appid)))
}

/// Current published revision of every non-fallback source (`0` when the
/// source has no index snapshot yet).
pub fn source_revisions(store: &SourceStore, rules: &[SourceRule]) -> BTreeMap<String, u64> {
    let mut revisions = BTreeMap::new();
    for rule in rules {
        if rule.role == SourceRole::FallbackIndexer {
            continue;
        }
        let revision = store
            .load_index(&rule.id)
            .ok()
            .flatten()
            .map(|index| index.revision)
            .unwrap_or(0);
        revisions.insert(rule.id.clone(), revision);
    }
    revisions
}

// ---- options / per-game orchestration -----------------------------------

#[derive(Debug, Clone, Copy)]
struct MatchOptions<'a> {
    force_details: bool,
    force_fallback: bool,
    only_source: Option<&'a str>,
}

#[allow(clippy::too_many_arguments)]
fn match_one(
    store: &SourceStore,
    rules: &[SourceRule],
    overrides: &Overrides,
    game: &GameQuery,
    name_hints: &[String],
    preferred_language: Option<&str>,
    previous: Option<&GameMatchRecord>,
    options: MatchOptions<'_>,
    ctx: &mut RequestContext,
    cancel: &AtomicBool,
    now: u64,
    on_phase: &mut dyn FnMut(&str),
) -> Result<GameMatchRecord> {
    let publishers = active_publishers(rules, overrides, game.appid);
    let fallback_rule = active_fallback(rules, overrides, game.appid);
    let publisher_ids: Vec<String> = publishers.iter().map(|rule| rule.id.clone()).collect();
    let hints = merged_hints(game, name_hints);
    let query = query_context(
        game,
        &hints,
        preferred_language,
        options.force_details,
        publisher_ids,
    );
    let subscriptions = subscriptions_snapshot(rules, overrides, game.appid);
    let key = input_key(
        &game.name,
        game.localized_name.as_deref(),
        &hints,
        preferred_language,
        &subscriptions,
    );
    // The host writes settings from another process; `load_settings` reads the
    // persisted revision so a mid-task disable/unsubscribe is detected below.
    let settings_revision = store.load_settings()?.revision;

    let mut indexes: BTreeMap<String, Option<SourceIndex>> = BTreeMap::new();
    for rule in &publishers {
        indexes.insert(rule.id.clone(), store.load_index(&rule.id)?);
    }

    // Candidates already built for this game (previous run) feed the DLC/lead
    // rules while this pass builds new ones.
    let mut known: Vec<SourceCandidate> = previous
        .map(|record| {
            record
                .sources
                .values()
                .flat_map(|source| source.candidates.iter().cloned())
                .collect()
        })
        .unwrap_or_default();

    if fallback_rule.is_some() {
        if let Some(fallback) = previous.and_then(|record| record.fallback.as_ref()) {
            if fallback.status == FallbackMatchStatus::Matched {
                known.extend(fallback.candidates.iter().cloned());
            }
        }
    }

    let mut sources: BTreeMap<String, SourceMatchRecord> = BTreeMap::new();
    let mut targets: Vec<&SourceRule> = Vec::new();
    match options.only_source {
        Some(target) => {
            let target_rule = publishers
                .iter()
                .find(|rule| rule.id == target)
                .copied()
                .ok_or_else(|| invalid("unknown_source"))?;
            targets.push(target_rule);
            // Preserve the other sources' records (only active sources survive
            // a disabled/unsubscribed source).
            if let Some(previous) = previous {
                for (source_id, record) in &previous.sources {
                    if source_id != target && publishers.iter().any(|rule| rule.id == *source_id) {
                        sources.insert(source_id.clone(), record.clone());
                    }
                }
            }
        }
        None => targets.extend(publishers.iter().copied()),
    }

    on_phase("details");
    for rule in targets {
        if cancelled(cancel) {
            return Err(cancelled_error());
        }
        let previous_source = previous.and_then(|record| record.sources.get(&rule.id));
        let source = match_publisher_source(
            store,
            rule,
            indexes.get(&rule.id).and_then(Option::as_ref),
            game,
            &query,
            previous_source,
            options.force_details,
            ctx,
            cancel,
            now,
            &mut known,
        )?;
        known.extend(source.candidates.iter().cloned());
        sources.insert(rule.id.clone(), source);
    }

    let publisher_matched = sources
        .values()
        .any(|source| source.status == SourceMatchStatus::Matched);
    let coverage_complete = !publishers.is_empty()
        && publishers.iter().all(|rule| {
            indexes
                .get(&rule.id)
                .and_then(Option::as_ref)
                .is_some_and(|index| index.complete)
        });

    on_phase("fallback");
    let known_before_fallback = known.len();
    let fallback = decide_fallback(
        &query,
        previous,
        &known,
        fallback_rule,
        !publishers.is_empty(),
        coverage_complete,
        publisher_matched,
        options.force_fallback,
        ctx,
        cancel,
        now,
    )?;

    // HGamer can identify an indexed publisher article whose title differs
    // from Steam. Resolve only that index hit, never crawl an upstream site.
    if !publisher_matched {
        if let Some(fallback) = &fallback {
            if fallback.status == FallbackMatchStatus::Matched {
                known.extend(fallback.candidates.iter().cloned());
                for rule in &publishers {
                    if options.only_source.is_some_and(|target| target != rule.id) {
                        continue;
                    }
                    let index = indexes.get(&rule.id).and_then(Option::as_ref);
                    if !index.is_some_and(|index| {
                        index.entries.iter().any(|entry| {
                            entry.kind != "entry"
                                && entry.kind != "category"
                                && upstream_identity(&entry.page, query.appid, &known).is_some()
                                && upstream_identity(
                                    &entry.page,
                                    query.appid,
                                    &known[..known_before_fallback],
                                )
                                .is_none()
                        })
                    }) {
                        continue;
                    }
                    let source = match_publisher_source(
                        store,
                        rule,
                        index,
                        game,
                        &query,
                        sources.get(&rule.id),
                        options.force_details,
                        ctx,
                        cancel,
                        now,
                        &mut known,
                    )?;
                    sources.insert(rule.id.clone(), source);
                }
            }
        }
    }

    let mut record = GameMatchRecord {
        schema_version: GAME_MATCH_SCHEMA_VERSION,
        appid: game.appid,
        name: game.name.clone(),
        localized_name: game.localized_name.clone(),
        input_key: key.clone(),
        status: GameMatchStatus::Pending,
        sources,
        fallback,
        updated_at: now,
    };
    record.status = derive_game_status(&record);
    // Commit-time binding (design §4.3, p1-contract §7.2): if the persisted
    // matching inputs changed while details/fallback were being fetched, do not
    // write this attempt and keep the previous record for the next pass.
    if settings_binding_changed(
        store,
        settings_revision,
        &key,
        game,
        name_hints,
        preferred_language,
        rules,
    )? {
        return Ok(previous
            .cloned()
            .unwrap_or_else(|| pending_record(game, key, now)));
    }
    store.save_match(&record)?;
    Ok(record)
}

/// True when the persisted settings revision changed during the task and the
/// game's matching-input digest (subscriptions/enabled/name inputs) changed with
/// it. Unrelated settings changes (for example another source's interval) keep
/// `false`, so they never discard a completed match.
fn settings_binding_changed(
    store: &SourceStore,
    captured_revision: u64,
    key: &str,
    game: &GameQuery,
    name_hints: &[String],
    preferred_language: Option<&str>,
    rules: &[SourceRule],
) -> Result<bool> {
    let settings = store.load_settings()?;
    if settings.revision == captured_revision {
        return Ok(false);
    }
    let overrides = Overrides {
        enabled: settings.enabled,
        added: settings.added,
        games: settings.games,
    };
    let hints = merged_hints(game, name_hints);
    let subscriptions = subscriptions_snapshot(rules, &overrides, game.appid);
    Ok(input_key(
        &game.name,
        game.localized_name.as_deref(),
        &hints,
        preferred_language,
        &subscriptions,
    ) != key)
}

/// Record returned when a first match attempt is discarded by the commit-time
/// binding: the game stays pending until the next match reads the new settings.
fn pending_record(game: &GameQuery, key: String, now: u64) -> GameMatchRecord {
    GameMatchRecord {
        schema_version: GAME_MATCH_SCHEMA_VERSION,
        appid: game.appid,
        name: game.name.clone(),
        localized_name: game.localized_name.clone(),
        input_key: key,
        status: GameMatchStatus::Pending,
        sources: BTreeMap::new(),
        fallback: None,
        updated_at: now,
    }
}

/// Whether the persisted record can be returned without recomputation
/// (p1-contract §7.2): same matching inputs, same source set/revisions, and a
/// terminal fallback result.
#[allow(clippy::too_many_arguments)]
fn record_reusable(
    store: &SourceStore,
    record: &GameMatchRecord,
    game: &GameQuery,
    name_hints: &[String],
    preferred_language: Option<&str>,
    rules: &[SourceRule],
    overrides: &Overrides,
) -> Result<bool> {
    let publishers = active_publishers(rules, overrides, game.appid);
    let publisher_ids: Vec<String> = publishers.iter().map(|rule| rule.id.clone()).collect();
    let hints = merged_hints(game, name_hints);
    let subscriptions = subscriptions_snapshot(rules, overrides, game.appid);
    if record.input_key
        != input_key(
            &game.name,
            game.localized_name.as_deref(),
            &hints,
            preferred_language,
            &subscriptions,
        )
    {
        return Ok(false);
    }
    let active: BTreeSet<&str> = publisher_ids.iter().map(String::as_str).collect();
    let stored: BTreeSet<&str> = record.sources.keys().map(String::as_str).collect();
    if active != stored {
        return Ok(false);
    }
    for rule in &publishers {
        let revision = store
            .load_index(&rule.id)?
            .map(|index| index.revision)
            .unwrap_or(0);
        let stored = record
            .sources
            .get(&rule.id)
            .map(|source| source.index_revision)
            .unwrap_or(0);
        if stored != revision {
            return Ok(false);
        }
    }
    let fallback_rule = active_fallback(rules, overrides, game.appid);
    let coverage_complete = !publishers.is_empty()
        && publishers.iter().all(|rule| {
            store
                .load_index(&rule.id)
                .map(|index| index.is_some_and(|index| index.complete))
                .unwrap_or(false)
        });
    let publisher_matched = record
        .sources
        .values()
        .any(|source| source.status == SourceMatchStatus::Matched);
    let fallback_terminal = match &record.fallback {
        None => false,
        Some(fallback) => match fallback.status {
            FallbackMatchStatus::Matched | FallbackMatchStatus::NoMatch => fallback_rule
                .is_some_and(|rule| {
                    let query = query_context(
                        game,
                        &hints,
                        preferred_language,
                        false,
                        publisher_ids.clone(),
                    );
                    fallback.query_key == fallback_query_key(&query, rule)
                }),
            FallbackMatchStatus::Disabled => fallback_rule.is_none() && !publishers.is_empty(),
            FallbackMatchStatus::NotNeeded => publisher_matched,
            FallbackMatchStatus::NotQueried => !coverage_complete,
            FallbackMatchStatus::Incomplete | FallbackMatchStatus::Failed => false,
        },
    };
    Ok(fallback_terminal)
}

// ---- publisher source matching -----------------------------------------

#[allow(clippy::too_many_arguments)]
fn match_publisher_source(
    store: &SourceStore,
    rule: &SourceRule,
    index: Option<&SourceIndex>,
    game: &GameQuery,
    query: &QueryContext,
    previous: Option<&SourceMatchRecord>,
    force_details: bool,
    ctx: &mut RequestContext,
    cancel: &AtomicBool,
    now: u64,
    known: &mut Vec<SourceCandidate>,
) -> Result<SourceMatchRecord> {
    let Some(index) = index else {
        // Without a published snapshot the source claims no coverage; any
        // previous candidates stay as evidence but never confirm a match.
        return Ok(SourceMatchRecord {
            index_revision: 0,
            status: SourceMatchStatus::NoIndex,
            candidates: previous
                .map(|source| source.candidates.clone())
                .unwrap_or_default(),
            detail_pages: Vec::new(),
            checked_at: now,
            diagnostics: Vec::new(),
        });
    };

    let mut candidates: Vec<SourceCandidate> = Vec::new();
    let mut detail_pages: Vec<String> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut detail_failed = false;

    // List-page (patch.moe / table) rows are matched entirely offline.
    if index.entries.iter().any(|entry| entry.kind == "entry") {
        let (found, found_diagnostics) = match_entry_rows(rule, index, game);
        candidates.extend(found);
        diagnostics.extend(found_diagnostics);
    }

    // Kagura articles/products/categories need their hit pages.
    let needs_pages = index.entries.iter().any(|entry| entry.kind != "entry");
    if needs_pages && !candidates.iter().any(candidate_is_credible) {
        let adapter = kagura::adapter_name(rule);
        let found = match_kagura_entries(
            store,
            rule,
            adapter,
            index,
            query,
            force_details,
            ctx,
            cancel,
            now,
            known,
            &mut detail_pages,
            &mut diagnostics,
            &mut detail_failed,
        )?;
        candidates.extend(found);
    }

    let mut validated = merge_and_validate(rule, candidates, &mut diagnostics);
    // The current attempt's own outcome decides the status; a retained candidate
    // from an earlier success must not upgrade a failed refresh to `matched`.
    let status = publisher_status(index, &validated, detail_failed);
    if detail_failed {
        // A failed detail refresh must not erase previously confirmed results.
        if let Some(previous) = previous {
            for candidate in &previous.candidates {
                if !validated
                    .iter()
                    .any(|seen| seen.candidate_id == candidate.candidate_id)
                {
                    validated.push(candidate.clone());
                }
            }
        }
    }
    Ok(SourceMatchRecord {
        index_revision: index.revision,
        status,
        candidates: validated,
        detail_pages,
        checked_at: now,
        diagnostics,
    })
}

fn publisher_status(
    index: &SourceIndex,
    candidates: &[SourceCandidate],
    detail_failed: bool,
) -> SourceMatchStatus {
    if candidates.iter().any(candidate_is_credible) {
        return SourceMatchStatus::Matched;
    }
    if detail_failed {
        return SourceMatchStatus::DetailIncomplete;
    }
    if !index.complete {
        return SourceMatchStatus::IndexIncomplete;
    }
    SourceMatchStatus::NoCandidate
}

fn merge_and_validate(
    rule: &SourceRule,
    candidates: Vec<SourceCandidate>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<SourceCandidate> {
    let mut validated = Vec::with_capacity(candidates.len());
    for candidate in identity::merge_candidates(&candidates) {
        match identity::validate_candidate(&candidate, rule) {
            Ok(()) => validated.push(candidate),
            Err(error) => diagnostics.push(Diagnostic {
                source_id: Some(rule.id.clone()),
                code: error.to_string(),
                detail: None,
            }),
        }
    }
    validated
}

// ---- list-page (kind = entry) matching ---------------------------------

/// Deterministic result of matching one game against a source's `kind="entry"`
/// index rows. Shared with the read-model `registry.matches` fallback.
pub(crate) enum EntrySelection<'a> {
    /// No entry carries the AppID or an exact title/alias.
    None,
    /// One artifact URL is selected; the entry is the language-preferred row.
    Selected(&'a SourceIndexEntry, &'a str),
    /// Several distinct artifact URLs: never auto-select.
    Ambiguous,
}

/// Matches `kind = "entry"` index rows: AppID first, then title/alias equality
/// over the manifest and localized names, then a deterministic ZIP/EXE URL.
pub(crate) fn select_entry<'a>(index: &'a SourceIndex, game: &GameQuery) -> EntrySelection<'a> {
    let mut targets: HashSet<String> = HashSet::new();
    for name in std::iter::once(&game.name).chain(game.localized_name.as_ref()) {
        let normal = validate::normal(name);
        if !normal.is_empty() {
            targets.insert(normal);
        }
    }
    let mut matched: Vec<&SourceIndexEntry> = Vec::new();
    for entry in &index.entries {
        if entry.kind != "entry" {
            continue;
        }
        let by_appid = entry.appid == Some(game.appid);
        let by_title = entry.appid.is_none()
            && !targets.is_empty()
            && entry_names(entry).iter().any(|name| targets.contains(name));
        if by_appid || by_title {
            matched.push(entry);
        }
    }
    if matched.is_empty() {
        return EntrySelection::None;
    }
    let mut urls: Vec<&str> = matched
        .iter()
        .filter_map(|entry| entry.url.as_deref())
        .collect();
    urls.sort_unstable();
    urls.dedup();
    if urls.len() > 1 {
        return EntrySelection::Ambiguous;
    }
    let Some(url) = urls.first().copied() else {
        return EntrySelection::None;
    };
    let selected = matched
        .iter()
        .filter(|entry| entry.url.as_deref() == Some(url))
        .find(|entry| {
            entry.language.as_deref().is_some_and(|language| {
                matches!(language.to_lowercase().as_str(), "zh-cn" | "zh_cn")
            })
        })
        .or_else(|| {
            matched
                .iter()
                .find(|entry| entry.url.as_deref() == Some(url))
        });
    match selected {
        Some(selected) => EntrySelection::Selected(selected, url),
        None => EntrySelection::None,
    }
}

fn match_entry_rows(
    rule: &SourceRule,
    index: &SourceIndex,
    game: &GameQuery,
) -> (Vec<SourceCandidate>, Vec<Diagnostic>) {
    match select_entry(index, game) {
        EntrySelection::None => (Vec::new(), Vec::new()),
        EntrySelection::Ambiguous => (
            Vec::new(),
            vec![Diagnostic {
                source_id: Some(rule.id.clone()),
                code: "source_match_ambiguous".to_string(),
                detail: Some("multiple artifact urls; manual selection required".to_string()),
            }],
        ),
        EntrySelection::Selected(selected, url) => {
            let candidate = candidate_from_index_match(
                &rule.id,
                game.appid,
                selected.appid,
                &selected.title,
                &selected.aliases,
                selected.language.as_deref().unwrap_or(""),
                &selected.page,
                url,
                index.checked_at,
            );
            (vec![candidate], Vec::new())
        }
    }
}

fn entry_names(entry: &SourceIndexEntry) -> Vec<String> {
    std::iter::once(&entry.title)
        .chain(entry.aliases.iter())
        .flat_map(|name| validate::split_names(name))
        .map(|name| validate::normal(&name))
        .filter(|name| !name.is_empty())
        .collect()
}

// ---- Kagura detail-page matching ---------------------------------------

#[allow(clippy::too_many_arguments)]
fn match_kagura_entries(
    store: &SourceStore,
    rule: &SourceRule,
    adapter: &str,
    index: &SourceIndex,
    query: &QueryContext,
    force_details: bool,
    ctx: &mut RequestContext,
    cancel: &AtomicBool,
    now: u64,
    known: &mut Vec<SourceCandidate>,
    detail_pages: &mut Vec<String>,
    diagnostics: &mut Vec<Diagnostic>,
    detail_failed: &mut bool,
) -> Result<Vec<SourceCandidate>> {
    let mut candidates: Vec<SourceCandidate> = Vec::new();

    // Non-DLC titles first so a base candidate exists before a short DLC title
    // is judged against it.
    let mut direct: Vec<&SourceIndexEntry> = index
        .entries
        .iter()
        .filter(|entry| {
            entry.kind != "entry" && entry.kind != "category" && entry_relevant(entry, query, known)
        })
        .collect();
    direct.sort_by_key(|entry| identity::variant(&entry.title) == Variant::Dlc);
    for entry in direct {
        if cancelled(cancel) {
            return Err(cancelled_error());
        }
        match resolve_fact(
            store,
            rule,
            adapter,
            index.revision,
            &entry.page,
            force_details,
            ctx,
            cancel,
            now,
            detail_pages,
        ) {
            Ok(fact) => {
                let found = candidates_from_fact(rule, &fact, query, known);
                known.extend(found.iter().cloned());
                candidates.extend(found);
            }
            Err(error) => {
                if cancelled(cancel) || error.to_string() == "task_cancelled" {
                    return Err(cancelled_error());
                }
                *detail_failed = true;
                diagnostics.push(diagnostic(rule, error.to_string()));
            }
        }
    }
    if candidates.iter().any(candidate_is_credible) {
        return Ok(candidates);
    }

    let mut categories: Vec<&SourceIndexEntry> = index
        .entries
        .iter()
        .filter(|entry| entry.kind == "category" && entry_relevant(entry, query, known))
        .collect();
    categories.sort_by_key(|entry| identity::variant(&entry.title) == Variant::Dlc);
    for entry in categories {
        process_category(
            store,
            rule,
            adapter,
            index.revision,
            &entry.page,
            query,
            force_details,
            ctx,
            cancel,
            now,
            known,
            detail_pages,
            diagnostics,
            detail_failed,
            &mut candidates,
        )?;
    }
    Ok(candidates)
}

/// Resolves a category page and every article it points at, following its
/// `next_page` pagination. Inline article bodies become candidates directly;
/// linked articles are resolved on demand.
#[allow(clippy::too_many_arguments)]
fn process_category(
    store: &SourceStore,
    rule: &SourceRule,
    adapter: &str,
    index_revision: u64,
    page: &str,
    query: &QueryContext,
    force_details: bool,
    ctx: &mut RequestContext,
    cancel: &AtomicBool,
    now: u64,
    known: &mut Vec<SourceCandidate>,
    detail_pages: &mut Vec<String>,
    diagnostics: &mut Vec<Diagnostic>,
    detail_failed: &mut bool,
    candidates: &mut Vec<SourceCandidate>,
) -> Result<()> {
    let mut queue: VecDeque<String> = VecDeque::from([canonical::canonical_url(page)]);
    let mut visited: HashSet<String> = HashSet::new();
    while let Some(page) = queue.pop_front() {
        if !visited.insert(page.clone()) {
            diagnostics.push(diagnostic(rule, "discovery_pagination_cycle"));
            continue;
        }
        if cancelled(cancel) {
            return Err(cancelled_error());
        }
        let fact = match resolve_fact(
            store,
            rule,
            adapter,
            index_revision,
            &page,
            force_details,
            ctx,
            cancel,
            now,
            detail_pages,
        ) {
            Ok(fact) => fact,
            Err(error) => {
                if cancelled(cancel) || error.to_string() == "task_cancelled" {
                    return Err(cancelled_error());
                }
                *detail_failed = true;
                diagnostics.push(diagnostic(rule, error.to_string()));
                continue;
            }
        };

        let inline_pages: HashSet<String> = fact
            .article_facts
            .iter()
            .map(|article| canonical::canonical_url(&article.page))
            .collect();
        let mut inline = fact.article_facts.clone();
        inline.sort_by_key(|article| identity::variant(&article.title) == Variant::Dlc);
        for article in inline {
            save_inline_fact(store, rule, index_revision, &article, now)?;
            let found = candidates_from_fact(rule, &article, query, known);
            known.extend(found.iter().cloned());
            candidates.extend(found);
        }

        let mut links: Vec<&FactLink> = fact
            .article_links
            .iter()
            .filter(|link| !inline_pages.contains(&canonical::canonical_url(&link.page)))
            .collect();
        links.sort_by_key(|link| identity::variant(&link.title) == Variant::Dlc);
        for link in links {
            if !lead_relevant(&link.title, query, known) {
                continue;
            }
            if cancelled(cancel) {
                return Err(cancelled_error());
            }
            match resolve_fact(
                store,
                rule,
                adapter,
                index_revision,
                &link.page,
                force_details,
                ctx,
                cancel,
                now,
                detail_pages,
            ) {
                Ok(article) => {
                    let found = candidates_from_fact(rule, &article, query, known);
                    known.extend(found.iter().cloned());
                    candidates.extend(found);
                }
                Err(error) => {
                    if cancelled(cancel) || error.to_string() == "task_cancelled" {
                        return Err(cancelled_error());
                    }
                    *detail_failed = true;
                    diagnostics.push(diagnostic(rule, error.to_string()));
                }
            }
        }

        if let Some(next) = &fact.next_page {
            let next = canonical::canonical_url(next);
            if !visited.contains(&next) {
                queue.push_back(next);
            } else {
                diagnostics.push(diagnostic(rule, "discovery_pagination_cycle"));
            }
        }
    }
    Ok(())
}

/// Loads a query-independent page fact from the shared detail cache, or resolves
/// and persists it. Cancellation is checked before every network request.
#[allow(clippy::too_many_arguments)]
fn resolve_fact(
    store: &SourceStore,
    rule: &SourceRule,
    adapter: &str,
    index_revision: u64,
    page: &str,
    force_details: bool,
    ctx: &mut RequestContext,
    cancel: &AtomicBool,
    now: u64,
    detail_pages: &mut Vec<String>,
) -> Result<PageFact> {
    let page = canonical::canonical_url(page);
    if !detail_pages.contains(&page) {
        detail_pages.push(page.clone());
    }
    if !force_details {
        if let Some(detail) = store.load_detail(&rule.id, &page)? {
            if detail.index_revision == index_revision && detail.complete {
                if let Some(fact) = detail.fact {
                    return Ok(fact);
                }
            }
        }
    }
    if cancelled(cancel) {
        return Err(cancelled_error());
    }
    let fact = resolve_page(rule, adapter, &page, ctx)?;
    store.save_detail(&DetailRecord {
        schema_version: SOURCE_DETAIL_SCHEMA_VERSION,
        source_id: rule.id.clone(),
        page: page.clone(),
        index_revision,
        checked_at: now,
        complete: true,
        fact: Some(fact.clone()),
        diagnostics: Vec::new(),
    })?;
    Ok(fact)
}

/// Persists an inline article body rendered inside a category page, so the next
/// game (or run) reuses it without a request.
fn save_inline_fact(
    store: &SourceStore,
    rule: &SourceRule,
    index_revision: u64,
    fact: &PageFact,
    now: u64,
) -> Result<()> {
    store.save_detail(&DetailRecord {
        schema_version: SOURCE_DETAIL_SCHEMA_VERSION,
        source_id: rule.id.clone(),
        page: canonical::canonical_url(&fact.page),
        index_revision,
        checked_at: now,
        complete: true,
        fact: Some(fact.clone()),
        diagnostics: Vec::new(),
    })
}

/// Builds candidates from one resolved page fact, porting `lead_relevant` /
/// `candidates_from_fact` without changing the old `discover()` behavior.
fn candidates_from_fact(
    rule: &SourceRule,
    fact: &PageFact,
    query: &QueryContext,
    known: &[SourceCandidate],
) -> Vec<SourceCandidate> {
    let mut related_query = query.clone();
    let title_relevant = identity::title_relevant(&fact.title, query);
    let upstream = upstream_identity(&fact.page, query.appid, known)
        .filter(|candidate| candidate.variant == identity::variant(&fact.title));
    let known_association = known.iter().any(|candidate| {
        candidate.source_page == fact.page
            && candidate.patch_title == fact.title
            && candidate
                .game_appid
                .is_none_or(|appid| appid == query.appid)
    });
    let base_link = fact.body_links.iter().any(|url| {
        let url = canonical::canonical_url(url);
        known
            .iter()
            .any(|candidate| candidate.variant == Variant::Base && candidate.source_page == url)
    });
    // A short DLC title is only promoted to a query hint when it either is a
    // title lead itself or explicitly links to a confirmed base page.
    if upstream.is_some()
        || known_association
        || (lead_relevant(&fact.title, query, known) && (title_relevant || base_link))
    {
        related_query.name_hints.push(fact.title.clone());
    }
    let mut found = identity::build_candidates(
        rule,
        &fact.title,
        &fact.page,
        &fact.text,
        &fact.steam_ids,
        &fact.aliases,
        "unknown",
        &fact.mirrors,
        &related_query,
    );
    for candidate in &mut found {
        if candidate.game_appid.is_none() {
            if let Some(upstream) = upstream {
                candidate.game_appid = Some(query.appid);
                candidate.match_basis = MatchBasis::VerifiedAlias;
                candidate.identity_evidence.push(IdentityEvidence {
                    kind: "verified_upstream".to_string(),
                    appid: Some(query.appid),
                    page: Some(upstream.source_page.clone()),
                });
                candidate
                    .provenance
                    .extend(upstream.provenance.iter().cloned());
            }
        }
        if let Some(published_at) = &fact.published_at {
            candidate.published_at = Some(published_at.clone());
        }
        if let Ok(value) = serde_json::to_value(&*candidate) {
            candidate.fingerprint = canonical::fingerprint(&value);
        }
    }
    found
}

// ---- fallback (HGamer) --------------------------------------------------

/// Decides and, when justified, executes the per-game fallback query.
#[allow(clippy::too_many_arguments)]
fn decide_fallback(
    query: &QueryContext,
    previous: Option<&GameMatchRecord>,
    known: &[SourceCandidate],
    fallback_rule: Option<&SourceRule>,
    has_publisher_sources: bool,
    coverage_complete: bool,
    publisher_matched: bool,
    force: bool,
    ctx: &mut RequestContext,
    cancel: &AtomicBool,
    now: u64,
) -> Result<Option<FallbackMatchRecord>> {
    let existing = previous.and_then(|record| record.fallback.as_ref());
    let key = fallback_rule.map(|rule| fallback_query_key(query, rule));
    if publisher_matched {
        // A confirmed publisher match hides the fallback; an existing terminal
        // record is retained as evidence.
        if let Some(existing) = existing {
            if is_terminal(existing.status) {
                return Ok(Some(existing.clone()));
            }
        }
        return Ok(Some(fallback_record(
            key.unwrap_or_default(),
            FallbackMatchStatus::NotNeeded,
            Vec::new(),
            now,
            Vec::new(),
        )));
    }
    if !has_publisher_sources {
        // No publisher coverage at all: never claim HGamer confirmed anything.
        return Ok(Some(fallback_record(
            key.unwrap_or_default(),
            FallbackMatchStatus::NotQueried,
            Vec::new(),
            now,
            Vec::new(),
        )));
    }
    let Some(rule) = fallback_rule else {
        // Not configured or disabled for this game: report "not found among
        // enabled sources", not "HGamer confirmed no patch".
        return Ok(Some(fallback_record(
            String::new(),
            FallbackMatchStatus::Disabled,
            Vec::new(),
            now,
            Vec::new(),
        )));
    };
    let key = key.unwrap_or_default();
    if !force {
        if let Some(existing) = existing {
            if existing.query_key == key
                && matches!(
                    existing.status,
                    FallbackMatchStatus::Matched | FallbackMatchStatus::NoMatch
                )
            {
                return Ok(Some(existing.clone()));
            }
        }
    }
    if !coverage_complete {
        // Publisher coverage is incomplete: do not turn matching into a network
        // fallback. A previous terminal result for the same query stays visible.
        if let Some(existing) = existing {
            if existing.query_key == key && is_terminal(existing.status) {
                return Ok(Some(existing.clone()));
            }
        }
        return Ok(Some(fallback_record(
            key,
            FallbackMatchStatus::NotQueried,
            Vec::new(),
            now,
            Vec::new(),
        )));
    }
    if cancelled(cancel) {
        return Err(cancelled_error());
    }
    let mut fallback_query = query.clone();
    fallback_query.source_context = vec![rule.id.clone()];
    let mut known_candidates = known.to_vec();
    if let Some(existing) = existing {
        for candidate in &existing.candidates {
            if !known_candidates
                .iter()
                .any(|known| known.candidate_id == candidate.candidate_id)
            {
                known_candidates.push(candidate.clone());
            }
        }
    }
    let discover_previous = DiscoverPrevious { known_candidates };
    match adapters::discover(rule, &fallback_query, ctx, &discover_previous) {
        Err(error) => {
            if cancelled(cancel) || error.to_string() == "task_cancelled" {
                return Err(cancelled_error());
            }
            Ok(Some(fallback_record(
                key,
                FallbackMatchStatus::Failed,
                Vec::new(),
                now,
                vec![diagnostic(rule, error.to_string())],
            )))
        }
        Ok(outcome) => {
            if cancelled(cancel) {
                return Err(cancelled_error());
            }
            let status = if outcome.candidates.iter().any(candidate_is_credible) {
                FallbackMatchStatus::Matched
            } else {
                match outcome.status.as_str() {
                    "complete" => FallbackMatchStatus::NoMatch,
                    "partial" => FallbackMatchStatus::Incomplete,
                    _ => FallbackMatchStatus::Failed,
                }
            };
            Ok(Some(fallback_record(
                key,
                status,
                outcome.candidates,
                now,
                outcome.diagnostics,
            )))
        }
    }
}

fn fallback_record(
    query_key: String,
    status: FallbackMatchStatus,
    candidates: Vec<SourceCandidate>,
    checked_at: u64,
    diagnostics: Vec<Diagnostic>,
) -> FallbackMatchRecord {
    FallbackMatchRecord {
        query_key,
        status,
        candidates,
        checked_at,
        diagnostics,
    }
}

fn is_terminal(status: FallbackMatchStatus) -> bool {
    matches!(
        status,
        FallbackMatchStatus::Matched
            | FallbackMatchStatus::NoMatch
            | FallbackMatchStatus::Disabled
            | FallbackMatchStatus::NotNeeded
    )
}

/// Canonical fallback query digest: the per-game query inputs plus the fallback
/// rule identity and the identity-encoding versions.
fn fallback_query_key(query: &QueryContext, rule: &SourceRule) -> String {
    #[derive(serde::Serialize)]
    struct FallbackQuery<'a> {
        name: &'a str,
        name_hints: &'a [String],
        preferred_language: Option<&'a str>,
        rule_fingerprint: String,
        parser_version: u32,
        query_version: u32,
        encoding_version: u32,
    }
    canonical::canonical_sha256(&FallbackQuery {
        name: &query.name,
        name_hints: &query.name_hints,
        preferred_language: query.preferred_language.as_deref(),
        rule_fingerprint: rule_fingerprint(rule),
        parser_version: PARSER_VERSION,
        query_version: QUERY_VERSION,
        encoding_version: SOURCES_ENCODING_VERSION,
    })
}

// ---- summaries ----------------------------------------------------------

fn summary_from_record(record: &GameMatchRecord) -> GameMatch {
    let credible: Vec<SourceCandidate> = record
        .sources
        .values()
        .flat_map(|source| source.candidates.iter())
        .filter(|candidate| candidate_is_credible(candidate))
        .cloned()
        .collect();
    let candidates = if !credible.is_empty() {
        credible
    } else {
        // No confirmed publisher match: publisher leads stay visible and a
        // positive fallback result is appended.
        let mut combined: Vec<SourceCandidate> = record
            .sources
            .values()
            .flat_map(|source| source.candidates.iter().cloned())
            .collect();
        if let Some(fallback) = &record.fallback {
            if fallback.status == FallbackMatchStatus::Matched {
                combined.extend(fallback.candidates.iter().cloned());
            }
        }
        dedupe_candidates(combined)
    };
    let mut diagnostics: Vec<Diagnostic> = record
        .sources
        .values()
        .flat_map(|source| source.diagnostics.iter().cloned())
        .collect();
    if let Some(fallback) = &record.fallback {
        diagnostics.extend(fallback.diagnostics.iter().cloned());
    }
    GameMatch {
        appid: record.appid,
        instance_id: None,
        status: derive_game_status(record),
        candidates,
        diagnostics,
        source_revisions: record
            .sources
            .iter()
            .map(|(source_id, source)| (source_id.clone(), source.index_revision))
            .collect(),
    }
}

fn dedupe_candidates(candidates: Vec<SourceCandidate>) -> Vec<SourceCandidate> {
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| seen.insert(candidate.candidate_id.clone()))
        .collect()
}

// ---- small helpers ------------------------------------------------------

fn dedupe_games(games: &[GameQuery]) -> Vec<GameQuery> {
    let mut seen = HashSet::new();
    games
        .iter()
        .filter(|game| seen.insert(game.appid))
        .cloned()
        .collect()
}

fn emit(
    on_progress: &mut impl FnMut(MatchProgress),
    phase: &str,
    completed: usize,
    total: usize,
    running: bool,
) {
    on_progress(MatchProgress {
        phase: phase.to_string(),
        completed,
        total,
        running,
    });
}

fn finish_progress(on_progress: &mut impl FnMut(MatchProgress), completed: usize, total: usize) {
    for phase in ["index", "details", "fallback"] {
        emit(on_progress, phase, completed, total, false);
    }
}

fn effective_enabled(rule: &SourceRule, overrides: &Overrides) -> bool {
    overrides
        .enabled
        .get(&rule.id)
        .copied()
        .unwrap_or(rule.enabled)
}

fn game_subscribed(overrides: &Overrides, appid: u32, source_id: &str) -> bool {
    overrides
        .games
        .get(&appid.to_string())
        .and_then(|sources| sources.get(source_id))
        .copied()
        .unwrap_or(true)
}

fn active_publishers<'a>(
    rules: &'a [SourceRule],
    overrides: &Overrides,
    appid: u32,
) -> Vec<&'a SourceRule> {
    let mut publishers: Vec<&SourceRule> = rules
        .iter()
        .filter(|rule| {
            rule.role == SourceRole::Publisher
                && effective_enabled(rule, overrides)
                && game_subscribed(overrides, appid, &rule.id)
        })
        .collect();
    publishers.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.id.cmp(&right.id))
    });
    publishers
}

fn active_fallback<'a>(
    rules: &'a [SourceRule],
    overrides: &Overrides,
    appid: u32,
) -> Option<&'a SourceRule> {
    let mut fallbacks: Vec<&SourceRule> = rules
        .iter()
        .filter(|rule| {
            rule.role == SourceRole::FallbackIndexer
                && effective_enabled(rule, overrides)
                && game_subscribed(overrides, appid, &rule.id)
        })
        .collect();
    fallbacks.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.id.cmp(&right.id))
    });
    fallbacks.into_iter().next()
}

fn subscriptions_snapshot(
    rules: &[SourceRule],
    overrides: &Overrides,
    appid: u32,
) -> BTreeMap<String, bool> {
    rules
        .iter()
        .map(|rule| {
            (
                rule.id.clone(),
                effective_enabled(rule, overrides) && game_subscribed(overrides, appid, &rule.id),
            )
        })
        .collect()
}

/// Merges the trusted localized name with the frontend hints exactly like the
/// old `merged_name_hints`: trimmed, deduped, at most 3 entries, localized name
/// first.
fn merged_hints(game: &GameQuery, name_hints: &[String]) -> Vec<String> {
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

fn query_context(
    game: &GameQuery,
    name_hints: &[String],
    preferred_language: Option<&str>,
    force: bool,
    publisher_ids: Vec<String>,
) -> QueryContext {
    QueryContext {
        appid: game.appid,
        name: game.name.clone(),
        name_hints: name_hints.to_vec(),
        preferred_language: preferred_language.map(str::to_string),
        legacy_match: false,
        force,
        source_context: publisher_ids,
        parser_version: PARSER_VERSION,
        query_version: QUERY_VERSION,
        settings_revision: 0,
        generation: None,
    }
}

fn entry_relevant(
    entry: &SourceIndexEntry,
    query: &QueryContext,
    known: &[SourceCandidate],
) -> bool {
    upstream_identity(&entry.page, query.appid, known).is_some()
        || std::iter::once(&entry.title)
            .chain(entry.aliases.iter())
            .any(|title| lead_relevant(title, query, known))
}

/// An AppID-confirmed fallback must explicitly name this exact publisher page.
/// A similar title, a foreign AppID or an unindexed URL cannot create a hit.
fn upstream_identity<'a>(
    page: &str,
    appid: u32,
    known: &'a [SourceCandidate],
) -> Option<&'a SourceCandidate> {
    let page = canonical::canonical_url(page);
    known.iter().find(|candidate| {
        candidate_is_credible(candidate)
            && candidate.game_appid == Some(appid)
            && candidate
                .upstream_page
                .as_ref()
                .is_some_and(|upstream| canonical::canonical_url(upstream) == page)
    })
}

/// Port of the old `lead_relevant` for a P3 fact/index title: title relevance,
/// a known patch title, or a DLC short title whose stem matches a query name.
fn lead_relevant(title: &str, query: &QueryContext, known: &[SourceCandidate]) -> bool {
    if identity::title_relevant(title, query)
        || known.iter().any(|candidate| candidate.patch_title == title)
    {
        return true;
    }
    if identity::variant(title) != Variant::Dlc {
        return false;
    }
    let stem = validate::discovery_normal(&identity::base_title(title));
    std::iter::once(&query.name)
        .chain(query.name_hints.iter())
        .chain(known.iter().map(|candidate| &candidate.game_title))
        .any(|name| {
            let base = name.split(['：', ':']).next().unwrap_or(name);
            stem == validate::discovery_normal(&identity::base_title(base))
        })
}

fn diagnostic(rule: &SourceRule, code: impl Into<String>) -> Diagnostic {
    Diagnostic {
        source_id: Some(rule.id.clone()),
        code: code.into(),
        detail: None,
    }
}

fn cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Relaxed) || crate::operation::check().is_err()
}

fn cancelled_error() -> std::io::Error {
    invalid("task_cancelled")
}
