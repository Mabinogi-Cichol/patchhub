//! Per-source index update (P2, design §4.1, §6; p1-contract §4, §5).
//!
//! One update is scoped to one source and never looks at the installed game
//! list. A complete result replaces `index/<source>.json`; partial, cancelled
//! and failed attempts save `index-progress/<source>.json` (or nothing when no
//! page was collected) and leave the official snapshot intact.

use super::adapters::{self, IndexOutcome};
use super::canonical;
use super::http::RequestContext;
use super::index::{self, content_revision, normalize_entries};
use super::model::{SourceIndex, SourceIndexEntry, SourceRole, SourceRule, SourceStatus};
use super::store::SourceStore;
use super::{invalid, Result, SOURCE_INDEX_SCHEMA_VERSION};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// Non-wire result of one source update. `revision`/`complete` describe the
/// currently published official snapshot; `SourceStatus` gains those fields
/// only with wire v5 (P4), so callers read them from this outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceUpdateOutcome {
    pub status: SourceStatus,
    pub revision: u64,
    pub complete: bool,
}

/// Updates one source's shared index once, so every game can reuse it.
///
/// There is deliberately no game/instance argument: the request count is
/// independent of the installed game count. `on_progress` receives the working
/// snapshot whenever one was published or saved as progress.
pub fn update_source(
    store: &SourceStore,
    rule: &SourceRule,
    force: bool,
    ctx: &mut RequestContext,
    cancel: &Arc<AtomicBool>,
    mut on_progress: impl FnMut(&SourceIndex),
) -> Result<SourceUpdateOutcome> {
    crate::operation::with_cancel(Arc::clone(cancel), || {
        // HGamer is queried per game during matching; it is never updated site-wide.
        if rule.role == SourceRole::FallbackIndexer {
            return Err(invalid("source_not_indexable"));
        }
        let settings = store.load_settings()?;
        if super::script::rule_changed(rule, &settings) {
            return Err(invalid("source_rule_changed"));
        }
        let adapter = adapters::source_adapter(
            rule,
            settings.script_sources.get(&rule.id).map(String::as_str),
        )?;
        let stored_official = store.load_index(&rule.id)?;
        let official = stored_official
            .clone()
            .filter(|index| index::compatible(index, rule));
        let progress = store
            .load_index_progress(&rule.id)?
            .filter(|index| index::compatible(index, rule));
        // Resume unfinished work first; otherwise the published snapshot only
        // supplies conditional validators. `force` disables those revalidators.
        let mut previous = progress.clone().or_else(|| official.clone());
        if force {
            if let Some(previous) = previous.as_mut() {
                previous.etag = None;
                previous.modified = None;
            }
        }

        let outcome = match adapter.collect_index(rule, previous.as_ref(), ctx, cancel) {
            Ok(outcome) => outcome,
            Err(error) => {
                if cancelled(cancel) {
                    return Err(invalid("task_cancelled"));
                }
                // An unreachable source is a status, not a hard task failure; its
                // last successful snapshot stays untouched. Malformed source data
                // (columns/forms/structure) is a hard error.
                if is_transport_failure(&error.to_string()) {
                    return finish_failure(store, rule, official.as_ref());
                }
                return Err(error);
            }
        };

        if cancelled(cancel) {
            save_progress(store, rule, official.as_ref(), &outcome, &mut on_progress)?;
            return Err(invalid("task_cancelled"));
        }

        // The host persists settings from another process, so the source may have
        // been disabled while the network work was running. Publishing then would
        // reintroduce a source the user just turned off (design §4.3); keep the
        // collected pages as private progress instead.
        let latest = store.load_settings()?;
        if super::script::rule_changed(rule, &latest) {
            return Err(invalid("source_rule_changed"));
        }
        if !latest
            .enabled
            .get(&rule.id)
            .copied()
            .unwrap_or(rule.enabled)
        {
            if !outcome.not_modified {
                save_progress(store, rule, official.as_ref(), &outcome, &mut on_progress)?;
            }
            return Err(invalid("source_not_enabled"));
        }

        if outcome.not_modified {
            if progress.is_none() {
                if let Some(current) = official.as_ref() {
                    return finish_not_modified(store, rule, current, &outcome, &mut on_progress);
                }
            }
            // A 304 without a usable published snapshot cannot reuse anything.
            return finish_failure(store, rule, official.as_ref());
        }

        if outcome.complete && outcome.pending.is_empty() {
            return publish(
                store,
                rule,
                stored_official.as_ref(),
                normalize_entries(outcome.entries.iter().cloned()),
                &outcome,
                &mut on_progress,
            );
        }

        save_progress(store, rule, official.as_ref(), &outcome, &mut on_progress)?;
        // A pass that collected nothing is a refresh failure, not a partial index.
        let code = if outcome.entries.is_empty() {
            if official.is_some() {
                "source_offline_cached"
            } else {
                "source_refresh_failed"
            }
        } else {
            "source_partial"
        };
        let status = SourceStatus {
            checked_at: outcome_checked_at(&outcome),
            cached_at: snapshot_cached_at(official.as_ref()),
            entry_count: snapshot_entry_count(official.as_ref()),
            code: code.to_string(),
        };
        store.save_status(&rule.id, &status)?;
        Ok(SourceUpdateOutcome {
            status,
            revision: snapshot_revision(official.as_ref()),
            complete: snapshot_complete(official.as_ref()),
        })
    })
}

