//! Per-source index snapshots (p1-contract §5): revision semantics, entry
//! normalization and the schema-1 → schema-2 migration converters.
//!
//! The index only holds fields the list page gives directly; detail scraping is
//! deliberately out of scope here. [`from_cache`]/[`from_discovery_index`] are
//! migration-only: nothing reads the legacy `cache/`/`discovery-index/` trees
//! after the one-time v1 → v2 upgrade.

use super::canonical;
use super::model::{DiscoveryIndexRecord, SourceCache, SourceIndex, SourceIndexEntry, SourceRule};
use super::SOURCE_INDEX_SCHEMA_VERSION;

pub fn parser_fingerprint(rule: &SourceRule) -> Option<String> {
    rule.script.as_ref().map(|_| super::rule_fingerprint(rule))
}

/// An old snapshot remains on disk but cannot authorize a new parser.
pub fn compatible(index: &SourceIndex, rule: &SourceRule) -> bool {
    index.source_id == rule.id
        && index.schema_version == SOURCE_INDEX_SCHEMA_VERSION
        && index.parser_fingerprint == parser_fingerprint(rule)
}

pub(crate) fn bind_candidate(rule: &SourceRule, candidate: &mut super::model::SourceCandidate) {
    let Some(fingerprint) = parser_fingerprint(rule) else {
        return;
    };
    if candidate.parser_fingerprint.is_none() {
        candidate.candidate_id =
            canonical::candidate_id(&serde_json::json!([candidate.candidate_id, fingerprint]));
    }
    candidate.parser_fingerprint = Some(fingerprint);
    candidate.fingerprint = canonical::fingerprint(
        &serde_json::to_value(&*candidate).expect("candidate serialization"),
    );
}

/// Content revision for a published index: bump only when the normalized
/// entries, `complete` or `pending` set actually changed. `checked_at` /
/// `updated_at` changes alone never bump the revision, so a re-check with the
/// same content does not force a rematch.
///
/// `previous` is the currently published index (if any); `next_entries` are
/// normalized before comparison so adapter ordering cannot bump the revision.
pub fn content_revision(
    previous_revision: u64,
    previous: Option<&SourceIndex>,
    next_entries: &[SourceIndexEntry],
    complete: bool,
    pending: &[String],
) -> u64 {
    let Some(previous) = previous else {
        return previous_revision.max(1);
    };
    let unchanged = previous.complete == complete
        && normalize_entries(previous.entries.clone()) == normalize_entries(next_entries.to_vec())
        && pending_key(&previous.pending) == pending_key(pending);
    if unchanged {
        previous_revision
    } else {
        previous_revision.saturating_add(1)
    }
}

/// Canonicalizes `page`, sorts/dedupes aliases and returns a stable,
/// duplicate-free entry list (sorted by page, then the remaining fields).
pub fn normalize_entries(
    entries: impl IntoIterator<Item = SourceIndexEntry>,
) -> Vec<SourceIndexEntry> {
    let mut entries: Vec<SourceIndexEntry> = entries.into_iter().map(normalize_entry).collect();
    entries.sort_by(|left, right| {
        left.page
            .cmp(&right.page)
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.url.cmp(&right.url))
            .then_with(|| left.language.cmp(&right.language))
            .then_with(|| left.date.cmp(&right.date))
            .then_with(|| left.appid.cmp(&right.appid))
            .then_with(|| left.aliases.cmp(&right.aliases))
    });
    entries.dedup();
    entries
}

fn normalize_entry(mut entry: SourceIndexEntry) -> SourceIndexEntry {
    entry.page = canonical::canonical_url(&entry.page);
    entry.aliases.sort();
    entry.aliases.dedup();
    entry
}

fn pending_key(pending: &[String]) -> Vec<String> {
    let mut key = pending.to_vec();
    key.sort();
    key.dedup();
    key
}

