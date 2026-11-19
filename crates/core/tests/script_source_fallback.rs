//! P4 per-game discovery through the production matcher and read/selection views.
use patchhub_core::{
    catalog::Platform,
    install::TransactionSnapshot,
    installer::{exe_resolve, manual_selection, resolve},
    projection::{self, HostCapabilities, ProjectionInputs},
    sources::{
        self,
        http::{FakeFetcher, FetchRequest, FetchResponse, Fetcher, RequestContext},
        matching,
        model::*,
        snapshot::SourceSnapshot,
        update, Sources,
    },
    steam,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
const PUBLISHER_ID: &str = "publisher-example-com";
const ID_A: &str = "a-example-com";
const ID_B: &str = "b-example-com";
const PUBLISHER_CODE: &str = r#"return {
    meta = { name = "Publisher", pages = { "https://publisher.example.com/" }, download_hosts = { "download.example.com" } },
    collect_index = function() return { entries = {}, complete = true } end,
}"#;
const DISCOVER_FN: &str = "discover=function(ctx,rule,query,previous) assert(query.appid==42); assert(query.terms[1]); return ctx.json_decode(ctx.get(rule.pages[1]).text) end";
fn fallback_code(prefix: &str, discover_fn: &str) -> String {
    format!(
        "return {{ meta = {{ name = '{prefix}', role = 'fallback_indexer', pages = {{'https://{prefix}.example.com/query'}}, priority = 10 }}, {discover_fn}\n}}"
    )
}
fn config() -> String {
    let mut config: Value = serde_json::from_str(sources::DEFAULT_RULES_JSON).unwrap();
    config["sources"]
        .as_array_mut()
        .unwrap()
        .retain(|r| r["id"] == "hgamer");
    config.to_string()
}
fn open(state: &Path) -> Sources {
    Sources::open_with_rules(state, &config()).unwrap()
}
fn setup() -> (tempfile::TempDir, Sources) {
    let temp = tempfile::tempdir().unwrap();
    let mut s = open(temp.path());
    s.registry_mut().set_enabled("hgamer", false).unwrap();
    s.import_script_source(None, 1, PUBLISHER_CODE).unwrap();
    s.store()
        .save_index(&SourceIndex {
            parser_fingerprint: sources::index::parser_fingerprint(
                s.registry()
                    .rules()
                    .iter()
                    .find(|r| r.id == PUBLISHER_ID)
                    .unwrap(),
            ),
            schema_version: sources::SOURCE_INDEX_SCHEMA_VERSION,
            source_id: PUBLISHER_ID.into(),
            revision: 1,
            complete: true,
            checked_at: 100,
            updated_at: 100,
            etag: None,
            modified: None,
            entries: vec![],
            pending: vec![],
        })
        .unwrap();
    (temp, s)
}
fn source_id(prefix: &str) -> &'static str {
    match prefix {
        "a" => ID_A,
        "b" => ID_B,
        _ => unreachable!(),
    }
}
fn add(s: &mut Sources, prefix: &str, discover_fn: &str) {
    let code = fallback_code(prefix, discover_fn);
    s.import_script_source(None, 1, &code).unwrap();
}
fn replace_script(s: &mut Sources, prefix: &str, discover_fn: &str) -> sources::Result<()> {
    let code = fallback_code(prefix, discover_fn);
    s.import_script_source(Some(source_id(prefix)), 1, &code)?;
    Ok(())
}
fn game() -> GameQuery {
    GameQuery {
        appid: 42,
        name: "Game".into(),
        localized_name: None,
        related_dlc: Vec::new(),
        instance_id: Some("test".into()),
    }
}
fn flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}
fn ctx(fake: &Arc<FakeFetcher>) -> RequestContext {
    RequestContext::with_settings(Box::new(Arc::clone(fake)), Duration::ZERO)
}
fn fake(bodies: &[Value]) -> Arc<FakeFetcher> {
    let fake = Arc::new(FakeFetcher::new());
    for body in bodies {
        fake.push(FetchResponse {
            status: 200,
            body: serde_json::to_vec(body).unwrap(),
            ..Default::default()
        });
    }
    fake
}
fn found(id: &str) -> Value {
    json!({"details":[{"title":"Game","page":format!("https://{id}.example.com/patch"),"steam_ids":[42],"text":"English patch","mirrors":[{"url":"https://download.example.com/a.zip","format":"unknown"},{"url":"https://download.example.com/a.exe","format":"unknown"}]}],"complete":true})
}
fn empty() -> Value {
    json!({"details":[],"complete":true})
}
fn run(s: &Sources, fake: &Arc<FakeFetcher>) -> Vec<GameMatch> {
    matching::match_games(
        s.store(),
        s.registry().rules(),
        s.registry().overrides(),
        &[game()],
        &[],
        false,
        &mut ctx(fake),
        &flag(),
        500,
        |_| {},
        |_| {},
    )
    .unwrap()
}
fn assert_hidden(s: &Sources) {
    assert!(s.registry().cached_candidates(&game()).is_empty());
    let discovery = s.registry().cached_discovery(&game());
    assert!(discovery.candidates.is_empty());
    assert_ne!(discovery.status, "no_match");
    assert!(matching::game_match_summary(s.store(), 42)
        .candidates
        .is_empty());
}

