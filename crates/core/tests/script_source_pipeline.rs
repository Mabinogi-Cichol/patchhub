//! P3 real update/matching/read/selection paths, using deterministic HTML transport.
use patchhub_core::{
    catalog::{Catalog, Platform},
    install::TransactionSnapshot,
    installer::{exe_resolve, manual_selection, resolve},
    projection::{self, CatalogView, HostCapabilities, ProjectionInputs},
    sources::{
        self,
        http::{FakeFetcher, FetchResponse, RequestContext},
        matching,
        model::*,
        snapshot::SourceSnapshot,
        update, Sources,
    },
    steam,
};
use serde_json::json;
use std::{
    fs,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

const CODE: &str = r#"
local M = {}
function M.collect_index(ctx, rule, previous)
  local entries = {}
  for _, node in ipairs(ctx.html(ctx.get(rule.pages[1]).text):select('article')) do
    entries[#entries+1] = {title=node:text(), page=ctx.url_join(rule.pages[1], node:attr('data-page')), kind='product'}
  end
  return {entries=entries, complete=true}
end
function M.resolve_page(ctx, rule, page)
  local doc = ctx.html(ctx.get(page).text)
  local mirrors = {}
  for _, link in ipairs(doc:select('a')) do
    mirrors[#mirrors+1] = {url=link:attr('href'), format='unknown'}
  end
  return {title=doc:select('h1')[1]:text(), page=page, kind='product', steam_ids={42,43}, mirrors=mirrors, text='English patch'}
end
return M
"#;
const LIST: &str = "<article data-page='/game'>Test Game</article>";
const DETAIL: &str = "<h1>Test Game</h1><a href='https://download.example.com/a.zip'>ZIP</a><a href='https://download.example.com/a.exe'>EXE</a>";
fn raw() -> RawSourceRule {
    serde_json::from_value(
        json!({"id":"custom", "name":"Custom", "adapter":"lua", "enabled":true,
        "pages":["https://example.com/list"], "page_hosts":["example.com"],
        "download_hosts":["download.example.com"], "root_layout":"game_root"}),
    )
    .unwrap()
}
fn open(state: &std::path::Path) -> Sources {
    Sources::open_with_rules(state, r#"{"schema_version":2,"sources":[]}"#).unwrap()
}
fn rule(s: &Sources) -> SourceRule {
    s.registry()
        .rules()
        .iter()
        .find(|r| r.id == "custom")
        .unwrap()
        .clone()
}
fn game(appid: u32) -> GameQuery {
    GameQuery {
        appid,
        name: "Test Game".into(),
        localized_name: None,
        instance_id: Some(format!("test-{appid}")),
    }
}
fn flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}
fn transport(bodies: &[&str]) -> Arc<FakeFetcher> {
    let fake = Arc::new(FakeFetcher::new());
    for body in bodies {
        fake.push(FetchResponse {
            status: 200,
            body: body.as_bytes().to_vec(),
            ..Default::default()
        });
    }
    fake
}
fn ctx(fake: &Arc<FakeFetcher>) -> RequestContext {
    RequestContext::with_settings(Box::new(Arc::clone(fake)), Duration::ZERO)
}
fn update(s: &Sources, fake: &Arc<FakeFetcher>) -> sources::Result<update::SourceUpdateOutcome> {
    update::update_source(s.store(), &rule(s), false, &mut ctx(fake), &flag(), |_| {})
}
fn matches(
    s: &Sources,
    fake: &Arc<FakeFetcher>,
    games: &[GameQuery],
) -> sources::Result<Vec<GameMatch>> {
    matching::match_games(
        s.store(),
        s.registry().rules(),
        s.registry().overrides(),
        games,
        &[],
        false,
        &mut ctx(fake),
        &flag(),
        500,
        |_| {},
        |_| {},
    )
}
fn seeded() -> (tempfile::TempDir, Sources, Vec<SourceCandidate>) {
    let temp = tempfile::tempdir().unwrap();
    let mut s = open(temp.path());
    s.import_script_source(&raw(), 1, CODE, false).unwrap();
    update(&s, &transport(&[LIST])).unwrap();
    let candidate = matches(&s, &transport(&[DETAIL]), &[game(42)]).unwrap()[0]
        .candidates
        .clone();
    (temp, s, candidate)
}
fn zip(candidates: &[SourceCandidate]) -> &SourceCandidate {
    candidates
        .iter()
        .find(|c| resolve::zip_selection_id(c).is_some())
        .unwrap()
}
fn exe(candidates: &[SourceCandidate]) -> &SourceCandidate {
    candidates
        .iter()
        .find(|c| exe_resolve::exe_selection_id(c).is_some())
        .unwrap()
}
fn assert_retired(s: &Sources, candidates: &[SourceCandidate]) {
    let registry = s.registry();
    assert!(registry.matches(&game(42), true).is_empty());
    assert!(registry.cached_candidates(&game(42)).is_empty());
    assert!(registry.cached_discovery(&game(42)).candidates.is_empty());
    assert!(registry.dynamic_zip_candidates(&game(42)).is_empty());
    assert!(resolve::resolve_zip(
        registry,
        &game(42),
        &resolve::zip_selection_id(zip(candidates)).unwrap()
    )
    .is_err());
    assert!(exe_resolve::resolve_exe(
        registry,
        &game(42),
        &exe_resolve::exe_selection_id(exe(candidates)).unwrap()
    )
    .is_err());
    for candidate in candidates {
        for mirror in &candidate.mirrors {
            let id = manual_selection::selection_id(candidate, &game(42), mirror).unwrap();
            assert!(manual_selection::resolve(registry, &game(42), &id, &mirror.format).is_err());
        }
    }
    assert!(matching::game_match_summary(s.store(), 42)
        .candidates
        .is_empty());
}

#[test]
fn non_table_update_matching_shared_details_and_all_selection_paths() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = open(temp.path());
    s.import_script_source(&raw(), 1, CODE, false).unwrap();
    let list = transport(&[LIST]);
    let out = update(&s, &list).unwrap();
    assert_eq!(out.revision, 1);
    assert_eq!(list.call_count(), 1);
    assert_eq!(list.calls()[0].url, "https://example.com/list");
    assert!(s
        .store()
        .load_detail("custom", "https://example.com/game")
        .unwrap()
        .is_none());
    let detail = transport(&[DETAIL]);
    let results = matches(&s, &detail, &[game(42), game(43)]).unwrap();
    assert_eq!(
        detail.call_count(),
        1,
        "detail cache is shared across games"
    );
    assert_eq!(detail.calls()[0].url, "https://example.com/game");
    assert!(results
        .iter()
        .all(|r| r.status == GameMatchStatus::PublisherMatch));
    let empty = transport(&[]);
    assert_eq!(matches(&s, &empty, &[game(42), game(43)]).unwrap(), results);
    assert_eq!(empty.call_count(), 0);
    let candidates = &results[0].candidates;
    let candidate = zip(candidates);
    assert!(candidate.parser_fingerprint.is_some());
    assert!(resolve::resolve_zip(
        s.registry(),
        &game(42),
        &resolve::zip_selection_id(candidate).unwrap()
    )
    .is_ok());
    assert!(exe_resolve::resolve_exe(
        s.registry(),
        &game(42),
        &exe_resolve::exe_selection_id(exe(candidates)).unwrap()
    )
    .is_ok());
    for mirror in &candidate.mirrors {
        let id = manual_selection::selection_id(candidate, &game(42), mirror).unwrap();
        assert!(manual_selection::resolve(s.registry(), &game(42), &id, &mirror.format).is_ok());
    }
}

#[test]
fn script_replacement_invalidates_before_update_after_update_restart_and_same_facts() {
    let (temp, mut s, a) = seeded();
    let old = s.store().load_index("custom").unwrap().unwrap();
    s.registry_mut().set_name("custom", "Display name").unwrap();
    assert_eq!(
        s.registry().cached_candidates(&game(42)),
        a,
        "display name is not parser identity"
    );
    s.import_script_source(&raw(), 1, &format!("{CODE}\n-- parser B"), true)
        .unwrap();
    assert_retired(&s, &a);
    let summary = s.registry().list_sources().remove(0);
    assert_eq!(summary.revision, 0);
    assert_eq!(summary.entry_count, 0);
    assert_eq!(summary.status, "not_refreshed");
    assert_eq!(
        s.store().load_index("custom").unwrap().unwrap(),
        old,
        "replacement preserves old file"
    );
    drop(s);
    let s = open(temp.path());
    assert_retired(&s, &a);
    let out = update(&s, &transport(&[LIST])).unwrap();
    assert_eq!(out.revision, old.revision + 1);
    assert_eq!(
        s.store().load_index("custom").unwrap().unwrap().entries,
        old.entries
    );
    assert_retired(&s, &a);
    let fresh = transport(&[DETAIL]);
    let b = matches(&s, &fresh, &[game(42)]).unwrap()[0]
        .candidates
        .clone();
    assert_eq!(
        fresh.call_count(),
        1,
        "old detail must be re-parsed even for equal list entries"
    );
    assert_ne!(zip(&a).candidate_id, zip(&b).candidate_id);
    assert_ne!(zip(&a).fingerprint, zip(&b).fingerprint);
    assert_ne!(
        resolve::zip_selection_id(zip(&a)),
        resolve::zip_selection_id(zip(&b))
    );
    assert_ne!(
        exe_resolve::exe_selection_id(exe(&a)),
        exe_resolve::exe_selection_id(exe(&b))
    );
    assert!(resolve::resolve_zip(
        s.registry(),
        &game(42),
        &resolve::zip_selection_id(zip(&a)).unwrap()
    )
    .is_err());
    assert!(exe_resolve::resolve_exe(
        s.registry(),
        &game(42),
        &exe_resolve::exe_selection_id(exe(&a)).unwrap()
    )
    .is_err());
    for a in &a {
        for mirror in &a.mirrors {
            assert!(manual_selection::resolve(
                s.registry(),
                &game(42),
                &manual_selection::selection_id(a, &game(42), mirror).unwrap(),
                &mirror.format
            )
            .is_err());
        }
    }
    assert!(resolve::resolve_zip(
        s.registry(),
        &game(42),
        &resolve::zip_selection_id(zip(&b)).unwrap()
    )
    .is_ok());
}

#[test]
fn readonly_projection_and_preview_reject_retired_selection_without_running_lua() {
    let (temp, mut s, a) = seeded();
    let games_dir = tempfile::tempdir().unwrap();
    let library = games_dir.path().join("Steam");
    fs::create_dir_all(library.join("steamapps/common/Test")).unwrap();
    fs::write(library.join("steamapps/appmanifest_42.acf"),r#""AppState" { "appid" "42" "name" "Test Game" "installdir" "Test" "buildid" "100" "StateFlags" "4" }"#).unwrap();
    let steam = steam::scan(&[library]);
    let mut query = game(42);
    query.instance_id = Some(steam.installs[0].instance_id.clone());
    {
        let snapshot = SourceSnapshot::load(temp.path(), std::slice::from_ref(&query), 500)
            .unwrap()
            .unwrap();
        let catalog = Catalog {
            schema_version: 1,
            revision: 1,
            releases: vec![],
        };
        let tx = TransactionSnapshot::default();
        let caps = HostCapabilities::default();
        let cancel = AtomicBool::new(false);
        let inputs = ProjectionInputs {
            steam: &steam,
            catalog: CatalogView::Development(&catalog),
            sources: Some(&snapshot),
            transactions: &tx,
            roots: &[],
            now: 500,
            platform: Platform::Windows,
            capabilities: &caps,
            cancel: &cancel,
        };
        assert!(
            !projection::project_instance(&inputs, query.instance_id.as_ref().unwrap())
                .unwrap()
                .patches
                .is_empty()
        );
        let report = projection::preview(
            &inputs,
            projection::PreviewRequest {
                instance_id: query.instance_id.clone().unwrap(),
                target: projection::PreviewTarget::PublisherZipSelection(
                    resolve::zip_selection_id(zip(&a)).unwrap(),
                ),
            },
        )
        .unwrap();
        assert!(report.blockers.is_empty());
        let report = projection::preview(
            &inputs,
            projection::PreviewRequest {
                instance_id: query.instance_id.clone().unwrap(),
                target: projection::PreviewTarget::PublisherExeSelection(
                    exe_resolve::exe_selection_id(exe(&a)).unwrap(),
                ),
            },
        )
        .unwrap();
        assert_eq!(report.blockers, vec!["exe_check_required"]);
    }
    // Any accidental VM execution in these read paths would never return.
    s.import_script_source(&raw(), 1, "while true do end", true)
        .unwrap();
    let paths = s.store().paths();
    let before = fs::read(paths.settings()).unwrap();
    let snapshot = SourceSnapshot::load(temp.path(), &[query.clone()], 500)
        .unwrap()
        .unwrap();
    let catalog = Catalog {
        schema_version: 1,
        revision: 1,
        releases: vec![],
    };
    let tx = TransactionSnapshot::default();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let inputs = ProjectionInputs {
        steam: &steam,
        catalog: CatalogView::Development(&catalog),
        sources: Some(&snapshot),
        transactions: &tx,
        roots: &[],
        now: 500,
        platform: Platform::Windows,
        capabilities: &caps,
        cancel: &cancel,
    };
    let row = projection::project_instance(&inputs, query.instance_id.as_ref().unwrap()).unwrap();
    assert!(row.patches.is_empty());
    assert!(row.matches.unwrap().is_empty());
    assert!(row.discovery.unwrap().candidates.is_empty());
    for target in [
        projection::PreviewTarget::PublisherZipSelection(
            resolve::zip_selection_id(zip(&a)).unwrap(),
        ),
        projection::PreviewTarget::PublisherExeSelection(
            exe_resolve::exe_selection_id(exe(&a)).unwrap(),
        ),
    ] {
        let report = projection::preview(
            &inputs,
            projection::PreviewRequest {
                instance_id: query.instance_id.clone().unwrap(),
                target,
            },
        );
        match report {
            Ok(report) => {
                assert!(!report.executable);
                assert!(report.blockers.contains(&"selection_not_found".into()));
            }
            Err(error) => assert_eq!(error.code, "selection_not_found"),
        }
    }
    assert_eq!(before, fs::read(paths.settings()).unwrap());
}

#[test]
fn partial_and_failed_script_attempts_preserve_official_index_and_old_parser_is_not_resumed() {
    let (_temp, mut s, _) = seeded();
    let old = s.store().load_index("custom").unwrap().unwrap();
    let partial = r#"return {collect_index=function(ctx,rule,previous) assert(previous==ctx.null); return {entries={{title='Partial',kind='product',page='https://example.com/p'}},pending={'https://example.com/next'},complete=false} end}"#;
    s.import_script_source(&raw(), 1, partial, true).unwrap();
    let out = update(&s, &transport(&[])).unwrap();
    assert_eq!(out.status.code, "source_partial");
    let progress = s.store().load_index_progress("custom").unwrap().unwrap();
    assert!(!progress.complete);
    assert_ne!(progress.parser_fingerprint, old.parser_fingerprint);
    assert_eq!(s.store().load_index("custom").unwrap().unwrap(), old);
    s.import_script_source(
        &raw(),
        1,
        "return {collect_index=function() error('broken list') end}",
        true,
    )
    .unwrap();
    let err = update(&s, &transport(&[])).unwrap_err();
    assert!(err.to_string().contains("script_source_runtime_error"));
    assert_eq!(s.store().load_index("custom").unwrap().unwrap(), old);
    assert_eq!(
        s.store().load_index_progress("custom").unwrap().unwrap(),
        progress
    );
    s.import_script_source(&raw(), 1, &format!("{CODE}\n-- new parser"), true)
        .unwrap();
    assert_eq!(
        update(&s, &transport(&[LIST])).unwrap().revision,
        old.revision + 1
    );
    assert!(s.store().load_index_progress("custom").unwrap().is_none());
}

#[test]
fn cancelled_update_and_detail_cannot_publish_success_even_if_script_catches_it() {
    let (temp, mut s, _) = seeded();
    let old = s.store().load_index("custom").unwrap().unwrap();
    let looping="return {collect_index=function() while true do pcall(function() while true do end end) end end}";
    s.import_script_source(&raw(), 1, looping, true).unwrap();
    let cancel = flag();
    let flag2 = Arc::clone(&cancel);
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        flag2.store(true, Ordering::Relaxed);
    });
    let err = update::update_source(
        s.store(),
        &rule(&s),
        false,
        &mut ctx(&transport(&[])),
        &cancel,
        |_| {},
    )
    .unwrap_err();
    thread.join().unwrap();
    assert_eq!(err.to_string(), "task_cancelled");
    assert_eq!(s.store().load_index("custom").unwrap().unwrap(), old);
    let detail_loop=CODE.replace("local doc = ctx.html(ctx.get(page).text)","while true do pcall(function() while true do end end) end\nlocal doc = ctx.html(ctx.get(page).text)");
    s.import_script_source(&raw(), 1, &detail_loop, true)
        .unwrap();
    update(&s, &transport(&[LIST])).unwrap();
    let before = fs::read(s.store().paths().matches(42)).unwrap();
    let cancel = flag();
    let flag2 = Arc::clone(&cancel);
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        flag2.store(true, Ordering::Relaxed);
    });
    let err = matching::match_games(
        s.store(),
        s.registry().rules(),
        s.registry().overrides(),
        &[game(42)],
        &[],
        false,
        &mut ctx(&transport(&[])),
        &cancel,
        501,
        |_| {},
        |_| {},
    )
    .unwrap_err();
    thread.join().unwrap();
    assert_eq!(err.to_string(), "task_cancelled");
    assert_eq!(before, fs::read(s.store().paths().matches(42)).unwrap());
    drop(temp);
}

