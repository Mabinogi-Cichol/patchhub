//! P1 schema-2 source data: index/detail/match persistence, the one-time
//! schema 1 → 2 migration, and the pure revision/input/schedule helpers
//! (p1-contract §12).

use patchhub_core::installer;
use patchhub_core::sources::{
    index::content_revision,
    model::*,
    r#match::{derive_game_status, input_key},
    schedule::{interval_hours, is_due, next_due_at},
    snapshot::SourceSnapshot,
    store::{detail_key, SourceStore},
    Sources, GAME_MATCH_SCHEMA_VERSION, SOURCE_DETAIL_SCHEMA_VERSION, SOURCE_INDEX_SCHEMA_VERSION,
    STORE_SCHEMA_VERSION,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

// ---- fixtures ------------------------------------------------------------

fn write_json(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn index_entry(title: &str, page: &str, url: &str) -> SourceIndexEntry {
    SourceIndexEntry {
        title: title.to_string(),
        aliases: Vec::new(),
        page: page.to_string(),
        kind: "entry".to_string(),
        appid: None,
        url: Some(url.to_string()),
        language: Some("en".to_string()),
        date: None,
    }
}

/// Minimal, structurally valid `SourceCandidate` JSON (the P1 code preserves
/// candidates verbatim and never re-derives them).
fn candidate_json(
    serial: u64,
    appid: u32,
    title: &str,
    source_id: &str,
    page: &str,
    basis: &str,
) -> Value {
    json!({
        "candidate_id": format!("candidate-{serial:024x}"),
        "source_id": source_id,
        "source_record_id": page,
        "game_appid": appid,
        "parent_appid": null,
        "game_title": title,
        "patch_title": format!("{title} Patch"),
        "variant": "base",
        "requires_dlc": null,
        "excludes_dlc": null,
        "patch_kind": "adult_content",
        "version_raw": null,
        "version_identity": "1.0",
        "published_at": null,
        "patch_updated_at": null,
        "source_page": page,
        "upstream_page": null,
        "format_reported": "zip",
        "format_observed": "zip",
        "capability": "discover_only",
        "match_basis": basis,
        "observed_at": "2024-01-02T00:00:00Z",
        "cache_status": "cached",
        "fingerprint": format!("fingerprint-{serial}"),
    })
}

fn candidate(
    serial: u64,
    appid: u32,
    title: &str,
    source_id: &str,
    page: &str,
    basis: &str,
) -> SourceCandidate {
    serde_json::from_value(candidate_json(serial, appid, title, source_id, page, basis)).unwrap()
}

fn discovery_success(
    appid: u32,
    source_id: &str,
    candidates: Vec<Value>,
    status: &str,
    checked_at: u64,
    query_key: &str,
) -> Value {
    let cached_at = if status == "complete" {
        Value::from(checked_at)
    } else {
        Value::Null
    };
    json!({
        "schema_version": 1,
        "source_id": source_id,
        "appid": appid,
        "success": {
            "status": status,
            "candidates": candidates,
            "diagnostics": [],
            "checked_at": checked_at,
            "cached_at": cached_at,
            "query_key": query_key,
            "parser_version": 2,
            "query_version": 1,
            "encoding_version": 1,
        },
        "attempt": null,
    })
}

fn source_match(index_revision: u64, status: SourceMatchStatus) -> SourceMatchRecord {
    SourceMatchRecord {
        index_revision,
        status,
        candidates: Vec::new(),
        detail_pages: Vec::new(),
        checked_at: 1,
        diagnostics: Vec::new(),
    }
}

fn fallback_match(status: FallbackMatchStatus) -> FallbackMatchRecord {
    FallbackMatchRecord {
        query_key: "q".to_string(),
        status,
        candidates: Vec::new(),
        checked_at: 1,
        diagnostics: Vec::new(),
    }
}

fn game(
    sources: &[(&str, SourceMatchStatus)],
    fallback: Option<FallbackMatchStatus>,
) -> GameMatchRecord {
    GameMatchRecord {
        schema_version: GAME_MATCH_SCHEMA_VERSION,
        appid: 42,
        name: "Game".to_string(),
        localized_name: None,
        input_key: "key".to_string(),
        status: GameMatchStatus::Pending,
        sources: sources
            .iter()
            .map(|(id, status)| (id.to_string(), source_match(1, *status)))
            .collect(),
        fallback: fallback.map(fallback_match),
        updated_at: 1,
    }
}

fn collect_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.insert(path.clone(), fs::read(path).unwrap());
            }
        }
    }
    files
}

fn has_temp_leftovers(root: &Path) -> bool {
    collect_files(root).keys().any(|path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension == "tmp")
    })
}

