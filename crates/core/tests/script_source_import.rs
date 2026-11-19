//! P1: import/read/persistence and single-file `meta` sandbox parsing.
//! No network fetch or index/discover entrypoint is executed during import.
use patchhub_core::sources::{
    script::validate_code, snapshot::SourceSnapshot, GameQuery, SourceRole, Sources,
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

const CODE: &str = r#"-- 中文 source
return {
    meta = {
        name = "Custom source",
        pages = { "https://example.com/" },
        page_hosts = { "example.com" },
        download_hosts = { "download.example.com" },
    },
    collect_index = function()
        error('entrypoint must not run on import')
    end,
}
"#;

const FALLBACK_CODE: &str = r#"return {
    meta = {
        name = "Fallback Custom",
        role = "fallback_indexer",
        pages = { "https://example.com/search" },
    },
    discover = function()
        return { details = {}, complete = true }
    end,
}
"#;

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
fn import_parses_meta_assigns_id_and_persists_one_complete_document() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    let original = sources.settings_revision();
    let builtin_fingerprints: Vec<_> = sources
        .registry()
        .rules()
        .iter()
        .map(Sources::rule_fingerprint)
        .collect();
    let imported = sources.import_script_source(None, 1, CODE).unwrap();
    let id = imported.rule.id.clone();
    assert_eq!(id, "example-com");
    assert_eq!(imported.name, "Custom source");
    assert_eq!(imported.settings_revision, original + 1);
    assert_eq!(
        imported.rule.script.as_ref().unwrap().sha256,
        format!("{:x}", Sha256::digest(CODE.as_bytes()))
    );

    let settings: Value = serde_json::from_slice(&bytes(&sources)).unwrap();
    assert_eq!(settings["revision"], original + 1);
    assert_eq!(settings["schema_version"], SOURCE_SETTINGS_SCHEMA_VERSION);
    assert_eq!(settings["script_sources"][&id], CODE);
    assert_eq!(settings["names"][&id], "Custom source");
    assert!(settings["added"][0].get("name").is_none());
    assert_eq!(
        settings["added"][0]["script"]["sha256"],
        imported.rule.script.as_ref().unwrap().sha256
    );
    for (index, expected) in builtin_fingerprints.iter().enumerate() {
        assert_eq!(
            &Sources::rule_fingerprint(&sources.registry().rules()[index]),
            expected
        );
        assert!(serde_json::to_value(&sources.registry().rules()[index])
            .unwrap()
            .get("script")
            .is_some());
    }
    let metadata = serde_json::to_string(&sources.registry().list_sources()).unwrap();
    assert!(!metadata.contains("entrypoint must not run on import"));
    drop(sources);

    let mut reopened = Sources::open(state.path()).unwrap();
    let document = reopened.get_script_source(&id).unwrap();
    assert_eq!(document.source_id, id);
    assert_eq!(document.name, "Custom source");
    assert_eq!(document.code, CODE);
    assert_eq!(document.api_version, 1);
    assert!(serde_json::to_value(&document.rule)
        .unwrap()
        .get("script")
        .is_none());
    assert_eq!(document.sha256, imported.rule.script.unwrap().sha256);

    // Importing scripts on the same primary host allocates unique incrementing IDs.
    let second = reopened.import_script_source(None, 1, CODE).unwrap();
    let third = reopened.import_script_source(None, 1, CODE).unwrap();
    assert_eq!(second.rule.id, "example-com-2");
    assert_eq!(third.rule.id, "example-com-3");
}

