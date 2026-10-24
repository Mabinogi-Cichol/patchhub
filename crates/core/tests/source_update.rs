//! P2 per-source index update: official snapshot vs. progress file, adapter
//! index collection, revision semantics, and P3 page facts (`resolve_page`).
//!
//! Every request goes through `FakeFetcher`; no test touches the network.

use std::fs;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use patchhub_core::sources::adapters::kagura::resolve_page;
use patchhub_core::sources::http::{FakeFetcher, FetchResponse, RequestContext};
use patchhub_core::sources::model::*;
use patchhub_core::sources::store::SourceStore;
use patchhub_core::sources::update::{update_source, SourceUpdateOutcome};

// ---- helpers -------------------------------------------------------------

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

fn response_with_etag(html: &str, final_url: &str, etag: &str) -> FetchResponse {
    FetchResponse {
        status: 200,
        body: html.as_bytes().to_vec(),
        final_url: final_url.to_string(),
        etag: Some(etag.to_string()),
        ..FetchResponse::default()
    }
}

fn run_update(
    store: &SourceStore,
    rule: &SourceRule,
    force: bool,
    fake: &Arc<FakeFetcher>,
) -> std::io::Result<SourceUpdateOutcome> {
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

fn patchmoe_rule() -> SourceRule {
    SourceRule {
        id: "patchmoe".to_string(),
        name: "patch.moe".to_string(),
        adapter: "patchmoe".to_string(),
        role: SourceRole::Publisher,
        discovery_only: false,
        priority: 100,
        enabled: true,
        pages: vec!["https://www.patch.moe/?flag=1".to_string()],
        page_hosts: vec!["www.patch.moe".to_string()],
        download_hosts: vec!["download.patch.moe".to_string()],
        root_layout: Some(RootLayout::GameRoot),
        columns: None,
        table_id: None,
        language: None,
        url: None,
    }
}

const PATCHMOE_FORMS: &str = r#"<html><body>
<form name="form_zh_cn" method="post" action="/zh"><input type="hidden" name="lang" value="zh-CN"></form>
<form name="form_en" method="post" action="/en"><input type="hidden" name="lang" value="en"></form>
</body></html>"#;

const PATCHMOE_INDEX: &str = include_str!("fixtures/sources/patchmoe-index.html");

/// Queues one full patch.moe collection: entry GET + two localized form POSTs.
fn push_patchmoe_full(fake: &FakeFetcher, html: &str, etag: Option<&str>) {
    match etag {
        Some(etag) => fake.push(response_with_etag(
            PATCHMOE_FORMS,
            "https://www.patch.moe/?flag=1",
            etag,
        )),
        None => fake.push(response(
            200,
            PATCHMOE_FORMS,
            "https://www.patch.moe/?flag=1",
        )),
    }
    fake.push(response(200, html, "https://www.patch.moe/zh"));
    fake.push(response(200, html, "https://www.patch.moe/en"));
}

fn table_rule() -> SourceRule {
    SourceRule {
        id: "table-source".to_string(),
        name: "Table".to_string(),
        adapter: "table_html".to_string(),
        role: SourceRole::Publisher,
        discovery_only: false,
        priority: 100,
        enabled: true,
        pages: vec![
            "https://example.com/list1".to_string(),
            "https://example.com/list2".to_string(),
        ],
        page_hosts: vec!["example.com".to_string()],
        download_hosts: vec!["download.example.com".to_string()],
        root_layout: Some(RootLayout::GameRoot),
        columns: Some(Columns {
            title: 0,
            download: 1,
            date: Some(2),
            language: Some(3),
            appid: Some(4),
        }),
        table_id: None,
        language: None,
        url: None,
    }
}

fn table_row(title: &str, slug: &str, appid: &str) -> String {
    format!(
        r#"<tr><td>{title}</td><td><a href="https://download.example.com/{slug}.zip">dl</a></td><td>2024/1/2</td><td>en</td><td>{appid}</td></tr>"#
    )
}

fn table_html(rows: &[String]) -> String {
    format!("<table>{}</table>", rows.concat())
}

fn kagura_cn_rule() -> SourceRule {
    SourceRule {
        id: "kagura-cn".to_string(),
        name: "Kagura CN".to_string(),
        adapter: "kagura_wordpress".to_string(),
        role: SourceRole::Publisher,
        discovery_only: true,
        priority: 10,
        enabled: true,
        pages: vec!["https://kagurafan.com/".to_string()],
        page_hosts: vec!["kagurafan.com".to_string()],
        download_hosts: vec!["kaguraserver.com".to_string()],
        root_layout: None,
        columns: None,
        table_id: None,
        language: None,
        url: None,
    }
}

fn kagura_en_rule() -> SourceRule {
    SourceRule {
        id: "kagura-en".to_string(),
        name: "Kagura EN".to_string(),
        adapter: "kagura_woocommerce".to_string(),
        role: SourceRole::Publisher,
        discovery_only: true,
        priority: 20,
        enabled: true,
        pages: vec!["https://www.kaguragames.com/patch/".to_string()],
        page_hosts: vec!["www.kaguragames.com".to_string()],
        download_hosts: vec!["kaguragamespatch.com".to_string()],
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

// ---- 1. patch.moe --------------------------------------------------------

#[test]
fn patchmoe_publishes_complete_index_with_entry_mapping() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = patchmoe_rule();
    let fake = Arc::new(FakeFetcher::new());
    push_patchmoe_full(&fake, PATCHMOE_INDEX, Some("\"v1\""));

    let mut progress_calls = 0;
    let mut ctx = context(&fake);
    let outcome = update_source(
        &store,
        &rule,
        true,
        &mut ctx,
        &AtomicBool::new(false),
        |_| progress_calls += 1,
    )
    .unwrap();

    assert_eq!(outcome.status.code, "source_refreshed");
    assert_eq!(outcome.revision, 1);
    assert!(outcome.complete);
    assert_eq!(outcome.status.entry_count, 6);
    assert_eq!(progress_calls, 1);
    assert_eq!(fake.call_count(), 3, "entry GET + two localized POSTs");

    let index = store.load_index("patchmoe").unwrap().unwrap();
    assert!(index.complete);
    assert_eq!(index.revision, 1);
    assert_eq!(index.schema_version, 1);
    assert_eq!(index.entries.len(), 6);
    assert_eq!(index.etag.as_deref(), Some("\"v1\""));
    assert!(
        store.load_index_progress("patchmoe").unwrap().is_none(),
        "a complete publication clears the progress file"
    );

    let kudo = index
        .entries
        .iter()
        .find(|entry| {
            entry.url.as_deref() == Some("https://download.patch.moe/Kudos_Kuppa_patch.zip")
        })
        .expect("Kudo's Kuppa is in the fixture");
    assert_eq!(kudo.title, "Kudo’s Kuppa");
    assert_eq!(kudo.kind, "entry");
    assert_eq!(kudo.page, "https://www.patch.moe/zh");
    assert_eq!(kudo.language.as_deref(), Some("zh-CN"));
    assert_eq!(kudo.date.as_deref(), Some("2026/6/23"));
    assert_eq!(kudo.appid, None);
}

#[test]
fn patchmoe_conditional_repeat_keeps_the_revision() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = patchmoe_rule();

    let first = Arc::new(FakeFetcher::new());
    push_patchmoe_full(&first, PATCHMOE_INDEX, Some("\"v1\""));
    run_update(&store, &rule, true, &first).unwrap();
    // Age the published snapshot so the `checked_at` refresh is observable
    // even when both attempts run inside the same wall-clock second.
    let mut published_index = store.load_index("patchmoe").unwrap().unwrap();
    published_index.checked_at = 100;
    store.save_index(&published_index).unwrap();
    let published = fs::read(store.paths().index("patchmoe")).unwrap();

    let second = Arc::new(FakeFetcher::new());
    second.push(response(304, "", ""));
    let outcome = run_update(&store, &rule, false, &second).unwrap();
    assert_eq!(outcome.status.code, "source_not_modified");
    assert_eq!(outcome.revision, 1, "a 304 does not bump the revision");
    assert!(outcome.complete);
    assert_eq!(outcome.status.entry_count, 6);
    assert_eq!(second.call_count(), 1);
    let calls = second.calls();
    assert!(calls[0]
        .headers
        .iter()
        .any(|(name, value)| name == "If-None-Match" && value == "\"v1\""));

    let index = store.load_index("patchmoe").unwrap().unwrap();
    assert_eq!(index.revision, 1);
    let before: serde_json::Value = serde_json::from_slice(&published).unwrap();
    let after: serde_json::Value =
        serde_json::from_slice(&fs::read(store.paths().index("patchmoe")).unwrap()).unwrap();
    assert_eq!(after["entries"], before["entries"], "304 keeps the entries");
    assert_ne!(after["checked_at"], before["checked_at"]);
}

#[test]
fn patchmoe_transport_failure_preserves_the_old_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = patchmoe_rule();

    let first = Arc::new(FakeFetcher::new());
    push_patchmoe_full(&first, PATCHMOE_INDEX, Some("\"v1\""));
    run_update(&store, &rule, true, &first).unwrap();
    let published = fs::read(store.paths().index("patchmoe")).unwrap();

    let failing = Arc::new(FakeFetcher::new());
    // The HTTP context retries one transport error, then reports failure.
    failing.push_error("discovery_network_error");
    failing.push_error("discovery_network_error");
    let outcome = run_update(&store, &rule, true, &failing).unwrap();

    assert_eq!(outcome.status.code, "source_offline_cached");
    assert_eq!(outcome.status.entry_count, 6);
    assert_eq!(outcome.revision, 1);
    assert!(outcome.complete);
    assert_eq!(
        fs::read(store.paths().index("patchmoe")).unwrap(),
        published,
        "the last successful snapshot must not be replaced"
    );
    let progress = store
        .load_index_progress("patchmoe")
        .unwrap()
        .expect("a failed attempt leaves the list page queued");
    assert!(!progress.complete);
    assert!(progress.entries.is_empty(), "nothing was collected");
    assert_eq!(progress.pending, vec!["https://www.patch.moe/".to_string()]);

    // The next successful attempt publishes and clears the queued progress.
    let recovered = Arc::new(FakeFetcher::new());
    push_patchmoe_full(&recovered, PATCHMOE_INDEX, Some("\"v2\""));
    let outcome = run_update(&store, &rule, false, &recovered).unwrap();
    assert_eq!(outcome.status.code, "source_refreshed");
    assert!(store.load_index_progress("patchmoe").unwrap().is_none());
}

