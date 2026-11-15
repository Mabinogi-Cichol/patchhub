//! Bundled revisions, read-only builtin sources and legacy user overrides.
use patchhub_core::sources::{self, builtin, snapshot::SourceSnapshot, Sources};
use serde_json::{json, Value};
use std::fs;

#[test]
fn bundled_sources_are_readable_and_read_only() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    let bytes = fs::read(sources.store().paths().settings()).unwrap();
    for id in sources.registry().rules().iter().map(|rule| &rule.id) {
        let document = sources.get_script_source(id).unwrap();
        assert_eq!(document.rule.adapter, "lua");
        assert_eq!(document.api_version, 1);
        assert_eq!(
            sources::script::validate_code(id, 1, document.code.as_bytes())
                .unwrap()
                .sha256,
            document.sha256
        );
        assert_eq!(document.sha256, builtin::document(id).unwrap().sha256);
    }
    assert_eq!(fs::read(sources.store().paths().settings()).unwrap(), bytes);
    // Bundled definitions change with the release, never through user imports.
    let mut document = sources.get_script_source("patchmoe").unwrap();
    document.rule.name = "作者修改名".into();
    assert_eq!(
        sources
            .import_script_source(&document.rule, 1, "return {}", true)
            .unwrap_err()
            .to_string(),
        "source_builtin_readonly"
    );
    assert_eq!(
        sources
            .registry_mut()
            .remove_source("patchmoe")
            .unwrap_err()
            .to_string(),
        "source_builtin_readonly"
    );
    assert_eq!(fs::read(sources.store().paths().settings()).unwrap(), bytes);
    assert_eq!(
        sources.get_script_source("patchmoe").unwrap().sha256,
        builtin::document("patchmoe").unwrap().sha256
    );
}