#[test]
fn multiple_scripts_are_all_queried_keep_source_identity_and_reuse_after_restart() {
    let (temp, mut s) = setup();
    add(&mut s, "b", DISCOVER_FN);
    add(&mut s, "a", DISCOVER_FN);
    let network = fake(&[found("a"), found("b")]);
    let result = run(&s, &network);
    assert_eq!(result[0].status, GameMatchStatus::FallbackMatch);
    assert_eq!(
        network
            .calls()
            .iter()
            .map(|r| r.url.as_str())
            .collect::<Vec<_>>(),
        vec!["https://a.example.com/query", "https://b.example.com/query"]
    );
    assert_eq!(result[0].candidates.len(), 4);
    assert!(result[0]
        .candidates
        .iter()
        .all(|c| c.parser_fingerprint.is_some()));
    for id in [ID_A, ID_B] {
        assert!(
            s.registry()
                .list_game_sources(&game())
                .iter()
                .find(|r| r.source.rule.id == id)
                .unwrap()
                .matched
        );
    }
    let record = s.store().load_match(42).unwrap().unwrap();
    assert!(record.fallback.unwrap().sources_fingerprint.is_some());
    let empty = fake(&[]);
    drop(s);
    let s = open(temp.path());
    assert_eq!(run(&s, &empty), result);
    assert_eq!(empty.call_count(), 0);
}