#[test]
fn category_inline_linked_details_and_pagination_use_the_same_script_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = open(temp.path());
    let code = r#"return {
        collect_index=function() return {entries={{title='Test Game',page='https://example.com/category',kind='category'}},complete=true} end,
        resolve_page=function(ctx,rule,page) return ctx.json_decode(ctx.get(page).text) end
    }"#;
    s.import_script_source(&raw(), 1, code, false).unwrap();
    update(&s, &transport(&[])).unwrap();
    let inline = json!({"title":"Test Game", "page":"https://example.com/inline", "kind":"article", "steam_ids":[42], "mirrors":[{"url":"https://download.example.com/a.zip","format":"zip"}]});
    let category=json!({"title":"Test Game", "page":"https://example.com/category", "kind":"category", "article_facts":[inline], "next_page":"https://example.com/page2"}).to_string();
    let category2=json!({"title":"Test Game", "page":"https://example.com/page2", "kind":"category", "article_links":[{"title":"Test Game","page":"https://example.com/linked"}]}).to_string();
    let linked=json!({"title":"Test Game", "page":"https://example.com/linked", "kind":"article", "steam_ids":[42], "mirrors":[{"url":"https://download.example.com/b.exe","format":"exe"}]}).to_string();
    let network = transport(&[&category, &category2, &linked]);
    let result = matches(&s, &network, &[game(42)]).unwrap();
    assert_eq!(result[0].status, GameMatchStatus::PublisherMatch);
    assert_eq!(network.call_count(), 3);
    assert!(zip(&result[0].candidates).parser_fingerprint.is_some());
    assert!(exe(&result[0].candidates).parser_fingerprint.is_some());
    assert!(s
        .store()
        .load_detail("custom", "https://example.com/inline")
        .unwrap()
        .is_some());
    assert!(s
        .store()
        .load_detail("custom", "https://example.com/linked")
        .unwrap()
        .is_some());
}