// ---- 1. record round trips, atomic writes, failure isolation -------------

#[test]
fn schema2_records_round_trip_and_failed_writes_keep_the_last_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    assert_eq!(STORE_SCHEMA_VERSION, 2);

    let index = SourceIndex {
        schema_version: SOURCE_INDEX_SCHEMA_VERSION,
        source_id: "patchmoe".to_string(),
        revision: 3,
        complete: true,
        checked_at: 10,
        updated_at: 9,
        etag: Some("etag-1".to_string()),
        modified: None,
        entries: vec![
            index_entry(
                "A",
                "https://www.patch.moe/",
                "https://download.patch.moe/a.zip",
            ),
            index_entry(
                "B",
                "https://www.patch.moe/",
                "https://download.patch.moe/b.zip",
            ),
        ],
        pending: Vec::new(),
    };
    store.save_index(&index).unwrap();
    assert_eq!(store.load_index("patchmoe").unwrap(), Some(index.clone()));
    assert!(store.paths().index("patchmoe").is_file());
    assert_eq!(
        read_json(&store.paths().index("patchmoe"))["schema_version"],
        json!(1)
    );

    let detail = DetailRecord {
        schema_version: SOURCE_DETAIL_SCHEMA_VERSION,
        source_id: "patchmoe".to_string(),
        page: "https://www.patch.moe/".to_string(),
        index_revision: 3,
        checked_at: 11,
        complete: true,
        fact: Some(PageFact {
            title: "Game".to_string(),
            page: "https://www.patch.moe/".to_string(),
            kind: "entry".to_string(),
            ..PageFact::default()
        }),
        diagnostics: Vec::new(),
    };
    store.save_detail(&detail).unwrap();
    // The key is derived from the canonical page: query/fragment are dropped.
    assert_eq!(
        store
            .load_detail("patchmoe", "https://www.patch.moe/?utm=x#top")
            .unwrap(),
        Some(detail.clone())
    );
    assert!(store
        .paths()
        .detail("patchmoe", &detail_key(&detail.page))
        .is_file());

    let record = GameMatchRecord {
        schema_version: GAME_MATCH_SCHEMA_VERSION,
        appid: 42,
        name: "Game".to_string(),
        localized_name: Some("游戏".to_string()),
        input_key: "digest".to_string(),
        status: GameMatchStatus::PublisherMatch,
        sources: BTreeMap::from([(
            "patchmoe".to_string(),
            SourceMatchRecord {
                index_revision: 3,
                status: SourceMatchStatus::Matched,
                candidates: vec![candidate(
                    1,
                    42,
                    "Game",
                    "patchmoe",
                    "https://www.patch.moe/",
                    "appid",
                )],
                detail_pages: vec!["https://www.patch.moe/".to_string()],
                checked_at: 11,
                diagnostics: Vec::new(),
            },
        )]),
        fallback: None,
        updated_at: 11,
    };
    store.save_match(&record).unwrap();
    assert_eq!(store.load_match(42).unwrap(), Some(record.clone()));
    assert!(store.paths().matches(42).is_file());
    assert!(!has_temp_leftovers(store.paths().root()));

    // A failed write must not disturb an already published snapshot.
    let index_before = fs::read(store.paths().index("patchmoe")).unwrap();
    fs::create_dir_all(store.paths().index("blocked")).unwrap();
    let blocked = SourceIndex {
        source_id: "blocked".to_string(),
        ..index.clone()
    };
    assert_eq!(
        store.save_index(&blocked).unwrap_err().to_string(),
        "linked_source_state"
    );
    assert_eq!(
        fs::read(store.paths().index("patchmoe")).unwrap(),
        index_before
    );

    let detail_before = fs::read(
        store
            .paths()
            .detail("patchmoe", &detail_key("https://www.patch.moe/")),
    )
    .unwrap();
    let blocked_page = "https://www.patch.moe/blocked/";
    fs::create_dir_all(store.paths().detail("patchmoe", &detail_key(blocked_page))).unwrap();
    let blocked_detail = DetailRecord {
        page: blocked_page.to_string(),
        ..detail.clone()
    };
    assert_eq!(
        store.save_detail(&blocked_detail).unwrap_err().to_string(),
        "linked_source_state"
    );
    assert_eq!(
        fs::read(
            store
                .paths()
                .detail("patchmoe", &detail_key("https://www.patch.moe/"))
        )
        .unwrap(),
        detail_before
    );
    assert_eq!(
        store
            .load_detail("patchmoe", "https://www.patch.moe/")
            .unwrap(),
        Some(detail)
    );
    assert!(!has_temp_leftovers(store.paths().root()));
}

