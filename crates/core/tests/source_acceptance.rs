//! P6 acceptance matrix (design §7 items 1–9) for the per-source index /
//! game-match refactor. Everything is fixture driven through `FakeFetcher` (or a
//! hookable fetcher for mid-task settings changes); no test touches the network.
//!
//! Run with:
//!
//! ```text
//! cargo test -p patchhub-core --test source_acceptance -- --nocapture --test-threads=1
//! ```
//!
//! Matrix mapping:
//!   matrix1_*  design §7.1  one source update shared by 1/10/any games
//!   matrix2_*  design §7.2  one shared detail page requested once
//!   matrix3_*  design §7.3  incremental/restart and duplicate AppID instances
//!   matrix4_*  design §7.4  six states and publisher-match hiding the fallback
//!   matrix5_*  design §7.5  rematch hit/no-match games, source-scoped rematch
//!   matrix6_*  design §7.6  source update is list-only; match fetches details
//!   matrix7_*  design §7.7  failure/cancel/removal and cache age
//!   matrix8_*  design §7.8  commit-time settings binding
//!   matrix9_*  design §7.9  warm/offline reads and request counts

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use patchhub_core::sources::http::{
    FakeFetcher, FetchRequest, FetchResponse, Fetcher, RequestContext,
};
use patchhub_core::sources::matching::{
    game_match_summary, match_games, rematch_game, rematch_source,
};
use patchhub_core::sources::model::*;
use patchhub_core::sources::registry::SourceRegistry;
use patchhub_core::sources::snapshot::SourceSnapshot;
use patchhub_core::sources::store::SourceStore;
use patchhub_core::sources::update::update_source;
use patchhub_core::sources::{GAME_MATCH_SCHEMA_VERSION, SOURCE_DETAIL_SCHEMA_VERSION};

// ---- fixtures ------------------------------------------------------------

const KAGURA_ENTRY: &str = "https://kagurafan.com/";
const HGAMER_BASE: &str = "https://hgamer.tw/zh-CN/patches";
const PATCHMOE_ENTRY: &str = "https://www.patch.moe/?flag=1";

const PATCHMOE_FORMS: &str = r#"<html><body>
<form name="form_zh_cn" method="post" action="/zh"><input type="hidden" name="lang" value="zh-CN"></form>
<form name="form_en" method="post" action="/en"><input type="hidden" name="lang" value="en"></form>
</body></html>"#;

/// Cropped live patch.moe list excerpt (same fixture as the P2 tests).
const PATCHMOE_INDEX: &str = include_str!("fixtures/sources/patchmoe-index.html");

