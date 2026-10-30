//! P3 incremental matching: index hits stay offline, detail pages are fetched
//! only on demand and shared per canonical URL, and HGamer fallback follows
//! design §3.2. Every request goes through `FakeFetcher`; no test uses network.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use patchhub_core::sources::http::{FakeFetcher, FetchResponse, RequestContext};
use patchhub_core::sources::matching::{
    game_match_summary, match_games, rematch_game, rematch_source, source_revisions,
};
use patchhub_core::sources::model::*;
use patchhub_core::sources::store::SourceStore;
use patchhub_core::sources::{GAME_MATCH_SCHEMA_VERSION, SOURCE_DETAIL_SCHEMA_VERSION};

// ---- fixtures ------------------------------------------------------------

const KAGURA_ENTRY: &str = "https://kagurafan.com/";
const HGAMER_BASE: &str = "https://hgamer.tw/zh-CN/patches";

fn kagura_rule() -> SourceRule {
    SourceRule {
        script: None,
        id: "kagura-cn".to_string(),
        name: "Kagura CN".to_string(),
        adapter: "kagura_wordpress".to_string(),
        role: SourceRole::Publisher,
        discovery_only: true,
        priority: 10,
        enabled: true,
        pages: vec![KAGURA_ENTRY.to_string()],
        page_hosts: vec!["kagurafan.com".to_string()],
        download_hosts: vec!["kaguraserver.com".to_string()],
        root_layout: None,
        columns: None,
        table_id: None,
        language: None,
        url: None,
    }
}

fn entry_rule() -> SourceRule {
    SourceRule {
        script: None,
        id: "table-source".to_string(),
        name: "Table".to_string(),
        adapter: "table_html".to_string(),
        role: SourceRole::Publisher,
        discovery_only: false,
        priority: 20,
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

fn hgamer_rule() -> SourceRule {
    SourceRule {
        script: None,
        id: "hgamer".to_string(),
        name: "HGamer".to_string(),
        adapter: "hgamer_html".to_string(),
        role: SourceRole::FallbackIndexer,
        discovery_only: true,
        priority: 100,
        enabled: true,
        pages: vec![HGAMER_BASE.to_string()],
        page_hosts: vec!["hgamer.tw".to_string()],
        download_hosts: Vec::new(),
        root_layout: None,
        columns: None,
        table_id: None,
        language: None,
        url: None,
    }
}

fn entry(title: &str, page: &str, kind: &str) -> SourceIndexEntry {
    SourceIndexEntry {
        title: title.to_string(),
        aliases: Vec::new(),
        page: page.to_string(),
        kind: kind.to_string(),
        appid: None,
        url: None,
        language: None,
        date: None,
    }
}

fn artifact_entry(
    title: &str,
    page: &str,
    url: &str,
    appid: Option<u32>,
    language: &str,
) -> SourceIndexEntry {
    SourceIndexEntry {
        title: title.to_string(),
        aliases: Vec::new(),
        page: page.to_string(),
        kind: "entry".to_string(),
        appid,
        url: Some(url.to_string()),
        language: Some(language.to_string()),
        date: None,
    }
}

fn index(
    source_id: &str,
    revision: u64,
    complete: bool,
    entries: Vec<SourceIndexEntry>,
) -> SourceIndex {
    SourceIndex {
        parser_fingerprint: None,
        schema_version: 1,
        source_id: source_id.to_string(),
        revision,
        complete,
        checked_at: 100,
        updated_at: 100,
        etag: None,
        modified: None,
        entries,
        pending: if complete {
            Vec::new()
        } else {
            vec!["https://kagurafan.com/page/2/".to_string()]
        },
    }
}

fn fact(page: &str, title: &str, appid: u32) -> PageFact {
    PageFact {
        title: title.to_string(),
        page: page.to_string(),
        kind: "article".to_string(),
        steam_ids: vec![appid],
        aliases: vec![title.to_string()],
        mirrors: vec![Mirror {
            url: format!("https://kaguraserver.com/{appid}.zip"),
            label: None,
            format: "zip".to_string(),
        }],
        text: "中文 补丁 v1.2.3 ZIP".to_string(),
        published_at: None,
        body_links: Vec::new(),
        article_links: Vec::new(),
        article_facts: Vec::new(),
        next_page: None,
    }
}

fn seed_detail(store: &SourceStore, rule: &SourceRule, revision: u64, fact: PageFact) {
    store
        .save_detail(&DetailRecord {
            schema_version: SOURCE_DETAIL_SCHEMA_VERSION,
            source_id: rule.id.clone(),
            page: fact.page.clone(),
            index_revision: revision,
            checked_at: 100,
            complete: true,
            fact: Some(fact),
            diagnostics: Vec::new(),
        })
        .unwrap();
}

fn article(title: &str, appids: &[u32]) -> String {
    let mut links = String::new();
    for appid in appids {
        links.push_str(&format!(
            "<a href=\"https://store.steampowered.com/app/{appid}/Game/\">Steam</a>"
        ));
        links.push_str(&format!(
            "<a href=\"https://kaguraserver.com/{appid}.zip\">下载 Mirror 1</a>"
        ));
    }
    format!(
        "<html><body><article><h1 class=\"entry-title\">{title}</h1>\
         <div class=\"entry-content\"><p>中文 补丁 v1.2.3 ZIP</p>{links}</div></article></body></html>"
    )
}

fn hgamer_search(term: &str, count: u32, patches: &[(&str, &str)]) -> String {
    let mut list = String::new();
    for (label, href) in patches {
        list.push_str(&format!("<li><a href=\"{href}\">{label}</a></li>"));
    }
    format!(
        "<html><body><div class=\"summary\"><h1>Search</h1>\
         <p>找到 {count} 笔 results for {term}</p></div>\
         <div><h2>补丁</h2><ul>{list}</ul></div></body></html>"
    )
}

fn hgamer_patch(title: &str, appid: u32) -> String {
    format!(
        "<html><body><div data-patch-body><h1>{title}</h1>\
         <div><a href=\"https://store.steampowered.com/app/{appid}/Game/\">Steam</a>\
         <p>ZIP</p><a href=\"https://mirror.example/{appid}.zip\">下载 Mirror 1</a></div>\
         </div></body></html>"
    )
}

fn game(appid: u32, name: &str) -> GameQuery {
    GameQuery {
        appid,
        name: name.to_string(),
        localized_name: None,
        instance_id: None,
    }
}

fn context(fake: &Arc<FakeFetcher>) -> RequestContext {
    RequestContext::with_settings(Box::new(Arc::clone(fake)), Duration::ZERO)
}

fn response(status: u16, html: &str, final_url: &str) -> FetchResponse {
    FetchResponse {
        status,
        body: html.as_bytes().to_vec(),
        final_url: final_url.to_string(),
        ..FetchResponse::default()
    }
}

fn run(
    store: &SourceStore,
    rules: &[SourceRule],
    overrides: &Overrides,
    games: &[GameQuery],
    fake: &Arc<FakeFetcher>,
    force_details: bool,
    now: u64,
) -> std::io::Result<Vec<GameMatch>> {
    run_input(
        store,
        rules,
        overrides,
        games,
        &[],
        fake,
        force_details,
        now,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_input(
    store: &SourceStore,
    rules: &[SourceRule],
    overrides: &Overrides,
    games: &[GameQuery],
    name_hints: &[String],

    fake: &Arc<FakeFetcher>,
    force_details: bool,
    now: u64,
) -> std::io::Result<Vec<GameMatch>> {
    let mut ctx = context(fake);
    match_games(
        store,
        rules,
        overrides,
        games,
        name_hints,
        force_details,
        &mut ctx,
        &Arc::new(AtomicBool::new(false)),
        now,
        |_| {},
        |_| {},
    )
}

#[allow(clippy::too_many_arguments)]
fn rematch(
    store: &SourceStore,
    rules: &[SourceRule],
    overrides: &Overrides,
    game: &GameQuery,
    name_hints: &[String],

    fake: &Arc<FakeFetcher>,
    now: u64,
) -> std::io::Result<GameMatch> {
    let mut ctx = context(fake);
    rematch_game(
        store,
        rules,
        overrides,
        game,
        name_hints,
        &mut ctx,
        &Arc::new(AtomicBool::new(false)),
        now,
        |_| {},
    )
}

fn hgamer_disabled() -> Overrides {
    Overrides {
        enabled: [("hgamer".to_string(), false)].into(),
        ..Overrides::default()
    }
}

fn record(store: &SourceStore, appid: u32) -> GameMatchRecord {
    store
        .load_match(appid)
        .unwrap()
        .unwrap_or_else(|| panic!("match record for {appid}"))
}

fn fallback_status(store: &SourceStore, appid: u32) -> FallbackMatchStatus {
    record(store, appid)
        .fallback
        .expect("fallback record")
        .status
}

// ---- 1. index hit: offline / only hit details ----------------------------

#[test]
fn index_hits_stay_offline_and_only_hit_detail_pages_are_fetched() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_rule();
    store
        .save_index(&index(
            &rule.id,
            1,
            true,
            vec![
                entry("Game", "https://kagurafan.com/game/", "article"),
                entry("Other Game", "https://kagurafan.com/other/", "article"),
            ],
        ))
        .unwrap();

    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(
        200,
        &article("Game", &[42]),
        "https://kagurafan.com/game/",
    ));
    let mut phases = Vec::new();
    let mut updates = Vec::new();
    let mut ctx = context(&fake);
    let results = match_games(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &[game(42, "Game")],
        &[],
        false,
        &mut ctx,
        &Arc::new(AtomicBool::new(false)),
        200,
        |progress| phases.push(progress),
        |update| updates.push(update.clone()),
    )
    .unwrap();

    assert_eq!(fake.call_count(), 1, "only the hit detail page is fetched");
    assert_eq!(fake.calls()[0].url, "https://kagurafan.com/game/");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, GameMatchStatus::PublisherMatch);
    assert!(!results[0].candidates.is_empty());
    assert_eq!(results[0].candidates[0].source_page, article_page());
    assert_eq!(updates.len(), 1, "the persisted row is published once");
    for phase in ["index", "details", "fallback"] {
        assert!(
            phases.iter().any(|progress| progress.phase == phase),
            "phase {phase} is reported"
        );
    }
    assert!(phases.last().is_some_and(|progress| !progress.running));

    // A second pass with identical inputs/snapshot reads the persisted record.
    let warm = Arc::new(FakeFetcher::new());
    let second = run(
        &store,
        &[rule],
        &Overrides::default(),
        &[game(42, "Game")],
        &warm,
        false,
        300,
    )
    .unwrap();
    assert_eq!(warm.call_count(), 0, "a terminal record is reused offline");
    assert_eq!(second[0].status, GameMatchStatus::PublisherMatch);
    assert_eq!(record(&store, 42).updated_at, 200, "no rewrite on reuse");
}

fn article_page() -> &'static str {
    "https://kagurafan.com/game/"
}

