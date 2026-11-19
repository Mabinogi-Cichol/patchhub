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
        let bundled_file = match id.as_str() {
            "patchmoe" => include_str!("../resources/source-scripts/patchmoe.lua"),
            "kagura-cn" => include_str!("../resources/source-scripts/kagura-cn.lua"),
            "kagura-en" => include_str!("../resources/source-scripts/kagura-en.lua"),
            "yozakura" => include_str!("../resources/source-scripts/yozakura.lua"),
            "hgamer" => include_str!("../resources/source-scripts/hgamer.lua"),
            _ => panic!("unexpected bundled source {id}"),
        };
        assert_eq!(document.code, bundled_file);
        assert_eq!(builtin::document(id).unwrap().code, bundled_file);
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
    let document = sources.get_script_source("patchmoe").unwrap();
    assert_eq!(document.name, "Shiravune");
    assert_eq!(
        sources
            .import_script_source(
                Some("patchmoe"),
                1,
                r#"return { meta = { pages = { "https://www.patch.moe/" } }, collect_index = function() return { entries = {}, complete = true } end }"#,
            )
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
        {"link": format!("{roses}&utm_source=steam#patch"), "title": {"rendered": "《被黑手浸染的玫瑰与百合》"}, "content": {"rendered": roses_body}},
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
            related_dlc: Vec::new(),
            instance_id: None,
        },
        GameQuery {
            appid: 4376090,
            name: "Strong-Willed Wife".into(),
            localized_name: None,
            related_dlc: Vec::new(),
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
    assert_eq!(
        fake.calls()[1..]
            .iter()
            .map(|request| request.url.as_str())
            .collect::<Vec<_>>(),
        [roses, wife]
    );
    for page in [roses, wife] {
        let detail = sources
            .store()
            .load_detail(&rule.id, page)
            .unwrap()
            .unwrap();
        assert_eq!(detail.details[0].page, page);
    }
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
    let document = sources.get_script_source("patchmoe").unwrap();
    let code = "-- user override\nreturn {}\n";
    let descriptor = sources::script::validate_code("patchmoe", 1, code.as_bytes()).unwrap();
    let path = sources.store().paths().settings();
    let mut settings: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    settings["schema_version"] = json!(3);
    settings["added"] = json!([{
        "id": document.rule.id,
        "name": "旧版用户覆盖",
        "adapter": "lua",
        "role": "publisher",
        "discovery_only": false,
        "priority": document.rule.priority,
        "enabled": true,
        "pages": document.rule.pages,
        "page_hosts": document.rule.page_hosts,
        "download_hosts": document.rule.download_hosts,
        "root_layout": "game_root",
        "script": descriptor,
    }]);
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
    assert_eq!(summary.name, "旧版用户覆盖");
    assert!(summary.rule.official_title_match);
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
    assert_eq!(summary.name, "Shiravune");
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
#[test]
fn retired_table_rules_are_filtered_until_the_next_settings_write() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    let user_code = r#"return { meta = { name = "User Lua", pages = { "https://user-lua.example/" } }, collect_index = function() return { entries = {}, complete = true } end }"#;
    let user_id = sources
        .import_script_source(None, 1, user_code)
        .unwrap()
        .rule
        .id;

    let path = sources.store().paths().settings();
    let mut settings: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for (id, adapter) in [("old-table", "table"), ("old-html", "table_html")] {
        let mut retired = settings["added"][0].clone();
        retired["id"] = json!(id);
        retired["adapter"] = json!(adapter);
        settings["added"].as_array_mut().unwrap().push(retired);
        settings["names"][id] = json!("Retired source");
        settings["enabled"][id] = json!(false);
        settings["intervals"][id] = json!(0);
    }
    let bytes = serde_json::to_vec(&settings).unwrap();
    fs::write(&path, &bytes).unwrap();
    drop(sources);

    let snapshot = SourceSnapshot::load(state.path(), &[], 0).unwrap().unwrap();
    assert!(snapshot.get_script_source(&user_id).is_ok());
    assert!(snapshot.get_script_source("old-table").is_err());
    assert!(snapshot.get_script_source("old-html").is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);

    let sources = Sources::open(state.path()).unwrap();
    let loaded = sources.store().load_settings().unwrap();
    assert_eq!(loaded.added.len(), 1);
    assert_eq!(loaded.added[0].id, user_id);
    assert!(sources.get_script_source("patchmoe").is_ok());
    assert!(sources.get_script_source("old-table").is_err());
    assert!(sources.get_script_source("old-html").is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);

    sources.store().update_settings(|_| Ok(())).unwrap();
    let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["added"].as_array().unwrap().len(), 1);
    for field in ["names", "enabled", "intervals"] {
        for id in ["old-table", "old-html"] {
            assert!(saved[field].get(id).is_none());
        }
    }
    assert_eq!(saved["script_sources"][&user_id], user_code);
}

