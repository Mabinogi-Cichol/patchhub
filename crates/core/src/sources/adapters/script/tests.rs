//! The original P0 regression corpus now exercises the production executor.
use crate as patchhub_core;
use crate::sources::adapters::script as runtime;
use serde::de::DeserializeOwned;

use mlua::{chunk::ChunkMode, HookTriggers, VmState};
use patchhub_core::{
    operation,
    sources::{
        adapters::{builtin_adapter, kagura},
        http::{FakeFetcher, FetchRequest, FetchResponse, Fetcher, RequestContext},
        model::{PageFact, SourceIndexEntry, SourceRule},
        validate::validate_rule,
        RulesConfig, DEFAULT_RULES_JSON,
    },
};
use serde::Deserialize;
use serde_json::{json, Value as Json};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

const CODE: &str =
    include_str!("../../../../tests/fixtures/script-sources/kagura-en-prototype.lua");
const LIST: &str = include_str!("../../../../tests/fixtures/script-sources/kagura-en.html");
const CATEGORY: &str =
    include_str!("../../../../tests/fixtures/script-sources/kagura-en-category.html");
const DETAIL: &str =
    include_str!("../../../../tests/fixtures/script-sources/kagura-en-detail.html");
const DETAIL_URL: &str = "https://www.kaguragames.com/product/sacrifice-sanctuary-the-ritual-maidens-life-with-monsters-patch/";

fn invoke<T: DeserializeOwned>(
    code: &str,
    entry: &str,
    rule: &SourceRule,
    input: &Json,
    ctx: &mut RequestContext,
    cancel: Arc<AtomicBool>,
    progress: &mut Vec<String>,
) -> mlua::Result<runtime::Invocation<T>> {
    runtime::execute(
        code,
        entry,
        rule,
        std::slice::from_ref(input),
        ctx,
        cancel,
        |message| progress.push(message.to_owned()),
        &std::cell::Cell::new("script_source_runtime_error"),
    )
}

fn syntax_check(code: &[u8]) -> mlua::Result<()> {
    crate::sources::script::validate_code("prototype", 1, code)
        .map(|_| ())
        .map_err(mlua::Error::external)
}

fn rule() -> SourceRule {
    let config: RulesConfig = serde_json::from_str(DEFAULT_RULES_JSON).unwrap();
    validate_rule(
        config
            .sources
            .iter()
            .find(|rule| rule.id == "kagura-en")
            .unwrap(),
    )
    .unwrap()
}

fn response(html: &str, url: &str) -> FetchResponse {
    FetchResponse {
        status: 200,
        body: html.as_bytes().to_vec(),
        final_url: url.to_string(),
        ..FetchResponse::default()
    }
}

fn context(fake: &Arc<FakeFetcher>) -> RequestContext {
    RequestContext::with_settings(Box::new(Arc::clone(fake)), Duration::ZERO)
}