#[test]
fn patchmoe_cancel_before_start_makes_no_request_and_saves_progress() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = patchmoe_rule();
    let fake = Arc::new(FakeFetcher::new());
    let mut ctx = context(&fake);
    let cancel = AtomicBool::new(true);
    let error = update_source(&store, &rule, true, &mut ctx, &cancel, |_| {}).unwrap_err();
    assert_eq!(error.to_string(), "task_cancelled");
    assert_eq!(
        fake.call_count(),
        0,
        "cancellation must not start a request"
    );

    let progress = store.load_index_progress("patchmoe").unwrap().unwrap();
    assert!(!progress.complete);
    assert_eq!(progress.pending, vec!["https://www.patch.moe/".to_string()]);
    assert!(store.load_index("patchmoe").unwrap().is_none());
}

#[test]
fn patchmoe_revision_only_tracks_content_changes() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = patchmoe_rule();

    let first = Arc::new(FakeFetcher::new());
    push_patchmoe_full(&first, PATCHMOE_INDEX, Some("\"v1\""));
    assert_eq!(run_update(&store, &rule, true, &first).unwrap().revision, 1);

    // Same entries, new validator: a full re-collection is still revision 1.
    let second = Arc::new(FakeFetcher::new());
    push_patchmoe_full(&second, PATCHMOE_INDEX, Some("\"v2\""));
    assert_eq!(
        run_update(&store, &rule, true, &second).unwrap().revision,
        1
    );

    // One genuinely new row bumps the revision exactly once.
    let grown = format!(
        "{PATCHMOE_INDEX}<table><tr><td class=\"title\">Brand New Game</td>\
         <td class=\"release\">2024/3/4</td>\
         <td class=\"patch\"><a href=\"https://download.patch.moe/Brand_New_Game_patch.zip\">dl</a>\
         </td></tr></table>"
    );
    let third = Arc::new(FakeFetcher::new());
    push_patchmoe_full(&third, &grown, Some("\"v3\""));
    let outcome = run_update(&store, &rule, true, &third).unwrap();
    assert_eq!(outcome.revision, 2);
    assert_eq!(outcome.status.entry_count, 7);
}