struct ReplacingFetcher {
    state: std::path::PathBuf,
    body: &'static str,
}
impl sources::http::Fetcher for ReplacingFetcher {
    fn fetch(&self, _: &sources::http::FetchRequest) -> sources::Result<FetchResponse> {
        let mut writer = open(&self.state);
        writer.import_script_source(
            &raw(),
            1,
            &format!("{CODE}\n-- replaced during network"),
            true,
        )?;
        Ok(FetchResponse {
            status: 200,
            body: self.body.as_bytes().to_vec(),
            ..Default::default()
        })
    }
}
#[test]
fn mid_task_replacement_does_not_publish_old_index_or_return_old_candidates() {
    let (temp, s, _) = seeded();
    let before = fs::read(s.store().paths().index("custom")).unwrap();
    let mut network = RequestContext::with_settings(
        Box::new(ReplacingFetcher {
            state: temp.path().into(),
            body: LIST,
        }),
        Duration::ZERO,
    );
    let err = update::update_source(s.store(), &rule(&s), false, &mut network, &flag(), |_| {})
        .unwrap_err();
    assert_eq!(err.to_string(), "source_rule_changed");
    assert_eq!(before, fs::read(s.store().paths().index("custom")).unwrap());
    assert_eq!(
        update(&s, &transport(&[])).unwrap_err().to_string(),
        "source_rule_changed"
    );
    assert_eq!(
        matches(&s, &transport(&[]), &[game(42)])
            .unwrap_err()
            .to_string(),
        "source_rule_changed"
    );
    let mut s = open(temp.path());
    s.import_script_source(&raw(), 1, CODE, true).unwrap();
    let before = fs::read(s.store().paths().matches(42)).unwrap();
    let mut network = RequestContext::with_settings(
        Box::new(ReplacingFetcher {
            state: temp.path().into(),
            body: DETAIL,
        }),
        Duration::ZERO,
    );
    let result = matching::rematch_game(
        s.store(),
        s.registry().rules(),
        s.registry().overrides(),
        &game(42),
        &[],
        &mut network,
        &flag(),
        600,
        |_| {},
    )
    .unwrap();
    assert!(result.candidates.is_empty());
    assert_eq!(before, fs::read(s.store().paths().matches(42)).unwrap());
    assert!(matching::match_summary_at(temp.path(), 42)
        .unwrap()
        .unwrap()
        .candidates
        .is_empty());
}

