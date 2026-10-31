//! Bundled revisions, read-only builtin sources and legacy user overrides.
use patchhub_core::sources::{self, builtin, snapshot::SourceSnapshot, Sources};
use serde_json::{json, Value};
use std::fs;

#[test]
fn bundled_sources_are_readable_and_read_only() {
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
