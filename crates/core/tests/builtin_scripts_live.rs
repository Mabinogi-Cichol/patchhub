//! Targeted metadata-only live evidence. No artifact downloads or installation.
use patchhub_core::sources::{self, adapters, http::RequestContext, SourceRule};
use scraper::{Html, Selector};
use std::sync::{atomic::AtomicBool, Arc};

#[test]
#[ignore = "live network: patch.moe index and one current detail for each Kagura site"]
fn bundled_publisher_metadata_live() {
    let config: sources::RulesConfig = serde_json::from_str(sources::DEFAULT_RULES_JSON).unwrap();
    let rule = |id: &str| -> SourceRule {
        sources::validate::validate_rule(config.sources.iter().find(|r| r.id == id).unwrap())
            .unwrap()
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
        let fact =
            adapters::source_adapter(&cn, None)?.resolve_page(&cn, &page, &mut ctx, &cancel)?;
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
    match adapters::source_adapter(&en, None)
        .unwrap()
        .resolve_page(&en, page, &mut ctx, &cancel)
    {
        Ok(fact) => println!(
            "kagura-en: page={} title={} steam_ids={:?} mirrors={}",
            fact.page,
            fact.title,
            fact.steam_ids,
            fact.mirrors.len()
        ),
        Err(error) => errors.push(format!("kagura-en: {error}")),
    }
    println!("logical metadata requests={}", ctx.visited.len());
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
