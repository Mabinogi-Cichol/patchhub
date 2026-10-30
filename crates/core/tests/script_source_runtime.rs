//! P2 production entry points; no direct evaluation shortcut or real websites.
use patchhub_core::{
    projection::DomainError,
    sources::{
        adapters::{script, DiscoverPrevious},
        http::{FakeFetcher, FetchRequest, FetchResponse, Fetcher, RequestContext},
        model::{QueryContext, RawSourceRule, SourceIndex, SourceRule},
        Sources,
    },
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

fn rule(code: &str) -> SourceRule {
    let raw: RawSourceRule = serde_json::from_value(json!({"id":"custom","name":"Script",
        "adapter":"lua","enabled":true,"pages":["https://example.com/"],
        "page_hosts":["example.com"],"download_hosts":["download.example.com"],"root_layout":"game_root"})).unwrap();
    let mut rule = patchhub_core::sources::validate::validate_rule(&raw).unwrap();
    // Runtime syntax-failure coverage needs a digest-valid but invalid source;
    // normal import rejects that source earlier.
    rule.script = Some(patchhub_core::sources::ScriptDescriptor {
        api_version: 1,
        sha256: format!("{:x}", Sha256::digest(code.as_bytes())),
    });
    rule
}
fn ctx(fake: &Arc<FakeFetcher>) -> RequestContext {
    RequestContext::with_settings(Box::new(Arc::clone(fake)), Duration::ZERO)
}
fn flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[test]
fn documented_publisher_example_imports_and_collects_direct_entries() {
    let code = include_str!(
        "../../../docs/features/script-source-adapters/examples/publisher/adapter.lua"
    );
    let raw: RawSourceRule = serde_json::from_str(include_str!(
        "../../../docs/features/script-source-adapters/examples/publisher/source.json"
    ))
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(temp.path()).unwrap();
    let imported = sources.import_script_source(&raw, 1, code, false).unwrap();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(FetchResponse { status: 200, final_url: raw.pages[0].clone(),
        body: br#"<table><tr data-appid="123"><td class="title">Game</td><td><a href="https://downloads.vendor.example/game.zip">Patch</a></td></tr></table>"#.to_vec(),
        ..FetchResponse::default() });
    let result =
        script::collect_index(&imported.rule, code, None, &mut ctx(&fake), flag(), |_| {}).unwrap();
    assert!(result.complete);
    assert_eq!(result.entries.len(), 1);
    assert_eq!(result.entries[0].appid, Some(123));
    assert_eq!(result.entries[0].kind, "entry");
    assert_eq!(
        result.entries[0].url.as_deref(),
        Some("https://downloads.vendor.example/game.zip")
    );
}
fn index(
    code: &str,
    fake: &Arc<FakeFetcher>,
) -> std::io::Result<patchhub_core::sources::adapters::IndexOutcome> {
    script::collect_index(&rule(code), code, None, &mut ctx(fake), flag(), |_| {})
}

#[test]
fn imported_non_table_source_executes_list_and_detail_with_host_bound_progress() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    let code = include_str!("fixtures/script-sources/kagura-en-prototype.lua");
    let mut raw = patchhub_core::sources::RawSourceRule::from(&rule(code));
    raw.pages = vec!["https://www.kaguragames.com/patch/".into()];
    raw.page_hosts = vec!["www.kaguragames.com".into()];
    sources.import_script_source(&raw, 1, code, false).unwrap();
    let document = sources.get_script_source("custom").unwrap();
    let rule = sources
        .registry()
        .rules()
        .iter()
        .find(|r| r.id == "custom")
        .unwrap();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(FetchResponse {
        status: 200,
        body: include_bytes!("fixtures/script-sources/kagura-en.html").to_vec(),
        final_url: rule.pages[0].clone(),
        ..FetchResponse::default()
    });
    let mut progress = Vec::new();
    let found = script::collect_index(
        rule,
        &document.code,
        None,
        &mut ctx(&fake),
        flag(),
        |event| progress.push(event),
    )
    .unwrap();
    assert!(found.complete);
    assert_eq!(found.entries.len(), 2);
    assert!(found.checked_at > 0);
    assert!(progress
        .iter()
        .all(|p| p.source_id == "custom" && p.entry == "collect_index"));
    let page = &found.entries[0].page;
    let detail = Arc::new(FakeFetcher::new());
    detail.push(FetchResponse {
        status: 200,
        body: include_bytes!("fixtures/script-sources/kagura-en-detail.html").to_vec(),
        final_url: page.clone(),
        ..FetchResponse::default()
    });
    let fact = script::resolve_page(
        rule,
        &document.code,
        page,
        &mut ctx(&detail),
        flag(),
        |_| {},
    )
    .unwrap();
    assert_eq!(fact.mirrors.len(), 4);
    assert!(fact.mirrors.iter().any(|m| m.format == "exe"));
}