// ---- 2. generic table ----------------------------------------------------

#[test]
fn table_collects_every_page_and_maps_columns() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = table_rule();

    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(
        200,
        &table_html(&[table_row("Game A", "a", "42")]),
        "https://example.com/list1",
    ));
    fake.push(response(
        200,
        &table_html(&[table_row("Game B", "b", "43")]),
        "https://example.com/list2",
    ));

    let outcome = run_update(&store, &rule, true, &fake).unwrap();
    assert_eq!(outcome.status.code, "source_refreshed");
    assert_eq!(outcome.status.entry_count, 2);
    assert_eq!(fake.call_count(), 2);

    let index = store.load_index("table-source").unwrap().unwrap();
    assert!(index.complete);
    assert_eq!(index.entries.len(), 2);
    let game_a = index
        .entries
        .iter()
        .find(|entry| entry.title == "Game A")
        .unwrap();
    assert_eq!(game_a.kind, "entry");
    assert_eq!(game_a.page, "https://example.com/list1");
    assert_eq!(
        game_a.url.as_deref(),
        Some("https://download.example.com/a.zip")
    );
    assert_eq!(game_a.language.as_deref(), Some("en"));
    assert_eq!(game_a.date.as_deref(), Some("2024/1/2"));
    assert_eq!(game_a.appid, Some(42));
    assert!(index.entries.iter().any(|entry| entry.title == "Game B"
        && entry.page == "https://example.com/list2"
        && entry.appid == Some(43)));
}