#[test]
fn meta_inference_handles_omitted_page_hosts_empty_tables_and_role_cross_checks() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();

    // Omit `name`, `role`, and pass empty Lua `{}` for `page_hosts`/`download_hosts`.
    let derived = r#"return {
        meta = {
            pages = {
                "https://sub.example.com/a",
                "https://sub.example.com/b",
                "https://cdn.example.com/list",
            },
            page_hosts = {},
            download_hosts = {},
        },
        collect_index = function() return { entries = {}, complete = true } end,
    }"#;
    let imported = sources.import_script_source(None, 1, derived).unwrap();
    assert_eq!(imported.rule.id, "sub-example-com");
    assert_eq!(imported.name, "sub.example.com");
    assert_eq!(imported.rule.role, SourceRole::Publisher);
    assert_eq!(
        imported.rule.page_hosts,
        vec!["sub.example.com", "cdn.example.com"]
    );
    assert!(imported.rule.download_hosts.is_empty());
    assert_eq!(imported.rule.priority, 100);
    assert!(!imported.rule.official_title_match);

    // Omit `role` while exporting `discover` infers `fallback_indexer`.
    let inferred_fallback = r#"return {
        meta = { pages = { "https://search.example.com/" } },
        discover = function() return { details = {}, complete = true } end,
    }"#;
    let fb = sources
        .import_script_source(None, 1, inferred_fallback)
        .unwrap();
    assert_eq!(fb.rule.role, SourceRole::FallbackIndexer);
    assert_eq!(fb.rule.id, "search-example-com");

    // Top-level infinite loops (even wrapped in pcall) must abort with init limit error.
    let initial = bytes(&sources);
    let rev = sources.settings_revision();
    for bad_init in [
        "while true do end",
        "pcall(function() while true do end end)",
    ] {
        let error = sources
            .import_script_source(None, 1, bad_init)
            .unwrap_err()
            .to_string();
        assert_eq!(error, "script_source_init_limit_exceeded", "{bad_init}");
        assert_eq!(bytes(&sources), initial);
        assert_eq!(sources.settings_revision(), rev);
    }

    for (bad_code, expected) in [
        (r#"return "not a table""#, "script_source_contract_error"),
        (
            r#"return { collect_index = function() end }"#,
            "invalid_source_meta",
        ),
        (
            r#"return { meta = { pages = {"https://example.com/"}, unknown_key = 1 }, collect_index = function() end }"#,
            "invalid_source_meta",
        ),
        (
            r#"return { meta = { pages = {"https://example.com/"} } }"#,
            "script_source_contract_error",
        ),
        (
            r#"return { meta = { pages = {"https://example.com/"} }, collect_index = function() end, discover = function() end }"#,
            "script_source_contract_error",
        ),
        (
            r#"return { meta = { role = "publisher", pages = {"https://example.com/"} }, discover = function() end }"#,
            "script_source_contract_error",
        ),
        (
            r#"return { meta = { role = "fallback_indexer", pages = {"https://example.com/"}, download_hosts = {"dl.example.com"} }, discover = function() end }"#,
            "invalid_source_rule",
        ),
        (
            r#"return { meta = { role = "fallback_indexer", pages = {"https://example.com/"}, official_title_match = true }, discover = function() end }"#,
            "invalid_source_rule",
        ),
        (
            r#"return { meta = { pages = {"http://example.com/"} }, collect_index = function() end }"#,
            "invalid_source_url",
        ),
        (
            r#"return { meta = { pages = {"https://127.0.0.1/"} }, collect_index = function() end }"#,
            "invalid_source_host",
        ),
        (
            r#"return { meta = { pages = {"https://site.local/"} }, collect_index = function() end }"#,
            "invalid_source_host",
        ),
    ] {
        let error = sources
            .import_script_source(None, 1, bad_code)
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "code={bad_code} error={error}");
        assert_eq!(bytes(&sources), initial);
        assert_eq!(sources.settings_revision(), rev);
    }
}

#[test]
fn removing_a_custom_source_clears_only_its_settings_and_allows_reimport() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    let id = sources.import_script_source(None, 1, CODE).unwrap().rule.id;
    sources.registry_mut().set_name(&id, "自定义来源").unwrap();
    sources.registry_mut().set_enabled(&id, false).unwrap();
    sources.registry_mut().set_update_interval(&id, 24).unwrap();
    sources
        .registry_mut()
        .set_game_source(42, &id, false)
        .unwrap();
    let before = sources.settings_revision();

    let revision = sources.registry_mut().remove_source(&id).unwrap();
    assert_eq!(revision, before + 1);
    assert_eq!(sources.settings_revision(), revision);
    assert!(!sources
        .registry()
        .list_sources()
        .iter()
        .any(|source| source.rule.id == id));
    assert_eq!(
        sources.get_script_source(&id).unwrap_err().to_string(),
        "unknown_source"
    );
    let settings: Value =
        serde_json::from_slice(&bytes(&sources)).expect("settings stay one document");
    for field in ["added", "script_sources"] {
        assert!(
            !settings[field].to_string().contains(&id),
            "{field} keeps no trace: {settings}"
        );
    }
    assert!(settings["enabled"].get(&id).is_none());
    assert!(settings["names"].get(&id).is_none());
    assert!(settings["intervals"].get(&id).is_none());
    // Other sources keep their own settings; unknown and builtin removal is refused.
    assert!(settings["enabled"].get("patchmoe").is_none());
    assert_eq!(
        sources
            .registry_mut()
            .remove_source(&id)
            .unwrap_err()
            .to_string(),
        "unknown_source"
    );
    assert_eq!(
        sources
            .registry_mut()
            .remove_source("patchmoe")
            .unwrap_err()
            .to_string(),
        "source_builtin_readonly"
    );
    // A deleted ID is freed and can be reallocated on fresh import.
    let reimported = sources.import_script_source(None, 1, CODE).unwrap();
    assert_eq!(reimported.rule.id, id);
    assert!(reimported.rule.enabled);
    assert!(sources.registry().list_sources().iter().any(|source| {
        source.rule.id == id && source.origin == "added" && source.name == "Custom source"
    }));
}