#[test]
fn partial_results_and_diagnostics_keep_host_identity() {
    let code = r#"return {collect_index=function(ctx,rule)
        ctx.progress('working', {source_id='spoofed'})
        return {entries={},pending={rule.pages[1]},complete=false,diagnostics={{code='incomplete',detail='page remains'}}}
    end}"#;
    let fake = Arc::new(FakeFetcher::new());
    let mut progress = Vec::new();
    let output = script::collect_index(&rule(code), code, None, &mut ctx(&fake), flag(), |p| {
        progress.push(p)
    })
    .unwrap();
    assert!(!output.complete);
    assert_eq!(output.pending, rule(code).pages);
    assert_eq!(output.diagnostics[0].source_id.as_deref(), Some("custom"));
    assert_eq!(
        output.diagnostics[0].detail.as_deref(),
        Some("page remains")
    );
    assert_eq!(progress[0].source_id, "custom");
}

#[test]
fn errors_keep_code_source_entry_and_lua_location() {
    let fake = Arc::new(FakeFetcher::new());
    for (code, expected) in [
        ("return {", "script_source_syntax_error"),
        ("error('module failed')", "script_source_runtime_error"),
        (
            "return {collect_index=function() error('entry failed') end}",
            "script_source_runtime_error",
        ),
        ("return {}", "script_source_contract_error"),
        (
            "return {collect_index=function() return {entries='bad'} end}",
            "script_source_contract_error",
        ),
        (
            "return {collect_index=function() return {candidate_id='forged'} end}",
            "script_source_contract_error",
        ),
    ] {
        let error = index(code, &fake).unwrap_err();
        let domain: DomainError = error.into();
        assert_eq!(domain.code, expected);
        let detail = domain.detail.unwrap();
        assert!(
            detail.contains("source=custom") && detail.contains("entry=collect_index"),
            "{detail}"
        );
        if expected == "script_source_runtime_error" {
            assert!(detail.contains("source/custom.lua:"), "{detail}");
        }
    }
    let valid = "return {collect_index=function() return {entries={},complete=true} end}";
    let error = script::collect_index(
        &rule("different"),
        valid,
        None,
        &mut ctx(&fake),
        flag(),
        |_| {},
    )
    .unwrap_err();
    assert!(error.to_string().contains("script_source_digest_mismatch"));
    let mut unsupported = rule(valid);
    unsupported.script.as_mut().unwrap().api_version = 2;
    let error: DomainError =
        script::collect_index(&unsupported, valid, None, &mut ctx(&fake), flag(), |_| {})
            .unwrap_err()
            .into();
    assert_eq!(error.code, "unsupported_script_api_version");
    assert!(error.detail.unwrap().contains("entry=collect_index"));
    assert_eq!(fake.call_count(), 0);
}

#[test]
fn source_facts_are_validated_and_script_rule_mutation_cannot_expand_network_access() {
    let fake = Arc::new(FakeFetcher::new());
    for code in [
        "return {collect_index=function() return {complete=true,entries={{title='Game',page='https://evil.example/',kind='product'}}} end}",
        "return {collect_index=function() return {complete=true,entries={{title='Game',page='https://example.com/',kind='entry',url='https://evil.example/a.zip'}}} end}",
        "return {collect_index=function() return {complete=true,pending={'https://example.com/'}} end}",
    ] { assert!(index(code,&fake).unwrap_err().to_string().starts_with("script_source_contract_error")); }
    let code="return {collect_index=function(ctx,rule) rule.page_hosts={'evil.example'}; ctx.get('https://evil.example/') end}";
    assert!(index(code, &fake)
        .unwrap_err()
        .to_string()
        .contains("invalid_discovery_url"));
    assert_eq!(fake.call_count(), 0);
    let code="return {resolve_page=function() return {title='Game',page='https://example.com/game/',kind='product',mirrors={{url='https://download.example.com/a.zip',format='exe'}}} end}";
    let fact = script::resolve_page(
        &rule(code),
        code,
        "https://example.com/game/",
        &mut ctx(&fake),
        flag(),
        |_| {},
    )
    .unwrap();
    assert_eq!(fact.mirrors[0].format, "zip");
}

