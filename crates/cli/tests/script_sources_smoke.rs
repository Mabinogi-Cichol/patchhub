//! Script management through real JSONL processes; no Lua execution or network.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn call(state: &Path, method: &str, mut params: Value) -> Value {
    params["host"] = json!({"roots":[],"catalog":null,"signed_catalog":null,"trust_keys":null,
        "state_dir":state,"platform":if cfg!(windows) {"windows"} else {"steamos"}});
    let mut child = Command::new(env!("CARGO_BIN_EXE_patchhub-core"))
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        child.stdin.take().unwrap(),
        "{}",
        json!({"protocol_version":5,"request_id":"script-smoke","method":method,"params":params})
    )
    .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    patchhub_protocol::validate_response(
        &response,
        "script-smoke",
        Some(serde_json::from_value(json!(method)).unwrap()),
    )
    .unwrap();
    response
}
fn rule() -> Value {
    json!({"id":"custom","name":"脚本原名","adapter":"lua","enabled":true,
        "pages":["https://example.com/"],"page_hosts":["example.com"],
        "download_hosts":["download.example.com"],"root_layout":"game_root"})
}
fn files(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(files(&path));
        } else {
            out.insert(path.clone(), fs::read(path).unwrap());
        }
    }
    out
}

#[test]
fn large_script_roundtrips_across_processes_and_explicit_replace_preserves_settings() {
    let temp = tempfile::tempdir().unwrap();
    // A valid top-level infinite loop proves saving compiles without executing.
    let code = format!(
        "-- {}\nwhile true do end\nreturn {{}}\n",
        "中文\n-- ".repeat(12000)
    );
    assert!(code.len() > 32768);
    let imported = call(
        temp.path(),
        "import_script_source",
        json!({"rule":rule(),"api_version":1,"code":code}),
    );
    assert_eq!(imported["ok"], true, "{imported}");
    assert_eq!(
        imported["result"]["rule"]["script"]["sha256"],
        format!("{:x}", Sha256::digest(code.as_bytes()))
    );
    let before = files(temp.path());
    let document = call(
        temp.path(),
        "get_script_source",
        json!({"source_id":"custom"}),
    );
    assert_eq!(document["result"]["code"], code);
    assert_eq!(
        files(temp.path()),
        before,
        "reading must not write source state"
    );
    let duplicate = call(
        temp.path(),
        "import_script_source",
        json!({"rule":rule(),"api_version":1,"code":code}),
    );
    assert_eq!(duplicate["error"]["code"], "script_source_exists");
    assert_eq!(files(temp.path()), before);
    for (method, params) in [
        (
            "set_source_enabled",
            json!({"source_id":"custom","enabled":false}),
        ),
        (
            "set_source_name",
            json!({"source_id":"custom","name":"用户名称"}),
        ),
        (
            "set_source_update_interval",
            json!({"source_id":"custom","interval_hours":72}),
        ),
        (
            "set_game_source",
            json!({"source_id":"custom","appid":42,"subscribed":false}),
        ),
    ] {
        assert_eq!(call(temp.path(), method, params)["ok"], true);
    }
    let before = files(temp.path());
    for (api, code, error) in [
        (1, "return {", "script_source_syntax_error"),
        (2, "return {}", "unsupported_script_api_version"),
    ] {
        let result = call(
            temp.path(),
            "import_script_source",
            json!({"rule":rule(),"api_version":api,"code":code,"replace":true}),
        );
        assert_eq!(result["error"]["code"], error, "{result}");
        assert_eq!(
            files(temp.path()),
            before,
            "failed replacement changed saved code"
        );
    }
    let replaced = call(
        temp.path(),
        "import_script_source",
        json!({"rule":rule(),"api_version":1,"code":"error('only on execution')","replace":true}),
    );
    assert_eq!(replaced["ok"], true, "{replaced}");
    let sources = call(temp.path(), "list_sources", json!({}));
    let source = sources["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "custom")
        .unwrap();
    assert_eq!(source["enabled"], false);
    assert_eq!(source["name"], "用户名称");
    assert_eq!(source["interval_hours"], 72);
    let subscriptions = call(temp.path(), "list_game_sources", json!({"appid":42}));
    let source = subscriptions["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "custom")
        .unwrap();
    assert_eq!(source["subscribed"], false);
    let reopened = call(
        temp.path(),
        "get_script_source",
        json!({"source_id":"custom"}),
    );
    assert_eq!(reopened["result"]["code"], "error('only on execution')");
}

#[test]
fn read_missing_state_is_read_only_and_builtin_rust_cannot_be_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let absent = temp.path().join("absent");
    let response = call(&absent, "get_script_source", json!({"source_id":"missing"}));
    assert_eq!(response["error"]["code"], "unknown_source");
    assert!(!absent.exists());
    let mut builtin = rule();
    builtin["id"] = json!("patchmoe");
    let response = call(
        temp.path(),
        "import_script_source",
        json!({"rule":builtin,"api_version":1,"code":"return {}","replace":true}),
    );
    assert_eq!(response["error"]["code"], "source_not_script");
}