#[test]
fn table_validation_failure_returns_an_error_and_keeps_the_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = table_rule();

    let good = Arc::new(FakeFetcher::new());
    good.push(response(
        200,
        &table_html(&[table_row("Game A", "a", "42")]),
        "https://example.com/list1",
    ));
    good.push(response(
        200,
        &table_html(&[table_row("Game B", "b", "43")]),
        "https://example.com/list2",
    ));
    run_update(&store, &rule, true, &good).unwrap();
    let published = fs::read(store.paths().index("table-source")).unwrap();

    let bad = Arc::new(FakeFetcher::new());
    bad.push(response(
        200,
        &table_html(&[table_row("Game A", "a", "not-an-appid")]),
        "https://example.com/list1",
    ));
    let error = run_update(&store, &rule, true, &bad).unwrap_err();
    assert_eq!(error.to_string(), "invalid_source_appid");

    assert_eq!(
        fs::read(store.paths().index("table-source")).unwrap(),
        published,
        "a hard validation failure must not touch the published index"
    );
    assert!(store.load_index_progress("table-source").unwrap().is_none());
}

// ---- 3. Kagura CN list index --------------------------------------------

const CN_PAGE_1: &str = r#"<html><body>
<a href="/category/patch/">Patch Category</a>
<article><h2 class="entry-title"><a href="/article/1/">Article One</a></h2></article>
<a rel="next" href="/page/2/">Next</a>
</body></html>"#;

const CN_PAGE_2: &str = r#"<html><body>
<article><h2 class="entry-title"><a href="/article/2/">Article Two</a></h2></article>
</body></html>"#;

fn assert_no_detail_or_category_requests(fake: &FakeFetcher) {
    assert!(
        !fake
            .calls()
            .iter()
            .any(|call| call.url.contains("/article/") || call.url.contains("/category/")),
        "the index update must not fetch detail/category pages"
    );
}

#[test]
fn kagura_cn_update_crawls_only_list_pages() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_cn_rule();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(200, CN_PAGE_1, "https://kagurafan.com/"));
    fake.push(response(200, CN_PAGE_2, "https://kagurafan.com/page/2/"));

    let outcome = run_update(&store, &rule, true, &fake).unwrap();
    assert_eq!(outcome.status.code, "source_refreshed");
    assert!(outcome.complete);
    assert_eq!(outcome.status.entry_count, 3);
    assert_eq!(fake.call_count(), 2, "both list pages, in order");
    let urls: Vec<String> = fake.calls().into_iter().map(|call| call.url).collect();
    assert_eq!(urls[0], "https://kagurafan.com/");
    assert_eq!(urls[1], "https://kagurafan.com/page/2/");
    assert_no_detail_or_category_requests(&fake);

    let index = store.load_index("kagura-cn").unwrap().unwrap();
    assert!(index.complete);
    assert_eq!(index.entries.len(), 3);
    assert!(index.entries.iter().any(|entry| {
        entry.title == "Patch Category"
            && entry.kind == "category"
            && entry.page == "https://kagurafan.com/category/patch/"
    }));
    assert!(index.entries.iter().any(|entry| {
        entry.title == "Article One"
            && entry.kind == "article"
            && entry.page == "https://kagurafan.com/article/1/"
    }));
    assert!(index.entries.iter().any(|entry| {
        entry.title == "Article Two"
            && entry.kind == "article"
            && entry.page == "https://kagurafan.com/article/2/"
    }));
}