// ---- 2. ten games share one index snapshot -------------------------------

#[test]
fn ten_games_share_one_index_and_requests_track_detail_pages_only() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_rule();
    let entries: Vec<SourceIndexEntry> = (1..=10)
        .map(|index| {
            entry(
                &format!("Game {index}"),
                &format!("https://kagurafan.com/game/{index}/"),
                "article",
            )
        })
        .collect();
    store
        .save_index(&index(&rule.id, 1, true, entries))
        .unwrap();
    for index in 1..=3 {
        let page = format!("https://kagurafan.com/game/{index}/");
        seed_detail(
            &store,
            &rule,
            1,
            fact(&page, &format!("Game {index}"), 2000 + index as u32),
        );
    }

    let fake = Arc::new(FakeFetcher::new());
    for index in 4..=10 {
        fake.push(response(
            200,
            &article(&format!("Game {index}"), &[2000 + index as u32]),
            &format!("https://kagurafan.com/game/{index}/"),
        ));
    }
    let games: Vec<GameQuery> = (1..=10)
        .map(|index| game(2000 + index as u32, &format!("Game {index}")))
        .collect();
    let results = run(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &games,
        &fake,
        false,
        200,
    )
    .unwrap();

    assert_eq!(fake.call_count(), 7, "three cached + seven fetched details");
    for call in fake.calls() {
        assert!(
            call.url.starts_with("https://kagurafan.com/game/"),
            "no list page is re-read: {}",
            call.url
        );
    }
    assert_eq!(results.len(), 10);
    assert!(results
        .iter()
        .all(|result| result.status == GameMatchStatus::PublisherMatch));
}

// ---- 3. two games, one shared detail page --------------------------------

#[test]
fn two_games_sharing_a_detail_page_fetch_it_once() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_rule();
    store
        .save_index(&index(
            &rule.id,
            1,
            true,
            vec![entry(
                "Shared Game",
                "https://kagurafan.com/game/shared/",
                "article",
            )],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(
        200,
        &article("Shared Game", &[42, 43]),
        "https://kagurafan.com/game/shared/",
    ));

    let games = [game(42, "Shared Game"), game(43, "Shared Game")];
    let results = run(
        &store,
        &[rule],
        &Overrides::default(),
        &games,
        &fake,
        false,
        200,
    )
    .unwrap();

    assert_eq!(fake.call_count(), 1, "one canonical page, one request");
    assert_eq!(results.len(), 2);
    assert!(results
        .iter()
        .all(|result| result.status == GameMatchStatus::PublisherMatch));
    assert_eq!(fake.calls()[0].url, "https://kagurafan.com/game/shared/");
}

// ---- 4. new game only matches itself; restart reuse ----------------------

