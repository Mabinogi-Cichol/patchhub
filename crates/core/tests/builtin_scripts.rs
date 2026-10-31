//! Bundled revisions, explicit user overrides and legacy-cache retirement.
use patchhub_core::sources::{self, builtin, snapshot::SourceSnapshot, Sources};
use serde_json::{json, Value};
use std::fs;

#[test]
fn bundled_documents_are_readable_without_state_or_execution() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    let bytes = fs::read(sources.store().paths().settings()).unwrap();
    for id in ["patchmoe", "kagura-cn", "kagura-en", "hgamer"] {
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
    let mut document = sources.get_script_source("patchmoe").unwrap();
    sources
        .registry_mut()
        .set_name("patchmoe", "用户名称")
        .unwrap();
    sources
        .registry_mut()
        .set_enabled("patchmoe", false)
        .unwrap();
    sources
        .registry_mut()
        .set_update_interval("patchmoe", 72)
        .unwrap();
    sources
        .registry_mut()
        .set_game_source(42, "patchmoe", false)
        .unwrap();
    document.rule.name = "作者修改名".into();
    let code = "while true do end\nreturn {}";
    sources
        .import_script_source(&document.rule, 1, code, true)
        .unwrap();
    let reopened = Sources::open(state.path()).unwrap();
    let summary = reopened
        .registry()
        .list_sources()
        .into_iter()
        .find(|s| s.rule.id == "patchmoe")
        .unwrap();
    assert!(!summary.rule.enabled);
    assert_eq!(summary.rule.name, "用户名称");
    assert_eq!(summary.interval_hours, 72);
    assert!(!reopened.registry().game_subscribed(42, "patchmoe"));
    assert_eq!(reopened.get_script_source("patchmoe").unwrap().code, code);
    // A later bundle/config cannot replace the explicit same-ID user rule.
    let mut rules: Value = serde_json::from_str(sources::DEFAULT_RULES_JSON).unwrap();
    rules["sources"][0]["name"] = json!("Later bundled name");
    rules["sources"][0]["pages"] = json!(["https://www.patch.moe/new/"]);
    let updated = Sources::open_with_rules(state.path(), &rules.to_string()).unwrap();
    assert_eq!(updated.get_script_source("patchmoe").unwrap().code, code);
    let before = fs::read(updated.store().paths().settings()).unwrap();
    let snapshot = SourceSnapshot::load(state.path(), &[], 0).unwrap().unwrap();
    assert_eq!(snapshot.get_script_source("patchmoe").unwrap().code, code);
    assert_eq!(
        fs::read(updated.store().paths().settings()).unwrap(),
        before
    );
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
