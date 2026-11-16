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

use super::adapters::script::DetailStepState;
use super::adapters::{self, DiscoverPrevious};
use super::http::RequestContext;
use super::identity;
use super::model::{
    CacheStatus, DetailRecord, Diagnostic, FallbackMatchRecord, FallbackMatchStatus, GameMatch,
    GameMatchRecord, GameMatchStatus, GameQuery, IdentityEvidence, MatchBasis, Overrides,
    QueryContext, SourceCandidate, SourceDetail, SourceIndex, SourceIndexEntry, SourceMatchRecord,
    SourceMatchStatus, SourceRole, SourceRule, Variant,
};
use super::r#match::{candidate_is_credible, derive_game_status, input_key};
use super::store::SourceStore;
use super::validate;
use super::{canonical, index};
use super::{
    invalid, rule_fingerprint, Result, GAME_MATCH_SCHEMA_VERSION, PARSER_VERSION, QUERY_VERSION,
    SOURCES_ENCODING_VERSION, SOURCE_DETAIL_SCHEMA_VERSION,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

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
/// `name_hints` are the untrusted frontend query inputs
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
    force_details: bool,
    ctx: &mut RequestContext,
    cancel: &Arc<AtomicBool>,
    now: u64,
    mut on_progress: impl FnMut(MatchProgress),
    mut on_update: impl FnMut(&GameMatch),
) -> Result<Vec<GameMatch>> {
    crate::operation::with_cancel(Arc::clone(cancel), || {
        let settings = store.load_settings()?;
        if rules
            .iter()
            .any(|rule| rule.script.is_some() && super::script::rule_changed(rule, &settings))
            || settings
                .added
                .iter()
                .any(|added| !rules.iter().any(|rule| rule.id == added.id))
        {
            return Err(invalid("source_rule_changed"));
        }
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
                    Some(record) => {
                        record_reusable(store, record, game, name_hints, rules, overrides)?
                    }
                    None => false,
                };
            if reused {
                let mut summary = summary_from_record(previous.as_ref().expect("reused record"));
                summary.instance_id = game.instance_id.clone();
                results.push(summary);
            } else {
                let record = {
                    let mut on_phase =
                        |phase: &str| emit(&mut on_progress, phase, index, total, true);
                    match_one(
                        store,
                        rules,
                        overrides,
                        game,
                        name_hints,
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
    })
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
    ctx: &mut RequestContext,
    cancel: &Arc<AtomicBool>,
    now: u64,
    mut on_update: impl FnMut(&GameMatch),
) -> Result<GameMatch> {
    crate::operation::with_cancel(Arc::clone(cancel), || {
        let previous = store.load_match(game.appid)?;
        let record = match_one(
            store,
            rules,
            overrides,
            game,
            name_hints,
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
    })
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
    ctx: &mut RequestContext,
    cancel: &Arc<AtomicBool>,
    now: u64,
    mut on_update: impl FnMut(&GameMatch),
) -> Result<Vec<GameMatch>> {
    crate::operation::with_cancel(Arc::clone(cancel), || {
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
    })
}

/// Pure read of a persisted match record (`matches/<appid>.json`), no network
/// and no writes. A missing record is `pending` with empty candidates.
pub fn game_match_summary(store: &SourceStore, appid: u32) -> GameMatch {
    match store.load_match(appid) {
        Ok(Some(record)) => {
            let settings = match store.load_settings() {
                Ok(settings) => settings,
                Err(error) => {
                    let error = crate::projection::DomainError::from(error);
                    return GameMatch {
                        appid,
                        instance_id: None,
                        status: GameMatchStatus::Failed,
                        candidates: Vec::new(),
                        diagnostics: vec![Diagnostic {
                            source_id: None,
                            code: error.code,
                            detail: error.detail,
                        }],
                        source_revisions: BTreeMap::new(),
                    };
                }
            };
            let builtins: super::model::RulesConfig =
                serde_json::from_str(super::DEFAULT_RULES_JSON).expect("embedded rules");
            let builtins = builtins
                .sources
                .iter()
                .map(validate::validate_rule)
                .collect::<Result<Vec<_>>>()
                .expect("embedded valid rules");
            let rules = merged_rules(&builtins, &settings.added);
            let overrides = Overrides {
                enabled: settings.enabled,
                added: settings.added,
                games: settings.games,
            };
            summary_from_record(&current_record(record, &rules, &overrides, |rule| {
                store.load_index(&rule.id).ok().flatten()
            }))
        }
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
            .filter(|index| index::compatible(index, rule))
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
    previous: Option<&GameMatchRecord>,
    options: MatchOptions<'_>,
    ctx: &mut RequestContext,
    cancel: &Arc<AtomicBool>,
    now: u64,
    on_phase: &mut dyn FnMut(&str),
) -> Result<GameMatchRecord> {
    let publishers = active_publishers(rules, overrides, game.appid);
    let fallback_rules = active_fallbacks(rules, overrides, game.appid);
    let publisher_ids: Vec<String> = publishers.iter().map(|rule| rule.id.clone()).collect();
    let hints = merged_hints(game, name_hints);
    let query = query_context(game, &hints, options.force_details, publisher_ids);
    let subscriptions = subscriptions_snapshot(rules, overrides, game.appid);
    let key = game_input_key(game, &hints, &subscriptions);
    // The host writes settings from another process; `load_settings` reads the
    // persisted revision so a mid-task disable/unsubscribe is detected below.
    let settings = store.load_settings()?;
    if rules
        .iter()
        .any(|rule| rule.script.is_some() && super::script::rule_changed(rule, &settings))
        || settings
            .added
            .iter()
            .any(|added| !rules.iter().any(|rule| rule.id == added.id))
    {
        return Err(invalid("source_rule_changed"));
    }
    let settings_revision = settings.revision;

    let mut indexes: BTreeMap<String, Option<SourceIndex>> = BTreeMap::new();
    for rule in &publishers {
        indexes.insert(
            rule.id.clone(),
            store
                .load_index(&rule.id)?
                .filter(|index| index::compatible(index, rule)),
        );
    }

    let previous = previous.map(|record| {
        current_record(record.clone(), rules, overrides, |rule| {
            indexes.get(&rule.id).and_then(Option::as_ref).cloned()
        })
    });
    let previous = previous.as_ref();

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

    if !fallback_rules.is_empty() {
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
            settings.script_sources.get(&rule.id).map(String::as_str),
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
        &fallback_rules,
        &settings.script_sources,
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
                            !entry.is_complete()
                                && entry.step_data.is_none()
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
                        settings.script_sources.get(&rule.id).map(String::as_str),
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
    if settings_binding_changed(store, settings_revision, &key, game, name_hints, rules)? {
        let latest = store.load_settings()?;
        let latest_rules = merged_rules(rules, &latest.added);
        let latest_overrides = Overrides {
            enabled: latest.enabled,
            added: latest.added,
            games: latest.games,
        };
        return Ok(previous
            .map(|record| {
                current_record(record.clone(), &latest_rules, &latest_overrides, |rule| {
                    store.load_index(&rule.id).ok().flatten()
                })
            })
            .unwrap_or_else(|| pending_record(game, key, now)));
    }
    if cancelled(cancel) {
        return Err(cancelled_error());
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
    rules: &[SourceRule],
) -> Result<bool> {
    let settings = store.load_settings()?;
    if settings.revision == captured_revision {
        return Ok(false);
    }
    if rules
        .iter()
        .any(|rule| super::script::rule_changed(rule, &settings))
    {
        return Ok(true);
    }
    let effective_rules = merged_rules(rules, &settings.added);
    let overrides = Overrides {
        enabled: settings.enabled,
        added: settings.added,
        games: settings.games,
    };
    let hints = merged_hints(game, name_hints);
    let subscriptions = subscriptions_snapshot(&effective_rules, &overrides, game.appid);
    Ok(game_input_key(game, &hints, &subscriptions) != key)
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
    rules: &[SourceRule],
    overrides: &Overrides,
) -> Result<bool> {
    if current_record(record.clone(), rules, overrides, |rule| {
        store.load_index(&rule.id).ok().flatten()
    }) != *record
    {
        return Ok(false);
    }
    let publishers = active_publishers(rules, overrides, game.appid);
    let publisher_ids: Vec<String> = publishers.iter().map(|rule| rule.id.clone()).collect();
    let hints = merged_hints(game, name_hints);
    let subscriptions = subscriptions_snapshot(rules, overrides, game.appid);
    if record.input_key != game_input_key(game, &hints, &subscriptions) {
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
            .filter(|index| index::compatible(index, rule))
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
    let fallback_rules = active_fallbacks(rules, overrides, game.appid);
    let coverage_complete = !publishers.is_empty()
        && publishers.iter().all(|rule| {
            store
                .load_index(&rule.id)
                .map(|index| {
                    index.is_some_and(|index| index::compatible(&index, rule) && index.complete)
                })
                .unwrap_or(false)
        });
    let publisher_matched = record
        .sources
        .values()
        .any(|source| source.status == SourceMatchStatus::Matched);
    let fallback_terminal = match &record.fallback {
        None => false,
        Some(fallback) => match fallback.status {
            FallbackMatchStatus::Matched | FallbackMatchStatus::NoMatch => {
                let query = query_context(game, &hints, false, publisher_ids.clone());
                !fallback_rules.is_empty()
                    && fallback.query_key == fallback_query_key(&query, &fallback_rules)
            }
            FallbackMatchStatus::Disabled => fallback_rules.is_empty() && !publishers.is_empty(),
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
    code: Option<&str>,
    index: Option<&SourceIndex>,
    game: &GameQuery,
    query: &QueryContext,
    previous: Option<&SourceMatchRecord>,
    force_details: bool,
    ctx: &mut RequestContext,
    cancel: &Arc<AtomicBool>,
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

    // Completed Step-1 entries (e.g. single-shot tables) project purely offline.
    let completed = match_completed_entries(rule, index, game);
    known.extend(completed.iter().cloned());
    candidates.extend(completed);

    // Incomplete entries step forward through Lua `resolve_detail` on demand.
    let has_incomplete = index.entries.iter().any(|entry| !entry.is_complete());
    if has_incomplete && !candidates.iter().any(candidate_is_credible) {
        let adapter = adapters::source_adapter(rule, code)?;
        let found = match_incomplete_entries(
            store,
            rule,
            &adapter,
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
    if detail_failed || !candidates.is_empty() {
        return SourceMatchStatus::DetailIncomplete;
    }
    if !index.complete {
        return SourceMatchStatus::IndexIncomplete;
    }
    SourceMatchStatus::NoCandidate
}

fn merge_and_validate(
    rule: &SourceRule,
    mut candidates: Vec<SourceCandidate>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<SourceCandidate> {
    for candidate in &mut candidates {
        index::bind_candidate(rule, candidate);
    }
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

// ---- completed-entry offline matching -----------------------------------

fn prefers_zh_cn(candidate: &SourceCandidate) -> bool {
    candidate.languages.iter().any(|language| {
        language.eq_ignore_ascii_case("zh-cn") || language.eq_ignore_ascii_case("zh_cn")
    })
}

fn effective_detail_for_entry(entry: &SourceIndexEntry, detail: &SourceDetail) -> SourceDetail {
    if entry.aliases.is_empty()
        || entry
            .aliases
            .iter()
            .all(|alias| detail.aliases.contains(alias))
    {
        return detail.clone();
    }
    let mut merged = detail.clone();
    for alias in &entry.aliases {
        if !merged.aliases.contains(alias) {
            merged.aliases.push(alias.clone());
        }
    }
    merged
}

fn confirmed_ids_for_game(
    entry_appid: Option<u32>,
    detail_ids: &[u32],
    appid: u32,
    related_dlc: &[crate::steam::SteamDlc],
) -> Vec<u32> {
    let mut confirmed = Vec::new();
    for id in entry_appid.into_iter().chain(detail_ids.iter().copied()) {
        let relevant = id == appid || related_dlc.iter().any(|dlc| dlc.appid == id);
        if relevant && !confirmed.contains(&id) {
            confirmed.push(id);
        }
    }
    confirmed
}

/// Deterministic offline match against index entries whose `details` were
/// already completed in Step 1 (`entry.is_complete()`). Shared with the
/// read-model `registry.matches` fallback.
pub(crate) fn match_completed_entries(
    rule: &SourceRule,
    index: &SourceIndex,
    game: &GameQuery,
) -> Vec<SourceCandidate> {
    let hints = merged_hints(game, &[]);
    let query = query_context(game, &hints, false, vec![rule.id.clone()]);
    let observed_at = Some(identity::iso8601_utc(index.checked_at));
    let mut candidates = Vec::new();

    for entry in index.entries.iter().filter(|entry| entry.is_complete()) {
        for detail in &entry.details {
            let effective = effective_detail_for_entry(entry, detail);
            let ids = confirmed_ids_for_game(
                entry.appid,
                &effective.steam_ids,
                game.appid,
                &game.related_dlc,
            );
            candidates.extend(identity::build_candidates_from_detail_with_options(
                rule,
                &effective,
                &ids,
                &query,
                observed_at.clone(),
                CacheStatus::Cached,
            ));
        }
    }

    // Deduplicate single-mirror rows sharing an identical download URL, preferring `zh-CN`.
    let mut deduped: Vec<SourceCandidate> = Vec::with_capacity(candidates.len());
    let mut single_mirror_pos: HashMap<String, usize> = HashMap::new();
    for candidate in candidates {
        if candidate.mirrors.len() == 1 {
            let url = candidate.mirrors[0].url.clone();
            if let Some(&pos) = single_mirror_pos.get(&url) {
                if !prefers_zh_cn(&deduped[pos]) && prefers_zh_cn(&candidate) {
                    deduped[pos] = candidate;
                }
                continue;
            }
            single_mirror_pos.insert(url, deduped.len());
        }
        deduped.push(candidate);
    }
    deduped
}

// ---- incomplete-entry Lua Step progression -------------------------------

#[allow(clippy::too_many_arguments)]
fn match_incomplete_entries(
    store: &SourceStore,
    rule: &SourceRule,
    adapter: &adapters::SourceAdapter<'_>,
    index: &SourceIndex,
    query: &QueryContext,
    force_details: bool,
    ctx: &mut RequestContext,
    cancel: &Arc<AtomicBool>,
    now: u64,
    known: &mut Vec<SourceCandidate>,
    detail_pages: &mut Vec<String>,
    diagnostics: &mut Vec<Diagnostic>,
    detail_failed: &mut bool,
) -> Result<Vec<SourceCandidate>> {
    let mut direct: Vec<&SourceIndexEntry> = index
        .entries
        .iter()
        .filter(|entry| {
            !entry.is_complete() && entry.step_data.is_none() && entry_relevant(entry, query, known)
        })
        .collect();
    direct.sort_by_key(|entry| identity::variant(&entry.title) == Variant::Dlc);

    let mut candidates = resolve_entry_batch(
        store,
        rule,
        adapter,
        index.revision,
        &direct,
        query,
        force_details,
        ctx,
        cancel,
        now,
        known,
        detail_pages,
        diagnostics,
        detail_failed,
    )?;

    if query.related_dlc.is_empty() && candidates.iter().any(candidate_is_credible) {
        return Ok(candidates);
    }

    let mut deferred: Vec<&SourceIndexEntry> = index
        .entries
        .iter()
        .filter(|entry| {
            !entry.is_complete() && entry.step_data.is_some() && entry_relevant(entry, query, known)
        })
        .collect();
    deferred.sort_by_key(|entry| identity::variant(&entry.title) == Variant::Dlc);

    let more = resolve_entry_batch(
        store,
        rule,
        adapter,
        index.revision,
        &deferred,
        query,
        force_details,
        ctx,
        cancel,
        now,
        known,
        detail_pages,
        diagnostics,
        detail_failed,
    )?;
    candidates.extend(more);
    Ok(candidates)
}

#[allow(clippy::too_many_arguments)]
fn resolve_entry_batch(
    store: &SourceStore,
    rule: &SourceRule,
    adapter: &adapters::SourceAdapter<'_>,
    index_revision: u64,
    entries: &[&SourceIndexEntry],
    query: &QueryContext,
    force_details: bool,
    ctx: &mut RequestContext,
    cancel: &Arc<AtomicBool>,
    now: u64,
    known: &mut Vec<SourceCandidate>,
    detail_pages: &mut Vec<String>,
    diagnostics: &mut Vec<Diagnostic>,
    detail_failed: &mut bool,
) -> Result<Vec<SourceCandidate>> {
    let mut candidates = Vec::new();
    for entry in entries {
        if cancelled(cancel) {
            return Err(cancelled_error());
        }
        let mut details = resolve_entry_details(
            store,
            rule,
            adapter,
            index_revision,
            entry,
            force_details,
            ctx,
            cancel,
            now,
            detail_pages,
            diagnostics,
            detail_failed,
        )?;
        details.sort_by_key(|detail| identity::variant(&detail.title) == Variant::Dlc);
        for detail in details {
            let effective = effective_detail_for_entry(entry, &detail);
            let found = candidates_from_detail(
                rule,
                &effective,
                entry.appid,
                query,
                known,
                adapter,
                ctx,
                cancel,
            )?;
            known.extend(found.iter().cloned());
            candidates.extend(found);
        }
    }
    Ok(candidates)
}

fn push_detail_page(detail_pages: &mut Vec<String>, page: &str) {
    let canonical = canonical::canonical_page_url(page);
    if !detail_pages.contains(&canonical) {
        detail_pages.push(canonical);
    }
}

/// Loads cached step progress / completed details for `entry`, or steps Lua
/// `resolve_detail(ctx, rule, state)` forward until `current_step >= total_steps`.
#[allow(clippy::too_many_arguments)]
fn resolve_entry_details(
    store: &SourceStore,
    rule: &SourceRule,
    adapter: &adapters::SourceAdapter<'_>,
    index_revision: u64,
    entry: &SourceIndexEntry,
    force_details: bool,
    ctx: &mut RequestContext,
    cancel: &Arc<AtomicBool>,
    now: u64,
    detail_pages: &mut Vec<String>,
    diagnostics: &mut Vec<Diagnostic>,
    detail_failed: &mut bool,
) -> Result<Vec<SourceDetail>> {
    let page = canonical::canonical_page_url(&entry.page);
    push_detail_page(detail_pages, &page);

    let mut state = None;
    if !force_details {
        if let Some(cached) = store.load_detail(&rule.id, &page)? {
            if cached.index_revision == index_revision && cached.current_step >= entry.current_step
            {
                if cached.is_complete() {
                    diagnostics.extend(cached.diagnostics);
                    for detail in &cached.details {
                        push_detail_page(detail_pages, &detail.page);
                    }
                    return Ok(cached.details);
                }
                state = Some(DetailStepState {
                    title: entry.title.clone(),
                    aliases: entry.aliases.clone(),
                    page: page.clone(),
                    appid: entry.appid,
                    current_step: cached.current_step,
                    total_steps: cached.total_steps,
                    step_data: cached.step_data,
                    details: cached.details,
                });
            }
        }
    }

    let mut state = state.unwrap_or_else(|| DetailStepState {
        title: entry.title.clone(),
        aliases: entry.aliases.clone(),
        page: page.clone(),
        appid: entry.appid,
        current_step: entry.current_step,
        total_steps: entry.total_steps,
        step_data: entry.step_data.clone(),
        details: entry.details.clone(),
    });

    while state.current_step < state.total_steps {
        if cancelled(cancel) {
            return Err(cancelled_error());
        }
        let outcome = match adapter.resolve_detail(rule, &state, ctx, cancel) {
            Ok(outcome) => outcome,
            Err(error) => {
                if cancelled(cancel) || error.to_string() == "task_cancelled" {
                    return Err(cancelled_error());
                }
                *detail_failed = true;
                diagnostics.push(error_diagnostic(rule, error));
                return Ok(state.details);
            }
        };
        if cancelled(cancel) {
            return Err(cancelled_error());
        }
        diagnostics.extend(outcome.diagnostics.iter().cloned());
        let advanced = outcome.current_step > state.current_step;
        if advanced {
            state.current_step = outcome.current_step;
        }
        state.total_steps = outcome.total_steps;
        state.step_data = outcome.step_data;
        state.details = outcome.details;
        store.save_detail(&DetailRecord {
            schema_version: SOURCE_DETAIL_SCHEMA_VERSION,
            source_id: rule.id.clone(),
            page: page.clone(),
            index_revision,
            checked_at: now,
            current_step: state.current_step,
            total_steps: state.total_steps,
            step_data: state.step_data.clone(),
            details: state.details.clone(),
            diagnostics: outcome.diagnostics,
        })?;
        if !advanced {
            *detail_failed = true;
            break;
        }
    }

    // Cache resolved sub-article pages (e.g. expanded inside a category step)
    // under their own canonical page key so direct hits reuse them offline.
    let mut sub_pages: BTreeMap<String, Vec<SourceDetail>> = BTreeMap::new();
    for detail in &state.details {
        let sub_page = canonical::canonical_page_url(&detail.page);
        push_detail_page(detail_pages, &sub_page);
        if sub_page != page {
            sub_pages.entry(sub_page).or_default().push(detail.clone());
        }
    }
    for (sub_page, sub_details) in sub_pages {
        store.save_detail(&DetailRecord {
            schema_version: SOURCE_DETAIL_SCHEMA_VERSION,
            source_id: rule.id.clone(),
            page: sub_page,
            index_revision,
            checked_at: now,
            current_step: 1,
            total_steps: 1,
            step_data: None,
            details: sub_details,
            diagnostics: Vec::new(),
        })?;
    }

    Ok(state.details)
}

#[allow(clippy::too_many_arguments)]
fn candidates_from_detail(
    rule: &SourceRule,
    detail: &SourceDetail,
    entry_appid: Option<u32>,
    query: &QueryContext,
    known: &[SourceCandidate],
    adapter: &adapters::SourceAdapter<'_>,
    ctx: &mut RequestContext,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<SourceCandidate>> {
    let mut check_ids = detail.steam_ids.clone();
    if let Some(id) = entry_appid {
        if !check_ids.contains(&id) {
            check_ids.push(id);
        }
    }
    let confirmed_id: Vec<u32> = match adapter.match_appid(rule, &check_ids, query, ctx, cancel)? {
        Some(false) => return Ok(Vec::new()),
        Some(true) => {
            let mut ids = vec![query.appid];
            for dlc in &query.related_dlc {
                if check_ids.contains(&dlc.appid) && !ids.contains(&dlc.appid) {
                    ids.push(dlc.appid);
                }
            }
            ids
        }
        None => Vec::new(),
    };

    let mut related_query = query.clone();
    let title_relevant = identity::title_relevant(&detail.title, query);
    let upstream = upstream_identity(&detail.page, query.appid, known)
        .filter(|candidate| candidate.variant == identity::variant(&detail.title));
    let known_association = known.iter().any(|candidate| {
        candidate.source_page == detail.page
            && candidate.patch_title == detail.title
            && candidate
                .game_appid
                .is_none_or(|appid| appid == query.appid)
    });
    let base_link = detail.body_links.iter().any(|url| {
        let url = canonical::canonical_page_url(url);
        known
            .iter()
            .any(|candidate| candidate.variant == Variant::Base && candidate.source_page == url)
    });
    // A short DLC title is only promoted to a query hint when it either is a
    // title lead itself or explicitly links to a confirmed base page.
    if upstream.is_some()
        || known_association
        || (lead_relevant(&detail.title, query, known) && (title_relevant || base_link))
    {
        related_query.name_hints.push(detail.title.clone());
    }

    let mut found = identity::build_candidates_from_detail(
        rule,
        detail,
        confirmed_id.as_slice(),
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
                if let Ok(value) = serde_json::to_value(&*candidate) {
                    candidate.fingerprint = canonical::fingerprint(&value);
                }
            }
        }
    }
    Ok(found)
}

// ---- per-game fallback aggregation --------------------------------------

/// Decides and, when justified, executes the per-game fallback query.
#[allow(clippy::too_many_arguments)]
fn decide_fallback(
    query: &QueryContext,
    previous: Option<&GameMatchRecord>,
    known: &[SourceCandidate],
    fallback_rules: &[&SourceRule],
    script_sources: &BTreeMap<String, String>,
    has_publisher_sources: bool,
    coverage_complete: bool,
    publisher_matched: bool,
    force: bool,
    ctx: &mut RequestContext,
    cancel: &Arc<AtomicBool>,
    now: u64,
) -> Result<Option<FallbackMatchRecord>> {
    let key = fallback_query_key(query, fallback_rules);
    let sources_fingerprint = fallback_set_fingerprint(fallback_rules);
    let record = |status, candidates, diagnostics| {
        Some(FallbackMatchRecord {
            sources_fingerprint: Some(sources_fingerprint.clone()),
            query_key: key.clone(),
            status,
            candidates,
            checked_at: now,
            diagnostics,
        })
    };
    let existing = previous
        .and_then(|record| record.fallback.as_ref())
        .filter(|existing| existing.query_key == key);
    if publisher_matched {
        if let Some(existing) = existing.filter(|existing| is_terminal(existing.status)) {
            return Ok(Some(existing.clone()));
        }
        return Ok(record(
            FallbackMatchStatus::NotNeeded,
            Vec::new(),
            Vec::new(),
        ));
    }
    if !has_publisher_sources {
        return Ok(record(
            FallbackMatchStatus::NotQueried,
            Vec::new(),
            Vec::new(),
        ));
    }
    if fallback_rules.is_empty() {
        return Ok(record(
            FallbackMatchStatus::Disabled,
            Vec::new(),
            Vec::new(),
        ));
    }
    if !force {
        if let Some(existing) = existing.filter(|existing| {
            matches!(
                existing.status,
                FallbackMatchStatus::Matched | FallbackMatchStatus::NoMatch
            )
        }) {
            return Ok(Some(existing.clone()));
        }
    }
    if !coverage_complete {
        if let Some(existing) = existing.filter(|existing| is_terminal(existing.status)) {
            return Ok(Some(existing.clone()));
        }
        return Ok(record(
            FallbackMatchStatus::NotQueried,
            Vec::new(),
            Vec::new(),
        ));
    }
    let mut candidates = Vec::new();
    let mut diagnostics = Vec::new();
    let mut complete = true;
    let mut successful = 0;
    for rule in fallback_rules {
        if cancelled(cancel) {
            return Err(cancelled_error());
        }
        let mut fallback_query = query.clone();
        fallback_query.source_context = vec![rule.id.clone()];
        fallback_query.force = force;
        let mut known_candidates = known.to_vec();
        known_candidates.extend(candidates.iter().cloned());
        if let Some(existing) = existing {
            known_candidates.extend(
                existing
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.source_id == rule.id)
                    .cloned(),
            );
        }
        let discover_previous = DiscoverPrevious {
            known_candidates: dedupe_candidates(known_candidates),
        };
        let outcome =
            adapters::source_adapter(rule, script_sources.get(&rule.id).map(String::as_str))
                .and_then(|adapter| {
                    adapter.discover(rule, &fallback_query, ctx, &discover_previous, cancel)
                });
        if cancelled(cancel) {
            return Err(cancelled_error());
        }
        match outcome {
            Err(error) => {
                complete = false;
                diagnostics.push(error_diagnostic(rule, error));
            }
            Ok(outcome) => {
                complete &= outcome.status == "complete";
                successful += usize::from(outcome.status != "failed");
                candidates.extend(outcome.candidates);
                diagnostics.extend(outcome.diagnostics);
            }
        }
    }
    let candidates = identity::merge_candidates(&candidates);
    let status = if candidates.iter().any(candidate_is_credible) {
        FallbackMatchStatus::Matched
    } else if complete && candidates.is_empty() {
        FallbackMatchStatus::NoMatch
    } else if successful == 0 && candidates.is_empty() {
        FallbackMatchStatus::Failed
    } else {
        FallbackMatchStatus::Incomplete
    };
    Ok(record(status, candidates, diagnostics))
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

fn fallback_set_fingerprint(rules: &[&SourceRule]) -> String {
    let source_fingerprints: BTreeMap<_, _> = rules
        .iter()
        .map(|rule| {
            let mut effective = (*rule).clone();
            effective.enabled = true;
            (rule.id.clone(), rule_fingerprint(&effective))
        })
        .collect();
    canonical::canonical_sha256(&serde_json::json!({"sources":source_fingerprints,
        "parser_version":PARSER_VERSION,"query_version":QUERY_VERSION,"encoding_version":SOURCES_ENCODING_VERSION}))
}

/// Source-only binding used by read-only projections without frontend name hints.
pub fn fallback_sources_fingerprint(
    rules: &[SourceRule],
    overrides: &Overrides,
    appid: u32,
) -> String {
    fallback_set_fingerprint(&active_fallbacks(rules, overrides, appid))
}

fn fallback_query_key(query: &QueryContext, rules: &[&SourceRule]) -> String {
    canonical::canonical_sha256(&serde_json::json!({"appid":query.appid,"name":query.name,
        "name_hints":query.name_hints,"sources_fingerprint":fallback_set_fingerprint(rules)}))
}

// ---- summaries ----------------------------------------------------------

fn merged_rules(builtins: &[SourceRule], added: &[SourceRule]) -> Vec<SourceRule> {
    builtins
        .iter()
        .filter(|rule| !added.iter().any(|added| added.id == rule.id))
        .chain(added)
        .cloned()
        .map(|mut rule| {
            super::builtin::bind(&mut rule);
            rule
        })
        .collect()
}

/// Remove retired parser/source-set results before reads or reuse.
/// Legacy Rust publisher caches keep their established revision semantics.
pub(crate) fn current_record(
    mut record: GameMatchRecord,
    rules: &[SourceRule],
    overrides: &Overrides,
    mut index: impl FnMut(&SourceRule) -> Option<SourceIndex>,
) -> GameMatchRecord {
    let mut stale = false;
    record.sources.retain(|source_id, source| {
        let Some(rule) = rules
            .iter()
            .find(|rule| rule.id == *source_id && rule.script.is_some())
        else {
            // A deleted user source cannot authorize the candidates it produced;
            // entries from the retired Rust publishers keep their old semantics.
            let keep = source
                .candidates
                .iter()
                .all(|candidate| candidate.parser_fingerprint.is_none());
            stale |= !keep;
            return keep;
        };
        let index = index(rule);
        let parser_current = rule.role == SourceRole::Publisher
            && index.as_ref().is_some_and(|index| {
                index::compatible(index, rule)
                    && source
                        .candidates
                        .iter()
                        .all(|candidate| candidate.parser_fingerprint == index.parser_fingerprint)
            });
        // A routine publisher refresh retires its own result, not an unchanged
        // fallback query. Parser changes still invalidate dependent evidence.
        stale |= !parser_current;
        parser_current && index.is_some_and(|index| index.revision == source.index_revision)
    });
    if let Some(fallback) = &record.fallback {
        stale |= fallback.sources_fingerprint.as_deref()
            != Some(&fallback_sources_fingerprint(
                rules,
                overrides,
                record.appid,
            ));
    }
    if stale {
        record.fallback = None;
        record.status = derive_game_status(&record);
    }
    record
}

pub(crate) fn summary_from_record(record: &GameMatchRecord) -> GameMatch {
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
            if matches!(
                fallback.status,
                FallbackMatchStatus::Matched | FallbackMatchStatus::Incomplete
            ) {
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

fn active_fallbacks<'a>(
    rules: &'a [SourceRule],
    overrides: &Overrides,
    appid: u32,
) -> Vec<&'a SourceRule> {
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
    fallbacks
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

fn game_input_key(
    game: &GameQuery,
    hints: &[String],
    subscriptions: &BTreeMap<String, bool>,
) -> String {
    let key = input_key(
        &game.name,
        game.localized_name.as_deref(),
        hints,
        subscriptions,
    );
    if game.related_dlc.is_empty() {
        key
    } else {
        canonical::canonical_sha256(&(key, &game.related_dlc))
    }
}

/// Merges the trusted localized name with the frontend hints exactly like the
/// old `merged_name_hints`, then adds Steam DLC names without truncation.
fn merged_hints(game: &GameQuery, name_hints: &[String]) -> Vec<String> {
    let mut hints: Vec<String> = Vec::new();
    if let Some(localized) = &game.localized_name {
        push_hint(&mut hints, localized);
    }
    for hint in name_hints {
        push_hint(&mut hints, hint);
    }
    for name in game.related_dlc.iter().filter_map(|dlc| dlc.name.as_ref()) {
        if !hints.contains(name) {
            hints.push(name.clone());
        }
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
    force: bool,
    publisher_ids: Vec<String>,
) -> QueryContext {
    QueryContext {
        appid: game.appid,
        name: game.name.clone(),
        localized_name: game.localized_name.clone(),
        related_dlc: game.related_dlc.clone(),
        name_hints: name_hints.to_vec(),

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
    entry.appid.is_some_and(|appid| {
        appid == query.appid || query.related_dlc.iter().any(|dlc| dlc.appid == appid)
    }) || upstream_identity(&entry.page, query.appid, known).is_some()
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
    let page = canonical::canonical_page_url(page);
    known.iter().find(|candidate| {
        candidate_is_credible(candidate)
            && candidate.game_appid == Some(appid)
            && candidate
                .upstream_page
                .as_ref()
                .is_some_and(|upstream| canonical::canonical_page_url(upstream) == page)
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

fn error_diagnostic(rule: &SourceRule, error: std::io::Error) -> Diagnostic {
    let error = crate::projection::DomainError::from(error);
    Diagnostic {
        source_id: Some(rule.id.clone()),
        code: error.code,
        detail: error.detail,
    }
}

fn cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Relaxed) || crate::operation::check().is_err()
}

fn cancelled_error() -> std::io::Error {
    invalid("task_cancelled")
}