#[test]
fn new_game_only_matches_itself_and_survives_a_restart() {
    let temp = tempfile::tempdir().unwrap();
    let rule = kagura_rule();
    {
        let store = SourceStore::open(temp.path()).unwrap();
        store
            .save_index(&index(
                &rule.id,
                1,
                true,
                vec![
                    entry("Game A", "https://kagurafan.com/game/a/", "article"),
                    entry("Game B", "https://kagurafan.com/game/b/", "article"),
                ],
            ))
            .unwrap();
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(
            200,
            &article("Game A", &[42]),
            "https://kagurafan.com/game/a/",
        ));
        let first = run(
            &store,
            std::slice::from_ref(&rule),
            &Overrides::default(),
            &[game(42, "Game A")],
            &fake,
            false,
            200,
        )
        .unwrap();
        assert_eq!(first[0].status, GameMatchStatus::PublisherMatch);
        assert_eq!(fake.call_count(), 1);
    }

    let store = SourceStore::open(temp.path()).unwrap(); // restart
    let new_game = Arc::new(FakeFetcher::new());
    new_game.push(response(
        200,
        &article("Game B", &[43]),
        "https://kagurafan.com/game/b/",
    ));
    let results = run(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &[game(42, "Game A"), game(43, "Game B")],
        &new_game,
        false,
        300,
    )
    .unwrap();
    assert_eq!(
        new_game.call_count(),
        1,
        "only the newly added game fetches its detail"
    );
    assert_eq!(new_game.calls()[0].url, "https://kagurafan.com/game/b/");
    assert_eq!(results.len(), 2);

    // A further pass after another restart is fully offline.
    drop(store);
    let store = SourceStore::open(temp.path()).unwrap();
    let warm = Arc::new(FakeFetcher::new());
    run(
        &store,
        &[rule],
        &Overrides::default(),
        &[game(42, "Game A"), game(43, "Game B")],
        &warm,
        false,
        400,
    )
    .unwrap();
    assert_eq!(warm.call_count(), 0);
}

// ---- 5. same appid, multiple instances -----------------------------------

#[test]
fn duplicate_appid_instances_match_once_and_query_fallback_once() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [kagura_rule(), hgamer_rule()];
    store
        .save_index(&index(
            &rules[0].id,
            1,
            true,
            vec![entry(
                "Other Game",
                "https://kagurafan.com/other/",
                "article",
            )],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(
        200,
        &hgamer_search("Game", 0, &[]),
        "https://hgamer.tw/zh-CN/search?q=Game",
    ));

    let mut first = game(42, "Game");
    first.instance_id = Some("instance-a".to_string());
    let mut second = game(42, "Game");
    second.instance_id = Some("instance-b".to_string());
    let results = run(
        &store,
        &rules,
        &Overrides::default(),
        &[first, second],
        &fake,
        false,
        200,
    )
    .unwrap();

    assert_eq!(results.len(), 1, "same AppID is matched once");
    assert_eq!(results[0].instance_id.as_deref(), Some("instance-a"));
    assert_eq!(fake.call_count(), 1, "fallback queried once");
    assert_eq!(fallback_status(&store, 42), FallbackMatchStatus::NoMatch);
    assert_eq!(results[0].status, GameMatchStatus::NoMatch);
}

// ---- 6. publish/fallback display matrix ----------------------------------