fn kagura_rule() -> SourceRule {
    SourceRule {
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

fn table_rule() -> SourceRule {
    SourceRule {
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

fn patchmoe_rule() -> SourceRule {
    SourceRule {
        id: "patchmoe".to_string(),
        name: "patch.moe".to_string(),
        adapter: "patchmoe".to_string(),
        role: SourceRole::Publisher,
        discovery_only: false,
        priority: 100,
        enabled: true,
        pages: vec![PATCHMOE_ENTRY.to_string()],
        page_hosts: vec!["www.patch.moe".to_string()],
        download_hosts: vec!["download.patch.moe".to_string()],
        root_layout: Some(RootLayout::GameRoot),
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
    index_at(source_id, revision, complete, entries, 100)
}

fn index_at(
    source_id: &str,
    revision: u64,
    complete: bool,
    entries: Vec<SourceIndexEntry>,
    checked_at: u64,
) -> SourceIndex {
    SourceIndex {
        schema_version: 1,
        source_id: source_id.to_string(),
        revision,
        complete,
        checked_at,
        updated_at: checked_at,
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

/// Kagura CN list page with one index record per `(title, href)`.
fn kagura_list(entries: &[(&str, &str)]) -> String {
    let mut html = String::from("<html><body>");
    for (title, href) in entries {
        html.push_str(&format!(
            "<article><h2 class=\"entry-title\"><a href=\"{href}\">{title}</a></h2></article>"
        ));
    }
    html.push_str("</body></html>");
    html
}

/// Article detail page with Steam identity + mirror (credible candidate).
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

/// Article detail page without any Steam identity (title-only lead).
fn lead_article(title: &str) -> String {
    format!(
        "<html><body><article><h1 class=\"entry-title\">{title}</h1>\
         <div class=\"entry-content\"><p>中文 补丁 v1.2.3 ZIP</p>\
         <a href=\"https://kaguraserver.com/lead.zip\">下载 Mirror 1</a></div>\
         </article></body></html>"
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

fn table_html(rows: &str) -> String {
    format!("<table>{rows}</table>")
}

fn table_row(title: &str, slug: &str, appid: &str) -> String {
    format!(
        r#"<tr><td>{title}</td><td><a href="https://download.example.com/{slug}.zip">dl</a></td><td>{appid}</td></tr>"#
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

fn instance_game(appid: u32, name: &str, instance: &str) -> GameQuery {
    GameQuery {
        instance_id: Some(instance.to_string()),
        ..game(appid, name)
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

fn push_patchmoe_full(fake: &FakeFetcher) {
    fake.push(response(200, PATCHMOE_FORMS, PATCHMOE_ENTRY));
    fake.push(response(200, PATCHMOE_INDEX, "https://www.patch.moe/zh"));
    fake.push(response(200, PATCHMOE_INDEX, "https://www.patch.moe/en"));
}

// ---- runners -------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn run_update(
    store: &SourceStore,
    rule: &SourceRule,
    force: bool,
    fake: &Arc<FakeFetcher>,
) -> std::io::Result<patchhub_core::sources::update::SourceUpdateOutcome> {
    let mut ctx = context(fake);
    update_source(
        store,
        rule,
        force,
        &mut ctx,
        &AtomicBool::new(false),
        |_| {},
    )
}

#[allow(clippy::too_many_arguments)]
fn run_match(
    store: &SourceStore,
    rules: &[SourceRule],
    overrides: &Overrides,
    games: &[GameQuery],
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
        &[],
        force_details,
        &mut ctx,
        &AtomicBool::new(false),
        now,
        |_| {},
        |_| {},
    )
}

fn run_rematch(
    store: &SourceStore,
    rules: &[SourceRule],
    overrides: &Overrides,
    game: &GameQuery,
    fake: &Arc<FakeFetcher>,
    now: u64,
) -> std::io::Result<GameMatch> {
    let mut ctx = context(fake);
    rematch_game(
        store,
        rules,
        overrides,
        game,
        &[],
        &mut ctx,
        &AtomicBool::new(false),
        now,
        |_| {},
    )
}

#[allow(clippy::too_many_arguments)]
fn run_rematch_source(
    store: &SourceStore,
    rules: &[SourceRule],
    overrides: &Overrides,
    games: &[GameQuery],
    source_id: &str,
    fake: &Arc<FakeFetcher>,
    now: u64,
) -> std::io::Result<Vec<GameMatch>> {
    let mut ctx = context(fake);
    rematch_source(
        store,
        rules,
        overrides,
        games,
        source_id,
        &[],
        &mut ctx,
        &AtomicBool::new(false),
        now,
        |_| {},
    )
}

fn record(store: &SourceStore, appid: u32) -> GameMatchRecord {
    store
        .load_match(appid)
        .unwrap()
        .unwrap_or_else(|| panic!("match record for {appid}"))
}

fn request_urls(fake: &FakeFetcher) -> Vec<String> {
    fake.calls().into_iter().map(|call| call.url).collect()
}

fn url_count(fake: &FakeFetcher, url: &str) -> usize {
    fake.calls().iter().filter(|call| call.url == url).count()
}

// ---- hookable fetcher (mid-task settings changes) ------------------------

type Hook = Box<dyn FnOnce() + Send>;

/// `FakeFetcher` variant that runs a one-shot hook before answering a chosen
/// call, so a test can change persisted settings while a task is in flight.
struct HookedFetcher {
    queue: Mutex<VecDeque<std::io::Result<FetchResponse>>>,
    calls: Mutex<Vec<FetchRequest>>,
    hook: Mutex<Option<(usize, Hook)>>,
}

impl HookedFetcher {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
            hook: Mutex::new(None),
        })
    }

    fn push(&self, response: FetchResponse) {
        self.queue.lock().unwrap().push_back(Ok(response));
    }

    fn fire_on_call(&self, index: usize, hook: impl FnOnce() + Send + 'static) {
        *self.hook.lock().unwrap() = Some((index, Box::new(hook)));
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

struct HookedHandle(Arc<HookedFetcher>);

impl HookedHandle {
    fn fetch_inner(&self, request: &FetchRequest) -> std::io::Result<FetchResponse> {
        let index = {
            let mut calls = self.0.calls.lock().unwrap();
            calls.push(request.clone());
            calls.len() - 1
        };
        let hook = {
            let mut guard = self.0.hook.lock().unwrap();
            if guard.as_ref().is_some_and(|(at, _)| *at == index) {
                guard.take().map(|(_, hook)| hook)
            } else {
                None
            }
        };
        if let Some(hook) = hook {
            hook();
        }
        self.0
            .queue
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(std::io::Error::other("hooked_fake_empty")))
    }
}

impl Fetcher for HookedHandle {
    fn fetch(&self, request: &FetchRequest) -> std::io::Result<FetchResponse> {
        self.fetch_inner(request)
    }
}

fn hooked_context(fake: &Arc<HookedFetcher>) -> RequestContext {
    RequestContext::with_settings(Box::new(HookedHandle(Arc::clone(fake))), Duration::ZERO)
}

/// Persists "source disabled" exactly like the host RPC
/// (`apply_overrides` + `bump_revision`).
fn disable_source(store: &SourceStore, source_id: &str) {
    let overrides = Overrides {
        enabled: BTreeMap::from([(source_id.to_string(), false)]),
        ..Overrides::default()
    };
    store.apply_overrides(&overrides).unwrap();
    store.bump_revision().unwrap();
}

/// Persists "game unsubscribed from this source".
fn unsubscribe_game(store: &SourceStore, appid: u32, source_id: &str) {
    let overrides = Overrides {
        games: BTreeMap::from([(
            appid.to_string(),
            BTreeMap::from([(source_id.to_string(), false)]),
        )]),
        ..Overrides::default()
    };
    store.apply_overrides(&overrides).unwrap();
    store.bump_revision().unwrap();
}

/// Persists an unrelated settings change (a different source's interval).
fn change_unrelated_interval(store: &SourceStore) {
    store.set_interval("hgamer", 24).unwrap();
    store.bump_revision().unwrap();
}

// ---- matrix 1: one source update, shared by 1/10/any games ---------------

#[test]
fn matrix1_one_update_snapshot_is_shared_and_index_requests_stay_constant() {
    println!("\nM1 one update shared by 1/10 games");
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_rule();
    let games: Vec<GameQuery> = (1..=10)
        .map(|i| game(1000 + i, &format!("Game {i}")))
        .collect();
    let list: Vec<(String, String)> = (1..=10)
        .map(|i| (format!("Game {i}"), format!("/article/{i}/")))
        .collect();
    let list_refs: Vec<(&str, &str)> = list.iter().map(|(t, p)| (t.as_str(), p.as_str())).collect();

    let update = Arc::new(FakeFetcher::new());
    update.push(response(200, &kagura_list(&list_refs), KAGURA_ENTRY));
    let outcome = run_update(&store, &rule, true, &update).unwrap();
    println!(
        "  update_source: requests={} code={} revision={} entries={}",
        update.call_count(),
        outcome.status.code,
        outcome.revision,
        outcome.status.entry_count
    );
    assert_eq!(update.call_count(), 1, "M1: list page only, no detail");
    assert_eq!(outcome.revision, 1);
    assert!(store.load_index("kagura-cn").unwrap().unwrap().complete);

    // 1 game: only its hit detail page is requested.
    let one = Arc::new(FakeFetcher::new());
    one.push(response(
        200,
        &article("Game 1", &[1001]),
        "https://kagurafan.com/article/1/",
    ));
    let first = run_match(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &games[..1],
        &one,
        false,
        200,
    )
    .unwrap();
    println!("  match 1 game: requests={}", one.call_count());
    assert_eq!(one.call_count(), 1);
    assert_eq!(first[0].status, GameMatchStatus::PublisherMatch);

    // 10 games: game 1's detail is reused; the other nine are fetched once each.
    let ten = Arc::new(FakeFetcher::new());
    for i in 2..=10 {
        ten.push(response(
            200,
            &article(&format!("Game {i}"), &[1000 + i]),
            &format!("https://kagurafan.com/article/{i}/"),
        ));
    }
    let all = run_match(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &games,
        &ten,
        false,
        300,
    )
    .unwrap();
    println!("  match 10 games: requests={}", ten.call_count());
    assert_eq!(ten.call_count(), 9, "M1: only the nine missing details");
    assert_eq!(
        ten.calls()[0].url,
        "https://kagurafan.com/article/2/",
        "M1: the first game's cached detail is not re-fetched"
    );
    assert!(all
        .iter()
        .all(|result| result.status == GameMatchStatus::PublisherMatch));

    // Restart: the persisted match records and detail cache make it offline.
    let reopened = SourceStore::open(temp.path()).unwrap();
    let warm = Arc::new(FakeFetcher::new());
    let after_restart = run_match(
        &reopened,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &games,
        &warm,
        false,
        400,
    )
    .unwrap();
    println!("  after restart: requests={}", warm.call_count());
    assert_eq!(warm.call_count(), 0, "M1: persisted records are reusable");
    assert!(after_restart
        .iter()
        .all(|result| result.status == GameMatchStatus::PublisherMatch));

    // A second source update is still one list request, regardless of the ten
    // installed games that now exist.
    let again = Arc::new(FakeFetcher::new());
    again.push(response(200, &kagura_list(&list_refs), KAGURA_ENTRY));
    let outcome = run_update(&store, &rule, true, &again).unwrap();
    println!(
        "  update with 10 games present: requests={} revision={}",
        again.call_count(),
        outcome.revision
    );
    assert_eq!(again.call_count(), 1, "M1: update stays source-scoped");
    assert_eq!(
        outcome.revision, 1,
        "M1: unchanged content keeps the revision"
    );
}

// ---- matrix 2: shared detail page ---------------------------------------

#[test]
fn matrix2_two_games_share_a_detail_page_and_requests_follow_hit_pages() {
    println!("\nM2 shared detail page");
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_rule();
    store
        .save_index(&index(
            "kagura-cn",
            1,
            true,
            vec![
                entry("Shared A", "https://kagurafan.com/article/a/", "article"),
                entry("Shared B", "https://kagurafan.com/article/b/", "article"),
                entry("Shared C", "https://kagurafan.com/article/c/", "article"),
            ],
        ))
        .unwrap();
    let index_bytes = std::fs::read(store.paths().index("kagura-cn")).unwrap();

    // Ten games only hit those three pages; title-only candidates are leads.
    let names = ["Shared A", "Shared B", "Shared C"];
    let games: Vec<GameQuery> = (0..10)
        .map(|i| game(2000 + i as u32, names[i % 3]))
        .collect();
    let fake = Arc::new(FakeFetcher::new());
    for page in ["a", "b", "c"] {
        fake.push(response(
            200,
            &lead_article(&format!("Shared {}", page.to_uppercase())),
            &format!("https://kagurafan.com/article/{page}/"),
        ));
    }
    let results = run_match(
        &store,
        &[rule],
        &Overrides::default(),
        &games,
        &fake,
        false,
        200,
    )
    .unwrap();
    let urls = request_urls(&fake);
    println!("  match 10 games: requests={} urls={:?}", urls.len(), urls);
    assert_eq!(results.len(), 10);
    assert_eq!(
        fake.call_count(),
        3,
        "M2: one request per distinct hit page"
    );
    assert_eq!(url_count(&fake, "https://kagurafan.com/article/a/"), 1);
    assert_eq!(url_count(&fake, "https://kagurafan.com/article/b/"), 1);
    assert_eq!(url_count(&fake, "https://kagurafan.com/article/c/"), 1);
    // Title-only leads are not confirmed matches, but they are displayed.
    assert!(results.iter().all(|result| !result.candidates.is_empty()));
    assert_eq!(
        std::fs::read(store.paths().index("kagura-cn")).unwrap(),
        index_bytes,
        "M2: matching never rewrites the source index"
    );
    let shared_record = record(&store, 2000);
    assert_eq!(
        shared_record.sources["kagura-cn"].detail_pages,
        vec!["https://kagurafan.com/article/a/".to_string()]
    );
}

// ---- matrix 3: incremental new game, restart, duplicate AppID ------------

#[test]
fn matrix3_new_game_matches_itself_and_duplicate_appids_query_fallback_once() {
    println!("\nM3 incremental / restart / duplicate appid");
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_rule();
    store
        .save_index(&index(
            "kagura-cn",
            1,
            true,
            vec![
                entry("Game A", "https://kagurafan.com/article/a/", "article"),
                entry("Game B", "https://kagurafan.com/article/b/", "article"),
            ],
        ))
        .unwrap();

    let a = game(3001, "Game A");
    let b = game(3002, "Game B");
    let first = Arc::new(FakeFetcher::new());
    first.push(response(
        200,
        &article("Game A", &[3001]),
        "https://kagurafan.com/article/a/",
    ));
    run_match(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        std::slice::from_ref(&a),
        &first,
        false,
        200,
    )
    .unwrap();
    assert_eq!(first.call_count(), 1);

    // Adding game B only fetches B's own hit page; A stays cached.
    let add = Arc::new(FakeFetcher::new());
    add.push(response(
        200,
        &article("Game B", &[3002]),
        "https://kagurafan.com/article/b/",
    ));
    let both = run_match(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &[a.clone(), b.clone()],
        &add,
        false,
        300,
    )
    .unwrap();
    println!(
        "  adding one game: requests={} urls={:?}",
        add.call_count(),
        request_urls(&add)
    );
    assert_eq!(add.call_count(), 1, "M3: only the new game's detail");
    assert_eq!(request_urls(&add), vec!["https://kagurafan.com/article/b/"]);
    assert!(both
        .iter()
        .all(|result| result.status == GameMatchStatus::PublisherMatch));
    let a_input_key = record(&store, 3001).input_key;
    assert!(!a_input_key.is_empty());

    // Restart: only reads persisted records and the shared detail cache.
    let reopened = SourceStore::open(temp.path()).unwrap();
    let warm = Arc::new(FakeFetcher::new());
    let after = run_match(
        &reopened,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &[a, b],
        &warm,
        false,
        400,
    )
    .unwrap();
    println!("  after restart: requests={}", warm.call_count());
    assert_eq!(warm.call_count(), 0);
    assert_eq!(record(&reopened, 3001).input_key, a_input_key);
    assert!(after
        .iter()
        .all(|result| result.status == GameMatchStatus::PublisherMatch));

    // Same AppID under two instances: one match result and one fallback query.
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [kagura_rule(), hgamer_rule()];
    store
        .save_index(&index(
            "kagura-cn",
            1,
            true,
            vec![entry("Other", "https://kagurafan.com/other/", "article")],
        ))
        .unwrap();
    let duplicate = [
        instance_game(42, "Game", "inst-1"),
        instance_game(42, "Game", "inst-2"),
    ];
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(
        200,
        &hgamer_search("Game", 0, &[]),
        "https://hgamer.tw/zh-CN/search?q=Game",
    ));
    let results = run_match(
        &store,
        &rules,
        &Overrides::default(),
        &duplicate,
        &fake,
        false,
        200,
    )
    .unwrap();
    println!(
        "  duplicate appid instances: requests={} results={}",
        fake.call_count(),
        results.len()
    );
    assert_eq!(
        fake.call_count(),
        1,
        "M3: one fallback query for the shared AppID"
    );
    assert_eq!(results.len(), 1, "M3: duplicate AppID is matched once");
    assert_eq!(results[0].instance_id.as_deref(), Some("inst-1"));
    assert_eq!(
        record(&store, 42).fallback.unwrap().status,
        FallbackMatchStatus::NoMatch
    );
}

// ---- matrix 4: six states + fallback hidden by publisher match ----------

#[test]
fn matrix4_six_states_and_disabled_or_incomplete_semantics() {
    println!("\nM4 six states");

    // pending: no record yet.
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    assert_eq!(
        game_match_summary(&store, 42).status,
        GameMatchStatus::Pending
    );

    // publisher_match: complete entry index matches offline.
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    store
        .save_index(&index(
            "table-source",
            1,
            true,
            vec![artifact_entry(
                "Game",
                "https://example.com/list",
                "https://download.example.com/game.zip",
                Some(42),
                "zh-CN",
            )],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let result = run_match(
        &store,
        &[table_rule()],
        &Overrides::default(),
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    assert_eq!(result[0].status, GameMatchStatus::PublisherMatch);
    assert_eq!(fake.call_count(), 0, "M4: index-only match is offline");

    // fallback_match: complete coverage, fallback returns a credible patch.
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [kagura_rule(), hgamer_rule()];
    store
        .save_index(&index(
            "kagura-cn",
            1,
            true,
            vec![entry("Other", "https://kagurafan.com/other/", "article")],
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
    let result = run_match(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    assert_eq!(result[0].status, GameMatchStatus::FallbackMatch);
    assert_eq!(result[0].candidates[0].source_id, "hgamer");

    // no_match: complete coverage, completed empty fallback.
    let empty = Arc::new(FakeFetcher::new());
    empty.push(response(
        200,
        &hgamer_search("Game", 0, &[]),
        "https://hgamer.tw/zh-CN/search?q=Game",
    ));
    let result = run_rematch(
        &store,
        &rules,
        &Overrides::default(),
        &game(42, "Game"),
        &empty,
        300,
    )
    .unwrap();
    assert_eq!(result.status, GameMatchStatus::NoMatch);
    assert_eq!(
        record(&store, 42).fallback.unwrap().status,
        FallbackMatchStatus::NoMatch
    );

    // incomplete: partial publisher coverage never queries fallback.
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    store
        .save_index(&index(
            "kagura-cn",
            1,
            false,
            vec![entry("Other", "https://kagurafan.com/other/", "article")],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let result = run_match(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    assert_eq!(result[0].status, GameMatchStatus::Incomplete);
    assert_eq!(fake.call_count(), 0);
    assert_eq!(
        record(&store, 42).sources["kagura-cn"].status,
        SourceMatchStatus::IndexIncomplete
    );
    assert_eq!(
        record(&store, 42).fallback.unwrap().status,
        FallbackMatchStatus::NotQueried
    );

    // disabled fallback: "not found among enabled sources", not HGamer-confirmed.
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    store
        .save_index(&index(
            "kagura-cn",
            1,
            true,
            vec![entry("Other", "https://kagurafan.com/other/", "article")],
        ))
        .unwrap();
    let disabled = Overrides {
        enabled: BTreeMap::from([("hgamer".to_string(), false)]),
        ..Overrides::default()
    };
    let fake = Arc::new(FakeFetcher::new());
    let result = run_match(
        &store,
        &rules,
        &disabled,
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    assert_eq!(result[0].status, GameMatchStatus::NoMatch);
    assert_eq!(fake.call_count(), 0);
    assert_eq!(
        record(&store, 42).fallback.unwrap().status,
        FallbackMatchStatus::Disabled
    );

    // network failure path: publisher detail + fallback both fail. The current
    // engine reports `incomplete` with diagnostics (recorded as a P6 deviation:
    // `failed` is only produced by migrated records, see p6-acceptance.md).
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    store
        .save_index(&index(
            "kagura-cn",
            1,
            true,
            vec![entry(
                "Game",
                "https://kagurafan.com/article/game/",
                "article",
            )],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    for _ in 0..2 {
        fake.push_error("discovery_network_error");
    }
    for _ in 0..2 {
        fake.push_error("discovery_network_error");
    }
    let result = run_match(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &fake,
        false,
        200,
    )
    .unwrap();
    println!(
        "  network failure: status={:?} diagnostics={}",
        result[0].status,
        result[0].diagnostics.len()
    );
    assert_eq!(result[0].status, GameMatchStatus::Incomplete);
    assert!(!result[0].diagnostics.is_empty());
    assert_eq!(
        record(&store, 42).sources["kagura-cn"].status,
        SourceMatchStatus::DetailIncomplete
    );
    assert_eq!(
        record(&store, 42).fallback.unwrap().status,
        FallbackMatchStatus::Failed
    );

    // failed: reachable for migrated records whose matched source failed.
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    store
        .save_match(&GameMatchRecord {
            schema_version: GAME_MATCH_SCHEMA_VERSION,
            appid: 99,
            name: "Migrated".to_string(),
            localized_name: None,
            input_key: String::new(),
            status: GameMatchStatus::Pending,
            sources: BTreeMap::from([(
                "kagura-cn".to_string(),
                SourceMatchRecord {
                    index_revision: 1,
                    status: SourceMatchStatus::DetailFailed,
                    candidates: Vec::new(),
                    detail_pages: Vec::new(),
                    checked_at: 1,
                    diagnostics: Vec::new(),
                },
            )]),
            fallback: Some(FallbackMatchRecord {
                query_key: "q".to_string(),
                status: FallbackMatchStatus::Failed,
                candidates: Vec::new(),
                checked_at: 1,
                diagnostics: Vec::new(),
            }),
            updated_at: 1,
        })
        .unwrap();
    assert_eq!(
        game_match_summary(&store, 99).status,
        GameMatchStatus::Failed
    );
}

#[test]
fn matrix4_a_later_publisher_match_hides_the_previous_fallback_candidates() {
    println!("\nM4 publisher match hides fallback");
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [kagura_rule(), hgamer_rule()];
    store
        .save_index(&index(
            "kagura-cn",
            1,
            true,
            vec![entry("Other", "https://kagurafan.com/other/", "article")],
        ))
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(
        200,
        &hgamer_search("Game", 1, &[("Game patch", "/zh-CN/patches/old")]),
        "https://hgamer.tw/zh-CN/search?q=Game",
    ));
    fake.push(response(
        200,
        &hgamer_patch("Game patch", 42),
        "https://hgamer.tw/zh-CN/patches/old",
    ));
    let first = run_match(
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
    let hgamer_id = first[0].candidates[0].candidate_id.clone();

    // The publisher index later contains a credible hit for the same game.
    store
        .save_index(&index(
            "kagura-cn",
            2,
            true,
            vec![entry(
                "Game",
                "https://kagurafan.com/article/game/",
                "article",
            )],
        ))
        .unwrap();
    let detail = Arc::new(FakeFetcher::new());
    detail.push(response(
        200,
        &article("Game", &[42]),
        "https://kagurafan.com/article/game/",
    ));
    let second = run_match(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &detail,
        false,
        300,
    )
    .unwrap();
    println!(
        "  status={:?} requests={}",
        second[0].status,
        detail.call_count()
    );
    assert_eq!(second[0].status, GameMatchStatus::PublisherMatch);
    assert!(!second[0]
        .candidates
        .iter()
        .any(|candidate| candidate.candidate_id == hgamer_id));
    assert_eq!(detail.call_count(), 1, "the new hit detail is fetched once");
}

// ---- matrix 5: rematch hit/no-match and source-scoped rematch -----------

#[test]
fn matrix5_rematch_refreshes_hits_and_rechecks_fallback_without_touching_index() {
    println!("\nM5 rematch");
    // Hit game: force refresh re-fetches the detail, never the index.
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_rule();
    store
        .save_index(&index(
            "kagura-cn",
            1,
            true,
            vec![entry(
                "Game",
                "https://kagurafan.com/article/game/",
                "article",
            )],
        ))
        .unwrap();
    let warm = Arc::new(FakeFetcher::new());
    warm.push(response(
        200,
        &article("Game", &[42]),
        "https://kagurafan.com/article/game/",
    ));
    run_match(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &[game(42, "Game")],
        &warm,
        false,
        200,
    )
    .unwrap();
    assert_eq!(warm.call_count(), 1);
    let index_bytes = std::fs::read(store.paths().index("kagura-cn")).unwrap();

    let forced = Arc::new(FakeFetcher::new());
    forced.push(response(
        200,
        &article("Game", &[42]),
        "https://kagurafan.com/article/game/",
    ));
    let result = run_rematch(
        &store,
        &[rule],
        &Overrides::default(),
        &game(42, "Game"),
        &forced,
        300,
    )
    .unwrap();
    println!(
        "  rematch hit: status={:?} requests={:?}",
        result.status,
        request_urls(&forced)
    );
    assert_eq!(result.status, GameMatchStatus::PublisherMatch);
    assert_eq!(
        forced.call_count(),
        1,
        "M5: only the hit detail is refreshed"
    );
    assert_eq!(
        std::fs::read(store.paths().index("kagura-cn")).unwrap(),
        index_bytes,
        "M5: rematch never updates the source index"
    );
    assert_eq!(record(&store, 42).sources["kagura-cn"].index_revision, 1);

    // No-match game: rematch actively re-queries the fallback.
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [kagura_rule(), hgamer_rule()];
    store
        .save_index(&index(
            "kagura-cn",
            1,
            true,
            vec![entry("Other", "https://kagurafan.com/other/", "article")],
        ))
        .unwrap();
    let empty = Arc::new(FakeFetcher::new());
    empty.push(response(
        200,
        &hgamer_search("Game", 0, &[]),
        "https://hgamer.tw/zh-CN/search?q=Game",
    ));
    let first = run_match(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &empty,
        false,
        200,
    )
    .unwrap();
    assert_eq!(first[0].status, GameMatchStatus::NoMatch);
    let requery = Arc::new(FakeFetcher::new());
    requery.push(response(
        200,
        &hgamer_search("Game", 1, &[("Game patch", "/zh-CN/patches/new")]),
        "https://hgamer.tw/zh-CN/search?q=Game",
    ));
    requery.push(response(
        200,
        &hgamer_patch("Game patch", 42),
        "https://hgamer.tw/zh-CN/patches/new",
    ));
    let result = run_rematch(
        &store,
        &rules,
        &Overrides::default(),
        &game(42, "Game"),
        &requery,
        300,
    )
    .unwrap();
    println!(
        "  rematch no-match: status={:?} requests={}",
        result.status,
        requery.call_count()
    );
    assert_eq!(result.status, GameMatchStatus::FallbackMatch);
    assert_eq!(requery.call_count(), 2, "M5: the fallback is re-queried");

    // Source-scoped rematch keeps other sources' records and does not write
    // any source index. This is the local equivalent of "another source's task
    // is not cancelled".
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rules = [kagura_rule(), table_rule()];
    store
        .save_index(&index(
            "kagura-cn",
            1,
            true,
            vec![entry("Other", "https://kagurafan.com/other/", "article")],
        ))
        .unwrap();
    store
        .save_index(&index(
            "table-source",
            1,
            true,
            vec![artifact_entry(
                "Game",
                "https://example.com/list",
                "https://download.example.com/game.zip",
                Some(42),
                "zh-CN",
            )],
        ))
        .unwrap();
    let none = Arc::new(FakeFetcher::new());
    run_match(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        &none,
        false,
        200,
    )
    .unwrap();
    let before = record(&store, 42);
    assert_eq!(
        before.sources["table-source"].status,
        SourceMatchStatus::Matched
    );
    let kagura_before = before.sources["kagura-cn"].clone();
    let kagura_index_bytes = std::fs::read(store.paths().index("kagura-cn")).unwrap();

    store
        .save_index(&index(
            "table-source",
            2,
            true,
            vec![artifact_entry(
                "Game",
                "https://example.com/list",
                "https://download.example.com/game.zip",
                Some(42),
                "zh-CN",
            )],
        ))
        .unwrap();
    let scoped = Arc::new(FakeFetcher::new());
    let results = run_rematch_source(
        &store,
        &rules,
        &Overrides::default(),
        &[game(42, "Game")],
        "table-source",
        &scoped,
        300,
    )
    .unwrap();
    let after = record(&store, 42);
    println!(
        "  rematch_source: requests={} status={:?} target_rev={} other_rev={}",
        scoped.call_count(),
        results[0].status,
        after.sources["table-source"].index_revision,
        after.sources["kagura-cn"].index_revision
    );
    assert_eq!(scoped.call_count(), 0, "entry sources rematch offline");
    assert_eq!(results[0].status, GameMatchStatus::PublisherMatch);
    assert_eq!(after.sources["table-source"].index_revision, 2);
    assert_eq!(
        after.sources["kagura-cn"], kagura_before,
        "M5: other source untouched"
    );
    assert_eq!(
        std::fs::read(store.paths().index("kagura-cn")).unwrap(),
        kagura_index_bytes
    );
}

// ---- matrix 6: source update list-only, match details separate ----------

#[test]
fn matrix6_update_is_list_only_and_match_details_are_a_separate_phase() {
    println!("\nM6 update vs match phases");
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_rule();

    let update = Arc::new(FakeFetcher::new());
    update.push(response(
        200,
        &kagura_list(&[("Game A", "/article/a/")]),
        KAGURA_ENTRY,
    ));
    let outcome = run_update(&store, &rule, true, &update).unwrap();
    let update_urls = request_urls(&update);
    println!(
        "  update_source urls={update_urls:?} revision={}",
        outcome.revision
    );
    assert_eq!(outcome.revision, 1);
    assert!(update_urls
        .iter()
        .all(|url| !url.contains("/article/") && !url.contains("/category/")));

    let games = [game(42, "Game A"), game(43, "Game B")];
    let detail = Arc::new(FakeFetcher::new());
    detail.push(response(
        200,
        &article("Game A", &[42]),
        "https://kagurafan.com/article/a/",
    ));
    let results = run_match(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &games,
        &detail,
        false,
        300,
    )
    .unwrap();
    let detail_urls = request_urls(&detail);
    println!("  match urls={detail_urls:?}");
    assert_eq!(results[0].status, GameMatchStatus::PublisherMatch);
    assert_eq!(results[1].status, GameMatchStatus::NoMatch);
    assert_eq!(detail_urls, vec!["https://kagurafan.com/article/a/"]);
    assert_eq!(detail.call_count(), 1, "M6: only the hit detail");

    // A new source revision only re-matches the games the caller passes (the
    // CLI passes the subscribed current-library games). Other persisted
    // records stay untouched.
    let update2 = Arc::new(FakeFetcher::new());
    update2.push(response(
        200,
        &kagura_list(&[("Game A", "/article/a/"), ("Game C", "/article/c/")]),
        KAGURA_ENTRY,
    ));
    let outcome = run_update(&store, &rule, true, &update2).unwrap();
    assert_eq!(outcome.revision, 2);
    let scoped = Arc::new(FakeFetcher::new());
    scoped.push(response(
        200,
        &article("Game A", &[42]),
        "https://kagurafan.com/article/a/",
    ));
    let results = run_rematch_source(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &games[..1],
        "kagura-cn",
        &scoped,
        400,
    )
    .unwrap();
    assert_eq!(results[0].status, GameMatchStatus::PublisherMatch);
    assert_eq!(scoped.call_count(), 1);
    assert_eq!(record(&store, 42).sources["kagura-cn"].index_revision, 2);
    let b = record(&store, 43);
    assert_eq!(
        b.sources["kagura-cn"].index_revision, 1,
        "M6: games not in the recompute set keep their revision"
    );
    assert_eq!(b.status, GameMatchStatus::NoMatch);
}

// ---- matrix 7: failure/cancel/removal and cache age ---------------------

#[test]
fn matrix7_failure_cancel_removal_and_cache_age() {
    println!("\nM7 failure/cancel/removal/age");
    // Failure preserves the published snapshot.
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = patchmoe_rule();
    let first = Arc::new(FakeFetcher::new());
    push_patchmoe_full(&first);
    run_update(&store, &rule, true, &first).unwrap();
    let published = std::fs::read(store.paths().index("patchmoe")).unwrap();
    let failing = Arc::new(FakeFetcher::new());
    for _ in 0..2 {
        failing.push_error("discovery_network_error");
    }
    let outcome = run_update(&store, &rule, true, &failing).unwrap();
    println!(
        "  transport failure: code={} revision={} entries={}",
        outcome.status.code, outcome.revision, outcome.status.entry_count
    );
    assert_eq!(outcome.status.code, "source_offline_cached");
    assert_eq!(outcome.revision, 1);
    assert_eq!(
        std::fs::read(store.paths().index("patchmoe")).unwrap(),
        published,
        "M7: a failed update keeps the last successful snapshot"
    );

    // Cancel keeps the old snapshot and saves the collected pages as progress.
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_rule();
    let good = Arc::new(FakeFetcher::new());
    good.push(response(
        200,
        &kagura_list(&[("Game A", "/article/a/")]),
        KAGURA_ENTRY,
    ));
    run_update(&store, &rule, true, &good).unwrap();
    let published = std::fs::read(store.paths().index("kagura-cn")).unwrap();

    let cancel = Arc::new(AtomicBool::new(false));
    let hooked = HookedFetcher::new();
    hooked.push(response(
        200,
        &kagura_list(&[("Game A", "/article/a/"), ("Game B", "/article/b/")]),
        KAGURA_ENTRY,
    ));
    let cancel_hook = Arc::clone(&cancel);
    hooked.fire_on_call(0, move || cancel_hook.store(true, Ordering::SeqCst));
    let mut ctx = hooked_context(&hooked);
    let error = update_source(&store, &rule, true, &mut ctx, &cancel, |_| {}).unwrap_err();
    println!("  cancel: error={} requests={}", error, hooked.call_count());
    assert_eq!(error.to_string(), "task_cancelled");
    assert_eq!(
        std::fs::read(store.paths().index("kagura-cn")).unwrap(),
        published,
        "M7: cancellation keeps the official snapshot"
    );
    let progress = store.load_index_progress("kagura-cn").unwrap().unwrap();
    assert_eq!(
        progress.entries.len(),
        2,
        "collected page is kept as progress"
    );

    // A complete update reflects withdrawn entries.
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = table_rule();
    let two = Arc::new(FakeFetcher::new());
    two.push(response(
        200,
        &table_html(&format!(
            "{}{}",
            table_row("Game A", "a", "42"),
            table_row("Game B", "b", "43")
        )),
        "https://example.com/list",
    ));
    run_update(&store, &rule, true, &two).unwrap();
    assert_eq!(
        store
            .load_index("table-source")
            .unwrap()
            .unwrap()
            .entries
            .len(),
        2
    );
    let one = Arc::new(FakeFetcher::new());
    one.push(response(
        200,
        &table_html(&table_row("Game A", "a", "42")),
        "https://example.com/list",
    ));
    let outcome = run_update(&store, &rule, true, &one).unwrap();
    let entries = store.load_index("table-source").unwrap().unwrap().entries;
    println!(
        "  withdrawn entry: revision={} entries={}",
        outcome.revision,
        entries.len()
    );
    assert_eq!(outcome.revision, 2);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].title, "Game A");

    // Cache age never blocks matching or manual-selection inputs.
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_rule();
    store
        .save_index(&index_at(
            "kagura-cn",
            1,
            true,
            vec![entry(
                "Game",
                "https://kagurafan.com/article/game/",
                "article",
            )],
            1,
        ))
        .unwrap();
    seed_detail(
        &store,
        &rule,
        1,
        fact("https://kagurafan.com/article/game/", "Game", 42),
    );
    let offline = Arc::new(FakeFetcher::new());
    let result = run_match(
        &store,
        &[rule],
        &Overrides::default(),
        &[game(42, "Game")],
        &offline,
        false,
        1_700_000_000,
    )
    .unwrap();
    println!(
        "  old cache age: status={:?} requests={} candidates={}",
        result[0].status,
        offline.call_count(),
        result[0].candidates.len()
    );
    assert_eq!(result[0].status, GameMatchStatus::PublisherMatch);
    assert_eq!(
        offline.call_count(),
        0,
        "an old detail cache is still reused"
    );
    assert!(!result[0].candidates.is_empty());
    let summary = game_match_summary(&store, 42);
    assert_eq!(summary.status, GameMatchStatus::PublisherMatch);
    assert!(
        !summary.candidates.is_empty(),
        "manual selection keeps its input"
    );

    // The registry read model also accepts an old entry index for install
    // resolution before/without a match task.
    let table = table_rule();
    store
        .save_index(&index_at(
            "table-source",
            1,
            true,
            vec![artifact_entry(
                "Game",
                "https://example.com/list",
                "https://download.example.com/game.zip",
                Some(42),
                "zh-CN",
            )],
            1,
        ))
        .unwrap();
    let registry = SourceRegistry::load(store, vec![table]).unwrap();
    let manual = registry.dynamic_zip_candidates(&game(42, "Game"));
    assert!(!manual.is_empty(), "old index still feeds manual selection");
}

// ---- matrix 8: commit-time settings binding -----------------------------

#[test]
fn matrix8_update_does_not_publish_when_source_is_disabled_mid_task() {
    println!("\nM8 update binding");
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(SourceStore::open(temp.path()).unwrap());
    let rule = patchmoe_rule();
    let hooked = HookedFetcher::new();
    hooked.push(response(200, PATCHMOE_FORMS, PATCHMOE_ENTRY));
    hooked.push(response(200, PATCHMOE_INDEX, "https://www.patch.moe/zh"));
    hooked.push(response(200, PATCHMOE_INDEX, "https://www.patch.moe/en"));
    let hook_store = Arc::clone(&store);
    hooked.fire_on_call(0, move || disable_source(&hook_store, "patchmoe"));
    let mut ctx = hooked_context(&hooked);
    let error = update_source(
        &store,
        &rule,
        true,
        &mut ctx,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap_err();
    println!(
        "  disabled mid-update: error={} requests={} published={:?} progress={:?}",
        error,
        hooked.call_count(),
        store.load_index("patchmoe").unwrap().map(|i| i.revision),
        store
            .load_index_progress("patchmoe")
            .unwrap()
            .map(|i| i.entries.len())
    );
    assert_eq!(error.to_string(), "source_not_enabled");
    assert!(
        store.load_index("patchmoe").unwrap().is_none(),
        "M8: a disabled source is not published"
    );
    assert!(store.load_index_progress("patchmoe").unwrap().is_some());
    assert!(
        store.load_status("patchmoe").unwrap().is_none(),
        "M8: no success status is written either"
    );
}

#[test]
fn matrix8_match_discards_a_result_when_settings_changed_mid_task() {
    println!("\nM8 match binding");
    // First match: source disabled while the hit detail is being fetched.
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(SourceStore::open(temp.path()).unwrap());
    let rule = kagura_rule();
    store
        .save_index(&index(
            "kagura-cn",
            1,
            true,
            vec![entry(
                "Game",
                "https://kagurafan.com/article/game/",
                "article",
            )],
        ))
        .unwrap();
    let hooked = HookedFetcher::new();
    hooked.push(response(
        200,
        &article("Game", &[42]),
        "https://kagurafan.com/article/game/",
    ));
    let hook_store = Arc::clone(&store);
    hooked.fire_on_call(0, move || disable_source(&hook_store, "kagura-cn"));
    let mut ctx = hooked_context(&hooked);
    let results = match_games(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &[game(42, "Game")],
        &[],
        false,
        &mut ctx,
        &AtomicBool::new(false),
        200,
        |_| {},
        |_| {},
    )
    .unwrap();
    println!(
        "  first match disabled mid-task: status={:?} persisted={} requests={}",
        results[0].status,
        store.load_match(42).unwrap().is_some(),
        hooked.call_count()
    );
    assert_eq!(results[0].status, GameMatchStatus::Pending);
    assert!(
        store.load_match(42).unwrap().is_none(),
        "M8: a discarded first attempt is not persisted"
    );

    // Second match: a previous publisher match is kept when the game is
    // unsubscribed while the forced detail refresh is in flight.
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(SourceStore::open(temp.path()).unwrap());
    let rule = kagura_rule();
    store
        .save_index(&index(
            "kagura-cn",
            1,
            true,
            vec![entry(
                "Game",
                "https://kagurafan.com/article/game/",
                "article",
            )],
        ))
        .unwrap();
    let warm = Arc::new(FakeFetcher::new());
    warm.push(response(
        200,
        &article("Game", &[42]),
        "https://kagurafan.com/article/game/",
    ));
    run_match(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &[game(42, "Game")],
        &warm,
        false,
        200,
    )
    .unwrap();
    let before = std::fs::read(store.paths().matches(42)).unwrap();
    let hooked = HookedFetcher::new();
    hooked.push(response(
        200,
        &article("Game", &[42]),
        "https://kagurafan.com/article/game/",
    ));
    let hook_store = Arc::clone(&store);
    hooked.fire_on_call(0, move || unsubscribe_game(&hook_store, 42, "kagura-cn"));
    let mut ctx = hooked_context(&hooked);
    let result = rematch_game(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &game(42, "Game"),
        &[],
        &mut ctx,
        &AtomicBool::new(false),
        300,
        |_| {},
    )
    .unwrap();
    println!(
        "  rematch unsubscribed mid-task: status={:?} requests={} bytes_unchanged={}",
        result.status,
        hooked.call_count(),
        std::fs::read(store.paths().matches(42)).unwrap() == before
    );
    assert_eq!(result.status, GameMatchStatus::PublisherMatch);
    assert_eq!(hooked.call_count(), 1);
    assert_eq!(
        std::fs::read(store.paths().matches(42)).unwrap(),
        before,
        "M8: the previous committed record is preserved"
    );

    // Unrelated settings changes (another source's interval) do not discard a
    // completed match.
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(SourceStore::open(temp.path()).unwrap());
    let rule = kagura_rule();
    store
        .save_index(&index(
            "kagura-cn",
            1,
            true,
            vec![entry(
                "Game",
                "https://kagurafan.com/article/game/",
                "article",
            )],
        ))
        .unwrap();
    let hooked = HookedFetcher::new();
    hooked.push(response(
        200,
        &article("Game", &[42]),
        "https://kagurafan.com/article/game/",
    ));
    let hook_store = Arc::clone(&store);
    hooked.fire_on_call(0, move || change_unrelated_interval(&hook_store));
    let mut ctx = hooked_context(&hooked);
    let results = match_games(
        &store,
        &[rule],
        &Overrides::default(),
        &[game(42, "Game")],
        &[],
        false,
        &mut ctx,
        &AtomicBool::new(false),
        200,
        |_| {},
        |_| {},
    )
    .unwrap();
    println!(
        "  unrelated settings change: status={:?} persisted={}",
        results[0].status,
        store.load_match(42).unwrap().is_some()
    );
    assert_eq!(results[0].status, GameMatchStatus::PublisherMatch);
    assert!(store.load_match(42).unwrap().is_some());
}

// ---- matrix 9: warm/offline reads and request counts --------------------

#[test]
fn matrix9_warm_match_is_offline_and_reads_do_not_write_state() {
    println!("\nM9 warm/offline reads");
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_rule();
    let list = kagura_list(&[("Game", "/article/game/")]);
    let update = Arc::new(FakeFetcher::new());
    update.push(response(200, &list, KAGURA_ENTRY));
    run_update(&store, &rule, true, &update).unwrap();
    let detail = Arc::new(FakeFetcher::new());
    detail.push(response(
        200,
        &article("Game", &[42]),
        "https://kagurafan.com/article/game/",
    ));
    run_match(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &[game(42, "Game")],
        &detail,
        false,
        200,
    )
    .unwrap();
    let warm = Arc::new(FakeFetcher::new());
    let result = run_match(
        &store,
        std::slice::from_ref(&rule),
        &Overrides::default(),
        &[game(42, "Game")],
        &warm,
        false,
        300,
    )
    .unwrap();
    println!(
        "  warm match: requests={} status={:?}",
        warm.call_count(),
        result[0].status
    );
    assert_eq!(warm.call_count(), 0);
    assert_eq!(result[0].status, GameMatchStatus::PublisherMatch);

    // Read-only source snapshot + match summary never write.
    let before = snapshot_files(temp.path());
    let snapshot = SourceSnapshot::load(temp.path(), &[game(42, "Game")], 1_700_000_000)
        .unwrap()
        .unwrap();
    assert!(snapshot.settings_revision() >= 1);
    let summary = game_match_summary(&store, 42);
    assert_eq!(summary.status, GameMatchStatus::PublisherMatch);
    let after = snapshot_files(temp.path());
    println!(
        "  read-only snapshot: files_before={} files_after={}",
        before.len(),
        after.len()
    );
    assert_eq!(
        before, after,
        "M9: scan-style reads do not write source state"
    );
}

/// Recursively hashes the state directory so a test can prove a read wrote
/// nothing (path -> bytes).
fn snapshot_files(root: &std::path::Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &std::path::Path, dir: &std::path::Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let relative = path
                    .strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                out.insert(relative, std::fs::read(&path).unwrap_or_default());
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut files);
    files
}