#[test]
fn mixed_success_failure_partial_and_complete_empty_have_distinct_outcomes() {
    let (_temp, mut s) = setup();
    add(&mut s, "a", DISCOVER_FN);
    add(
        &mut s,
        "b",
        "discover=function() error('broken source') end",
    );
    let result = run(&s, &fake(&[found("a")]));
    assert_eq!(result[0].status, GameMatchStatus::FallbackMatch);
    assert_eq!(result[0].candidates.len(), 2);
    let diagnostic = result[0]
        .diagnostics
        .iter()
        .find(|d| d.source_id.as_deref() == Some(ID_B))
        .unwrap();
    assert_eq!(diagnostic.code, "script_source_runtime_error");
    assert!(diagnostic
        .detail
        .as_ref()
        .unwrap()
        .contains("entry=discover"));
    assert!(
        !s.registry()
            .list_game_sources(&game())
            .iter()
            .find(|r| r.source.rule.id == ID_B)
            .unwrap()
            .matched
    );
    // One successful empty response plus one failure is incomplete, never no-match.
    replace_script(&mut s, "a", &format!("{DISCOVER_FN}\n-- empty attempt")).unwrap();
    assert_eq!(
        run(&s, &fake(&[empty()]))[0].status,
        GameMatchStatus::Incomplete
    );
    replace_script(&mut s, "b", DISCOVER_FN).unwrap();
    assert_eq!(
        run(&s, &fake(&[empty(), empty()]))[0].status,
        GameMatchStatus::NoMatch
    );
    // Partial success with no facts remains incomplete; a title lead stays visible.
    replace_script(&mut s, "a", &format!("{DISCOVER_FN}\n-- partial attempt")).unwrap();
    assert_eq!(
        run(
            &s,
            &fake(&[json!({"details":[],"complete":false}), empty()])
        )[0]
        .status,
        GameMatchStatus::Incomplete
    );
    let mut lead = found("a");
    lead["details"][0]["steam_ids"] = json!([]);
    let result = run(&s, &fake(&[lead, empty()]));
    assert_eq!(result[0].status, GameMatchStatus::Incomplete);
    assert!(!result[0].candidates.is_empty());
    assert!(!s.registry().cached_candidates(&game()).is_empty());
    for prefix in ["a", "b"] {
        replace_script(&mut s, prefix, "discover=function() error('failure') end").unwrap();
    }
    assert_eq!(run(&s, &fake(&[]))[0].status, GameMatchStatus::Incomplete);
    assert_eq!(
        s.store()
            .load_match(42)
            .unwrap()
            .unwrap()
            .fallback
            .unwrap()
            .status,
        FallbackMatchStatus::Failed
    );
}

#[test]
fn source_set_subscription_and_same_fact_code_replacement_invalidate_cache_and_old_choices() {
    let (temp, mut s) = setup();
    add(&mut s, "a", DISCOVER_FN);
    let result = run(&s, &fake(&[found("a")]));
    let old = result[0].candidates.clone();
    let old_key = s
        .store()
        .load_match(42)
        .unwrap()
        .unwrap()
        .fallback
        .unwrap()
        .query_key;
    s.registry_mut().set_name(ID_A, "Display").unwrap();
    assert_eq!(s.registry().cached_candidates(&game()), old);
    add(&mut s, "b", DISCOVER_FN);
    assert_hidden(&s);
    let network = fake(&[found("a"), empty()]);
    run(&s, &network);
    assert_eq!(network.call_count(), 2);
    assert_ne!(
        s.store()
            .load_match(42)
            .unwrap()
            .unwrap()
            .fallback
            .unwrap()
            .query_key,
        old_key
    );
    s.registry_mut().set_game_source(42, ID_A, false).unwrap();
    assert_hidden(&s);
    let network = fake(&[found("b")]);
    let result = run(&s, &network);
    assert_eq!(network.call_count(), 1);
    assert!(result[0].candidates.iter().all(|c| c.source_id == ID_B));
    s.registry_mut().set_enabled(ID_B, false).unwrap();
    assert_hidden(&s);
    s.registry_mut().set_enabled(ID_B, true).unwrap();
    s.registry_mut().set_game_source(42, ID_A, true).unwrap();
    replace_script(
        &mut s,
        "a",
        &format!("{DISCOVER_FN}\n-- same facts, new code"),
    )
    .unwrap();
    assert_hidden(&s);
    drop(s);
    let s = open(temp.path());
    assert_hidden(&s);
    let result = run(&s, &fake(&[found("a"), empty()]));
    assert_ne!(result[0].candidates[0].candidate_id, old[0].candidate_id);
    for candidate in old {
        for mirror in &candidate.mirrors {
            let id = manual_selection::selection_id(&candidate, &game(), mirror).unwrap();
            assert!(manual_selection::resolve(s.registry(), &game(), &id, &mirror.format).is_err());
        }
    }
}