/// Converts a v1 table-source cache into a complete or partial index snapshot.
pub fn from_cache(source_id: &str, cache: &SourceCache) -> SourceIndex {
    let entries = normalize_entries(cache.entries.iter().map(|entry| SourceIndexEntry {
        title: entry.title.clone(),
        aliases: entry.aliases.clone(),
        // Table sources: the list page is all the index can point at; the
        // artifact URL lives in `url`.
        page: entry.source_page.clone(),
        kind: "entry".to_string(),
        appid: entry.appid,
        url: Some(entry.url.clone()),
        language: Some(entry.language.clone()),
        date: entry.date.clone(),
    }));
    let complete = cache.cached_at.is_some();
    SourceIndex {
        parser_fingerprint: None,
        schema_version: SOURCE_INDEX_SCHEMA_VERSION,
        source_id: source_id.to_string(),
        revision: content_revision(0, None, &entries, complete, &[]),
        complete,
        checked_at: cache.checked_at.or(cache.cached_at).unwrap_or(0),
        updated_at: cache.cached_at.unwrap_or(0),
        etag: cache.etag.clone(),
        modified: cache.modified.clone(),
        entries,
        pending: Vec::new(),
    }
}

/// Converts a v1 shared discovery index (Kagura) into an index snapshot.
/// `pending` is preserved verbatim; a non-empty queue means the snapshot is not
/// complete yet.
pub fn from_discovery_index(source_id: &str, record: &DiscoveryIndexRecord) -> SourceIndex {
    let mut seen = std::collections::BTreeSet::new();
    let mut entries = Vec::with_capacity(record.index.records.len());
    for item in &record.index.records {
        let page = canonical::canonical_url(&item.page);
        if !seen.insert(page.clone()) {
            continue;
        }
        entries.push(SourceIndexEntry {
            title: item.title.clone(),
            aliases: Vec::new(),
            page,
            kind: item.kind.clone(),
            appid: None,
            url: None,
            language: None,
            date: None,
        });
    }
    let entries = normalize_entries(entries);
    let pending = record.index.pending.clone();
    let complete = pending.is_empty();
    SourceIndex {
        parser_fingerprint: None,
        schema_version: SOURCE_INDEX_SCHEMA_VERSION,
        source_id: source_id.to_string(),
        revision: content_revision(0, None, &entries, complete, &pending),
        complete,
        checked_at: record.checked_at,
        updated_at: record.checked_at,
        etag: None,
        modified: None,
        pending,
        entries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::model::{DiscoveryIndex, IndexRecord, SourceEntry};

    fn entry(title: &str, page: &str) -> SourceIndexEntry {
        SourceIndexEntry {
            title: title.to_string(),
            aliases: Vec::new(),
            page: page.to_string(),
            kind: "entry".to_string(),
            appid: None,
            url: None,
            language: None,
            date: None,
        }
    }

    fn index(entries: Vec<SourceIndexEntry>, complete: bool, pending: Vec<String>) -> SourceIndex {
        SourceIndex {
            parser_fingerprint: None,
            schema_version: SOURCE_INDEX_SCHEMA_VERSION,
            source_id: "patchmoe".to_string(),
            revision: 1,
            complete,
            checked_at: 100,
            updated_at: 100,
            etag: None,
            modified: None,
            entries,
            pending,
        }
    }

    #[test]
    fn revision_bumps_only_on_real_content_changes() {
        let previous = index(vec![entry("A", "https://example.com/a")], true, Vec::new());
        // Same content: order/time differences do not bump.
        let mut reordered = previous.clone();
        reordered.checked_at = 200;
        reordered.updated_at = 200;
        assert_eq!(
            content_revision(7, Some(&reordered), &previous.entries, true, &[]),
            7
        );
        // Entries changed.
        assert_eq!(
            content_revision(
                7,
                Some(&previous),
                &[
                    entry("A", "https://example.com/a"),
                    entry("B", "https://example.com/b")
                ],
                true,
                &[]
            ),
            8
        );
        // Completeness changed.
        assert_eq!(
            content_revision(7, Some(&previous), &previous.entries, false, &[]),
            8
        );
        // Pending changed.
        assert_eq!(
            content_revision(
                7,
                Some(&previous),
                &previous.entries,
                true,
                &["https://example.com/page/2/".to_string()]
            ),
            8
        );
        // No previous index starts at revision 1.
        assert_eq!(content_revision(0, None, &previous.entries, true, &[]), 1);
    }

    #[test]
    fn cache_entries_map_url_appid_language_and_date() {
        let cache = SourceCache {
            entries: vec![SourceEntry {
                title: "Game".to_string(),
                aliases: vec!["Alias".to_string()],
                url: "https://download.patch.moe/g.zip".to_string(),
                source_page: "https://www.patch.moe/?lang=en".to_string(),
                language: "en".to_string(),
                date: Some("2024/1/2".to_string()),
                appid: Some(42),
            }],
            etag: Some("etag".to_string()),
            modified: Some("modified".to_string()),
            cached_at: Some(300),
            checked_at: Some(310),
        };
        let index = from_cache("patchmoe", &cache);
        assert_eq!(index.revision, 1);
        assert!(index.complete);
        assert_eq!(index.checked_at, 310);
        assert_eq!(index.updated_at, 300);
        assert_eq!(index.etag.as_deref(), Some("etag"));
        assert_eq!(index.modified.as_deref(), Some("modified"));
        assert_eq!(
            index.entries,
            vec![SourceIndexEntry {
                title: "Game".to_string(),
                aliases: vec!["Alias".to_string()],
                page: "https://www.patch.moe/".to_string(),
                kind: "entry".to_string(),
                appid: Some(42),
                url: Some("https://download.patch.moe/g.zip".to_string()),
                language: Some("en".to_string()),
                date: Some("2024/1/2".to_string()),
            }]
        );
        // No cached_at means the v1 cache never completed.
        let partial = from_cache(
            "patchmoe",
            &SourceCache {
                cached_at: None,
                ..cache
            },
        );
        assert!(!partial.complete);
        assert_eq!(partial.updated_at, 0);
    }

    #[test]
    fn discovery_index_records_map_and_pending_marks_incomplete() {
        let record = DiscoveryIndexRecord {
            schema_version: 1,
            checked_at: 500,
            index: DiscoveryIndex {
                parser_version: 2,
                encoding_version: 1,
                records: vec![
                    IndexRecord {
                        title: "Patch A".to_string(),
                        page: "https://kagurafan.com/p/1/".to_string(),
                        kind: "product".to_string(),
                    },
                    IndexRecord {
                        title: "Patch A duplicate".to_string(),
                        page: "https://kagurafan.com/p/1/?utm=x".to_string(),
                        kind: "product".to_string(),
                    },
                ],
                pending: vec!["https://kagurafan.com/page/3/".to_string()],
                checked_at: 490,
            },
        };
        let index = from_discovery_index("kagura-cn", &record);
        assert_eq!(index.revision, 1);
        assert!(!index.complete);
        assert_eq!(index.checked_at, 500);
        assert_eq!(
            index.pending,
            vec!["https://kagurafan.com/page/3/".to_string()]
        );
        // Duplicate pages collapse; the first record wins.
        assert_eq!(index.entries.len(), 1);
        assert_eq!(index.entries[0].title, "Patch A");
        assert_eq!(index.entries[0].kind, "product");

        let complete = from_discovery_index(
            "kagura-cn",
            &DiscoveryIndexRecord {
                checked_at: 500,
                index: DiscoveryIndex {
                    pending: Vec::new(),
                    ..record.index
                },
                ..record
            },
        );
        assert!(complete.complete);
    }

    #[test]
    fn table_rows_sharing_one_list_page_are_not_collapsed() {
        // patch.moe rows all share the localized list page; dedupe must keep
        // every distinct artifact.
        let cache = SourceCache {
            entries: vec![
                SourceEntry {
                    title: "A".to_string(),
                    aliases: Vec::new(),
                    url: "https://download.patch.moe/a.zip".to_string(),
                    source_page: "https://www.patch.moe/".to_string(),
                    language: "en".to_string(),
                    date: None,
                    appid: None,
                },
                SourceEntry {
                    title: "B".to_string(),
                    aliases: Vec::new(),
                    url: "https://download.patch.moe/b.zip".to_string(),
                    source_page: "https://www.patch.moe/".to_string(),
                    language: "en".to_string(),
                    date: None,
                    appid: None,
                },
            ],
            ..SourceCache::default()
        };
        assert_eq!(from_cache("patchmoe", &cache).entries.len(), 2);
    }
}