#[test]
fn empty_builtin_registry_recovers_all_five_sources_via_ordinary_import() {
    use sources::{
        http::{FakeFetcher, FetchResponse, RequestContext},
        matching::match_games,
        model::{GameMatchStatus, GameQuery, Overrides, SourceRole},
        update::update_source,
    };
    use std::sync::{atomic::AtomicBool, Arc};
    use std::time::Duration;

    let state = tempfile::tempdir().unwrap();
    let mut sources =
        Sources::open_with_rules(state.path(), r#"{"schema_version":2,"sources":[]}"#).unwrap();
    assert!(sources.registry().rules().is_empty());

    let scripts = [
        (
            "patch-moe",
            "Shiravune",
            SourceRole::Publisher,
            include_str!("../resources/source-scripts/patchmoe.lua"),
        ),
        (
            "kagurafan-com",
            "Kagura Games 国内",
            SourceRole::Publisher,
            include_str!("../resources/source-scripts/kagura-cn.lua"),
        ),
        (
            "kaguragames-com",
            "Kagura Games 海外",
            SourceRole::Publisher,
            include_str!("../resources/source-scripts/kagura-en.lua"),
        ),
        (
            "yozakuragames-com",
            "YOZAKURA",
            SourceRole::Publisher,
            include_str!("../resources/source-scripts/yozakura.lua"),
        ),
        (
            "hgamer-tw",
            "HGamer 补丁索引",
            SourceRole::FallbackIndexer,
            include_str!("../resources/source-scripts/hgamer.lua"),
        ),
    ];

    for (expected_id, expected_name, expected_role, code) in scripts {
        let imported = sources.import_script_source(None, 1, code).unwrap();
        assert_eq!(imported.rule.id, expected_id);
        assert_eq!(imported.name, expected_name);
        assert_eq!(imported.rule.role, expected_role);
        let summary = sources
            .registry()
            .list_sources()
            .into_iter()
            .find(|s| s.rule.id == expected_id)
            .unwrap();
        assert_eq!(summary.origin, "added");
        assert_eq!(summary.name, expected_name);
    }
    drop(sources);

    let sources =
        Sources::open_with_rules(state.path(), r#"{"schema_version":2,"sources":[]}"#).unwrap();
    assert_eq!(sources.registry().rules().len(), 5);

    // Exercise one-shot (`patchmoe`), multi-step category (`kagura-cn`), and recovered metadata.
    let patchmoe_fake = Arc::new(FakeFetcher::new());
    patchmoe_fake.push(FetchResponse {
        status: 200,
        final_url: "https://www.patch.moe/?flag=1".into(),
        body: br#"<form name="form_zh_cn" method="post" action="/zh"><input type="hidden" name="lang" value="zh-CN"></form>"#.to_vec(),
        ..FetchResponse::default()
    });
    patchmoe_fake.push(FetchResponse {
        status: 200,
        final_url: "https://www.patch.moe/zh".into(),
        body: br#"<table><tr><td class="title">Recovered Game</td><td class="release">2026/1/2</td><td class="patch"><a href="https://download.patch.moe/recovered.zip">dl</a></td></tr></table>"#.to_vec(),
        ..FetchResponse::default()
    });
    let patchmoe_rule = sources
        .registry()
        .rules()
        .iter()
        .find(|r| r.id == "patch-moe")
        .unwrap()
        .clone();
    let mut ctx =
        RequestContext::with_settings(Box::new(Arc::clone(&patchmoe_fake)), Duration::ZERO);
    let cancel = Arc::new(AtomicBool::new(false));
    let patchmoe_out = update_source(
        sources.store(),
        &patchmoe_rule,
        true,
        &mut ctx,
        &cancel,
        |_| {},
    )
    .unwrap();
    assert!(patchmoe_out.complete);
    let patchmoe_idx = sources.store().load_index("patch-moe").unwrap().unwrap();
    assert!(patchmoe_idx.entries[0].is_complete());

    let kagura_cn_rule = sources
        .registry()
        .rules()
        .iter()
        .find(|r| r.id == "kagurafan-com")
        .unwrap()
        .clone();
    let cn_fake = Arc::new(FakeFetcher::new());
    cn_fake.push(FetchResponse {
        status: 200,
        final_url: "https://kagurafan.com/".into(),
        body: br#"<html><body><a href="/category/recovered/">Recovered Game</a></body></html>"#
            .to_vec(),
        ..FetchResponse::default()
    });
    let mut cn_ctx = RequestContext::with_settings(Box::new(Arc::clone(&cn_fake)), Duration::ZERO);
    let cn_out = update_source(
        sources.store(),
        &kagura_cn_rule,
        true,
        &mut cn_ctx,
        &cancel,
        |_| {},
    )
    .unwrap();
    assert!(cn_out.complete);
    let cn_idx = sources
        .store()
        .load_index("kagurafan-com")
        .unwrap()
        .unwrap();
    assert!(!cn_idx.entries[0].is_complete());

    let match_fake = Arc::new(FakeFetcher::new());
    match_fake.push(FetchResponse {
        status: 200,
        final_url: "https://kagurafan.com/category/recovered/".into(),
        body: br#"<html><body><h1>Recovered Category</h1><article><h2 class="entry-title"><a href="/article/recovered/">Recovered Game</a></h2></article></body></html>"#.to_vec(),
        ..FetchResponse::default()
    });
    match_fake.push(FetchResponse {
        status: 200,
        final_url: "https://kagurafan.com/article/recovered/".into(),
        body: br#"<html><body><article><h1 class="entry-title">Recovered Game</h1><div class="entry-content"><a href="https://store.steampowered.com/app/42/">Steam</a><a href="https://kaguraserver.com/r.zip">Mirror 1</a></div></article></body></html>"#.to_vec(),
        ..FetchResponse::default()
    });
    let mut match_ctx =
        RequestContext::with_settings(Box::new(Arc::clone(&match_fake)), Duration::ZERO);
    let matches = match_games(
        sources.store(),
        sources.registry().rules(),
        &Overrides::default(),
        &[GameQuery {
            appid: 42,
            name: "Recovered Game".into(),
            localized_name: None,
            related_dlc: Vec::new(),
            instance_id: None,
        }],
        &[],
        false,
        &mut match_ctx,
        &cancel,
        200,
        |_| {},
        |_| {},
    )
    .unwrap();
    assert_eq!(matches[0].status, GameMatchStatus::PublisherMatch);
    assert!(matches[0]
        .candidates
        .iter()
        .any(|c| c.source_id == "patch-moe"
            && c.mirrors[0].url == "https://download.patch.moe/recovered.zip"));
    assert!(matches[0]
        .candidates
        .iter()
        .any(|c| c.source_id == "kagurafan-com"
            && c.mirrors[0].url == "https://kaguraserver.com/r.zip"));
}