#[test]
fn discover_passes_query_and_previous_without_accepting_candidate_authority() {
    let fake = Arc::new(FakeFetcher::new());
    let query = QueryContext {
        appid: 42,
        name: "Game".into(),
        name_hints: vec![],
        legacy_match: false,
        force: true,
        source_context: vec![],
        parser_version: 2,
        query_version: 1,
        settings_revision: 1,
        generation: None,
    };
    let code = r#"return {discover=function(ctx,rule,query,previous)
        assert(query.appid==42 and query.force and #query.terms>0)
        assert(query.settings_revision==nil and previous.known_candidates~=nil)
        return {pages={{fact={title='Game',page='https://example.com/game/',kind='article',steam_ids={42}},reported_format='zip'}},
            identities={{appid=42,page='https://example.com/game/',aliases={'Game'}}},complete=true}
    end}"#;
    let result = script::discover(
        &rule(code),
        code,
        &query,
        &DiscoverPrevious::default(),
        &mut ctx(&fake),
        flag(),
        |_| {},
    )
    .unwrap();
    assert!(result.complete);
    assert_eq!(result.pages[0].fact.steam_ids, [42]);
    assert_eq!(result.identities[0].appid, 42);
    let code="return {discover=function() return {pages={{fact={title='Game',page='https://example.com/',kind='article',candidate_id='forged'}}},complete=true} end}";
    assert!(script::discover(
        &rule(code),
        code,
        &query,
        &DiscoverPrevious::default(),
        &mut ctx(&fake),
        flag(),
        |_| {}
    )
    .unwrap_err()
    .to_string()
    .contains("script_source_contract_error"));
}

#[test]
fn conditional_requests_raw_post_and_legacy_chinese_encoding_use_existing_context() {
    let fake = Arc::new(FakeFetcher::new());
    fake.push(FetchResponse {
        status: 304,
        final_url: "https://example.com/".into(),
        ..FetchResponse::default()
    });
    let (body, _, _) = encoding_rs::GBK.encode("中文");
    fake.push(FetchResponse {
        status: 200,
        final_url: "https://example.com/post".into(),
        body: body.into_owned(),
        content_type: Some("text/html; charset=gbk".into()),
        ..FetchResponse::default()
    });
    let code = r#"return {collect_index=function(ctx,rule)
        assert(ctx.get(rule.pages[1],{etag='v1',modified='yesterday'}).status==304)
        local response=ctx.post('https://example.com/post','application/json','{"q":"Game"}')
        return {complete=true,entries={{title=response.text,page=rule.pages[1],kind='entry',url='https://download.example.com/a.zip'}}}
    end}"#;
    let result = index(code, &fake).unwrap();
    assert_eq!(result.entries[0].title, "中文");
    assert!(fake.calls()[0]
        .headers
        .contains(&("If-None-Match".into(), "v1".into())));
    assert_eq!(fake.calls()[1].method, "POST");
    assert_eq!(
        fake.calls()[1].body.as_deref(),
        Some(br#"{"q":"Game"}"#.as_slice())
    );
    let code = "return {collect_index=function() return {not_modified=true} end}";
    assert!(index(code, &Arc::new(FakeFetcher::new())).is_err());
    let previous = SourceIndex {
        parser_fingerprint: None,
        schema_version: 1,
        source_id: "custom".into(),
        revision: 1,
        complete: true,
        checked_at: 1,
        updated_at: 1,
        etag: Some("v1".into()),
        modified: None,
        entries: result.entries,
        pending: vec![],
    };
    let result = script::collect_index(
        &rule(code),
        code,
        Some(&previous),
        &mut ctx(&fake),
        flag(),
        |_| {},
    )
    .unwrap();
    assert!(result.not_modified);
    assert_eq!(result.entries, previous.entries);
}

struct CancellingFetcher(Arc<AtomicBool>);
impl Fetcher for CancellingFetcher {
    fn fetch(&self, _: &FetchRequest) -> std::io::Result<FetchResponse> {
        self.0.store(true, Ordering::Relaxed);
        Err(std::io::Error::other("task_cancelled"))
    }
}

#[test]
fn captured_cancellation_cannot_return_a_successful_index() {
    let cancel = flag();
    let mut context = RequestContext::with_settings(
        Box::new(CancellingFetcher(Arc::clone(&cancel))),
        Duration::ZERO,
    );
    let code="return {collect_index=function(ctx) pcall(ctx.get,'https://example.com/'); return {complete=true,entries={}} end}";
    let error =
        script::collect_index(&rule(code), code, None, &mut context, cancel, |_| {}).unwrap_err();
    assert_eq!(error.to_string(), "task_cancelled");
}

#[test]
fn production_executor_cancels_top_level_and_protected_entry_loops() {
    for code in ["while true do end", "pcall(function() while true do end end)",
        "return {collect_index=function() while true do end end}",
        "return {collect_index=function() string.gsub('x','x',function() while true do pcall(function() while true do end end) end end) end}"] {
        let cancel = flag();
        let worker_cancel = Arc::clone(&cancel);
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            worker_cancel.store(true, Ordering::Relaxed);
        });
        let fake = Arc::new(FakeFetcher::new());
        let result = script::collect_index(&rule(code), code, None, &mut ctx(&fake), cancel, |_| {});
        worker.join().unwrap();
        assert_eq!(result.unwrap_err().to_string(), "task_cancelled");
    }
}