#[test]
fn publisher_match_does_not_query_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [kagura_rule(), hgamer_rule()];
    store
        .save_index(&index(
            &rules[0].id,
            1,
            true,
            vec![entry("Game", "https://kagurafan.com/game/", "article")],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(
        200,
        &article("Game", &[42]),
        "https://kagurafan.com/game/",
    ));

    let results = run(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    assert_eq!(fake.call_count(), 1, "no fallback request");
    assert_eq!(results[0].status, GameMatchStatus::PublisherMatch);
    assert_eq!(
        record(&store, 42).fallback.unwrap().status,
        FallbackMatchStatus::NotNeeded
    );
}

#[test]
fn fallback_match_and_complete_no_match() {
    // HGamer hit when publisher coverage is complete and has no match.
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [kagura_rule(), hgamer_rule()];
    store
        .save_index(&index(
            &rules[0].id,
            1,
            true,
            vec![entry(
                "Other Game",
                "https://kagurafan.com/other/",
                "article",
            )],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(
        200,
        &hgamer_search("Game", 1, &[("Game patch", "/zh-CN/patches/patch-1")]),
        "https://hgamer.tw/zh-CN/search?q=Game",
    ));
    fake.push(response(
        200,
        &hgamer_patch("Game patch", 42),
        "https://hgamer.tw/zh-CN/patches/patch-1",
    ));
    let results = run(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    assert_eq!(results[0].status, GameMatchStatus::FallbackMatch);
    assert!(!results[0].candidates.is_empty());
    assert_eq!(results[0].candidates[0].source_id, "hgamer");
    assert_eq!(
        record(&store, 42).fallback.unwrap().status,
        FallbackMatchStatus::Matched
    );
    assert_eq!(fake.call_count(), 2);

    // Both complete, both empty: confirmed "no patch in enabled sources".
    // A manual rematch re-queries HGamer and records the completed empty result.
    let empty = Arc::new(FakeFetcher::new());
    empty.push(response(
        200,
        &hgamer_search("Game", 0, &[]),
        "https://hgamer.tw/zh-CN/search?q=Game",
    ));
    let summary = rematch(
        &store,
        &rules,
        &Overrides::default(),
        &game(42, "Game"),
        &[],
        &empty,
        300,
    )
    .unwrap();
    assert_eq!(summary.status, GameMatchStatus::NoMatch);
    assert_eq!(
        record(&store, 42).fallback.unwrap().status,
        FallbackMatchStatus::NoMatch
    );
}

#[test]
fn incomplete_publisher_coverage_never_confirms_no_patch() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [kagura_rule(), hgamer_rule()];
    store
        .save_index(&index(
            &rules[0].id,
            1,
            false,
            vec![entry(
                "Other Game",
                "https://kagurafan.com/other/",
                "article",
            )],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let results = run(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    assert_eq!(
        fake.call_count(),
        0,
        "no fallback network for partial coverage"
    );
    assert_eq!(results[0].status, GameMatchStatus::Incomplete);
    assert_eq!(
        record(&store, 42).fallback.unwrap().status,
        FallbackMatchStatus::NotQueried
    );
    assert_eq!(
        record(&store, 42).sources["kagura-cn"].status,
        SourceMatchStatus::IndexIncomplete
    );
}

#[test]
fn disabled_fallback_reports_disabled_not_confirmed_no_match() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [kagura_rule(), hgamer_rule()];
    store
        .save_index(&index(
            &rules[0].id,
            1,
            true,
            vec![entry(
                "Other Game",
                "https://kagurafan.com/other/",
                "article",
            )],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let results = run(
        &store,
        &rules,
        &hgamer_disabled(),
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    assert_eq!(fake.call_count(), 0);
    assert_eq!(results[0].status, GameMatchStatus::NoMatch);
    assert_eq!(
        record(&store, 42).fallback.unwrap().status,
        FallbackMatchStatus::Disabled
    );

    // Disabled fallback plus incomplete publisher coverage stays incomplete.
    let mut incomplete = index(
        &rules[0].id,
        2,
        false,
        vec![entry(
            "Other Game",
            "https://kagurafan.com/other/",
            "article",
        )],
    );
    incomplete.pending = vec!["https://kagurafan.com/page/2/".to_string()];
    store.save_index(&incomplete).unwrap();
    let results = run(
        &store,
        &rules,
        &hgamer_disabled(),
        &[game(42, "Game")],
        &fake,
        false,
        300,
    )
    .unwrap();
    assert_eq!(results[0].status, GameMatchStatus::Incomplete);
}

#[test]
fn missing_publisher_index_is_incomplete_and_skips_fallback_network() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [kagura_rule(), hgamer_rule()];
    let fake = Arc::new(FakeFetcher::new());
    let results = run(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    assert_eq!(fake.call_count(), 0, "no index means no fallback network");
    assert_eq!(results[0].status, GameMatchStatus::Incomplete);
    let saved = record(&store, 42);
    assert_eq!(
        saved.sources["kagura-cn"].status,
        SourceMatchStatus::NoIndex
    );
    assert_eq!(saved.sources["kagura-cn"].index_revision, 0);
    assert_eq!(
        saved.fallback.unwrap().status,
        FallbackMatchStatus::NotQueried
    );
}

#[test]
fn no_enabled_publisher_source_is_incomplete_without_fallback_query() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [kagura_rule(), hgamer_rule()];
    let overrides = Overrides {
        enabled: [("kagura-cn".to_string(), false)].into(),
        ..Overrides::default()
    };
    let fake = Arc::new(FakeFetcher::new());
    let results = run(
        &store,
        &rules,
        &overrides,
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    assert_eq!(fake.call_count(), 0);
    assert_eq!(results[0].status, GameMatchStatus::Incomplete);
    assert_eq!(
        record(&store, 42).fallback.unwrap().status,
        FallbackMatchStatus::NotQueried
    );
}

#[test]
fn a_later_publisher_match_hides_the_previous_fallback_candidates() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [entry_rule(), hgamer_rule()];
    store
        .save_index(&index(
            &rules[0].id,
            1,
            true,
            vec![artifact_entry(
                "Other Game",
                "https://example.com/list",
                "https://download.example.com/other.zip",
                Some(7),
                "en",
            )],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(
        200,
        &hgamer_search("Game", 1, &[("Game patch", "/zh-CN/patches/patch-1")]),
        "https://hgamer.tw/zh-CN/search?q=Game",
    ));
    fake.push(response(
        200,
        &hgamer_patch("Game patch", 42),
        "https://hgamer.tw/zh-CN/patches/patch-1",
    ));
    let first = run(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    assert_eq!(first[0].status, GameMatchStatus::FallbackMatch);
    assert!(first[0].candidates.iter().all(|c| c.source_id == "hgamer"));

    // The table source now publishes the game: no network, publisher wins.
    store
        .save_index(&index(
            &rules[0].id,
            2,
            true,
            vec![
                artifact_entry(
                    "Other Game",
                    "https://example.com/list",
                    "https://download.example.com/other.zip",
                    Some(7),
                    "en",
                ),
                artifact_entry(
                    "Game",
                    "https://example.com/list",
                    "https://download.example.com/game.zip",
                    Some(42),
                    "en",
                ),
            ],
        ))
        .unwrap();
    let warm = Arc::new(FakeFetcher::new());
    let second = run(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &warm,
        false,
        300,
    )
    .unwrap();
    assert_eq!(warm.call_count(), 0);
    assert_eq!(second[0].status, GameMatchStatus::PublisherMatch);
    assert!(!second[0].candidates.is_empty());
    assert!(second[0]
        .candidates
        .iter()
        .all(|candidate| candidate.source_id == "table-source"));
    // The old fallback result is retained as evidence but no longer displayed.
    assert_eq!(
        record(&store, 42).fallback.unwrap().status,
        FallbackMatchStatus::Matched
    );
}

// ---- 7. rematch_game ----------------------------------------------------

#[test]
fn rematch_game_refreshes_hit_details_without_touching_the_index() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_rule();
    store
        .save_index(&index(
            &rule.id,
            1,
            true,
            vec![entry("Game", "https://kagurafan.com/game/", "article")],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(
        200,
        &article("Game", &[42]),
        "https://kagurafan.com/game/",
    ));
    run(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    assert_eq!(record(&store, 42).updated_at, 200);

    let refreshed = Arc::new(FakeFetcher::new());
    refreshed.push(response(
        200,
        &article("Game", &[42]),
        "https://kagurafan.com/game/",
    ));
    let summary = rematch(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &game(42, "Game"),
        &[],
        &refreshed,
        300,
    )
    .unwrap();

    assert_eq!(refreshed.call_count(), 1, "forced detail refresh");
    assert_eq!(refreshed.calls()[0].url, "https://kagurafan.com/game/");
    assert_eq!(summary.status, GameMatchStatus::PublisherMatch);
    assert_eq!(record(&store, 42).updated_at, 300);
    let index = store.load_index(&rule.id).unwrap().unwrap();
    assert_eq!(index.revision, 1, "rematch never updates the index");
}

#[test]
fn rematch_game_requeries_fallback_for_a_no_match_game() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [kagura_rule(), hgamer_rule()];
    store
        .save_index(&index(
            &rules[0].id,
            1,
            true,
            vec![entry(
                "Other Game",
                "https://kagurafan.com/other/",
                "article",
            )],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(
        200,
        &hgamer_search("Game", 0, &[]),
        "https://hgamer.tw/zh-CN/search?q=Game",
    ));
    run(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    assert_eq!(fallback_status(&store, 42), FallbackMatchStatus::NoMatch);

    let again = Arc::new(FakeFetcher::new());
    again.push(response(
        200,
        &hgamer_search("Game", 0, &[]),
        "https://hgamer.tw/zh-CN/search?q=Game",
    ));
    let summary = rematch(
        &store,
        &rules,
        &Overrides::default(),
        &game(42, "Game"),
        &[],
        &again,
        300,
    )
    .unwrap();
    assert_eq!(again.call_count(), 1, "manual rematch re-queries HGamer");
    assert_eq!(summary.status, GameMatchStatus::NoMatch);
}

// ---- 8. rematch_source --------------------------------------------------

#[test]
fn rematch_source_recomputes_only_that_source_and_reuses_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [kagura_rule(), entry_rule(), hgamer_rule()];
    store
        .save_index(&index(
            &rules[0].id,
            1,
            true,
            vec![entry("Game", "https://kagurafan.com/game/", "article")],
        ))
        .unwrap();
    store
        .save_index(&index(
            &rules[1].id,
            1,
            true,
            vec![artifact_entry(
                "Game",
                "https://example.com/list",
                "https://download.example.com/game.zip",
                Some(42),
                "en",
            )],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(
        200,
        &article("Game", &[42]),
        "https://kagurafan.com/game/",
    ));
    run(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    let before = record(&store, 42);
    let table_before = before.sources["table-source"].clone();
    assert_eq!(before.status, GameMatchStatus::PublisherMatch);

    // The Kagura source published new content; its revision moves to 2.
    let mut updated = index(
        &rules[0].id,
        2,
        true,
        vec![entry("Game", "https://kagurafan.com/game/", "article")],
    );
    updated.checked_at = 300;
    updated.updated_at = 300;
    store.save_index(&updated).unwrap();

    let refreshed = Arc::new(FakeFetcher::new());
    refreshed.push(response(
        200,
        &article("Game", &[42]),
        "https://kagurafan.com/game/",
    ));
    let mut ctx = context(&refreshed);
    let mut updates = Vec::new();
    let results = rematch_source(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        "kagura-cn",
        &[],
        &mut ctx,
        &Arc::new(AtomicBool::new(false)),
        400,
        |summary| updates.push(summary.clone()),
    )
    .unwrap();

    assert_eq!(refreshed.call_count(), 1, "hit details are refreshed");
    assert_eq!(refreshed.calls()[0].url, "https://kagurafan.com/game/");
    assert_eq!(results.len(), 1);
    assert_eq!(updates.len(), 1);
    let after = record(&store, 42);
    assert_eq!(
        after.sources["table-source"], table_before,
        "other source records are preserved"
    );
    assert_eq!(after.sources["kagura-cn"].index_revision, 2);
    assert_eq!(after.updated_at, 400);
    assert_eq!(
        store.load_index(&rules[0].id).unwrap().unwrap().revision,
        2,
        "rematch_source does not write indexes"
    );

    // A no-match game keeps its completed fallback result; a source update does
    // not clear it and does not trigger a new HGamer query.
    store
        .save_index(&index(
            &rules[0].id,
            3,
            true,
            vec![entry(
                "Other Game",
                "https://kagurafan.com/other/",
                "article",
            )],
        ))
        .unwrap();
    let fallback_fake = Arc::new(FakeFetcher::new());
    fallback_fake.push(response(
        200,
        &hgamer_search("Missing", 0, &[]),
        "https://hgamer.tw/zh-CN/search?q=Missing",
    ));
    run(
        &store,
        &rules,
        &Overrides::default(),
        &[game(43, "Missing")],
        &fallback_fake,
        false,
        500,
    )
    .unwrap();
    assert_eq!(fallback_status(&store, 43), FallbackMatchStatus::NoMatch);

    let mut bumped = index(
        &rules[0].id,
        4,
        true,
        vec![entry(
            "Other Game",
            "https://kagurafan.com/other/",
            "article",
        )],
    );
    bumped.checked_at = 600;
    store.save_index(&bumped).unwrap();
    let no_network = Arc::new(FakeFetcher::new());
    let mut ctx = context(&no_network);
    rematch_source(
        &store,
        &rules,
        &Overrides::default(),
        &[game(43, "Missing")],
        "kagura-cn",
        &[],
        &mut ctx,
        &Arc::new(AtomicBool::new(false)),
        700,
        |_| {},
    )
    .unwrap();
    assert_eq!(no_network.call_count(), 0, "terminal fallback is reused");
    assert_eq!(fallback_status(&store, 43), FallbackMatchStatus::NoMatch);
}

// ---- 9. failure / cancellation ------------------------------------------

#[test]
fn detail_failure_keeps_previous_candidates_and_reports_incomplete() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [kagura_rule(), hgamer_rule()];
    store
        .save_index(&index(
            &rules[0].id,
            1,
            true,
            vec![entry("Game", "https://kagurafan.com/game/", "article")],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(
        200,
        &article("Game", &[42]),
        "https://kagurafan.com/game/",
    ));
    run(
        &store,
        &rules,
        &hgamer_disabled(),
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    let previous = record(&store, 42);
    assert_eq!(
        previous.sources["kagura-cn"].status,
        SourceMatchStatus::Matched
    );

    // The forced refresh fails: the confirmed candidate survives, the source is
    // not allowed to claim completeness.
    let failing = Arc::new(FakeFetcher::new());
    failing.push_error("discovery_network_error");
    failing.push_error("discovery_network_error");
    let summary = rematch(
        &store,
        &rules,
        &hgamer_disabled(),
        &game(42, "Game"),
        &[],
        &failing,
        300,
    )
    .unwrap();

    assert_eq!(summary.status, GameMatchStatus::Incomplete);
    let after = record(&store, 42);
    let source = &after.sources["kagura-cn"];
    assert_eq!(source.status, SourceMatchStatus::DetailIncomplete);
    assert_eq!(
        source.candidates, previous.sources["kagura-cn"].candidates,
        "previously found candidates are preserved"
    );
    assert!(!after.sources["kagura-cn"].diagnostics.is_empty());
}

/// Completed games remain committed when cancellation arrives between games.
#[test]
fn cancellation_saves_finished_games_and_returns_task_cancelled() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_rule();
    store
        .save_index(&index(
            &rule.id,
            1,
            true,
            vec![
                entry("Game A", "https://kagurafan.com/game/a/", "article"),
                entry("Game B", "https://kagurafan.com/game/b/", "article"),
            ],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(
        200,
        &article("Game A", &[42]),
        "https://kagurafan.com/game/a/",
    ));
    let cancel = Arc::new(AtomicBool::new(false));
    let mut ctx = context(&fake);
    let error = match match_games(
        &store,
        &[rule],
        &Overrides::default(),
        &[game(42, "Game A"), game(43, "Game B")],
        &[],
        false,
        &mut ctx,
        &cancel,
        200,
        |_| {},
        |_| {
            cancel.store(true, Ordering::SeqCst);
        },
    ) {
        Ok(_) => panic!("the task must be cancelled"),
        Err(error) => error,
    };
    assert_eq!(error.to_string(), "task_cancelled");
    assert_eq!(fake.call_count(), 1);
    assert!(
        store.load_match(42).unwrap().is_some(),
        "finished games are saved before cancellation"
    );
    assert!(store.load_match(43).unwrap().is_none());
}

// ---- 10. input_key / incremental recomputation --------------------------

#[test]
fn matching_input_changes_trigger_a_rematch_but_unrelated_settings_do_not() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_rule();
    store
        .save_index(&index(
            &rule.id,
            1,
            true,
            vec![entry("Game", "https://kagurafan.com/game/", "article")],
        ))
        .unwrap();
    seed_detail(
        &store,
        &rule,
        1,
        fact("https://kagurafan.com/game/", "Game", 42),
    );
    let rules = std::slice::from_ref(&rule);
    let no_network = Arc::new(FakeFetcher::new());

    let first = run(
        &store,
        rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &no_network,
        false,
        100,
    )
    .unwrap();
    assert_eq!(first[0].status, GameMatchStatus::PublisherMatch);
    let key = record(&store, 42).input_key;
    assert_eq!(no_network.call_count(), 0);

    // Same inputs: reuse without rewriting.
    run(
        &store,
        rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &no_network,
        false,
        200,
    )
    .unwrap();
    assert_eq!(record(&store, 42).updated_at, 100);

    // Name change: recompute even though the cached detail is reused.
    run(
        &store,
        rules,
        &Overrides::default(),
        &[game(42, "Game 2")],
        &no_network,
        false,
        300,
    )
    .unwrap();
    let renamed = record(&store, 42);
    assert_eq!(renamed.updated_at, 300);
    assert_ne!(renamed.input_key, key);

    // Localized name change: recompute.
    let mut localized = game(42, "Game");
    localized.localized_name = Some("游戏".to_string());
    run(
        &store,
        rules,
        &Overrides::default(),
        &[localized],
        &no_network,
        false,
        400,
    )
    .unwrap();
    assert_eq!(record(&store, 42).updated_at, 400);

    // Subscription change: recompute.
    let unsubscribed = Overrides {
        games: [("42".to_string(), [("kagura-cn".to_string(), false)].into())].into(),
        ..Overrides::default()
    };
    run(
        &store,
        rules,
        &unsubscribed,
        &[game(42, "Game")],
        &no_network,
        false,
        500,
    )
    .unwrap();
    assert_eq!(record(&store, 42).updated_at, 500);

    // Back to the base inputs, then only an unrelated game's subscription and
    // an unknown source flag change: no recomputation.
    run(
        &store,
        rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &no_network,
        false,
        600,
    )
    .unwrap();
    assert_eq!(record(&store, 42).updated_at, 600);
    let unrelated = Overrides {
        enabled: [("some-other-source".to_string(), false)].into(),
        games: [("999".to_string(), [("kagura-cn".to_string(), false)].into())].into(),
        ..Overrides::default()
    };
    run(
        &store,
        rules,
        &unrelated,
        &[game(42, "Game")],
        &no_network,
        false,
        700,
    )
    .unwrap();
    assert_eq!(
        record(&store, 42).updated_at,
        600,
        "unrelated settings do not trigger a rematch"
    );

    // Frontend name hints are matching inputs: a change recomputes, the same
    // hints reuse the record.
    let hints = vec!["Alias".to_string()];
    run_input(
        &store,
        rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &hints,
        &no_network,
        false,
        800,
    )
    .unwrap();
    let hinted = record(&store, 42);
    assert_eq!(hinted.updated_at, 800);
    assert_ne!(hinted.input_key, key);
    run_input(
        &store,
        rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &hints,
        &no_network,
        false,
        900,
    )
    .unwrap();
    assert_eq!(
        record(&store, 42).updated_at,
        800,
        "unchanged hints reuse the persisted record"
    );

    assert_eq!(no_network.call_count(), 0);
}

// ---- entry (patch.moe / table) sources ----------------------------------

#[test]
fn legacy_language_digests_are_recomputed_then_reused_without_clearing_sources() {
    use patchhub_core::sources::{
        canonical::canonical_sha256, rule_fingerprint, PARSER_VERSION, QUERY_VERSION,
        SOURCES_ENCODING_VERSION,
    };
    for preferred in [serde_json::Value::Null, serde_json::json!("zh-CN")] {
        let temp = tempfile::tempdir().unwrap();
        let store = SourceStore::open(temp.path()).unwrap();
        let rules = [kagura_rule(), hgamer_rule()];
        store
            .save_index(&index(&rules[0].id, 1, true, vec![]))
            .unwrap();
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(
            200,
            &hgamer_search("Game", 0, &[]),
            "https://hgamer.tw/zh-CN/search?q=Game",
        ));
        run(
            &store,
            &rules,
            &Overrides::default(),
            &[game(42, "Game")],
            &fake,
            false,
            100,
        )
        .unwrap();
        let mut old = record(&store, 42);
        let current_input = old.input_key.clone();
        old.input_key = canonical_sha256(
            &serde_json::json!({"name":"Game","localized_name":null,"name_hints":[],"preferred_language":preferred,
            "subscriptions":{rules[0].id.clone():true,rules[1].id.clone():true}}),
        );
        let legacy_fallback = canonical_sha256(
            &serde_json::json!({"name":"Game","name_hints":[],"preferred_language":preferred,
            "rule_fingerprint":rule_fingerprint(&rules[1]),"parser_version":PARSER_VERSION,"query_version":QUERY_VERSION,"encoding_version":SOURCES_ENCODING_VERSION}),
        );
        old.fallback.as_mut().unwrap().query_key = legacy_fallback.clone();
        store.save_match(&old).unwrap();
        assert_ne!(old.input_key, current_input);
        for now in [200, 400] {
            let network = Arc::new(FakeFetcher::new());
            network.push(response(
                200,
                &hgamer_search("Game", 0, &[]),
                "https://hgamer.tw/zh-CN/search?q=Game",
            ));
            run(
                &store,
                &rules,
                &Overrides::default(),
                &[game(42, "Game")],
                &network,
                false,
                now,
            )
            .unwrap();
            assert_eq!(network.call_count(), 1);
            let updated = record(&store, 42);
            assert_eq!(updated.input_key, current_input);
            assert_ne!(
                updated.fallback.as_ref().unwrap().query_key,
                legacy_fallback
            );
            let warm = Arc::new(FakeFetcher::new());
            run(
                &store,
                &rules,
                &Overrides::default(),
                &[game(42, "Game")],
                &warm,
                false,
                now + 1,
            )
            .unwrap();
            assert_eq!(warm.call_count(), 0);
            assert_eq!(record(&store, 42).updated_at, now);
            assert_eq!(store.load_index(&rules[0].id).unwrap().unwrap().revision, 1);
            if now == 200 {
                let mut stale_fallback = record(&store, 42);
                stale_fallback.fallback.as_mut().unwrap().query_key = legacy_fallback.clone();
                store.save_match(&stale_fallback).unwrap();
            }
        }
    }
}

#[test]
fn entry_sources_match_offline_and_flag_conflicting_urls() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [entry_rule()];
    store
        .save_index(&index(
            &rules[0].id,
            1,
            true,
            vec![
                artifact_entry(
                    "Game",
                    "https://example.com/list",
                    "https://download.example.com/game.zip",
                    Some(42),
                    "en",
                ),
                artifact_entry(
                    "Other",
                    "https://example.com/list",
                    "https://download.example.com/other.zip",
                    Some(7),
                    "en",
                ),
            ],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let results = run(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    assert_eq!(fake.call_count(), 0, "list entries never need network");
    assert_eq!(results[0].status, GameMatchStatus::PublisherMatch);
    assert_eq!(results[0].candidates[0].match_basis, MatchBasis::Appid);
    assert_eq!(
        results[0].candidates[0].mirrors[0].url,
        "https://download.example.com/game.zip"
    );

    // One game, two different artifact URLs: never auto-select.
    store
        .save_index(&index(
            &rules[0].id,
            2,
            true,
            vec![
                artifact_entry(
                    "Game",
                    "https://example.com/list",
                    "https://download.example.com/a.zip",
                    Some(42),
                    "en",
                ),
                artifact_entry(
                    "Game",
                    "https://example.com/list",
                    "https://download.example.com/b.zip",
                    Some(42),
                    "zh-CN",
                ),
            ],
        ))
        .unwrap();
    let results = run(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &fake,
        false,
        300,
    )
    .unwrap();
    assert_eq!(results[0].status, GameMatchStatus::NoMatch);
    assert_eq!(
        record(&store, 42).sources["table-source"].status,
        SourceMatchStatus::NoCandidate
    );
    assert!(record(&store, 42).sources["table-source"]
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "source_match_ambiguous"));

    // Same URL in two languages: prefer zh-CN instead of declaring ambiguity.
    store
        .save_index(&index(
            &rules[0].id,
            3,
            true,
            vec![
                artifact_entry(
                    "Game",
                    "https://example.com/list",
                    "https://download.example.com/game.zip",
                    Some(42),
                    "en",
                ),
                artifact_entry(
                    "Game",
                    "https://example.com/list",
                    "https://download.example.com/game.zip",
                    Some(42),
                    "zh-CN",
                ),
            ],
        ))
        .unwrap();
    let results = run(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &fake,
        false,
        400,
    )
    .unwrap();
    assert_eq!(results[0].status, GameMatchStatus::PublisherMatch);
    assert_eq!(
        results[0].candidates[0].languages,
        vec!["zh-CN".to_string()]
    );
}

#[test]
fn source_revisions_and_summary_are_pure_reads() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [kagura_rule(), entry_rule(), hgamer_rule()];
    store
        .save_index(&index(&rules[0].id, 1, true, Vec::new()))
        .unwrap();
    store
        .save_index(&index(&rules[1].id, 3, false, Vec::new()))
        .unwrap();

    let revisions = source_revisions(&store, &rules);
    assert_eq!(revisions.get("kagura-cn"), Some(&1));
    assert_eq!(revisions.get("table-source"), Some(&3));
    assert!(
        !revisions.contains_key("hgamer"),
        "the fallback source has no index revision"
    );

    let pending = game_match_summary(&store, 42);
    assert_eq!(pending.status, GameMatchStatus::Pending);
    assert!(pending.candidates.is_empty());
    assert!(pending.source_revisions.is_empty());

    // `matches/<appid>.json` round trip through the pure summary.
    let mut match_record = GameMatchRecord {
        schema_version: GAME_MATCH_SCHEMA_VERSION,
        appid: 42,
        name: "Game".to_string(),
        localized_name: None,
        input_key: "key".to_string(),
        status: GameMatchStatus::PublisherMatch,
        sources: [(
            "table-source".to_string(),
            SourceMatchRecord {
                index_revision: 3,
                status: SourceMatchStatus::Matched,
                candidates: vec![candidate("https://example.com/list", 42)],
                detail_pages: Vec::new(),
                checked_at: 10,
                diagnostics: Vec::new(),
            },
        )]
        .into(),
        fallback: Some(FallbackMatchRecord {
            sources_fingerprint: None,
            query_key: "q".to_string(),
            status: FallbackMatchStatus::Matched,
            candidates: vec![candidate("https://hgamer.tw/p/1", 42)],
            checked_at: 10,
            diagnostics: Vec::new(),
        }),
        updated_at: 10,
    };
    store.save_match(&match_record).unwrap();
    let summary = game_match_summary(&store, 42);
    assert_eq!(summary.status, GameMatchStatus::PublisherMatch);
    assert_eq!(summary.source_revisions.get("table-source"), Some(&3));
    assert_eq!(summary.candidates.len(), 1);
    assert_eq!(summary.candidates[0].source_id, "table-source");
    assert!(
        summary
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "should_not_appear"),
        "diagnostics come from the record"
    );

    // A disabled fallback with no publisher record displays no candidates, and
    // keeps the P1 contract status (disabled means "not in enabled sources").
    store
        .update_settings(|settings| {
            settings.enabled.insert("hgamer".into(), false);
            Ok(())
        })
        .unwrap();
    match_record.status = GameMatchStatus::Pending;
    match_record.sources.clear();
    match_record.fallback = Some(FallbackMatchRecord {
        sources_fingerprint: Some(
            patchhub_core::sources::matching::fallback_sources_fingerprint(
                &[],
                &Overrides::default(),
                42,
            ),
        ),
        query_key: "q".to_string(),
        status: FallbackMatchStatus::Disabled,
        candidates: vec![candidate("https://hgamer.tw/p/1", 42)],
        checked_at: 10,
        diagnostics: Vec::new(),
    });
    store.save_match(&match_record).unwrap();
    let summary = game_match_summary(&store, 42);
    assert!(summary.candidates.is_empty());
    assert_eq!(summary.status, GameMatchStatus::NoMatch);
}

fn candidate(page: &str, appid: u32) -> SourceCandidate {
    let mut candidate = SourceCandidate {
        parser_fingerprint: None,
        candidate_id: "candidate-000000000000000000000000".to_string(),
        source_id: "table-source".to_string(),
        source_record_id: page.to_string(),
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
        languages: vec!["en".to_string()],
        version_raw: None,
        version_evidence: Vec::new(),
        version_identity: "page-unknown".to_string(),
        published_at: None,
        patch_updated_at: None,
        source_page: page.to_string(),
        upstream_page: None,
        mirrors: Vec::new(),
        format_reported: ArtifactFormat::Zip,
        format_observed: ArtifactFormat::Zip,
        capability: Capability::DiscoverOnly,
        identity_evidence: Vec::new(),
        provenance: Vec::new(),
        match_basis: MatchBasis::Appid,
        observed_at: "2024-01-01T00:00:00+00:00".to_string(),
        cache_status: CacheStatus::Cached,
        warnings: Vec::new(),
        fingerprint: "fingerprint".to_string(),
        related_index_pages: Vec::new(),
    };
    candidate.fingerprint =
        patchhub_core::sources::canonical::fingerprint(&serde_json::to_value(&candidate).unwrap());
    candidate
}
const VBF_PAGE: &str = "https://kagurafan.com/2020/02/07/venus-blood-frontier/";

fn vbf_upstream(appid: u32, page: &str) -> SourceCandidate {
    let rule = hgamer_rule();
    let query = QueryContext {
        appid,
        name: "VenusBlood FRONTIER International".into(),
        name_hints: vec![],

        legacy_match: false,
        force: false,
        source_context: vec![],
        parser_version: 2,
        query_version: 1,
        settings_revision: 0,
        generation: None,
    };
    let mut candidates = patchhub_core::sources::identity::build_candidates(
        &rule,
        "VenusBlood FRONTIER International 中文無碼補丁包",
        "https://hgamer.tw/zh-TW/patches/vbf",
        "中文 补丁 v5",
        &[appid],
        std::slice::from_ref(&query.name),
        "unknown",
        &[Mirror {
            url: page.into(),
            label: None,
            format: "web".into(),
        }],
        &query,
    );
    let mut candidate = candidates.remove(0);
    candidate.upstream_page = Some(page.into());
    candidate
}

fn seed_vbf_fallback(store: &SourceStore, appid: u32, upstream: &str) {
    store
        .save_match(&GameMatchRecord {
            schema_version: GAME_MATCH_SCHEMA_VERSION,
            appid: 1189440,
            name: "VenusBlood FRONTIER International".into(),
            localized_name: None,
            input_key: String::new(),
            status: GameMatchStatus::FallbackMatch,
            sources: Default::default(),
            fallback: Some(FallbackMatchRecord {
                sources_fingerprint: Some(
                    patchhub_core::sources::matching::fallback_sources_fingerprint(
                        &[hgamer_rule()],
                        &Overrides::default(),
                        1189440,
                    ),
                ),
                query_key: String::new(),
                status: FallbackMatchStatus::Matched,
                candidates: vec![vbf_upstream(appid, upstream)],
                checked_at: 100,
                diagnostics: vec![],
            }),
            updated_at: 100,
        })
        .unwrap();
}

fn vbf_article() -> String {
    "<article><h1 class=\"entry-title\">Venus Blood Frontier</h1><div class=\"entry-content\">中文无码补丁包v5 <a href=\"https://kaguraserver.com/venusblood-frontier-cn-v5.exe\">神乐下载</a></div></article>".into()
}

#[test]
fn vbf_cached_appid_upstream_resolves_indexed_article_without_title_guessing() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(tmp.path()).unwrap();
    let rules = vec![kagura_rule(), hgamer_rule()];
    store
        .save_index(&index(
            "kagura-cn",
            1,
            true,
            vec![entry("Venus Blood Frontier", VBF_PAGE, "article")],
        ))
        .unwrap();
    seed_vbf_fallback(&store, 1189440, VBF_PAGE);
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(200, &vbf_article(), VBF_PAGE));
    let results = run(
        &store,
        &rules,
        &Overrides::default(),
        &[game(1189440, "VenusBlood FRONTIER International")],
        &fake,
        false,
        200,
    )
    .unwrap();
    assert_eq!(results[0].status, GameMatchStatus::PublisherMatch);
    assert_eq!(fake.call_count(), 1);
    assert!(results[0]
        .candidates
        .iter()
        .all(|c| c.source_id == "kagura-cn"));
    let candidate = &results[0].candidates[0];
    assert_eq!(candidate.game_appid, Some(1189440));
    assert_eq!(candidate.match_basis, MatchBasis::VerifiedAlias);
    assert_eq!(candidate.source_page, VBF_PAGE);
    assert!(candidate
        .identity_evidence
        .iter()
        .any(|e| e.kind == "verified_upstream"));
    let offline = Arc::new(FakeFetcher::new());
    let again = run(
        &store,
        &rules,
        &Overrides::default(),
        &[game(1189440, "VenusBlood FRONTIER International")],
        &offline,
        false,
        300,
    )
    .unwrap();
    assert_eq!(again[0].status, GameMatchStatus::PublisherMatch);
    assert_eq!(offline.call_count(), 0);
}

#[test]
fn vbf_upstream_does_not_fetch_unindexed_or_foreign_appid_pages() {
    for (appid, upstream) in [
        (42, VBF_PAGE),
        (1189440, "https://kagurafan.com/not-indexed/"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let store = SourceStore::open(tmp.path()).unwrap();
        let rules = vec![kagura_rule(), hgamer_rule()];
        store
            .save_index(&index(
                "kagura-cn",
                1,
                true,
                vec![entry("Venus Blood Frontier", VBF_PAGE, "article")],
            ))
            .unwrap();
        seed_vbf_fallback(&store, appid, upstream);
        // With incomplete coverage no new HGamer request is made.
        let mut incomplete = store.load_index("kagura-cn").unwrap().unwrap();
        incomplete.complete = false;
        store.save_index(&incomplete).unwrap();
        let fake = Arc::new(FakeFetcher::new());
        let results = run(
            &store,
            &rules,
            &Overrides::default(),
            &[game(1189440, "VenusBlood FRONTIER International")],
            &fake,
            false,
            200,
        )
        .unwrap();
        assert_ne!(results[0].status, GameMatchStatus::PublisherMatch);
        assert!(record(&store, 1189440).sources["kagura-cn"]
            .candidates
            .is_empty());
        assert_eq!(fake.call_count(), 0);
    }
}
#[test]
fn vbf_new_fallback_resolves_indexed_upstream_in_same_task() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(tmp.path()).unwrap();
    let rules = vec![kagura_rule(), hgamer_rule()];
    store
        .save_index(&index(
            "kagura-cn",
            1,
            true,
            vec![entry("Venus Blood Frontier", VBF_PAGE, "article")],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let name = "VenusBlood FRONTIER International";
    fake.push(response(
        200,
        &hgamer_search(
            name,
            1,
            &[(
                "VenusBlood FRONTIER International patch",
                "/zh-CN/patches/vbf",
            )],
        ),
        "https://hgamer.tw/zh-CN/search?q=VenusBlood%20FRONTIER%20International",
    ));
    let patch = hgamer_patch("VenusBlood FRONTIER International patch", 1189440)
        .replace("https://mirror.example/1189440.zip", VBF_PAGE);
    fake.push(response(200, &patch, "https://hgamer.tw/zh-CN/patches/vbf"));
    fake.push(response(200, &vbf_article(), VBF_PAGE));
    let results = run(
        &store,
        &rules,
        &Overrides::default(),
        &[game(1189440, name)],
        &fake,
        false,
        200,
    )
    .unwrap();
    assert_eq!(results[0].status, GameMatchStatus::PublisherMatch);
    assert_eq!(fake.call_count(), 3);
    assert!(results[0]
        .candidates
        .iter()
        .all(|c| c.source_id == "kagura-cn"));
    assert_eq!(
        record(&store, 1189440).fallback.unwrap().status,
        FallbackMatchStatus::Matched
    );
    // A failed refresh with the same saved upstream evidence is attempted once.
    let failed = Arc::new(FakeFetcher::new());
    failed.push(response(404, "removed", VBF_PAGE));
    run(
        &store,
        &rules,
        &Overrides::default(),
        &[game(1189440, name)],
        &failed,
        true,
        300,
    )
    .unwrap();
    assert_eq!(failed.call_count(), 1);
    assert_eq!(
        record(&store, 1189440).sources["kagura-cn"].status,
        SourceMatchStatus::DetailIncomplete
    );
}
