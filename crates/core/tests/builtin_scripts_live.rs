//! Targeted metadata-only live evidence. No artifact downloads or installation.
use patchhub_core::sources::{self, adapters, http::RequestContext, SourceRule};
use scraper::{Html, Selector};
use std::sync::{atomic::AtomicBool, Arc};

#[test]
#[ignore = "live network: patch.moe index and one current detail for each Kagura site"]
fn bundled_publisher_metadata_live() {
    let builtins = sources::builtin::default_builtins();
    let rule = |id: &str| -> SourceRule {
        builtins
            .iter()
            .find(|entry| entry.rule.id == id)
            .unwrap()
            .rule
            .clone()
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let mut errors = Vec::new();
    let mut ctx = RequestContext::new().unwrap();
    let patchmoe = rule("patchmoe");
    match adapters::source_adapter(&patchmoe, None)
        .unwrap()
        .collect_index(&patchmoe, None, &mut ctx, &cancel)
    {
        Ok(index) => {
            println!(
                "patchmoe: complete={} entries={} diagnostics={:?}",
                index.complete,
                index.entries.len(),
                index.diagnostics
            );
            if !index.complete || index.entries.is_empty() {
                errors.push("patchmoe index incomplete".to_owned());
            }
        }
        Err(error) => errors.push(format!("patchmoe: {error}")),
    }
    // Select a current article URL from the actual CN homepage in test code;
    // the production adapter alone parses the chosen detail.
    let cn = rule("kagura-cn");
    let result = (|| -> sources::Result<()> {
        let response = ctx.fetch(&cn.pages[0], &cn.page_hosts)?;
        if response.status != 200 {
            return Err(sources::invalid(format!("source_http_{}", response.status)));
        }
        let html = Html::parse_document(&response.text());
        let link = html
            .select(&Selector::parse("article .entry-title a[href]").unwrap())
            .next()
            .and_then(|n| n.attr("href"))
            .ok_or_else(|| sources::invalid("live_article_missing"))?;
        let page = url::Url::parse(&response.final_url)
            .unwrap()
            .join(link)
            .unwrap()
            .to_string();
        let state = adapters::script::DetailStepState {
            title: "live".into(),
            aliases: Vec::new(),
            page: page.clone(),
            appid: None,
            current_step: 1,
            total_steps: 2,
            step_data: None,
            details: Vec::new(),
        };
        let outcome =
            adapters::source_adapter(&cn, None)?.resolve_detail(&cn, &state, &mut ctx, &cancel)?;
        let fact = outcome
            .details
            .first()
            .ok_or_else(|| sources::invalid("live_detail_missing"))?;
        println!(
            "kagura-cn: page={} title={} steam_ids={:?} mirrors={}",
            fact.page,
            fact.title,
            fact.steam_ids,
            fact.mirrors.len()
        );
        Ok(())
    })();
    if let Err(error) = result {
        errors.push(format!("kagura-cn: {error}"));
    }
    let en = rule("kagura-en");
    let page = "https://www.kaguragames.com/product/sacrifice-sanctuary-the-ritual-maidens-life-with-monsters-patch/";
    let en_state = adapters::script::DetailStepState {
        title: "Sacrifice Sanctuary".into(),
        aliases: Vec::new(),
        page: page.into(),
        appid: None,
        current_step: 1,
        total_steps: 2,
        step_data: None,
        details: Vec::new(),
    };
    match adapters::source_adapter(&en, None)
        .unwrap()
        .resolve_detail(&en, &en_state, &mut ctx, &cancel)
    {
        Ok(outcome) => {
            if let Some(fact) = outcome.details.first() {
                println!(
                    "kagura-en: page={} title={} steam_ids={:?} mirrors={}",
                    fact.page,
                    fact.title,
                    fact.steam_ids,
                    fact.mirrors.len()
                );
            } else {
                errors.push("kagura-en: empty details".to_string());
            }
        }
        Err(error) => errors.push(format!("kagura-en: {error}")),
    }
    println!("logical metadata requests={}", ctx.visited.len());
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
#[ignore = "live network: Summer Memories base and DLC publisher metadata"]
fn kagura_summer_memories_base_and_dlc_live() {
    use sources::{matching, model::GameQuery, update, Sources};

    let state = tempfile::tempdir().unwrap();
    let mut registry = Sources::open(state.path()).unwrap();
    let document = registry.get_script_source("kagura-cn").unwrap();
    let modified_code = document.code.replacen(
        r#"pages = {"https://kagurafan.com/"}"#,
        r#"pages = { "https://kagurafan.com/category/补丁/夏日狂想曲：乡间的难忘回忆/", "https://kagurafan.com/category/补丁/夏日狂想曲/" }"#,
        1,
    );
    let imported = registry
        .import_script_source(None, 1, &modified_code)
        .unwrap()
        .rule;
    let cancel = Arc::new(AtomicBool::new(false));
    let mut ctx = RequestContext::new().unwrap();
    let update =
        update::update_source(registry.store(), &imported, true, &mut ctx, &cancel, |_| {})
            .unwrap();
    assert!(update.complete, "{update:?}");

    let game = GameQuery {
        appid: 1227890,
        name: "Summer Memories".into(),
        localized_name: Some("夏日狂想曲：乡间的难忘回忆".into()),
        related_dlc: vec![patchhub_core::steam::SteamDlc {
            appid: 1390150,
            name: Some("夏日狂想曲+".into()),
        }],
        instance_id: None,
    };
    let result = matching::match_games(
        registry.store(),
        &[imported],
        registry.registry().overrides(),
        &[game],
        &[],
        false,
        &mut ctx,
        &cancel,
        1,
        |_| {},
        |_| {},
    )
    .unwrap();
    let found = &result[0];
    for candidate in &found.candidates {
        println!(
            "title={} variant={:?} mirrors={}",
            candidate.patch_title,
            candidate.variant,
            candidate.mirrors.len()
        );
    }
    assert!(
        found
            .candidates
            .iter()
            .any(|candidate| candidate.game_appid == Some(1227890)
                && candidate.variant == sources::model::Variant::Base
                && !candidate.mirrors.is_empty()),
        "{found:?}"
    );
    assert!(
        found
            .candidates
            .iter()
            .any(|candidate| candidate.game_appid == Some(1227890)
                && candidate.variant == sources::model::Variant::Dlc
                && !candidate.mirrors.is_empty()),
        "{found:?}"
    );
}