#[test]
fn invalid_imports_and_builtin_protection_do_not_modify_settings() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    let initial = bytes(&sources);
    for (source_id, version, code, expected) in [
        (None, 2, CODE, "unsupported_script_api_version"),
        (None, 1, "return {", "script_source_syntax_error"),
        (Some("missing"), 1, CODE, "unknown_source"),
        (Some("patchmoe"), 1, CODE, "source_builtin_readonly"),
        (Some("BadId"), 1, CODE, "invalid_source_id"),
    ] {
        let error = sources
            .import_script_source(source_id, version, code)
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(bytes(&sources), initial);
        assert_eq!(sources.settings_revision(), 1);
    }
    assert_eq!(
        validate_code("bad", 1, &[0xff]).unwrap_err().to_string(),
        "invalid_script_source_utf8"
    );
    assert!(validate_code("bad", 1, b"\x1bLua").is_err());
}

#[test]
fn explicit_replace_preserves_user_name_and_settings_and_changes_parsing_identity() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    let id = sources.import_script_source(None, 1, CODE).unwrap().rule.id;
    sources.registry_mut().set_enabled(&id, false).unwrap();
    sources.registry_mut().set_name(&id, "My name").unwrap();
    sources
        .registry_mut()
        .set_game_source(42, &id, false)
        .unwrap();
    sources.registry_mut().set_update_interval(&id, 12).unwrap();
    let old = fingerprint(&sources, &id);
    sources
        .registry_mut()
        .set_name(&id, "New display name")
        .unwrap();
    assert_eq!(fingerprint(&sources, &id), old);

    let before = sources.settings_revision();
    let replaced_code = CODE.replace("Custom source", "Ignored New Default Name");
    let replaced = sources
        .import_script_source(Some(&id), 1, &replaced_code)
        .unwrap();
    assert_eq!(replaced.rule.id, id);
    assert_eq!(replaced.name, "New display name");
    assert_eq!(replaced.settings_revision, before + 1);
    assert!(!replaced.rule.enabled);
    assert_ne!(fingerprint(&sources, &id), old);
    assert!(!sources.registry().game_subscribed(42, &id));
    let summary = sources
        .registry()
        .list_sources()
        .into_iter()
        .find(|s| s.rule.id == id)
        .unwrap();
    assert_eq!(summary.name, "New display name");
    assert_eq!(summary.interval_hours, 12);

    sources
        .import_script_source(Some(&id), 1, FALLBACK_CODE)
        .unwrap();
    assert_eq!(
        sources.registry().rules().last().unwrap().role,
        SourceRole::FallbackIndexer
    );
    assert_eq!(sources.store().settings().unwrap().intervals[&id], 12);
    assert!(sources.registry_mut().set_update_interval(&id, 24).is_err());
}