#[test]
fn unchanged_script_failures_keep_last_success_without_publishing_false_completion() {
    let (_temp, s, a) = seeded();
    let index = s.store().load_index("custom").unwrap().unwrap();
    let detail = s
        .store()
        .load_detail("custom", "https://example.com/game")
        .unwrap()
        .unwrap();
    assert!(update(&s, &transport(&[])).is_err());
    assert_eq!(s.store().load_index("custom").unwrap().unwrap(), index);
    let result = matching::rematch_game(
        s.store(),
        s.registry().rules(),
        s.registry().overrides(),
        &game(42),
        &[],
        &mut ctx(&transport(&[])),
        &flag(),
        600,
        |_| {},
    )
    .unwrap();
    assert_eq!(result.status, GameMatchStatus::Incomplete);
    assert_eq!(result.candidates, a);
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == "script_source_runtime_error"
            && d.detail
                .as_ref()
                .is_some_and(|s| s.contains("entry=resolve_page"))));
    assert_eq!(
        s.store()
            .load_detail("custom", "https://example.com/game")
            .unwrap()
            .unwrap(),
        detail
    );
}

#[test]
fn unreadable_parser_settings_are_reported_instead_of_authorizing_cached_candidates() {
    let (_temp, s, _) = seeded();
    fs::write(s.store().paths().settings(), b"invalid JSON").unwrap();
    let summary = matching::game_match_summary(s.store(), 42);
    assert_eq!(summary.status, GameMatchStatus::Failed);
    assert!(summary.candidates.is_empty());
    assert!(!summary.diagnostics.is_empty());
}