#[test]
fn kagura_cn_page_failure_keeps_the_official_snapshot_and_saves_pending() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_cn_rule();

    let first = Arc::new(FakeFetcher::new());
    first.push(response(200, CN_PAGE_1, "https://kagurafan.com/"));
    first.push(response(200, CN_PAGE_2, "https://kagurafan.com/page/2/"));
    run_update(&store, &rule, true, &first).unwrap();
    let published = fs::read(store.paths().index("kagura-cn")).unwrap();

    let failing = Arc::new(FakeFetcher::new());
    failing.push(response(200, CN_PAGE_1, "https://kagurafan.com/"));
    // The HTTP context retries one transport error, then reports failure.
    failing.push_error("discovery_network_error");
    failing.push_error("discovery_network_error");
    let outcome = run_update(&store, &rule, true, &failing).unwrap();

    assert_eq!(outcome.status.code, "source_partial");
    assert_eq!(outcome.status.entry_count, 3, "old snapshot entry count");
    assert_eq!(outcome.revision, 1, "the official revision is unchanged");
    assert!(
        outcome.complete,
        "the official snapshot is still the last complete one"
    );
    assert_eq!(
        fs::read(store.paths().index("kagura-cn")).unwrap(),
        published,
        "a partial pass must not replace the official snapshot"
    );

    let progress = store
        .load_index_progress("kagura-cn")
        .unwrap()
        .expect("partial pass saves progress");
    assert!(!progress.complete);
    assert_eq!(progress.pending, vec!["https://kagurafan.com/page/2/"]);
    assert_eq!(progress.entries.len(), 2, "page 1 entries are kept");
    assert_no_detail_or_category_requests(&failing);
}

#[test]
fn kagura_cn_cancel_saves_progress_and_resume_publishes() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_cn_rule();

    // First-ever update fails on page 2: no official snapshot exists yet, so
    // the outcome must not claim coverage and only progress is saved.
    let failing = Arc::new(FakeFetcher::new());
    failing.push(response(200, CN_PAGE_1, "https://kagurafan.com/"));
    failing.push_error("discovery_network_error");
    failing.push_error("discovery_network_error");
    let partial = run_update(&store, &rule, true, &failing).unwrap();
    assert_eq!(partial.status.code, "source_partial");
    assert_eq!(partial.revision, 0, "no official snapshot yet");
    assert!(
        !partial.complete,
        "a partial first pass never claims coverage"
    );
    assert_eq!(partial.status.entry_count, 0);
    assert!(partial.status.cached_at.is_none());
    assert!(store.load_index("kagura-cn").unwrap().is_none());

    let mut progress = store.load_index_progress("kagura-cn").unwrap().unwrap();
    assert!(!progress.complete);
    assert_eq!(progress.entries.len(), 2);
    assert_eq!(progress.pending, vec!["https://kagurafan.com/page/2/"]);

    // Cancelling before the queued page is fetched returns task_cancelled but
    // keeps the already collected entries and the remaining page queue.
    let cancelled = Arc::new(FakeFetcher::new());
    let mut ctx = context(&cancelled);
    let cancel = AtomicBool::new(true);
    let error = update_source(&store, &rule, true, &mut ctx, &cancel, |_| {}).unwrap_err();
    assert_eq!(error.to_string(), "task_cancelled");
    assert_eq!(cancelled.call_count(), 0);
    progress = store.load_index_progress("kagura-cn").unwrap().unwrap();
    assert_eq!(progress.entries.len(), 2);
    assert_eq!(progress.pending, vec!["https://kagurafan.com/page/2/"]);
    assert!(store.load_index("kagura-cn").unwrap().is_none());

    // The next update resumes from the queued page and publishes the complete
    // snapshot without re-fetching the already collected page 1.
    let resumed = Arc::new(FakeFetcher::new());
    resumed.push(response(200, CN_PAGE_2, "https://kagurafan.com/page/2/"));
    let outcome = run_update(&store, &rule, true, &resumed).unwrap();
    assert_eq!(outcome.status.code, "source_refreshed");
    assert!(outcome.complete);
    assert_eq!(outcome.revision, 1, "first published revision");
    assert_eq!(outcome.status.entry_count, 3);
    assert_eq!(
        resumed.calls()[0].url,
        "https://kagurafan.com/page/2/",
        "resume starts from pending, not the entry page"
    );
    assert_eq!(resumed.call_count(), 1);

    let index = store.load_index("kagura-cn").unwrap().unwrap();
    assert!(index.complete);
    assert_eq!(index.entries.len(), 3);
    assert!(store.load_index_progress("kagura-cn").unwrap().is_none());
}