// ---- 2. schema 1 -> 2 migration ------------------------------------------

fn build_v1_state(state: &Path) -> PathBuf {
    let root = state.join("sources/rust-v1");
    write_json(
        &root.join("schema.json"),
        &json!({"schema_version": 1, "created_at": 123}),
    );
    write_json(
        &root.join("settings.json"),
        &json!({
            "schema_version": 1,
            "revision": 7,
            "enabled": {"patchmoe": false, "hgamer": true},
            "added": [{
                "id": "extra",
                "name": "Extra",
                "adapter": "kagura_wordpress",
                "role": "publisher",
                "discovery_only": true,
                "priority": 50,
                "enabled": true,
                "pages": ["https://extra.example/"],
                "page_hosts": ["extra.example"],
                "download_hosts": ["extra.example"]
            }],
            "games": {"42": {"patchmoe": false}}
        }),
    );
    write_json(
        &root.join("status/patchmoe.json"),
        &json!({"checked_at": 100, "cached_at": 90, "entry_count": 2, "code": "source_refreshed"}),
    );
    write_json(
        &root.join("cache/patchmoe.json"),
        &json!({
            "entries": [
                {
                    "title": "Game 42 EN",
                    "aliases": [],
                    "url": "https://download.patch.moe/42.zip",
                    "source_page": "https://www.patch.moe/",
                    "language": "en",
                    "date": "2024/1/2",
                    "appid": 42
                },
                {
                    "title": "Game 42 JP",
                    "aliases": ["Game 42"],
                    "url": "https://download.patch.moe/42-jp.zip",
                    "source_page": "https://www.patch.moe/",
                    "language": "ja",
                    "date": null,
                    "appid": 42
                }
            ],
            "etag": "etag-1",
            "modified": "mod-1",
            "cached_at": 90,
            "checked_at": 100
        }),
    );
    // Same source also has a legacy discovery index: the cache snapshot must win,
    // while its entries/pending are preserved and a diagnostic is recorded.
    write_json(
        &root.join("discovery-index/patchmoe.json"),
        &json!({
            "schema_version": 1,
            "checked_at": 110,
            "index": {
                "parser_version": 2,
                "encoding_version": 1,
                "records": [{
                    "title": "Patch Page",
                    "page": "https://www.patch.moe/p/7/",
                    "kind": "product"
                }],
                "pending": ["https://www.patch.moe/page/2/"],
                "checked_at": 110
            }
        }),
    );
    write_json(
        &root.join("discovery-index/kagura-cn.json"),
        &json!({
            "schema_version": 1,
            "checked_at": 200,
            "index": {
                "parser_version": 2,
                "encoding_version": 1,
                "records": [{
                    "title": "Kagura Patch",
                    "page": "https://kagurafan.com/p/1/",
                    "kind": "product"
                }],
                "pending": [],
                "checked_at": 200
            }
        }),
    );
    write_json(
        &root.join("discovery-index/kagura-en.json"),
        &json!({
            "schema_version": 1,
            "checked_at": 210,
            "index": {
                "parser_version": 2,
                "encoding_version": 1,
                "records": [{
                    "title": "Kagura EN Patch",
                    "page": "https://www.kaguragames.com/p/9/",
                    "kind": "article"
                }],
                "pending": ["https://www.kaguragames.com/page/2/"],
                "checked_at": 210
            }
        }),
    );
    write_json(
        &root.join("discovery/kagura-cn/42.json"),
        &discovery_success(
            42,
            "kagura-cn",
            vec![candidate_json(
                1,
                42,
                "Game 42",
                "kagura-cn",
                "https://kagurafan.com/p/42/",
                "appid",
            )],
            "complete",
            200,
            "query-42-kagura",
        ),
    );
    write_json(
        &root.join("discovery/hgamer/42.json"),
        &discovery_success(
            42,
            "hgamer",
            vec![candidate_json(
                2,
                42,
                "Game 42",
                "hgamer",
                "https://hgamer.tw/p/42/",
                "appid",
            )],
            "complete",
            200,
            "query-42-hgamer",
        ),
    );
    // Completed empty result on both a publisher and the fallback source.
    write_json(
        &root.join("discovery/kagura-cn/43.json"),
        &discovery_success(43, "kagura-cn", Vec::new(), "complete", 220, "query-43"),
    );
    write_json(
        &root.join("discovery/hgamer/43.json"),
        &discovery_success(43, "hgamer", Vec::new(), "complete", 220, "query-43-hgamer"),
    );
    // Candidate without any index snapshot: must be no_index, never "matched".
    write_json(
        &root.join("discovery/extra/44.json"),
        &discovery_success(
            44,
            "extra",
            vec![candidate_json(
                3,
                44,
                "Game 44",
                "extra",
                "https://extra.example/p/44/",
                "appid",
            )],
            "complete",
            300,
            "query-44",
        ),
    );
    // Title-only lead must stay a candidate but not claim a match.
    write_json(
        &root.join("discovery/kagura-cn/45.json"),
        &discovery_success(
            45,
            "kagura-cn",
            vec![candidate_json(
                4,
                45,
                "Game 45",
                "kagura-cn",
                "https://kagurafan.com/p/45/",
                "title",
            )],
            "complete",
            230,
            "query-45",
        ),
    );
    write_json(
        &root.join("discovery/hgamer/45.json"),
        &discovery_success(45, "hgamer", Vec::new(), "complete", 230, "query-45-hgamer"),
    );
    root
}

