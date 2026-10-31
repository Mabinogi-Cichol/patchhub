use patchhub_core::sources::{
    self, adapters,
    http::{FakeFetcher, FetchResponse, RequestContext},
    identity, QueryContext, RawSourceRule,
};
use serde_json::{json, Value};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Duration;

fn query() -> QueryContext {
    QueryContext {
        appid: 42,
        name: "Game".into(),
        name_hints: vec![],
        force: false,
        legacy_match: false,
        source_context: vec![],
        parser_version: 2,
        query_version: 1,
        settings_revision: 0,
        generation: None,
    }
}
fn normalize(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for key in [
                "candidate_id",
                "fingerprint",
                "parser_fingerprint",
                "observed_at",
            ] {
                map.remove(key);
            }
            for value in map.values_mut() {
                normalize(value);
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize(item);
            }
        }
        _ => {}
    }
}
fn run(case: &Value) -> sources::Result<Value> {
    let raw: RawSourceRule = serde_json::from_value(case["rule"].clone()).unwrap();
    let rule = sources::validate::validate_rule(&raw)?;
    let fake = Arc::new(FakeFetcher::new());
    for response in case["responses"].as_array().unwrap() {
        fake.push(FetchResponse {
            status: 200,
            body: response["body"].as_str().unwrap().as_bytes().to_vec(),
            final_url: response["url"].as_str().unwrap().into(),
            ..FetchResponse::default()
        });
    }
    let mut ctx = RequestContext::with_settings(Box::new(Arc::clone(&fake)), Duration::ZERO);
    let cancel = Arc::new(AtomicBool::new(false));
    let mut output = match case["kind"].as_str().unwrap() {
        "index" => {
            let result = adapters::source_adapter(&rule, None)?
                .collect_index(&rule, None, &mut ctx, &cancel)?;
            json!({"entries":result.entries,"pending":result.pending,"complete":result.complete,"etag":result.etag,"modified":result.modified,"not_modified":result.not_modified})
        }
        "detail" => {
            let result = adapters::source_adapter(&rule, None)?.resolve_page(
                &rule,
                case["page"].as_str().unwrap(),
                &mut ctx,
                &cancel,
            )?;
            serde_json::to_value(result).unwrap()
        }
        "discover" => {
            let result = adapters::source_adapter(&rule, None)?.discover(
                &rule,
                &query(),
                &mut ctx,
                &adapters::DiscoverPrevious::default(),
                &cancel,
            )?;
            let candidates = identity::enrich_candidates(&result.candidates, &result.identities);
            json!({"candidates":candidates,"identities":result.identities.iter().map(|i|json!({"appid":i.appid,"page":i.page,"aliases":i.aliases})).collect::<Vec<_>>(),
                "diagnostics":result.diagnostics,"status":result.status})
        }
        _ => unreachable!(),
    };
    normalize(&mut output);
    Ok(
        json!({"result":output,"requests":fake.calls().iter().map(|r|json!({"url":r.url,"method":r.method,"body":r.body})).collect::<Vec<_>>()}),
    )
}

#[test]
fn builtin_scripts_match_recorded_legacy_facts_and_requests() {
    let inputs: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/builtin-migration/inputs.json")).unwrap();
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/builtin-migration/expected.json")).unwrap();
    for case in inputs {
        let name = case["name"].as_str().unwrap();
        assert_eq!(run(&case).unwrap(), expected[name], "{name}");
    }
}