// ---- 4. Kagura EN product cards -----------------------------------------

const EN_PAGE: &str = r#"<html><body>
<li class="product type-product">
  <h2 class="woocommerce-loop-product__title">Game A</h2>
  <a href="/product/game-a/">Game A</a>
</li>
<li class="product type-product">
  <h2 class="woocommerce-loop-product__title">Game B</h2>
  <a href="/product/game-b/">Game B</a>
</li>
<div class="widget">
  <li class="product type-product">
    <h2 class="woocommerce-loop-product__title">Widget Patch</h2>
    <a href="/product/widget/">Widget Patch</a>
  </li>
</div>
<div class="related">
  <li class="product type-product">
    <h2 class="woocommerce-loop-product__title">Related Patch</h2>
    <a href="/product/related/">Related Patch</a>
  </li>
</div>
</body></html>"#;

#[test]
fn kagura_en_index_contains_product_cards_only() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_en_rule();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(200, EN_PAGE, "https://www.kaguragames.com/patch/"));

    let outcome = run_update(&store, &rule, true, &fake).unwrap();
    assert_eq!(outcome.status.code, "source_refreshed");
    assert_eq!(
        fake.call_count(),
        1,
        "a single entry keeps the old behavior"
    );
    assert_no_detail_or_category_requests(&fake);

    let index = store.load_index("kagura-en").unwrap().unwrap();
    assert!(index.complete);
    assert_eq!(index.entries.len(), 2, "widget/related cards are excluded");
    assert!(index.entries.iter().any(|entry| {
        entry.title == "Game A"
            && entry.kind == "product"
            && entry.page == "https://www.kaguragames.com/product/game-a/"
    }));
    assert!(index.entries.iter().all(|entry| entry.kind == "product"));
}

/// Second configured entry root (the WooCommerce product-category view), with
/// its own pagination.
const EN_ENTRY_2: &str = r#"<html><body>
<li class="product type-product">
  <h2 class="woocommerce-loop-product__title">Game C</h2>
  <a href="/product/game-c/">Game C</a>
</li>
<a rel="next" href="/product-category/patches/patch/page/2/">Next</a>
</body></html>"#;

const EN_ENTRY_2_PAGE_2: &str = r#"<html><body>
<li class="product type-product">
  <h2 class="woocommerce-loop-product__title">Game D</h2>
  <a href="/product/game-d/">Game D</a>
</li>
</body></html>"#;

fn kagura_en_multi_entry_rule() -> SourceRule {
    SourceRule {
        pages: vec![
            "https://www.kaguragames.com/patch/".to_string(),
            "https://www.kaguragames.com/product-category/patches/patch/".to_string(),
        ],
        ..kagura_en_rule()
    }
}

