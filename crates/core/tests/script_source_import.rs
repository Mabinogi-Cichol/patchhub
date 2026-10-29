//! P1: import/read/persistence only. No source script is executed or fetched.
use patchhub_core::sources::{
    script::validate_code, snapshot::SourceSnapshot, GameQuery, RawSourceRule, SourceRole, Sources,
    SOURCE_SETTINGS_SCHEMA_VERSION,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
};

const CODE: &str = "-- 中文 source\r\nerror('top level must not run')\r\nreturn {}\r\n";

fn rule(id: &str) -> RawSourceRule {
    serde_json::from_value(json!({"id":id,"name":"Custom source","adapter":"lua",
        "enabled":true,"pages":["https://example.com/"],"page_hosts":["example.com"],
        "download_hosts":["download.example.com"],"root_layout":"game_root"}))
    .unwrap()
}

fn bytes(sources: &Sources) -> Vec<u8> {
    fs::read(sources.store().paths().settings()).unwrap()
}
fn fingerprint(sources: &Sources, id: &str) -> String {
    Sources::rule_fingerprint(
        sources
            .registry()
            .rules()
            .iter()
            .find(|rule| rule.id == id)
            .unwrap(),
    )
}
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(tree(&path));
        } else {
            files.insert(path.clone(), fs::read(path).unwrap());
        }
    }
    files
}

#[test]
fn import_compiles_only_and_persists_one_complete_document() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    let original = sources.settings_revision();
    let legacy_fingerprints: Vec<_> = sources
        .registry()
        .rules()
        .iter()
        .map(Sources::rule_fingerprint)
        .collect();
    let imported = sources
        .import_script_source(&rule("custom"), 1, CODE, false)
        .unwrap();
    assert_eq!(imported.settings_revision, original + 1);
    assert_eq!(
        imported.rule.script.as_ref().unwrap().sha256,
        format!("{:x}", Sha256::digest(CODE.as_bytes()))
    );
    let settings: Value = serde_json::from_slice(&bytes(&sources)).unwrap();
    assert_eq!(settings["revision"], original + 1);
    assert_eq!(settings["schema_version"], SOURCE_SETTINGS_SCHEMA_VERSION);
    assert_eq!(settings["script_sources"]["custom"], CODE);
    assert_eq!(
        settings["added"][0]["script"]["sha256"],
        imported.rule.script.as_ref().unwrap().sha256
    );
    for (index, expected) in legacy_fingerprints.iter().enumerate() {
        assert_eq!(
            &Sources::rule_fingerprint(&sources.registry().rules()[index]),
            expected
        );
        assert!(serde_json::to_value(&sources.registry().rules()[index])
            .unwrap()
            .get("script")
            .is_none());
    }
    let metadata = serde_json::to_string(&sources.registry().list_sources()).unwrap();
    assert!(!metadata.contains("top level must not run"));
    drop(sources);
    let reopened = Sources::open(state.path()).unwrap();
    let document = reopened.get_script_source("custom").unwrap();
    assert_eq!(document.code, CODE);
    assert_eq!(document.api_version, 1);
    assert!(serde_json::to_value(&document.rule)
        .unwrap()
        .get("script")
        .is_none());
    assert_eq!(document.sha256, imported.rule.script.unwrap().sha256);
}

#[test]
fn invalid_imports_and_implicit_overwrites_do_not_modify_settings() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    let initial = bytes(&sources);
    for (raw, version, code, replace, expected) in [
        (
            rule("custom"),
            2,
            "return {}",
            false,
            "unsupported_script_api_version",
        ),
        (
            rule("custom"),
            1,
            "return {",
            false,
            "script_source_syntax_error",
        ),
        (rule("custom"), 1, "return {}", true, "unknown_source"),
        (rule("patchmoe"), 1, "return {}", true, "source_not_script"),
        (
            rule("patchmoe"),
            1,
            "return {}",
            false,
            "script_source_exists",
        ),
    ] {
        let error = sources
            .import_script_source(&raw, version, code, replace)
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(bytes(&sources), initial);
        assert_eq!(sources.settings_revision(), 1);
    }
    let mut wrong = rule("bad");
    wrong.adapter = "patchmoe".into();
    assert_eq!(
        sources
            .import_script_source(&wrong, 1, CODE, false)
            .unwrap_err()
            .to_string(),
        "source_not_script"
    );
    assert!(sources.registry_mut().add_source(&rule("bypass")).is_err());
    assert_eq!(bytes(&sources), initial);
    assert_eq!(
        validate_code("bad", 1, &[0xff]).unwrap_err().to_string(),
        "invalid_script_source_utf8"
    );
    assert!(validate_code("bad", 1, b"\x1bLua").is_err());
    sources
        .import_script_source(&rule("custom"), 1, CODE, false)
        .unwrap();
    let before = bytes(&sources);
    assert_eq!(
        sources
            .import_script_source(&rule("custom"), 1, "return {}", false)
            .unwrap_err()
            .to_string(),
        "script_source_exists"
    );
    assert_eq!(bytes(&sources), before);
}