fn execute(code: &str, input: Json, fake: &Arc<FakeFetcher>) -> mlua::Result<Json> {
    invoke(
        code,
        "run",
        &rule(),
        &input,
        &mut context(fake),
        Arc::new(AtomicBool::new(false)),
        &mut Vec::new(),
    )
    .map(|result| result.value)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexOutput {
    entries: Vec<SourceIndexEntry>,
    pending: Vec<String>,
    complete: bool,
}

#[test]
fn recorded_non_table_list_matches_existing_rust_parser() {
    let mut rule = rule();
    rule.pages.truncate(1); // This test exercises the independent /patch/ root.
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(LIST, &rule.pages[0]));
    let mut progress = Vec::new();
    let actual = invoke::<IndexOutput>(
        CODE,
        "collect_index",
        &rule,
        &Json::Null,
        &mut context(&fake),
        Arc::new(AtomicBool::new(false)),
        &mut progress,
    )
    .unwrap();
    let expected_fake = Arc::new(FakeFetcher::new());
    expected_fake.push(response(LIST, &rule.pages[0]));
    let expected = builtin_adapter(&rule.adapter)
        .unwrap()
        .collect_index(
            &rule,
            None,
            &mut context(&expected_fake),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(actual.value.entries, expected.entries);
    assert_eq!(actual.value.pending, expected.pending);
    assert_eq!(actual.value.complete, expected.complete);
    assert_eq!(actual.value.entries.len(), 2);
    assert_eq!(fake.call_count(), 1);
    assert_eq!(progress, rule.pages);
    println!(
        "Lua 5.4 real-list sample VM allocations: {} bytes",
        actual.lua_bytes
    );
}

#[test]
fn recorded_detail_matches_rust_with_no_invented_steam_id() {
    let rule = rule();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(DETAIL, DETAIL_URL));
    let actual = invoke::<PageFact>(
        CODE,
        "resolve_page",
        &rule,
        &json!(DETAIL_URL),
        &mut context(&fake),
        Arc::new(AtomicBool::new(false)),
        &mut Vec::new(),
    )
    .unwrap()
    .value;
    let expected_fake = Arc::new(FakeFetcher::new());
    expected_fake.push(response(DETAIL, DETAIL_URL));
    let expected = kagura::resolve_page(
        &rule,
        kagura::adapter_name(&rule),
        DETAIL_URL,
        &mut context(&expected_fake),
    )
    .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual.mirrors.len(), 4);
    // The recorded live patch page contains no Steam app link. Synthetic
    // identity coverage below is deliberately separate from that evidence.
    assert!(actual.steam_ids.is_empty());
}

#[test]
fn synthetic_steam_link_on_recorded_detail_is_extracted() {
    let html = DETAIL.replace(
        "</div>",
        "<a href=\"https://store.steampowered.com/app/4294967295/\">Steam</a></div>",
    );
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(&html, DETAIL_URL));
    let fact = invoke::<PageFact>(
        CODE,
        "resolve_page",
        &rule(),
        &json!(DETAIL_URL),
        &mut context(&fake),
        Arc::new(AtomicBool::new(false)),
        &mut Vec::new(),
    )
    .unwrap()
    .value;
    assert_eq!(fact.steam_ids, [u32::MAX]);
}

#[test]
fn recorded_pagination_reaches_next_url_and_reports_network_failure() {
    let mut rule = rule();
    rule.pages.remove(0);
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(CATEGORY, &rule.pages[0]));
    fake.push_error("fixture_next_page_unavailable");
    fake.push_error("fixture_next_page_unavailable"); // Existing HTTP transport retries once.
    let error = invoke::<IndexOutput>(
        CODE,
        "collect_index",
        &rule,
        &Json::Null,
        &mut context(&fake),
        Arc::new(AtomicBool::new(false)),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("fixture_next_page_unavailable"),
        "{error}"
    );
    assert_eq!(
        fake.calls().last().unwrap().url,
        "https://www.kaguragames.com/product-category/patches/patch/page/2/"
    );
    // This prototype propagates exceptions. Publishing partial progress after
    // an exception is not promised by API v1.
}

#[test]
fn multi_root_and_pagination_use_the_same_api_with_a_synthetic_terminal_page() {
    let rule = rule();
    let fake = Arc::new(FakeFetcher::new());
    fake.push(response(LIST, &rule.pages[0]));
    fake.push(response(CATEGORY, &rule.pages[1]));
    // Reuse recorded cards as a synthetic terminal page. This checks traversal
    // and dedupe; it does not claim that the live catalog ends at page 2.
    fake.push(response(
        LIST,
        "https://www.kaguragames.com/product-category/patches/patch/page/2/",
    ));
    let result = invoke::<IndexOutput>(
        CODE,
        "collect_index",
        &rule,
        &Json::Null,
        &mut context(&fake),
        Arc::new(AtomicBool::new(false)),
        &mut Vec::new(),
    )
    .unwrap()
    .value;
    assert!(result.complete);
    assert!(result.pending.is_empty());
    assert_eq!(result.entries.len(), 2); // The two real roots list the same recorded products.
    assert_eq!(fake.call_count(), 3);
}