#[test]
fn snapshot_and_edit_reads_capture_the_same_rule_and_code_without_writes() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    let id = sources.import_script_source(None, 1, CODE).unwrap().rule.id;
    let before = tree(state.path());
    let snapshot = SourceSnapshot::load(
        state.path(),
        &[GameQuery {
            appid: 42,
            name: "Game".into(),
            localized_name: None,
            related_dlc: Vec::new(),
            instance_id: None,
        }],
        1,
    )
    .unwrap()
    .unwrap();
    assert_eq!(snapshot.get_script_source(&id).unwrap().code, CODE);
    assert_eq!(tree(state.path()), before);

    let loop_entry = r#"return {
        meta = { name = "Loop", pages = { "https://example.com/" } },
        collect_index = function() while true do end end,
    }"#;
    sources
        .import_script_source(Some(&id), 1, loop_entry)
        .unwrap();
    assert_eq!(snapshot.get_script_source(&id).unwrap().code, CODE);
    assert_eq!(sources.get_script_source(&id).unwrap().code, loop_entry);
    let reopened = SourceSnapshot::load(state.path(), &[], 2).unwrap().unwrap();
    assert_eq!(reopened.get_script_source(&id).unwrap().code, loop_entry);
    assert!(serde_json::to_string(&sources.registry().list_sources())
        .unwrap()
        .contains("sha256"));
}

#[test]
fn digest_mismatch_and_missing_code_are_source_errors_without_fallback() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    let bundled_count = sources.registry().rules().len();
    let id = sources.import_script_source(None, 1, CODE).unwrap().rule.id;
    let path = sources.store().paths().settings();
    let original: Value = serde_json::from_slice(&bytes(&sources)).unwrap();
    for missing in [false, true] {
        let mut damaged = original.clone();
        if missing {
            damaged["script_sources"]
                .as_object_mut()
                .unwrap()
                .remove(&id);
        } else {
            damaged["script_sources"][&id] = json!("return {}");
        }
        fs::write(&path, serde_json::to_vec(&damaged).unwrap()).unwrap();
        let snapshot = SourceSnapshot::load(state.path(), &[], 1).unwrap().unwrap();
        let error = snapshot.get_script_source(&id).unwrap_err().to_string();
        assert!(
            error.contains("script_source_digest_mismatch") && error.contains(&id),
            "{error}"
        );
        let reopened = Sources::open(state.path()).unwrap();
        assert!(reopened.get_script_source(&id).is_err());
        assert_eq!(reopened.registry().rules().len(), bundled_count + 1);
    }
}

#[test]
fn stale_writer_preserves_other_process_script_name_subscription_and_interval() {
    let state = tempfile::tempdir().unwrap();
    let mut stale = Sources::open(state.path()).unwrap();
    let mut writer = Sources::open(state.path()).unwrap();
    let id = writer.import_script_source(None, 1, CODE).unwrap().rule.id;
    writer.registry_mut().set_name(&id, "Kept").unwrap();
    writer
        .registry_mut()
        .set_game_source(42, &id, false)
        .unwrap();
    writer.registry_mut().set_update_interval(&id, 24).unwrap();
    stale.registry_mut().set_enabled("patchmoe", false).unwrap();
    assert_eq!(stale.get_script_source(&id).unwrap().code, CODE);
    let persisted = stale.store().settings().unwrap();
    assert_eq!(persisted.names[&id], "Kept");
    assert!(!persisted.games["42"][&id]);
    assert_eq!(persisted.intervals[&id], 24);
    assert_eq!(persisted.revision, 6);
}

#[test]
fn racing_first_imports_with_same_host_assign_distinct_auto_ids() {
    let state = tempfile::tempdir().unwrap();
    Sources::open(state.path()).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|i| {
            let barrier = Arc::clone(&barrier);
            let path = state.path().to_owned();
            std::thread::spawn(move || {
                let mut sources = Sources::open(&path).unwrap();
                let code = format!(
                    r#"return {{
                        meta = {{ name = "Race {i}", pages = {{ "https://racing.example/" }} }},
                        collect_index = function() return {{ entries = {{}}, complete = true }} end,
                    }}"#
                );
                barrier.wait();
                sources.import_script_source(None, 1, &code)
            })
        })
        .collect();
    let mut ids: Vec<String> = handles
        .into_iter()
        .map(|h| h.join().unwrap().unwrap().rule.id)
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["racing-example", "racing-example-2"]);
    let reopened = Sources::open(state.path()).unwrap();
    assert_eq!(reopened.settings_revision(), 3);
    for id in ids {
        reopened.get_script_source(&id).unwrap();
    }
}

