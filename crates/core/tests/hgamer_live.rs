//! Live HGamer verification (`#[ignore]`; never part of the default suite).
//!
//! Run with `cargo test -p patchhub-core --test hgamer_live -- --ignored
//! --nocapture` to re-check the server-rendered search summary/detail parsing
//! against the real site. Only list/detail metadata pages are requested; no
//! artifact is downloaded and no page is persisted.
use patchhub_core::sources::adapters::{self, DiscoverPrevious};
use patchhub_core::sources::http::RequestContext;
use patchhub_core::sources::model::{QueryContext, RulesConfig, SourceRule};
use patchhub_core::sources::{validate, DEFAULT_RULES_JSON, PARSER_VERSION, QUERY_VERSION};
use std::time::Instant;

fn live_rule() -> SourceRule {
    let config: RulesConfig = serde_json::from_str(DEFAULT_RULES_JSON).unwrap();
    config
        .sources
        .iter()
        .map(|raw| validate::validate_rule(raw).unwrap())
        .find(|rule| rule.id == "hgamer")
        .unwrap()
}

fn run(appid: u32, name: &str) {
    let rule = live_rule();
    let query = QueryContext {
        appid,
        name: name.to_string(),
        name_hints: Vec::new(),

        legacy_match: false,
        force: true,
        source_context: Vec::new(),
        parser_version: PARSER_VERSION,
        query_version: QUERY_VERSION,
        settings_revision: 1,
        generation: None,
    };
    let mut ctx = RequestContext::new().unwrap();
    let started = Instant::now();
    let outcome = adapters::source_adapter(&rule, None)
        .unwrap()
        .discover(
            &rule,
            &query,
            &mut ctx,
            &DiscoverPrevious::default(),
            &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();
    let mut visited: Vec<String> = ctx.visited.into_iter().collect();
    visited.sort();
    println!("\n=== appid={appid} name={name}");
    println!("elapsed_ms={}", started.elapsed().as_millis());
    println!("logical_requests={}", visited.len());
    for url in &visited {
        println!("  GET {url}");
    }
    println!(
        "status={} candidates={} identities={} diagnostics={:?}",
        outcome.status,
        outcome.candidates.len(),
        outcome.identities.len(),
        outcome
            .diagnostics
            .iter()
            .map(|d| d.code.clone())
            .collect::<Vec<_>>()
    );
    for candidate in &outcome.candidates {
        println!(
            "  candidate appid={:?} basis={:?} title={:?} page={} mirrors={:?}",
            candidate.game_appid,
            candidate.match_basis,
            candidate.patch_title,
            candidate.source_page,
            candidate
                .mirrors
                .iter()
                .map(|m| (m.url.clone(), m.format.clone()))
                .collect::<Vec<_>>(),
        );
    }
}

#[test]
#[ignore = "live network: verifies the server-rendered HGamer search/detail pages"]
fn live_hgamer_discover() {
    run(1_250_180, "Re:從H開始的異世界生活");
    run(1_227_890, "夏日狂想曲：乡间的难忘回忆");
}