#[test]
fn hgamer_alone_and_hgamer_plus_script_are_queried_without_indexing_fallbacks() {
    let (_temp, mut s) = setup();
    s.registry_mut().set_enabled("hgamer", true).unwrap();
    // Use the same recorded search shape as the maintained adapter's offline tests.
    let search = "<div class='summary'><h1>Search</h1><p>找到 0 笔 results for Game</p></div>";
    let network = Arc::new(FakeFetcher::new());
    network.push(FetchResponse {
        status: 200,
        body: search.as_bytes().to_vec(),
        ..Default::default()
    });
    assert_eq!(run(&s, &network)[0].status, GameMatchStatus::NoMatch);
    assert_eq!(network.call_count(), 1);
    add(&mut s, "a", DISCOVER_FN);
    let network = fake(&[found("a")]);
    network.push(FetchResponse {
        status: 200,
        body: search.as_bytes().to_vec(),
        ..Default::default()
    });
    assert_eq!(run(&s, &network)[0].status, GameMatchStatus::FallbackMatch);
    assert_eq!(network.call_count(), 2);
    let rule = s.registry().rules().iter().find(|r| r.id == ID_A).unwrap();
    assert_eq!(
        update::update_source(s.store(), rule, true, &mut ctx(&fake(&[])), &flag(), |_| {})
            .unwrap_err()
            .to_string(),
        "source_not_indexable"
    );
    assert_eq!(
        s.registry_mut()
            .set_update_interval(ID_A, 1)
            .unwrap_err()
            .to_string(),
        "source_not_indexable"
    );
}

#[test]
fn publisher_hit_skips_all_fallbacks_and_incomplete_coverage_never_confirms_no_patch() {
    let (_temp, mut s) = setup();
    add(&mut s, "a", "discover=function() while true do end end");
    let mut index = s.store().load_index(PUBLISHER_ID).unwrap().unwrap();
    index.entries.push(SourceIndexEntry {
        title: "Game".into(),
        page: "https://publisher.example.com/".into(),
        appid: Some(42),
        aliases: vec![],
        current_step: 1,
        total_steps: 1,
        step_data: None,
        details: vec![SourceDetail {
            title: "Game".into(),
            page: "https://publisher.example.com/".into(),
            steam_ids: vec![42],
            mirrors: vec![Mirror {
                url: "https://download.example.com/game.zip".into(),
                label: None,
                format: "zip".into(),
            }],
            languages: vec!["en".into()],
            reported_format: Some("zip".into()),
            ..Default::default()
        }],
    });
    s.store().save_index(&index).unwrap();
    let network = fake(&[]);
    assert_eq!(run(&s, &network)[0].status, GameMatchStatus::PublisherMatch);
    assert_eq!(network.call_count(), 0);
    index.revision += 1;
    index.entries.clear();
    index.complete = false;
    s.store().save_index(&index).unwrap();
    let result = run(&s, &network);
    assert_eq!(result[0].status, GameMatchStatus::Incomplete);
    assert_eq!(network.call_count(), 0);
}

#[test]
fn query_identities_enrich_facts_but_foreign_appids_do_not_authorize_this_game() {
    let (_temp, mut s) = setup();
    add(&mut s, "a", DISCOVER_FN);
    let mut response = found("a");
    response["details"][0]["title"] = json!("Another official title");
    response["details"][0]["steam_ids"] = json!([]);
    response["identities"] = json!([{"appid":42,"page":"https://a.example.com/game","aliases":["Another official title"]}]);
    let result = run(&s, &fake(&[response.clone()]));
    assert_eq!(result[0].status, GameMatchStatus::FallbackMatch);
    assert!(result[0]
        .candidates
        .iter()
        .all(|c| c.match_basis == MatchBasis::VerifiedAlias));
    response["identities"][0]["appid"] = json!(43);
    replace_script(&mut s, "a", &format!("{DISCOVER_FN}\n-- foreign")).unwrap();
    assert!(run(&s, &fake(&[response]))[0].candidates.is_empty());
}