#[test]
fn explicit_environment_has_no_files_processes_or_loaders() {
    let value = execute(
        r#"return { run = function(ctx)
        for _, key in ipairs({"io", "os", "package", "debug", "ffi", "require", "dofile",
            "loadfile", "load", "coroutine", "collectgarbage", "print"}) do
            assert(_G[key] == nil, key)
        end
        assert(string.dump == nil)
        assert(_VERSION == "Lua 5.4")
        return true
    end }"#,
        Json::Null,
        &Arc::new(FakeFetcher::new()),
    )
    .unwrap();
    assert_eq!(value, true);
}

#[test]
fn import_compilation_does_not_execute_top_level_and_rejects_bytecode() {
    syntax_check(b"while true do end").unwrap();
    syntax_check(b"error('must not execute during import')").unwrap();
    assert!(syntax_check(b"return {").is_err());
    let lua = runtime::vm().unwrap();
    let binary = lua.load("return {}").into_function().unwrap().dump(false);
    assert!(syntax_check(&binary).is_err());
}

#[test]
fn owned_dom_handles_survive_parent_collection_and_stay_document_local() {
    let result = execute(
        r#"return { run = function(ctx)
        local child = ctx.html('<div data-x="one"><b>中文 &amp; text</b></div>'):select('b')[1]
        local other = ctx.html('<div data-x="two"><b>wrong</b></div>'):select('b')[1]
        local keep = {}
        for i = 1, 2000 do keep[i] = {i, i} end
        assert(child:text() == '中文 & text')
        assert(child:parent():attr('data-x') == 'one')
        assert(other:parent():attr('data-x') == 'two')
        assert(child:attr('absent') == nil)
        return ctx.url_join('https://www.kaguragames.com/catalog/', '../product/a/?x=1')
    end }"#,
        Json::Null,
        &Arc::new(FakeFetcher::new()),
    )
    .unwrap();
    assert_eq!(result, "https://www.kaguragames.com/product/a/?x=1");
}

#[test]
fn scoped_http_callback_expires_when_borrow_ends() {
    let lua = runtime::vm().unwrap();
    let fake = Arc::new(FakeFetcher::new());
    let mut ctx = context(&fake);
    let function = lua
        .scope(|scope| {
            scope.create_function_mut(|_, ()| {
                ctx.fetch("https://www.kaguragames.com/", &rule().page_hosts)
                    .map(|response| response.status)
                    .map_err(mlua::Error::external)
            })
        })
        .unwrap();
    assert!(function.call::<u16>(()).is_err());
    assert_eq!(fake.call_count(), 0);
}

#[test]
fn utf8_arrays_nulls_and_numeric_boundaries_roundtrip() {
    let input = json!({"empty": [], "object": {}, "null": null,
        "text": "中文🎮", "appid": u32::MAX, "integer": i64::MAX, "fraction": 1.25});
    let actual = execute(
        r#"return { run = function(ctx, rule, input)
        assert(input.null == ctx.null)
        assert(ctx.json_encode(ctx.array()) == '[]')
        assert(ctx.json_encode({}) == '{}')
        return ctx.json_decode(ctx.json_encode(input))
    end }"#,
        input.clone(),
        &Arc::new(FakeFetcher::new()),
    )
    .unwrap();
    assert_eq!(actual, input);
}

#[test]
fn malformed_values_css_and_script_errors_are_explicit() {
    for (code, expected) in [
        (
            "return {run = function() error('broken parser') end}",
            "broken parser",
        ),
        (
            "return {run = function(ctx) return ctx.html(''):select('[') end}",
            "invalid CSS selector",
        ),
        (
            "return {run = function(ctx) return ctx.json_encode({1, a=2}) end}",
            "mixed script table",
        ),
        (
            "return {run = function(ctx) return ctx.json_encode({[1]=1, [3]=3}) end}",
            "sparse script array",
        ),
        (
            "return {run = function(ctx) local t={} t.t=t return ctx.json_encode(t) end}",
            "recursive script table",
        ),
        (
            "return {run = function(ctx) return ctx.json_decode('18446744073709551615') end}",
            "signed 64-bit range",
        ),
        (
            "return {run = function() return math.huge end}",
            "non-finite script number",
        ),
        ("return {}", "function"),
        ("return 1", "module must return a table"),
    ] {
        let error = execute(code, Json::Null, &Arc::new(FakeFetcher::new())).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "expected {expected}: {error}"
        );
    }
    let fake = Arc::new(FakeFetcher::new());
    let error = invoke::<PageFact>(
        "return {resolve_page=function() return {steam_ids={'not an ID'}} end}",
        "resolve_page",
        &rule(),
        &Json::Null,
        &mut context(&fake),
        Arc::new(AtomicBool::new(false)),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("invalid type") || error.to_string().contains("missing field")
    );
}