/// Publishes a complete collection as the new official snapshot.
fn publish(
    store: &SourceStore,
    rule: &SourceRule,
    official: Option<&SourceIndex>,
    entries: Vec<SourceIndexEntry>,
    outcome: &IndexOutcome,
    on_progress: &mut impl FnMut(&SourceIndex),
) -> Result<SourceUpdateOutcome> {
    let checked_at = outcome_checked_at(outcome);
    let fingerprint = index::parser_fingerprint(rule);
    let revision = if official.is_some_and(|old| old.parser_fingerprint != fingerprint) {
        snapshot_revision(official).saturating_add(1)
    } else {
        content_revision(snapshot_revision(official), official, &entries, true, &[])
    };
    let index = SourceIndex {
        parser_fingerprint: index::parser_fingerprint(rule),
        schema_version: SOURCE_INDEX_SCHEMA_VERSION,
        source_id: rule.id.clone(),
        revision,
        complete: true,
        checked_at,
        updated_at: checked_at,
        etag: outcome.etag.clone(),
        modified: outcome.modified.clone(),
        entries,
        pending: Vec::new(),
    };
    store.save_index(&index)?;
    store.clear_index_progress(&rule.id)?;
    let status = SourceStatus {
        checked_at,
        cached_at: Some(checked_at),
        entry_count: index.entries.len() as u64,
        code: "source_refreshed".to_string(),
    };
    store.save_status(&rule.id, &status)?;
    on_progress(&index);
    Ok(SourceUpdateOutcome {
        status,
        revision,
        complete: true,
    })
}

/// A `304`: keep the published entries and only refresh `checked_at`/validators.
fn finish_not_modified(
    store: &SourceStore,
    rule: &SourceRule,
    current: &SourceIndex,
    outcome: &IndexOutcome,
    on_progress: &mut impl FnMut(&SourceIndex),
) -> Result<SourceUpdateOutcome> {
    let checked_at = outcome_checked_at(outcome);
    let index = SourceIndex {
        checked_at,
        etag: outcome.etag.clone().or_else(|| current.etag.clone()),
        modified: outcome
            .modified
            .clone()
            .or_else(|| current.modified.clone()),
        ..current.clone()
    };
    store.save_index(&index)?;
    let status = SourceStatus {
        checked_at,
        cached_at: Some(current.updated_at),
        entry_count: current.entries.len() as u64,
        code: "source_not_modified".to_string(),
    };
    store.save_status(&rule.id, &status)?;
    on_progress(&index);
    Ok(SourceUpdateOutcome {
        status,
        revision: current.revision,
        complete: current.complete,
    })
}