#[test]
fn schema1_state_migrates_to_schema2_without_losing_old_files() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let root = build_v1_state(&state);
    let old_before = collect_files(&root);

    let store = SourceStore::open(&state).unwrap();

    // --- settings v1 -> v2 -------------------------------------------------
    let settings = store.load_settings().unwrap();
    assert_eq!(settings.schema_version, STORE_SCHEMA_VERSION);
    assert_eq!(settings.revision, 7);
    assert!(
        settings.intervals.is_empty(),
        "default automatic updates are off"
    );
    assert_eq!(settings.enabled.get("patchmoe"), Some(&false));
    assert_eq!(
        settings
            .games
            .get("42")
            .and_then(|game| game.get("patchmoe")),
        Some(&false)
    );
    assert_eq!(settings.added.len(), 1);
    assert_eq!(settings.added[0].id, "extra");
    assert_eq!(
        read_json(&root.join("settings.json"))["schema_version"],
        json!(2)
    );

    // --- cache/<source>.json -> index/<source>.json ------------------------
    let patchmoe = store.load_index("patchmoe").unwrap().unwrap();
    assert_eq!(patchmoe.schema_version, SOURCE_INDEX_SCHEMA_VERSION);
    assert_eq!(patchmoe.revision, 1);
    assert!(
        !patchmoe.complete,
        "unfinished discovery pages keep it incomplete"
    );
    assert_eq!(patchmoe.etag.as_deref(), Some("etag-1"));
    assert_eq!(patchmoe.modified.as_deref(), Some("mod-1"));
    assert_eq!(patchmoe.checked_at, 100);
    assert_eq!(patchmoe.updated_at, 90);
    assert_eq!(
        patchmoe.pending,
        vec!["https://www.patch.moe/page/2/".to_string()]
    );
    assert_eq!(
        patchmoe.entries.len(),
        3,
        "cache rows plus discovery record"
    );
    let cache_row = patchmoe
        .entries
        .iter()
        .find(|entry| entry.url.as_deref() == Some("https://download.patch.moe/42.zip"))
        .unwrap();
    assert_eq!(cache_row.kind, "entry");
    assert_eq!(cache_row.page, "https://www.patch.moe/");
    assert_eq!(cache_row.appid, Some(42));
    assert_eq!(cache_row.language.as_deref(), Some("en"));
    assert_eq!(cache_row.date.as_deref(), Some("2024/1/2"));
    assert_eq!(
        patchmoe
            .entries
            .iter()
            .find(|entry| entry.page == "https://www.patch.moe/p/7/")
            .map(|entry| entry.kind.as_str()),
        Some("product")
    );
    assert!(
        store
            .migration_diagnostics()
            .iter()
            .any(
                |diagnostic| diagnostic.code == "source_index_migration_conflict"
                    && diagnostic.source_id.as_deref() == Some("patchmoe")
            ),
        "the cache/discovery-index conflict must be reported, not silent"
    );

    // --- discovery-index -> index (complete flag follows pending) ----------
    let kagura_cn = store.load_index("kagura-cn").unwrap().unwrap();
    assert!(kagura_cn.complete);
    assert!(kagura_cn.pending.is_empty());
    assert_eq!(kagura_cn.revision, 1);
    assert_eq!(kagura_cn.entries.len(), 1);
    assert_eq!(kagura_cn.entries[0].page, "https://kagurafan.com/p/1/");
    assert_eq!(kagura_cn.entries[0].kind, "product");
    let kagura_en = store.load_index("kagura-en").unwrap().unwrap();
    assert!(!kagura_en.complete);
    assert_eq!(
        kagura_en.pending,
        vec!["https://www.kaguragames.com/page/2/".to_string()]
    );
    assert!(store.load_index("extra").unwrap().is_none());

    // --- per-game discovery -> matches -------------------------------------
    let matched = store.load_match(42).unwrap().unwrap();
    assert_eq!(matched.name, "Game 42");
    assert_eq!(matched.status, GameMatchStatus::PublisherMatch);
    assert_eq!(matched.updated_at, 200);
    assert!(
        matched.input_key.is_empty(),
        "v1 cannot rebuild the input key"
    );
    let source = matched.sources.get("kagura-cn").unwrap();
    assert_eq!(source.status, SourceMatchStatus::Matched);
    assert_eq!(source.index_revision, 1);
    assert_eq!(source.checked_at, 200);
    assert_eq!(
        source.detail_pages,
        vec!["https://kagurafan.com/p/42/".to_string()]
    );
    assert_eq!(source.candidates.len(), 1);
    assert_eq!(source.candidates[0].fingerprint, "fingerprint-1");
    assert_eq!(
        source.candidates[0].candidate_id,
        "candidate-000000000000000000000001"
    );
    let fallback = matched.fallback.as_ref().unwrap();
    assert_eq!(fallback.status, FallbackMatchStatus::Matched);
    assert_eq!(fallback.query_key, "query-42-hgamer");
    assert_eq!(fallback.candidates[0].fingerprint, "fingerprint-2");

    let clean = store.load_match(43).unwrap().unwrap();
    assert_eq!(clean.status, GameMatchStatus::NoMatch);
    assert_eq!(clean.name, "43", "no candidate leaves the AppID as name");
    assert_eq!(
        clean.sources.get("kagura-cn").unwrap().status,
        SourceMatchStatus::NoCandidate
    );
    assert_eq!(
        clean.fallback.as_ref().unwrap().status,
        FallbackMatchStatus::NoMatch
    );

    let no_index = store.load_match(44).unwrap().unwrap();
    assert_eq!(no_index.status, GameMatchStatus::Incomplete);
    let extra = no_index.sources.get("extra").unwrap();
    assert_eq!(extra.status, SourceMatchStatus::NoIndex);
    assert_eq!(extra.index_revision, 0);
    assert_eq!(extra.candidates.len(), 1);
    assert!(no_index.fallback.is_none());
    assert_eq!(no_index.updated_at, 300);

    let title_only = store.load_match(45).unwrap().unwrap();
    assert_eq!(title_only.status, GameMatchStatus::NoMatch);
    let leads = title_only.sources.get("kagura-cn").unwrap();
    assert_eq!(leads.status, SourceMatchStatus::NoCandidate);
    assert_eq!(leads.candidates.len(), 1, "candidate evidence is preserved");

    // --- old status/entries stay readable and untouched --------------------
    let status = store.load_status("patchmoe").unwrap().unwrap();
    assert_eq!(status.code, "source_refreshed");
    assert_eq!(status.checked_at, 100);
    assert_eq!(
        read_json(&root.join("schema.json"))["schema_version"],
        json!(2)
    );
    for path in old_before.keys() {
        assert!(path.is_file(), "old file {} must survive", path.display());
    }
    let old_after = collect_files(&root);
    for (path, bytes) in &old_before {
        // `settings.json` (upgraded) and `schema.json` (completion marker) are
        // the only files the migration replaces; all legacy source data is kept
        // byte-for-byte.
        let name = path.file_name().and_then(|name| name.to_str());
        if name == Some("settings.json") || name == Some("schema.json") {
            continue;
        }
        assert_eq!(
            old_after.get(path),
            Some(bytes),
            "old file {} must keep its bytes",
            path.display()
        );
    }
    drop(store);

    // --- re-open is idempotent --------------------------------------------
    let new_before = collect_files(&root);
    let reopened = SourceStore::open(&state).unwrap();
    assert_eq!(reopened.revision().unwrap(), 7);
    assert_eq!(
        reopened.load_settings().unwrap().schema_version,
        STORE_SCHEMA_VERSION
    );
    let new_after = collect_files(&root);
    assert_eq!(
        new_before, new_after,
        "a second open must not rewrite migrated state"
    );
}