#[test]
fn network_reuses_domain_redirect_cookie_form_and_encoding_policy() {
    let fake = Arc::new(FakeFetcher::new());
    let base = "https://www.kaguragames.com/";
    let mut first = response("form", base);
    first
        .headers
        .push(("Set-Cookie".into(), "lang=zh; Path=/".into()));
    fake.push(first);
    fake.push(FetchResponse {
        status: 302,
        location: Some("/done".into()),
        ..FetchResponse::default()
    });
    fake.push(response("完成", "https://www.kaguragames.com/done"));
    let result = execute(r#"return {run=function(ctx)
        assert(ctx.get('https://www.kaguragames.com/').text == 'form')
        return ctx.form('https://www.kaguragames.com/select', {{'lang', '简体 中文'}, {'lang', 'en'}}).text
    end}"#, Json::Null, &fake).unwrap();
    assert_eq!(result, "完成");
    let calls = fake.calls();
    assert_eq!(calls[1].method, "POST");
    assert_eq!(
        calls[1].body.as_deref(),
        Some(b"lang=%E7%AE%80%E4%BD%93+%E4%B8%AD%E6%96%87&lang=en".as_slice())
    );
    assert!(calls[1]
        .headers
        .iter()
        .any(|(key, value)| key == "Cookie" && value == "lang=zh"));
    assert_eq!(calls[2].method, "GET");
    let fake = Arc::new(FakeFetcher::new());
    let error = execute(
        "return {run=function(ctx) return ctx.get('https://example.com/') end}",
        Json::Null,
        &fake,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("invalid_discovery_url"),
        "{error}"
    );
    assert_eq!(fake.call_count(), 0);
    fake.push(FetchResponse {
        status: 302,
        location: Some("https://example.com/".into()),
        ..FetchResponse::default()
    });
    assert!(execute(
        "return {run=function(ctx) return ctx.get('https://www.kaguragames.com/') end}",
        Json::Null,
        &fake
    )
    .is_err());
    assert_eq!(fake.call_count(), 1);
}

// Synchronize cancellation with the first VM hook so this proves interruption
// during execution, rather than cancellation before the script starts.
fn cancel_running_chunk(code: &str) {
    let lua = runtime::vm().unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    let samples = Arc::new(AtomicUsize::new(0));
    let (started, wait) = mpsc::channel();
    let worker_flag = Arc::clone(&flag);
    let worker = std::thread::spawn(move || {
        wait.recv().unwrap();
        worker_flag.store(true, Ordering::Relaxed);
    });
    let observed = Arc::clone(&samples);
    let cancel = Arc::clone(&flag);
    lua.set_global_hook(
        HookTriggers::new().every_nth_instruction(10_000),
        move |_, _| {
            if observed.fetch_add(1, Ordering::Relaxed) == 0 {
                started.send(()).unwrap();
            }
            if cancel.load(Ordering::Relaxed) {
                Err(mlua::Error::runtime("task_cancelled"))
            } else {
                Ok(VmState::Continue)
            }
        },
    )
    .unwrap();
    let chunk = lua
        .load(code)
        .set_mode(ChunkMode::Text)
        .set_environment(runtime::environment(&lua, &flag).unwrap())
        .into_function()
        .unwrap();
    let error =
        operation::with_cancel(Arc::clone(&flag), || runtime::call(chunk, (), &flag)).unwrap_err();
    worker.join().unwrap();
    assert!(samples.load(Ordering::Relaxed) > 0);
    assert!(error.to_string().contains("task_cancelled"), "{error}");
}

#[test]
fn module_top_and_protected_infinite_loops_are_cancellable() {
    cancel_running_chunk("while true do end");
    cancel_running_chunk("pcall(function() while true do end end)");
    cancel_running_chunk("while true do pcall(function() while true do end end) end");
    cancel_running_chunk("xpcall(function() while true do end end, function(e) return e end)");
}