/// Saves the unfinished entries plus the remaining page queue. The official
/// snapshot is not touched.
fn save_progress(
    store: &SourceStore,
    rule: &SourceRule,
    official: Option<&SourceIndex>,
    outcome: &IndexOutcome,
    on_progress: &mut impl FnMut(&SourceIndex),
) -> Result<()> {
    let mut pending: Vec<String> = outcome
        .pending
        .iter()
        .map(|page| canonical::canonical_url(page))
        .collect();
    pending.sort();
    pending.dedup();
    let progress = SourceIndex {
        parser_fingerprint: index::parser_fingerprint(rule),
        schema_version: SOURCE_INDEX_SCHEMA_VERSION,
        source_id: rule.id.clone(),
        revision: snapshot_revision(official).max(1),
        complete: false,
        checked_at: outcome_checked_at(outcome),
        updated_at: snapshot_cached_at(official).unwrap_or(0),
        etag: outcome.etag.clone(),
        modified: outcome.modified.clone(),
        entries: normalize_entries(outcome.entries.iter().cloned()),
        pending,
    };
    store.save_index_progress(&progress)?;
    on_progress(&progress);
    Ok(())
}

/// Records an unreachable source: the last successful snapshot is preserved and
/// the unfinished list pages stay queued as progress (an existing, richer queue
/// is kept as-is).
fn finish_failure(
    store: &SourceStore,
    rule: &SourceRule,
    official: Option<&SourceIndex>,
) -> Result<SourceUpdateOutcome> {
    if store.load_index_progress(&rule.id)?.is_none() {
        let mut pending: Vec<String> = rule
            .pages
            .iter()
            .map(|page| canonical::canonical_url(page))
            .collect();
        pending.sort();
        pending.dedup();
        store.save_index_progress(&SourceIndex {
            parser_fingerprint: index::parser_fingerprint(rule),
            schema_version: SOURCE_INDEX_SCHEMA_VERSION,
            source_id: rule.id.clone(),
            revision: snapshot_revision(official).max(1),
            complete: false,
            checked_at: adapters::now_unix(),
            updated_at: snapshot_cached_at(official).unwrap_or(0),
            etag: None,
            modified: None,
            entries: Vec::new(),
            pending,
        })?;
    }
    let status = SourceStatus {
        checked_at: adapters::now_unix(),
        cached_at: snapshot_cached_at(official),
        entry_count: snapshot_entry_count(official),
        code: if official.is_some() {
            "source_offline_cached"
        } else {
            "source_refresh_failed"
        }
        .to_string(),
    };
    store.save_status(&rule.id, &status)?;
    Ok(SourceUpdateOutcome {
        status,
        revision: snapshot_revision(official),
        complete: snapshot_complete(official),
    })
}

fn outcome_checked_at(outcome: &IndexOutcome) -> u64 {
    if outcome.checked_at == 0 {
        adapters::now_unix()
    } else {
        outcome.checked_at
    }
}

fn snapshot_revision(official: Option<&SourceIndex>) -> u64 {
    official.map(|index| index.revision).unwrap_or(0)
}

fn snapshot_complete(official: Option<&SourceIndex>) -> bool {
    official.is_some_and(|index| index.complete)
}

fn snapshot_entry_count(official: Option<&SourceIndex>) -> u64 {
    official
        .map(|index| index.entries.len() as u64)
        .unwrap_or(0)
}

fn snapshot_cached_at(official: Option<&SourceIndex>) -> Option<u64> {
    official.map(|index| index.updated_at)
}

fn cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Relaxed) || crate::operation::check().is_err()
}

/// Transport failures become a source status; everything else (malformed
/// pages/columns/forms) is surfaced as an error.
fn is_transport_failure(code: &str) -> bool {
    matches!(
        code,
        "discovery_network_error" | "discovery_retry_exhausted"
    ) || code.starts_with("discovery_http_")
        || code.starts_with("source_http_")
}