#[test]
fn cancellation_in_later_source_does_not_commit_an_aggregate_success() {
    let (_temp, mut s) = setup();
    add(&mut s, "a", DISCOVER_FN);
    add(&mut s, "b", DISCOVER_FN);
    run(&s, &fake(&[found("a"), found("b")]));
    let before = fs::read(s.store().paths().matches(42)).unwrap();
    replace_script(
        &mut s,
        "b",
        "discover=function() while true do pcall(function() while true do end end) end end",
    )
    .unwrap();
    let cancel = flag();
    let flag = Arc::clone(&cancel);
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        flag.store(true, Ordering::Relaxed);
    });
    let err = matching::match_games(
        s.store(),
        s.registry().rules(),
        s.registry().overrides(),
        &[game()],
        &[],
        false,
        &mut ctx(&fake(&[found("a")])),
        &cancel,
        600,
        |_| {},
        |_| {},
    )
    .unwrap_err();
    thread.join().unwrap();
    assert_eq!(err.to_string(), "task_cancelled");
    assert_eq!(fs::read(s.store().paths().matches(42)).unwrap(), before);
}

struct ReplaceDuringQuery {
    add_source: bool,
    state: std::path::PathBuf,
}
impl Fetcher for ReplaceDuringQuery {
    fn fetch(&self, _: &FetchRequest) -> sources::Result<FetchResponse> {
        let mut s = open(&self.state);
        if self.add_source {
            s.import_script_source(None, 1, &fallback_code("b", DISCOVER_FN))?;
        } else {
            replace_script(&mut s, "a", "discover=function() while true do end end")?;
        }
        Ok(FetchResponse {
            status: 200,
            body: serde_json::to_vec(&found("a")).unwrap(),
            ..Default::default()
        })
    }
}
#[test]
fn mid_query_replacement_preserves_disk_but_does_not_return_old_positive() {
    for add_source in [false, true] {
        let (temp, mut s) = setup();
        add(&mut s, "a", DISCOVER_FN);
        run(&s, &fake(&[found("a")]));
        let before = fs::read(s.store().paths().matches(42)).unwrap();
        let mut network = RequestContext::with_settings(
            Box::new(ReplaceDuringQuery {
                add_source,
                state: temp.path().into(),
            }),
            Duration::ZERO,
        );
        let result = matching::rematch_game(
            s.store(),
            s.registry().rules(),
            s.registry().overrides(),
            &game(),
            &[],
            &mut network,
            &flag(),
            600,
            |_| {},
        )
        .unwrap();
        assert!(result.candidates.is_empty());
        assert_eq!(fs::read(s.store().paths().matches(42)).unwrap(), before);
        assert_hidden(&open(temp.path()));
        let error = matching::match_games(
            s.store(),
            s.registry().rules(),
            s.registry().overrides(),
            &[game()],
            &[],
            false,
            &mut ctx(&fake(&[])),
            &flag(),
            700,
            |_| {},
            |_| {},
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "source_rule_changed");
    }
}