#[test]
fn readonly_projection_reads_v1_state_without_running_the_migration() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let root = build_v1_state(&state);
    let before = collect_files(&root);

    let snapshot = SourceSnapshot::load(&state, &[], 1_234).unwrap().unwrap();
    assert_eq!(snapshot.settings_revision(), 7);
    assert_eq!(
        collect_files(&root),
        before,
        "projection before the first writing open must not create or migrate files"
    );
}

// ---- 3. migration failure keeps every old byte ---------------------------

#[test]
fn failed_migration_reports_a_specific_code_and_preserves_old_files() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let root = build_v1_state(&state);
    // Corrupt one legacy per-game record.
    fs::write(root.join("discovery/kagura-cn/42.json"), b"{not json").unwrap();
    let before = collect_files(&root);

    let error = match SourceStore::open(&state) {
        Ok(_) => panic!("corrupt legacy discovery must fail the migration"),
        Err(error) => error,
    };
    assert_eq!(error.to_string(), "source_migration_failed");

    let after = collect_files(&root);
    for (path, bytes) in &before {
        assert_eq!(
            after.get(path),
            Some(bytes),
            "migration failure must not modify/delete {}, ",
            path.display()
        );
    }
    // Settings stay v1 and the completion marker is not advanced.
    assert_eq!(
        read_json(&root.join("settings.json"))["schema_version"],
        json!(1)
    );
    assert_eq!(
        read_json(&root.join("schema.json"))["schema_version"],
        json!(1)
    );
    // Re-running after the corrupt file is fixed succeeds (idempotent retry).
    fs::write(
        root.join("discovery/kagura-cn/42.json"),
        serde_json::to_vec_pretty(&discovery_success(
            42,
            "kagura-cn",
            vec![candidate_json(
                1,
                42,
                "Game 42",
                "kagura-cn",
                "https://kagurafan.com/p/42/",
                "appid",
            )],
            "complete",
            200,
            "query-42-kagura",
        ))
        .unwrap(),
    )
    .unwrap();
    let retried = SourceStore::open(&state).unwrap();
    assert_eq!(
        retried.load_settings().unwrap().schema_version,
        STORE_SCHEMA_VERSION
    );
    assert!(retried.load_match(42).unwrap().is_some());
}

