use patchhub_core::{
    installer::resolve,
    sources::{
        browser::{BrowserFetcher, BrowserResponse},
        http::RequestContext,
        matching,
        model::*,
        update, Sources,
    },
};
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

const URL: &str = "https://www.patch.moe/?flag=1";
const ROOT: &str = r#"<form name="form_en" method="post" action="/"><input name="lang" value="en"></form>
<form name="form_zh_tw" method="post" action="/"><input name="lang" value="zh"></form>
<form name="form_zh_cn" method="post" action="/"><input name="lang" value="cn"></form>"#;
fn response(url: &str, method: &str, body: Option<&str>, text: &str) -> BrowserResponse {
    BrowserResponse {
        url: url.into(),
        method: method.into(),
        body: body.map(Into::into),
        response_url: url.into(),
        status: 200,
        text: text.into(),
    }
}
fn table(title: &str) -> String {
    format!(
        r#"<table><tr><td class="title">{title}</td><td class="release">2025/1/21</td><td class="patch"><a href="https://download.patch.moe/SACRIFICE_VILLAINS_Patch.zip">DLC</a></td></tr></table>"#
    )
}
fn context(pages: Vec<BrowserResponse>) -> RequestContext {
    RequestContext::with_settings(Box::new(BrowserFetcher::new(pages)), Duration::ZERO)
}
fn rule(s: &Sources) -> SourceRule {
    s.registry()
        .rules()
        .iter()
        .find(|r| r.id == "patchmoe")
        .unwrap()
        .clone()
}
fn cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[test]
fn captured_three_language_pages_use_real_lua_update_matching_and_zip_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let s = Sources::open(dir.path()).unwrap();
    let rule = rule(&s);
    let mut pages = vec![response(URL, "GET", None, ROOT)];
    for (lang, title) in [
        ("en", "SACRIFICE VILLAINS"),
        ("zh", "SACRIFICE VILLAINS：犧牲惡棍"),
        ("cn", "SACRIFICE VILLAINS：牺牲恶棍"),
    ] {
        pages.push(response(
            "https://www.patch.moe/",
            "POST",
            Some(&format!("lang={lang}")),
            &table(title),
        ));
    }
    let mut ctx = context(pages);
    let result =
        update::update_source(s.store(), &rule, true, &mut ctx, &cancel(), |_| {}).unwrap();
    assert!(result.complete);
    assert_eq!(result.status.entry_count, 1);
    let game = GameQuery {
        appid: 3137110,
        name: "SACRIFICE VILLAINS".into(),
        localized_name: Some("SACRIFICE VILLAINS：牺牲恶棍".into()),
        instance_id: None,
    };
    let matches = matching::match_games(
        s.store(),
        &[rule],
        s.registry().overrides(),
        std::slice::from_ref(&game),
        &[],
        false,
        &mut ctx,
        &cancel(),
        500,
        |_| {},
        |_| {},
    )
    .unwrap();
    assert_eq!(matches[0].candidates.len(), 1);
    assert_eq!(matches[0].status, GameMatchStatus::Incomplete);
    assert_eq!(matches[0].candidates[0].match_basis, MatchBasis::Title);
    let reopened = Sources::open(dir.path()).unwrap();
    let candidate = reopened
        .registry()
        .dynamic_zip_candidates(&game)
        .into_iter()
        .next()
        .unwrap();
    let selection = resolve::zip_selection_id(&candidate).unwrap();
    let resolved = resolve::resolve_zip(reopened.registry(), &game, &selection).unwrap();
    assert_eq!(
        resolved.download_url,
        "https://download.patch.moe/SACRIFICE_VILLAINS_Patch.zip"
    );
}

#[test]
fn verification_and_age_pages_record_attempt_without_publishing_empty_index() {
    for (text,code) in [("<title>One moment, please...</title>Please wait while your request is being verified...","source_browser_verification_required"),
        ("<h1>Age Verification</h1>Are you 18 years or older?","source_age_verification_required")] {
        let dir=tempfile::tempdir().unwrap();let s=Sources::open(dir.path()).unwrap();let rule=rule(&s);
        let result=update::update_source(s.store(),&rule,true,&mut context(vec![response(URL,"GET",None,text)]),&cancel(),|_|{}).unwrap();
        assert!(!result.complete);assert_eq!(result.status.code,code);assert!(result.status.checked_at>0);
        assert!(s.store().load_index("patchmoe").unwrap().is_none());
        let reopened=Sources::open(dir.path()).unwrap();
        let summary=reopened.registry().list_sources().into_iter().find(|v|v.rule.id=="patchmoe").unwrap();
        assert_eq!(summary.status,code);assert!(summary.checked_at.is_some());assert_eq!(summary.entry_count,0);
    }
}

#[test]
fn browser_response_must_match_lua_request() {
    let dir = tempfile::tempdir().unwrap();
    let s = Sources::open(dir.path()).unwrap();
    let rule = rule(&s);
    let pages = vec![
        response(URL, "GET", None, ROOT),
        response(
            "https://www.patch.moe/",
            "POST",
            Some("lang=wrong"),
            &table("Other Game"),
        ),
    ];
    let result = update::update_source(
        s.store(),
        &rule,
        true,
        &mut context(pages),
        &cancel(),
        |_| {},
    )
    .unwrap();
    assert!(!result.complete);
    assert!(s.store().load_index("patchmoe").unwrap().is_none());
}