#[test]
fn old_schema3_settings_upgrade_smoothly_and_legacy_scripts_keep_working() {
    let state = tempfile::tempdir().unwrap();
    let sources = Sources::open(state.path()).unwrap();
    let root = sources.store().paths().root().to_owned();
    let path = sources.store().paths().settings();
    let legacy_code =
        r#"return { collect_index = function() return { entries = {}, complete = true } end }"#;
    let legacy_desc = validate_code("legacy-custom", 1, legacy_code.as_bytes()).unwrap();

    let mut old: Value = serde_json::from_slice(&bytes(&sources)).unwrap();
    old["schema_version"] = json!(3);
    old["revision"] = json!(9);
    old["names"] = json!({ "patchmoe": "My publisher" });
    old["added"] = json!([
        {
            "id": "legacy-custom",
            "name": "Legacy Custom Display",
            "adapter": "lua",
            "role": "publisher",
            "discovery_only": false,
            "priority": 50,
            "enabled": true,
            "pages": ["https://legacy.example/list"],
            "page_hosts": ["legacy.example"],
            "download_hosts": ["dl.legacy.example"],
            "root_layout": "game_root",
            "url": "https://legacy.example/old",
            "language": "en",
            "table_id": "t1",
            "columns": { "title": 0, "download": 1 },
            "script": legacy_desc,
        },
        {
            "id": "retired-table",
            "name": "Old Table",
            "adapter": "table_html",
            "role": "publisher",
            "discovery_only": false,
            "priority": 100,
            "enabled": true,
            "pages": ["https://retired.example/"],
            "page_hosts": ["retired.example"],
            "download_hosts": ["dl.retired.example"],
        }
    ]);
    old["script_sources"] = json!({ "legacy-custom": legacy_code });
    fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();

    // A stale invalid v1 cache must NOT rerun v1->v2 migration when schema_json == 2.
    fs::create_dir(root.join("cache")).unwrap();
    fs::write(root.join("cache/patchmoe.json"), b"not a legacy cache").unwrap();

    // Readonly snapshot migrates schema<=3 rules in memory without touching disk.
    let before = tree(state.path());
    let snap = SourceSnapshot::load(state.path(), &[], 1).unwrap().unwrap();
    assert_eq!(tree(state.path()), before);
    let legacy_doc = snap.get_script_source("legacy-custom").unwrap();
    assert_eq!(legacy_doc.name, "Legacy Custom Display");
    assert!(legacy_doc.rule.official_title_match);
    assert!(snap.get_script_source("retired-table").is_err());

    // Normal open upgrades settings to schema 4 and keeps legacy script usable.
    let mut reopened = Sources::open(state.path()).unwrap();
    assert_eq!(reopened.settings_revision(), 9);
    let settings = reopened.store().settings().unwrap();
    assert_eq!(settings.schema_version, SOURCE_SETTINGS_SCHEMA_VERSION);
    assert_eq!(settings.names["patchmoe"], "My publisher");
    assert_eq!(settings.names["legacy-custom"], "Legacy Custom Display");
    assert_eq!(settings.added.len(), 1);
    assert!(settings.added[0].official_title_match);
    assert_eq!(
        reopened.get_script_source("legacy-custom").unwrap().code,
        legacy_code
    );
    assert_eq!(
        fs::read(root.join("cache/patchmoe.json")).unwrap(),
        b"not a legacy cache"
    );

    // Re-opening is idempotent.
    let after = tree(state.path());
    Sources::open(state.path()).unwrap();
    assert_eq!(tree(state.path()), after);

    // Editing a legacy source requires `meta` and rejects scripts without it cleanly.
    let err = reopened
        .import_script_source(Some("legacy-custom"), 1, legacy_code)
        .unwrap_err()
        .to_string();
    assert!(err.contains("invalid_source_meta"), "{err}");
    assert_eq!(
        reopened.get_script_source("legacy-custom").unwrap().code,
        legacy_code
    );

    // Providing valid `meta` succeeds while preserving the user's migrated display name.
    let upgraded = reopened
        .import_script_source(Some("legacy-custom"), 1, CODE)
        .unwrap();
    assert_eq!(upgraded.rule.id, "legacy-custom");
    assert_eq!(upgraded.name, "Legacy Custom Display");
}