// ---- 4. content revision -------------------------------------------------

#[test]
fn content_revision_ignores_timestamps_but_tracks_real_changes() {
    let previous = SourceIndex {
        schema_version: SOURCE_INDEX_SCHEMA_VERSION,
        source_id: "patchmoe".to_string(),
        revision: 4,
        complete: true,
        checked_at: 100,
        updated_at: 100,
        etag: Some("etag".to_string()),
        modified: None,
        entries: vec![index_entry(
            "A",
            "https://www.patch.moe/",
            "https://download.patch.moe/a.zip",
        )],
        pending: Vec::new(),
    };
    // A newer check with identical content must not bump the revision.
    let mut recheck = previous.clone();
    recheck.checked_at = 500;
    recheck.updated_at = 500;
    recheck.etag = Some("etag-2".to_string());
    assert_eq!(
        content_revision(4, Some(&recheck), &previous.entries, true, &[]),
        4
    );
    // Real content changes bump exactly once.
    let mut entries = previous.entries.clone();
    entries.push(index_entry(
        "B",
        "https://www.patch.moe/",
        "https://download.patch.moe/b.zip",
    ));
    assert_eq!(content_revision(4, Some(&previous), &entries, true, &[]), 5);
    assert_eq!(
        content_revision(4, Some(&previous), &previous.entries, false, &[]),
        5
    );
    assert_eq!(
        content_revision(
            4,
            Some(&previous),
            &previous.entries,
            true,
            &["https://www.patch.moe/page/2/".to_string()]
        ),
        5
    );
    // A brand-new index starts at revision 1.
    assert_eq!(content_revision(0, None, &previous.entries, true, &[]), 1);
}

// ---- 5. input key --------------------------------------------------------

#[test]
fn input_key_changes_only_with_matching_inputs() {
    let subscriptions = BTreeMap::from([("patchmoe".to_string(), true)]);
    let base = input_key("Game", Some("游戏"), &[], &subscriptions);
    assert_eq!(base, input_key("Game", Some("游戏"), &[], &subscriptions));
    assert_ne!(base, input_key("Game 2", Some("游戏"), &[], &subscriptions));
    assert_ne!(base, input_key("Game", None, &[], &subscriptions));
    assert_ne!(base, input_key("Game", Some("Game"), &[], &subscriptions));
    // Name hints participate in the digest.
    assert_ne!(
        base,
        input_key("Game", Some("游戏"), &["Alias".to_string()], &subscriptions)
    );
    assert_ne!(
        base,
        input_key(
            "Game",
            Some("游戏"),
            &[],
            &BTreeMap::from([("patchmoe".to_string(), false)])
        )
    );
    // Map insertion order is irrelevant; the encoded key is deterministic.
    let mut other = BTreeMap::new();
    other.insert("hgamer".to_string(), false);
    other.insert("patchmoe".to_string(), true);
    assert_eq!(
        input_key("Game", Some("游戏"), &[], &other),
        input_key(
            "Game",
            Some("游戏"),
            &[],
            &BTreeMap::from([
                ("hgamer".to_string(), false),
                ("patchmoe".to_string(), true),
            ])
        )
    );
    assert_ne!(base, input_key("Game", Some("游戏"), &[], &other));
}