#[test]
fn explicit_replace_preserves_user_settings_and_changes_parsing_identity() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    sources
        .import_script_source(&rule("custom"), 1, CODE, false)
        .unwrap();
    sources.registry_mut().set_enabled("custom", false).unwrap();
    sources
        .registry_mut()
        .set_name("custom", "My name")
        .unwrap();
    sources
        .registry_mut()
        .set_game_source(42, "custom", false)
        .unwrap();
    sources
        .registry_mut()
        .set_update_interval("custom", 12)
        .unwrap();
    let old = fingerprint(&sources, "custom");
    sources
        .registry_mut()
        .set_name("custom", "New display name")
        .unwrap();
    assert_eq!(fingerprint(&sources, "custom"), old);
    let before = sources.settings_revision();
    let replaced = sources
        .import_script_source(&rule("custom"), 1, "return {}", true)
        .unwrap();
    assert_eq!(replaced.settings_revision, before + 1);
    assert!(!replaced.rule.enabled);
    assert_ne!(fingerprint(&sources, "custom"), old);
    assert!(!sources.registry().game_subscribed(42, "custom"));
    let summary = sources
        .registry()
        .list_sources()
        .into_iter()
        .find(|s| s.rule.id == "custom")
        .unwrap();
    assert_eq!(summary.rule.name, "New display name");
    assert_eq!(summary.interval_hours, 12);
    let mut fallback = rule("custom");
    fallback.role = Some("fallback_indexer".into());
    fallback.discovery_only = Some(true);
    fallback.root_layout = None;
    fallback.download_hosts = Some(vec![]);
    sources
        .import_script_source(&fallback, 1, CODE, true)
        .unwrap();
    assert_eq!(
        sources.registry().rules().last().unwrap().role,
        SourceRole::FallbackIndexer
    );
    assert_eq!(sources.store().settings().unwrap().intervals["custom"], 12);
    assert!(sources
        .registry_mut()
        .set_update_interval("custom", 24)
        .is_err());
}

#[test]
fn snapshot_and_edit_reads_capture_the_same_rule_and_code_without_writes() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    sources
        .import_script_source(&rule("custom"), 1, CODE, false)
        .unwrap();
    let before = tree(state.path());
    let snapshot = SourceSnapshot::load(
        state.path(),
        &[GameQuery {
            appid: 42,
            name: "Game".into(),
            localized_name: None,
            instance_id: None,
        }],
        1,
    )
    .unwrap()
    .unwrap();
    assert_eq!(snapshot.get_script_source("custom").unwrap().code, CODE);
    assert_eq!(tree(state.path()), before);
    sources
        .import_script_source(&rule("custom"), 1, "while true do end", true)
        .unwrap();
    assert_eq!(snapshot.get_script_source("custom").unwrap().code, CODE);
    assert_eq!(
        sources.get_script_source("custom").unwrap().code,
        "while true do end"
    );
    let reopened = SourceSnapshot::load(state.path(), &[], 2).unwrap().unwrap();
    assert_eq!(
        reopened.get_script_source("custom").unwrap().code,
        "while true do end"
    );
    assert!(serde_json::to_string(&sources.registry().list_sources())
        .unwrap()
        .contains("sha256"));
}

#[test]
fn digest_mismatch_and_missing_code_are_source_errors_without_fallback() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    sources
        .import_script_source(&rule("custom"), 1, CODE, false)
        .unwrap();
    let path = sources.store().paths().settings();
    let original: Value = serde_json::from_slice(&bytes(&sources)).unwrap();
    for missing in [false, true] {
        let mut damaged = original.clone();
        if missing {
            damaged["script_sources"]
                .as_object_mut()
                .unwrap()
                .remove("custom");
        } else {
            damaged["script_sources"]["custom"] = json!("return {}");
        }
        fs::write(&path, serde_json::to_vec(&damaged).unwrap()).unwrap();
        let snapshot = SourceSnapshot::load(state.path(), &[], 1).unwrap().unwrap();
        let error = snapshot
            .get_script_source("custom")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("script_source_digest_mismatch") && error.contains("custom"),
            "{error}"
        );
        let reopened = Sources::open(state.path()).unwrap();
        assert!(reopened.get_script_source("custom").is_err());
        assert_eq!(reopened.registry().rules().len(), 5); // Other sources remain available.
    }
}