#[test]
fn script_source_has_no_import_size_or_host_count_ceiling() {
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    let pages: Vec<String> = (0..12)
        .map(|i| format!(r#""https://h{i}.example.com/page/{i}""#))
        .collect();
    let code = format!(
        "{}\nreturn {{\n  meta = {{ name = \"Large\", pages = {{ {} }} }},\n  collect_index = function() return {{ entries = {{}}, complete = true }} end,\n}}",
        "-- comment 中文\n".repeat(20_000),
        pages.join(", "),
    );
    let imported = sources.import_script_source(None, 1, &code).unwrap();
    assert_eq!(imported.rule.pages.len(), 12);
    assert_eq!(imported.rule.page_hosts.len(), 12);
    assert_eq!(
        sources.get_script_source(&imported.rule.id).unwrap().code,
        code
    );
}

#[cfg(windows)]
#[test]
fn failed_atomic_replace_keeps_old_bytes_and_registry_then_can_retry() {
    use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
    let state = tempfile::tempdir().unwrap();
    let mut sources = Sources::open(state.path()).unwrap();
    let id = sources.import_script_source(None, 1, CODE).unwrap().rule.id;
    let before = bytes(&sources);
    let before_memory = sources.store().settings().unwrap();
    let held = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(sources.store().paths().settings())
        .unwrap();
    assert!(sources
        .import_script_source(Some(&id), 1, FALLBACK_CODE)
        .is_err());
    assert_eq!(bytes(&sources), before);
    assert_eq!(sources.store().settings().unwrap(), before_memory);
    assert_eq!(sources.get_script_source(&id).unwrap().code, CODE);
    drop(held);
    sources
        .import_script_source(Some(&id), 1, FALLBACK_CODE)
        .unwrap();
    assert_eq!(sources.get_script_source(&id).unwrap().code, FALLBACK_CODE);
}

#[test]
fn ai_prompt_and_documentation_examples_stay_aligned_with_core_parser() {
    use patchhub_core::sources::{
        script::{parse_script, AI_SOURCE_PROMPT, SCRIPT_API_VERSION},
        validate::{build_rule_from_meta, generate_source_id},
    };
    use std::collections::HashSet;

    assert!(AI_SOURCE_PROMPT.contains(
        "https://raw.githubusercontent.com/Mabinogi-Cichol/patchhub/main/docs/custom-subscription-sources.md"
    ));
    assert!(AI_SOURCE_PROMPT.contains(
        "https://github.com/Mabinogi-Cichol/patchhub/blob/main/docs/custom-subscription-sources.md"
    ));

    let doc_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/custom-subscription-sources.md");
    let doc = fs::read_to_string(&doc_path).expect("read docs/custom-subscription-sources.md");
    let normalized = doc.replace("\r\n", "\n");

    let mut examples = Vec::new();
    let mut rest = normalized.as_str();
    while let Some(start) = rest.find("```lua\n") {
        let after_fence = &rest[start + "```lua\n".len()..];
        let end = after_fence
            .find("\n```")
            .expect("unclosed lua fenced code block in documentation");
        let block = &after_fence[..end];
        if block.contains("meta =") {
            examples.push(block.to_string());
        }
        rest = &after_fence[end + "\n```".len()..];
    }

    assert!(
        examples.len() >= 3,
        "expected at least 3 complete single-file lua examples with meta, got {}",
        examples.len()
    );

    let mut ids = HashSet::new();
    for (idx, code) in examples.iter().enumerate() {
        let label = format!("doc-example-{}", idx + 1);
        let parsed = parse_script(&label, SCRIPT_API_VERSION, code.as_bytes())
            .unwrap_or_else(|e| panic!("{label} failed parse_script: {e}"));
        let first_url = parsed.meta.pages.first().expect("non-empty pages");
        let host = parsed
            .meta
            .page_hosts
            .as_ref()
            .and_then(|h| h.first())
            .cloned()
            .unwrap_or_else(|| {
                url::Url::parse(first_url)
                    .unwrap()
                    .host_str()
                    .unwrap()
                    .to_string()
            });
        let id = generate_source_id(&host, &ids);
        let (rule, _default_name) = build_rule_from_meta(
            id.clone(),
            "lua".to_string(),
            true,
            &parsed.meta,
            parsed.role,
            parsed.descriptor,
        )
        .unwrap_or_else(|e| panic!("{label} failed build_rule_from_meta: {e}"));
        ids.insert(rule.id);
    }
}