// ---- 6. schedule ---------------------------------------------------------

#[test]
fn schedule_boundaries_match_the_p1_contract() {
    assert_eq!(next_due_at(0, 1_000), None, "0 hours disables updates");
    assert_eq!(next_due_at(24, 0), None, "no attempt means nothing is due");
    assert_eq!(next_due_at(6, 1_000), Some(1_000 + 6 * 3_600));
    assert_eq!(next_due_at(u64::MAX, u64::MAX), Some(u64::MAX));
    assert!(!is_due(1_000, None));
    assert!(!is_due(1_000 - 1, Some(1_000)));
    assert!(is_due(1_000, Some(1_000)));
    assert!(is_due(1_001, Some(1_000)));

    let settings = SettingsFile {
        schema_version: STORE_SCHEMA_VERSION,
        revision: 1,
        enabled: BTreeMap::new(),
        added: Vec::new(),
        games: BTreeMap::new(),
        intervals: BTreeMap::from([("patchmoe".to_string(), 12)]),
    };
    assert_eq!(interval_hours(&settings, "patchmoe"), 12);
    assert_eq!(interval_hours(&settings, "kagura-cn"), 0);
}

// ---- 7. top-level status derivation (design §3.2) ------------------------

#[test]
fn game_status_derivation_covers_the_five_rules() {
    assert_eq!(
        derive_game_status(&game(
            &[("patchmoe", SourceMatchStatus::Matched)],
            Some(FallbackMatchStatus::Matched)
        )),
        GameMatchStatus::PublisherMatch
    );
    assert_eq!(
        derive_game_status(&game(
            &[("kagura-cn", SourceMatchStatus::NoCandidate)],
            Some(FallbackMatchStatus::Matched)
        )),
        GameMatchStatus::FallbackMatch
    );
    assert_eq!(
        derive_game_status(&game(
            &[("kagura-cn", SourceMatchStatus::NoCandidate)],
            Some(FallbackMatchStatus::NoMatch)
        )),
        GameMatchStatus::NoMatch
    );
    assert_eq!(
        derive_game_status(&game(
            &[("patchmoe", SourceMatchStatus::NoIndex)],
            Some(FallbackMatchStatus::Matched)
        )),
        GameMatchStatus::Incomplete
    );
    assert_eq!(
        derive_game_status(&game(
            &[("kagura-cn", SourceMatchStatus::NoCandidate)],
            None
        )),
        GameMatchStatus::Incomplete
    );
    assert_eq!(
        derive_game_status(&game(
            &[("kagura-cn", SourceMatchStatus::DetailFailed)],
            Some(FallbackMatchStatus::Failed)
        )),
        GameMatchStatus::Failed
    );
    assert_eq!(
        derive_game_status(&game(&[], Some(FallbackMatchStatus::Disabled))),
        GameMatchStatus::NoMatch
    );
    assert_eq!(
        derive_game_status(&game(&[], None)),
        GameMatchStatus::Pending
    );
}

// ---- 8. detail keys are stable and shared per canonical page -------------

#[test]
fn detail_keys_are_stable_per_canonical_page_and_shared() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let page = "https://kagurafan.com/p/42/";
    let key = detail_key(page);
    assert_eq!(key.len(), 32);
    assert!(key
        .chars()
        .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character)));
    assert_eq!(key, detail_key(page), "same canonical page -> same key");
    assert_ne!(key, detail_key("https://kagurafan.com/p/43/"));

    let detail = DetailRecord {
        schema_version: SOURCE_DETAIL_SCHEMA_VERSION,
        source_id: "kagura-cn".to_string(),
        page: page.to_string(),
        index_revision: 1,
        checked_at: 10,
        complete: true,
        fact: Some(PageFact {
            title: "Game".to_string(),
            page: page.to_string(),
            kind: "article".to_string(),
            ..PageFact::default()
        }),
        diagnostics: Vec::new(),
    };
    store.save_detail(&detail).unwrap();
    // A second game referencing the same canonical URL reuses the single file.
    assert_eq!(store.load_detail("kagura-cn", page).unwrap(), Some(detail));
    let files: Vec<_> = fs::read_dir(store.paths().details("kagura-cn"))
        .unwrap()
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(
        files[0].as_ref().unwrap().path(),
        store.paths().detail("kagura-cn", &key)
    );
}

// ---- 9. P5: legacy files migrate once and never feed the new read model ---