#[test]
fn yozakura_matches_english_steam_names_and_keeps_each_download() {
    use sources::{
        http::{FakeFetcher, FetchResponse, RequestContext},
        matching::match_games,
        model::{GameQuery, MatchBasis, Overrides},
        update::update_source,
    };
    use std::sync::{atomic::AtomicBool, Arc};
    use std::time::Duration;

    let state = tempfile::tempdir().unwrap();
    let sources = Sources::open(state.path()).unwrap();
    let rule = sources
        .registry()
        .rules()
        .iter()
        .find(|r| r.id == "yozakura")
        .unwrap();
    let roses = "https://yozakuragames.com/?p=161";
    let wife = "https://yozakuragames.com/?p=150";
    let steam = r#"<a href="https://store.steampowered.com/app/4370020/">Steam商店</a>"#;
    let roses_body = format!(
        r#"{steam}
        <a href="https://www.zakuragames.com/roses-cn.zip">DLC补丁下载 (中文版)</a>
        <a href="https://www.zakuragames.com/roses-jp.zip">DLCパッチダウンロード（日本語版）</a>
        <a href="https://www.zakuragames.com/roses-en.zip">DLC Patch Download（EN）</a>
        <a href="https://www.dropbox.com/share/roses.zip?dl=0">Dropbox</a>"#
    );
    let wife_body =
        format!(r#"{steam}<a href="https://www.zakuragames.com/wife.exe">补丁下载</a>"#);
    let posts = json!([
        {"link": roses, "title": {"rendered": "《被黑手浸染的玫瑰与百合》"}, "content": {"rendered": roses_body}},
        {"link": wife, "title": {"rendered": "《被击垮而堕落的强气人妻》"}, "content": {"rendered": wife_body}},
        {"link": "https://yozakuragames.com/?p=123", "title": {"rendered": "无补丁作品"}, "content": {"rendered": steam}},
    ]);
    let fake = Arc::new(FakeFetcher::new());
    let push = |url: &str, body: String| {
        fake.push(FetchResponse {
            status: 200,
            final_url: url.into(),
            body: body.into_bytes(),
            content_type: Some("text/html; charset=utf-8".into()),
            ..Default::default()
        })
    };
    push(&format!("{}&offset=0", rule.pages[0]), posts.to_string());
    let mut ctx = RequestContext::with_settings(Box::new(Arc::clone(&fake)), Duration::ZERO);
    let cancel = Arc::new(AtomicBool::new(false));
    let update = update_source(sources.store(), rule, false, &mut ctx, &cancel, |_| {}).unwrap();
    assert!(update.complete, "{update:?}");
    assert_eq!(update.status.entry_count, 2);
    push(
        roses,
        format!(
            r#"<div class="h-page-title__outer"><h1>《被黑手浸染的玫瑰与百合》</h1></div><div class="colibri-post-content">{roses_body}</div>"#
        ),
    );
    push(
        wife,
        format!(
            r#"<div class="h-page-title__outer"><h1>《被击垮而堕落的强气人妻》</h1></div><div class="colibri-post-content">{wife_body}</div>"#
        ),
    );
    let games = [
        GameQuery {
            appid: 4370020,
            name: "Stained Roses & Lilies".into(),
            localized_name: None,
            instance_id: None,
        },
        GameQuery {
            appid: 4376090,
            name: "Strong-Willed Wife".into(),
            localized_name: None,
            instance_id: None,
        },
    ];
    let matches = match_games(
        sources.store(),
        std::slice::from_ref(rule),
        &Overrides::default(),
        &games,
        &[],
        false,
        &mut ctx,
        &cancel,
        1,
        |_| {},
        |_| {},
    )
    .unwrap();
    assert_eq!(fake.call_count(), 3);
    let candidates = &matches[0].candidates;
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    let candidate = &candidates[0];
    assert_eq!(candidate.source_page, roses);
    assert_eq!(candidate.match_basis, MatchBasis::Appid);
    assert!(candidate.languages.is_empty());
    assert_eq!(
        candidate
            .mirrors
            .iter()
            .map(|mirror| (
                mirror.url.rsplit('/').next().unwrap(),
                mirror.label.as_deref().unwrap(),
            ))
            .collect::<Vec<_>>(),
        [
            ("roses-cn.zip", "DLC补丁下载 (中文版)"),
            ("roses-jp.zip", "DLCパッチダウンロード（日本語版）"),
            ("roses-en.zip", "DLC Patch Download（EN）"),
        ]
    );
    let candidate = &matches[1].candidates[0];
    assert_eq!(candidate.game_appid, Some(4376090));
    assert_eq!(candidate.source_page, wife);
    assert_eq!(candidate.mirrors[0].format, "exe");
}

#[test]
fn legacy_user_override_still_loads_and_deleting_it_restores_the_bundle() {
    let state = tempfile::tempdir().unwrap();
    let sources = Sources::open(state.path()).unwrap();
    let mut document = sources.get_script_source("patchmoe").unwrap();
    document.rule.name = "旧版用户覆盖".into();
    let code = "-- user override\nreturn {}\n";
    let descriptor = sources::script::validate_code("patchmoe", 1, code.as_bytes()).unwrap();
    let mut rule = sources::validate::validate_rule(&document.rule).unwrap();
    rule.script = Some(descriptor);
    let path = sources.store().paths().settings();
    let mut settings: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    settings["added"] = json!([serde_json::to_value(&rule).unwrap()]);
    settings["script_sources"] = json!({"patchmoe": code});
    fs::write(&path, serde_json::to_vec(&settings).unwrap()).unwrap();
    drop(sources);

    // An older settings file keeps its user override: it stays readable and editable.
    let mut sources = Sources::open(state.path()).unwrap();
    assert_eq!(sources.get_script_source("patchmoe").unwrap().code, code);
    let summary = sources
        .registry()
        .list_sources()
        .into_iter()
        .find(|s| s.rule.id == "patchmoe")
        .unwrap();
    assert_eq!(summary.origin, "added");
    assert_eq!(summary.rule.name, "旧版用户覆盖");
    // Deleting the user rule restores the bundled definition.
    sources.registry_mut().remove_source("patchmoe").unwrap();
    assert_eq!(
        sources.get_script_source("patchmoe").unwrap().sha256,
        builtin::document("patchmoe").unwrap().sha256
    );
    let summary = sources
        .registry()
        .list_sources()
        .into_iter()
        .find(|s| s.rule.id == "patchmoe")
        .unwrap();
    assert_eq!(summary.origin, "builtin");
}

#[test]
fn old_unbound_builtin_index_is_preserved_but_cannot_authorize_results() {
    let state = tempfile::tempdir().unwrap();
    let sources = Sources::open(state.path()).unwrap();
    let rule = sources
        .registry()
        .rules()
        .iter()
        .find(|r| r.id == "patchmoe")
        .unwrap();
    let old = sources::index::from_cache(
        "patchmoe",
        &sources::SourceCache {
            entries: vec![sources::SourceEntry {
                title: "Game".into(),
                aliases: vec![],
                url: "https://download.patch.moe/game.zip".into(),
                source_page: rule.pages[0].clone(),
                language: "en".into(),
                date: None,
                appid: Some(42),
            }],
            cached_at: Some(1),
            checked_at: Some(1),
            etag: None,
            modified: None,
        },
    );
    sources.store().save_index(&old).unwrap();
    let bytes = fs::read(sources.store().paths().index("patchmoe")).unwrap();
    assert!(!sources::index::compatible(&old, rule));
    let snapshot = SourceSnapshot::load(state.path(), &[], 0).unwrap().unwrap();
    assert_eq!(
        snapshot.get_script_source("patchmoe").unwrap().api_version,
        1
    );
    let summary = sources
        .registry()
        .list_sources()
        .into_iter()
        .find(|s| s.rule.id == "patchmoe")
        .unwrap();
    assert_eq!(summary.status, "not_refreshed");
    assert_eq!(summary.entry_count, 0);
    assert_eq!(
        fs::read(sources.store().paths().index("patchmoe")).unwrap(),
        bytes
    );
}