#[test]
fn kagura_en_crawls_every_configured_entry_root() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_en_multi_entry_rule();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(200, EN_PAGE, "https://www.kaguragames.com/patch/"));
    fake.push(response(
        200,
        EN_ENTRY_2,
        "https://www.kaguragames.com/product-category/patches/patch/",
    ));
    fake.push(response(
        200,
        EN_ENTRY_2_PAGE_2,
        "https://www.kaguragames.com/product-category/patches/patch/page/2/",
    ));

    let outcome = run_update(&store, &rule, true, &fake).unwrap();
    assert_eq!(outcome.status.code, "source_refreshed");
    assert!(outcome.complete);
    assert_eq!(outcome.status.entry_count, 4);

    let urls: Vec<String> = fake.calls().into_iter().map(|call| call.url).collect();
    assert_eq!(
        urls,
        vec![
            "https://www.kaguragames.com/patch/",
            "https://www.kaguragames.com/product-category/patches/patch/",
            "https://www.kaguragames.com/product-category/patches/patch/page/2/",
        ],
        "both configured entry roots and the second root's pagination are fetched"
    );
    assert!(
        !urls.iter().any(|url| url.contains("/product/")),
        "only list pages are fetched, never product details"
    );

    let index = store.load_index("kagura-en").unwrap().unwrap();
    assert!(index.complete);
    assert_eq!(index.entries.len(), 4);
    let titles: Vec<&str> = index
        .entries
        .iter()
        .map(|entry| entry.title.as_str())
        .collect();
    for title in ["Game A", "Game B", "Game C", "Game D"] {
        assert!(titles.contains(&title), "missing {title}: {titles:?}");
    }
    assert!(index.entries.iter().all(|entry| entry.kind == "product"));
}

// ---- 5. resolve_page facts (P3 interface) --------------------------------

const ARTICLE: &str = r#"<html><body><article>
<h1 class="entry-title">Game</h1>
<time class="published" datetime="2024-01-02">Jan 2</time>
<div class="entry-content">
<p>中文 补丁 v1.2.3 ZIP</p>
<a href="https://kaguraserver.com/game_v1.2.3.zip">下载 Mirror 1</a>
<a href="https://store.steampowered.com/app/42/Game/">Steam</a>
<a href="/related/">Related</a>
</div>
</article></body></html>"#;

const CATEGORY: &str = r#"<html><body>
<h1>Patch Category</h1>
<article><h2 class="entry-title"><a href="/article/1/">Article One</a></h2>
<div class="entry-content"><p>中文 v1.0</p>
<a href="https://kaguraserver.com/one.zip">下载</a></div></article>
<article><h2 class="entry-title"><a href="/article/2/">Article Two</a></h2></article>
<a rel="next" href="/category/patch/page/2/">Next</a>
</body></html>"#;

const PRODUCT: &str = r#"<html><body><div class="product">
<h1 class="entry-title">Game A</h1>
<div id="tab-patch">
<p>English Patch v1.0</p>
<a href="https://kaguragamespatch.com/game-a.zip">Download</a>
<a href="https://store.steampowered.com/app/77/Game_A/">Steam</a>
</div>
</div></body></html>"#;

#[test]
fn resolve_page_parses_article_facts() {
    let rule = kagura_cn_rule();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(200, ARTICLE, "https://kagurafan.com/game/"));
    let mut ctx = context(&fake);
    let fact = resolve_page(
        &rule,
        "kagura_cn_html",
        "https://kagurafan.com/game/?utm=x",
        &mut ctx,
    )
    .unwrap();

    assert_eq!(fact.kind, "article");
    assert_eq!(fact.title, "Game");
    assert_eq!(fact.page, "https://kagurafan.com/game/");
    assert_eq!(fact.steam_ids, vec![42]);
    assert_eq!(fact.mirrors.len(), 1);
    assert_eq!(
        fact.mirrors[0].url,
        "https://kaguraserver.com/game_v1.2.3.zip"
    );
    assert!(fact.text.contains("v1.2.3"));
    assert_eq!(fact.published_at.as_deref(), Some("2024-01-02"));
    assert!(fact
        .body_links
        .contains(&"https://kagurafan.com/related/".to_string()));
    assert!(fact.article_links.is_empty());
    assert!(fact.article_facts.is_empty());
    assert_eq!(fake.call_count(), 1);
}

#[test]
fn resolve_page_parses_category_with_inline_articles() {
    let rule = kagura_cn_rule();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(
        200,
        CATEGORY,
        "https://kagurafan.com/category/patch/",
    ));
    let mut ctx = context(&fake);
    let fact = resolve_page(
        &rule,
        "kagura_cn_html",
        "https://kagurafan.com/category/patch/",
        &mut ctx,
    )
    .unwrap();

    assert_eq!(fact.kind, "category");
    assert_eq!(fact.title, "Patch Category");
    assert_eq!(fact.page, "https://kagurafan.com/category/patch/");
    assert_eq!(fact.article_links.len(), 2);
    assert_eq!(fact.article_links[0].title, "Article One");
    assert_eq!(
        fact.article_links[0].page,
        "https://kagurafan.com/article/1/"
    );
    assert_eq!(
        fact.next_page.as_deref(),
        Some("https://kagurafan.com/category/patch/page/2/")
    );
    // Only the article rendered in full becomes an inline fact.
    assert_eq!(fact.article_facts.len(), 1);
    let inline = &fact.article_facts[0];
    assert_eq!(inline.kind, "article");
    assert_eq!(inline.title, "Article One");
    assert_eq!(inline.page, "https://kagurafan.com/article/1/");
    assert_eq!(inline.mirrors.len(), 1);
    assert!(inline.text.contains("v1.0"));
    assert_eq!(fake.call_count(), 1);
}