#[test]
fn legacy_files_migrate_once_and_new_snapshots_win() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let root = build_v1_state(&state);
    let store = SourceStore::open(&state).unwrap();
    // The first open migrated the legacy cache/discovery trees.
    assert!(store.load_index("patchmoe").unwrap().is_some());
    assert!(store.load_match(42).unwrap().is_some());

    let fresh = SourceIndex {
        schema_version: SOURCE_INDEX_SCHEMA_VERSION,
        source_id: "patchmoe".to_string(),
        revision: 9,
        complete: true,
        checked_at: 500,
        updated_at: 500,
        etag: Some("fresh-etag".to_string()),
        modified: None,
        entries: vec![index_entry(
            "Fresh",
            "https://www.patch.moe/",
            "https://download.patch.moe/fresh.zip",
        )],
        pending: Vec::new(),
    };
    store.save_index(&fresh).unwrap();
    let mut matched_candidate = candidate(
        7,
        42,
        "Game 42",
        "patchmoe",
        "https://www.patch.moe/",
        "appid",
    );
    matched_candidate.mirrors = vec![Mirror {
        url: "https://download.patch.moe/fresh.zip".to_string(),
        label: None,
        format: "zip".to_string(),
    }];
    let record = GameMatchRecord {
        schema_version: GAME_MATCH_SCHEMA_VERSION,
        appid: 42,
        name: "Game 42".to_string(),
        localized_name: None,
        input_key: "key".to_string(),
        status: GameMatchStatus::PublisherMatch,
        sources: BTreeMap::from([(
            "patchmoe".to_string(),
            SourceMatchRecord {
                index_revision: 9,
                status: SourceMatchStatus::Matched,
                candidates: vec![matched_candidate],
                detail_pages: Vec::new(),
                checked_at: 500,
                diagnostics: Vec::new(),
            },
        )]),
        fallback: None,
        updated_at: 500,
    };
    store.save_match(&record).unwrap();
    drop(store);

    // Legacy files stay on disk but are no longer read.
    assert!(root.join("cache/patchmoe.json").is_file());
    assert!(root.join("discovery/kagura-cn/42.json").is_file());

    let reopened = SourceStore::open(&state).unwrap();
    assert!(
        reopened.migration_diagnostics().is_empty(),
        "the migration must not run a second time"
    );
    assert_eq!(
        reopened.load_index("patchmoe").unwrap(),
        Some(fresh.clone())
    );
    assert_eq!(reopened.load_match(42).unwrap(), Some(record.clone()));
    drop(reopened);

    // A read model opened from the same directory sees the new snapshot. The
    // v1 fixture disables patchmoe, so re-enable it before reading matches.
    let mut sources = Sources::open(&state).unwrap();
    sources
        .registry_mut()
        .set_enabled("patchmoe", true)
        .unwrap();
    sources
        .registry_mut()
        .set_game_source(42, "patchmoe", true)
        .unwrap();
    assert_eq!(
        sources
            .store()
            .load_index("patchmoe")
            .unwrap()
            .unwrap()
            .revision,
        9
    );
    let game = GameQuery {
        appid: 42,
        name: "Game 42".to_string(),
        localized_name: None,
        instance_id: None,
    };
    let matches = sources.registry().matches(&game, false);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].release_id, "patchmoe-42");
    assert_eq!(matches[0].url, "https://download.patch.moe/fresh.zip");
}

#[test]
fn installer_resolve_works_from_a_published_index_before_any_match_task() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let store = SourceStore::open(&state).unwrap();
    store
        .save_index(&SourceIndex {
            schema_version: SOURCE_INDEX_SCHEMA_VERSION,
            source_id: "patchmoe".to_string(),
            revision: 1,
            complete: true,
            checked_at: 100,
            updated_at: 100,
            etag: None,
            modified: None,
            entries: vec![index_entry(
                "Game 42",
                "https://www.patch.moe/",
                "https://download.patch.moe/42.zip",
            )],
            pending: Vec::new(),
        })
        .unwrap();
    drop(store);

    let sources = Sources::open(&state).unwrap();
    let game = GameQuery {
        appid: 42,
        name: "Game 42".to_string(),
        localized_name: None,
        instance_id: None,
    };
    // No `matches/<appid>.json` exists yet: the entry-index fallback keeps
    // installer resolution available right after a source update.
    let candidates = sources.registry().index_candidates(&game);
    assert_eq!(candidates.len(), 1);
    let selection = installer::zip_selection_id(&candidates[0]).unwrap();
    let resolved = installer::resolve_zip(sources.registry(), &game, &selection).unwrap();
    assert_eq!(resolved.download_url, "https://download.patch.moe/42.zip");
    let matches = sources.registry().matches(&game, false);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].release_id, "patchmoe-42");
}