#[test]
fn nested_protected_loop_in_c_callback_is_cancellable() {
    cancel_running_chunk("string.gsub('x', 'x', function() while true do pcall(function() while true do end end) end end)");
    cancel_running_chunk("table.sort({3,2,1}, function() while true do pcall(function() while true do end end) end end)");
    cancel_running_chunk("xpcall(function() error('test') end, function() while true do pcall(function() while true do end end) end end)");
}

struct CancelOnRequest(Arc<AtomicBool>);
impl Fetcher for CancelOnRequest {
    fn fetch(&self, _: &FetchRequest) -> std::io::Result<FetchResponse> {
        self.0.store(true, Ordering::Relaxed);
        Err(std::io::Error::other("task_cancelled"))
    }
}

#[test]
fn caught_host_cancellation_cannot_publish_success() {
    let flag = Arc::new(AtomicBool::new(false));
    for code in [
        "return {run=function(ctx) pcall(ctx.get, 'https://www.kaguragames.com/'); return true end}",
        "return {run=function(ctx) pcall(ctx.get, 'https://www.kaguragames.com/'); while true do end end}",
    ] {
        flag.store(false, Ordering::Relaxed);
        let mut ctx = RequestContext::with_settings(Box::new(CancelOnRequest(Arc::clone(&flag))), Duration::ZERO);
        let error = invoke::<Json>(code, "run", &rule(), &Json::Null, &mut ctx,
            Arc::clone(&flag), &mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains("task_cancelled"), "{error}");
    }
}

#[test]
fn calls_have_independent_globals() {
    let code = "local count = 0; return {run=function() count=count+1; return count end}";
    let fake = Arc::new(FakeFetcher::new());
    assert_eq!(execute(code, Json::Null, &fake).unwrap(), 1);
    assert_eq!(execute(code, Json::Null, &fake).unwrap(), 1);
}

#[test]
fn protected_calls_keep_normal_lua_result_and_handler_semantics() {
    let result = execute(r#"return {run=function()
        local ok, a, b = pcall(function(x) return x, 2 end, '中文')
        assert(ok and a == '中文' and b == 2)
        local failed, message = pcall(function() error('ordinary failure') end)
        assert(not failed and tostring(message):find('ordinary failure', 1, true))
        local failed2, handled = xpcall(function(x) error(x) end, function(e) return 'handled:' .. tostring(e) end, 'boom')
        assert(not failed2 and handled:find('handled:', 1, true))
        return true
    end}"#, Json::Null, &Arc::new(FakeFetcher::new())).unwrap();
    assert_eq!(result, true);
}

#[test]
#[ignore = "P0 explicit resource/cancellation measurement; not a performance threshold"]
fn measure_c_library_cancellation_boundary() {
    // string.rep is one uninterruptible Lua C-library call. Request cancellation
    // while it is running, record the cost, then refuse to publish its result.
    let lua = runtime::vm().unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    let (started, wait) = mpsc::channel();
    let cancel = Arc::clone(&flag);
    let worker = std::thread::spawn(move || {
        wait.recv().unwrap();
        cancel.store(true, Ordering::Relaxed);
    });
    let env = runtime::environment(&lua, &flag).unwrap();
    env.set(
        "begin_measurement",
        lua.create_function(move |_, ()| {
            started.send(()).unwrap();
            Ok(())
        })
        .unwrap(),
    )
    .unwrap();
    let function = lua
        .load("begin_measurement(); local text=string.rep('x', 64*1024*1024); return #text")
        .set_environment(env)
        .into_function()
        .unwrap();
    let start = Instant::now();
    let error = operation::with_cancel(Arc::clone(&flag), || runtime::call(function, (), &flag))
        .unwrap_err();
    worker.join().unwrap();
    assert!(error.to_string().contains("task_cancelled"));
    println!(
        "64 MiB string.rep: elapsed={:?}, retained Lua bytes={}, test executable bytes={}",
        start.elapsed(),
        lua.used_memory(),
        std::fs::metadata(std::env::current_exe().unwrap())
            .unwrap()
            .len()
    );
}