#[test]
fn stale_writer_preserves_other_process_script_name_subscription_and_interval() {
    let state = tempfile::tempdir().unwrap();
    let mut stale = Sources::open(state.path()).unwrap();
    let mut writer = Sources::open(state.path()).unwrap();
    writer
        .import_script_source(&rule("custom"), 1, CODE, false)
        .unwrap();
    writer.registry_mut().set_name("custom", "Kept").unwrap();
    writer
        .registry_mut()
        .set_game_source(42, "custom", false)
        .unwrap();
    writer
        .registry_mut()
        .set_update_interval("custom", 24)
        .unwrap();
    stale.registry_mut().set_enabled("patchmoe", false).unwrap();
    assert_eq!(stale.get_script_source("custom").unwrap().code, CODE);
    let persisted = stale.store().settings().unwrap();
    assert_eq!(persisted.names["custom"], "Kept");
    assert!(!persisted.games["42"]["custom"]);
    assert_eq!(persisted.intervals["custom"], 24);
    assert_eq!(persisted.revision, 6);
}

#[test]
fn racing_first_imports_of_the_same_id_have_exactly_one_winner() {
    let state = tempfile::tempdir().unwrap();
    Sources::open(state.path()).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|i| {
            let barrier = Arc::clone(&barrier);
            let path = state.path().to_owned();
            std::thread::spawn(move || {
                let mut sources = Sources::open(&path).unwrap();
                barrier.wait();
                sources.import_script_source(
                    &rule("racing"),
                    1,
                    &format!("return {{value={i}}}"),
                    false,
                )
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(results
        .iter()
        .filter_map(|r| r.as_ref().err())
        .all(|e| e.to_string() == "script_source_exists"));
    let reopened = Sources::open(state.path()).unwrap();
    assert_eq!(reopened.settings_revision(), 2);
    reopened.get_script_source("racing").unwrap();
}

#[test]
fn old_settings_upgrade_once_without_remigrating_the_schema2_namespace() {
    let state = tempfile::tempdir().unwrap();
    let sources = Sources::open(state.path()).unwrap();
    let root = sources.store().paths().root().to_owned();
    let path = sources.store().paths().settings();
    let mut old: Value = serde_json::from_slice(&bytes(&sources)).unwrap();
    old["schema_version"] = json!(2);
    old["revision"] = json!(9);
    old["names"] = json!({"patchmoe":"My publisher"});
    old.as_object_mut().unwrap().remove("script_sources");
    fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    // A stale invalid v1 cache would fail if upgrading settings reran v1->v2.
    fs::create_dir(root.join("cache")).unwrap();
    fs::write(root.join("cache/patchmoe.json"), b"not a legacy cache").unwrap();
    let before = tree(state.path());
    SourceSnapshot::load(state.path(), &[], 1).unwrap();
    assert_eq!(tree(state.path()), before);
    let reopened = Sources::open(state.path()).unwrap();
    assert_eq!(reopened.settings_revision(), 9);
    assert_eq!(
        reopened.store().settings().unwrap().schema_version,
        SOURCE_SETTINGS_SCHEMA_VERSION
    );
    assert!(reopened
        .store()
        .settings()
        .unwrap()
        .script_sources
        .is_empty());
    assert_eq!(
        fs::read(root.join("cache/patchmoe.json")).unwrap(),
        b"not a legacy cache"
    );
    let after = tree(state.path());
    Sources::open(state.path()).unwrap();
    assert_eq!(tree(state.path()), after);
}

#[test]
fn script_source_has_no_import_size_or_host_count_ceiling() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    let mut raw = rule("large");
    raw.pages = (0..12)
        .map(|i| format!("https://example.com/page/{i}"))
        .collect();
    raw.page_hosts = (0..12)
        .map(|i| format!("h{i}.example.com"))
        .chain(["example.com".into()])
        .collect();
    let code = "-- comment 中文\n".repeat(20_000) + "return {}";
    sources.import_script_source(&raw, 1, &code, false).unwrap();
    assert_eq!(sources.get_script_source("large").unwrap().code, code);
}

#[cfg(windows)]
#[test]
fn failed_atomic_replace_keeps_old_bytes_and_registry_then_can_retry() {
    use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    sources
        .import_script_source(&rule("custom"), 1, CODE, false)
        .unwrap();
    let before = bytes(&sources);
    let before_memory = sources.store().settings().unwrap();
    let held = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(sources.store().paths().settings())
        .unwrap();
    assert!(sources
        .import_script_source(&rule("custom"), 1, "return {}", true)
        .is_err());
    assert_eq!(bytes(&sources), before);
    assert_eq!(sources.store().settings().unwrap(), before_memory);
    assert_eq!(sources.get_script_source("custom").unwrap().code, CODE);
    drop(held);
    sources
        .import_script_source(&rule("custom"), 1, "return {}", true)
        .unwrap();
    assert_eq!(
        sources.get_script_source("custom").unwrap().code,
        "return {}"
    );
}