#[test]
fn resolve_page_parses_en_product_facts_and_reuses_the_page() {
    let rule = kagura_en_rule();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(
        200,
        PRODUCT,
        "https://www.kaguragames.com/product/game-a/",
    ));
    let mut ctx = context(&fake);
    let page = "https://www.kaguragames.com/product/game-a/";
    let fact = resolve_page(&rule, "kagura_en_html", page, &mut ctx).unwrap();

    assert_eq!(fact.kind, "product");
    assert_eq!(fact.title, "Game A");
    assert_eq!(fact.page, page);
    assert_eq!(fact.steam_ids, vec![77]);
    assert_eq!(fact.mirrors.len(), 1);
    assert_eq!(
        fact.mirrors[0].url,
        "https://kaguragamespatch.com/game-a.zip"
    );
    assert!(fact.text.contains("English Patch"));
    assert_eq!(fake.call_count(), 1);

    // Query variants of the same canonical page share the task's response cache.
    let again = resolve_page(
        &rule,
        "kagura_en_html",
        "https://www.kaguragames.com/product/game-a/?utm=1",
        &mut ctx,
    )
    .unwrap();
    assert_eq!(again, fact);
    assert_eq!(fake.call_count(), 1, "same canonical page -> one request");
}

#[test]
fn resolve_page_rejects_pages_outside_the_rule_hosts() {
    let rule = kagura_cn_rule();
    let fake = Arc::new(FakeFetcher::new());
    let mut ctx = context(&fake);
    let error =
        resolve_page(&rule, "kagura_cn_html", "https://evil.example/x/", &mut ctx).unwrap_err();
    assert_eq!(error.to_string(), "invalid_discovery_url");
    assert_eq!(fake.call_count(), 0);
}

// ---- 6. source-scoped update / URL dedupe --------------------------------

#[test]
fn update_source_is_source_scoped_and_dedupes_urls_within_one_task() {
    // The call takes only source-level inputs (store, rule, force, one
    // RequestContext, cancel, progress); no installed-game list participates,
    // so the request count cannot grow with the game count.
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let rule = kagura_cn_rule();

    // A wrapper list page refers to the same category twice and loops its
    // `next` link back to itself: the repeated canonical URL is fetched/stored
    // once and does not make the index incomplete.
    const WRAPPER: &str = r#"<html><body>
<a href="/category/patch/">Patch</a>
<a href="/category/patch/?ref=again">Patch Again</a>
<article><h2 class="entry-title"><a href="/article/1/">Article One</a></h2></article>
<a rel="next" href="/">Next</a>
</body></html>"#;
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(200, WRAPPER, "https://kagurafan.com/"));
    let outcome = run_update(&store, &rule, true, &fake).unwrap();

    assert_eq!(outcome.status.code, "source_refreshed");
    assert_eq!(
        fake.call_count(),
        1,
        "the self-referenced page is fetched once"
    );
    assert_no_detail_or_category_requests(&fake);
    let index = store.load_index("kagura-cn").unwrap().unwrap();
    assert!(index.complete);
    assert_eq!(index.entries.len(), 2, "one category + one article");
    let categories: Vec<_> = index
        .entries
        .iter()
        .filter(|entry| entry.kind == "category")
        .collect();
    assert_eq!(categories.len(), 1);
    assert_eq!(categories[0].title, "Patch", "first occurrence wins");
}

#[test]
fn fallback_indexer_is_not_globally_indexable() {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::open(temp.path()).unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let error = run_update(&store, &hgamer_rule(), true, &fake).unwrap_err();
    assert_eq!(error.to_string(), "source_not_indexable");
    assert_eq!(fake.call_count(), 0);
    assert!(store.load_index("hgamer").unwrap().is_none());
}