#[test]
fn readonly_projection_and_old_selection_resolution_do_not_execute_replacement_code() {
    let (temp, mut s) = setup();
    add(&mut s, "a", DISCOVER_FN);
    let old = run(&s, &fake(&[found("a")]))[0].candidates.clone();
    replace_script(&mut s, "a", "discover=function() while true do end end").unwrap();
    assert_hidden(&s);
    for candidate in &old {
        if let Some(id) = resolve::zip_selection_id(candidate) {
            assert!(resolve::resolve_zip(s.registry(), &game(), &id).is_err());
        }
        if let Some(id) = exe_resolve::exe_selection_id(candidate) {
            assert!(exe_resolve::resolve_exe(s.registry(), &game(), &id).is_err());
        }
    }
    let games = tempfile::tempdir().unwrap();
    let library = games.path().join("Steam");
    fs::create_dir_all(library.join("steamapps/common/Game")).unwrap();
    fs::write(library.join("steamapps/appmanifest_42.acf"),r#""AppState" { "appid" "42" "name" "Game" "installdir" "Game" "buildid" "1" "StateFlags" "4" }"#).unwrap();
    let steam = steam::scan(&[library]);
    let snapshot = SourceSnapshot::load(temp.path(), &[game()], 500)
        .unwrap()
        .unwrap();
    let tx = TransactionSnapshot::default();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let row = projection::project_instance(
        &ProjectionInputs {
            steam: &steam,
            sources: Some(&snapshot),
            transactions: &tx,
            roots: &[],
            now: 500,
            platform: Platform::Windows,
            capabilities: &caps,
            cancel: &cancel,
        },
        &steam.installs[0].instance_id,
    )
    .unwrap();
    assert!(row.discovery.unwrap().candidates.is_empty());
    assert!(row.patches.is_empty());
}

#[test]
fn unbound_negative_cache_is_recomputed_and_source_order_does_not_change_the_key() {
    let (_temp, mut s) = setup();
    add(&mut s, "a", DISCOVER_FN);
    add(&mut s, "b", DISCOVER_FN);
    run(&s, &fake(&[empty(), empty()]));
    let mut record = s.store().load_match(42).unwrap().unwrap();
    let key = record.fallback.as_ref().unwrap().query_key.clone();
    record.fallback.as_mut().unwrap().sources_fingerprint = None;
    s.store().save_match(&record).unwrap();
    assert_ne!(s.registry().cached_discovery(&game()).status, "no_match");
    let network = fake(&[empty(), empty()]);
    assert_eq!(run(&s, &network)[0].status, GameMatchStatus::NoMatch);
    assert_eq!(network.call_count(), 2);
    let mut rules = s.registry().rules().to_vec();
    rules.reverse();
    let network = fake(&[]);
    matching::match_games(
        s.store(),
        &rules,
        s.registry().overrides(),
        &[game()],
        &[],
        false,
        &mut ctx(&network),
        &flag(),
        600,
        |_| {},
        |_| {},
    )
    .unwrap();
    assert_eq!(network.call_count(), 0);
    assert_eq!(
        s.store()
            .load_match(42)
            .unwrap()
            .unwrap()
            .fallback
            .unwrap()
            .query_key,
        key
    );
}

#[test]
fn name_hints_and_explicit_rematch_query_all_sources_and_expose_force_to_scripts() {
    let (_temp, mut s) = setup();
    add(&mut s, "a", DISCOVER_FN);
    add(&mut s, "b", DISCOVER_FN);
    run(&s, &fake(&[empty(), empty()]));
    let before = s
        .store()
        .load_match(42)
        .unwrap()
        .unwrap()
        .fallback
        .unwrap()
        .query_key;
    let network = fake(&[empty(), empty()]);
    matching::match_games(
        s.store(),
        s.registry().rules(),
        s.registry().overrides(),
        &[game()],
        &["Alias".into()],
        false,
        &mut ctx(&network),
        &flag(),
        600,
        |_| {},
        |_| {},
    )
    .unwrap();
    assert_eq!(network.call_count(), 2);
    assert_ne!(
        s.store()
            .load_match(42)
            .unwrap()
            .unwrap()
            .fallback
            .unwrap()
            .query_key,
        before
    );
    let discover = DISCOVER_FN.replace(
        "assert(query.appid==42)",
        "assert(query.appid==42); assert(query.force)",
    );
    for prefix in ["a", "b"] {
        replace_script(&mut s, prefix, &discover).unwrap();
    }
    let network = fake(&[empty(), empty()]);
    let result = matching::rematch_game(
        s.store(),
        s.registry().rules(),
        s.registry().overrides(),
        &game(),
        &[],
        &mut ctx(&network),
        &flag(),
        700,
        |_| {},
    )
    .unwrap();
    assert_eq!(network.call_count(), 2);
    assert_eq!(result.status, GameMatchStatus::NoMatch);
}
