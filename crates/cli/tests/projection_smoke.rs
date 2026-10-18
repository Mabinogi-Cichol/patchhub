//! Actual JSONL subprocess coverage for the projection adapter (current v4).
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn wire_params(method: &str, mut legacy: Value) -> Value {
    let object = legacy.as_object_mut().unwrap();
    let mut host = serde_json::Map::new();
    for key in [
        "roots",
        "catalog",
        "signed_catalog",
        "trust_keys",
        "state_dir",
        "platform",
    ] {
        let default = match key {
            "roots" => json!([]),
            "platform" => json!(if cfg!(windows) { "windows" } else { "steamos" }),
            _ => Value::Null,
        };
        host.insert(
            key.into(),
            object
                .remove(key)
                .filter(|v| !v.is_null())
                .unwrap_or(default),
        );
    }
    let keys: &[&str] = match method {
        "preview" => &["instance_id", "release_id", "selection_id"],
        "install_patch" => &[
            "instance_id",
            "release_id",
            "selection_id",
            "archive",
            "operation",
            "cleanup_policy",
        ],
        _ => &[],
    };
    object.retain(|key, value| keys.contains(&key.as_str()) && !value.is_null());
    if method == "install_patch" {
        object.entry("operation").or_insert(json!("overwrite"));
        object.entry("cleanup_policy").or_insert(json!("disabled"));
    }
    object.insert("host".into(), Value::Object(host));
    legacy
}

fn call(method: &str, params: Value) -> Value {
    let mut process = Command::new(env!("CARGO_BIN_EXE_steam-patch"))
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let request = json!({"protocol_version":5,"request_id":"projection-smoke","method":method,"params":wire_params(method, params)});
    writeln!(process.stdin.take().unwrap(), "{request}").unwrap();
    let output = process.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.len() <= 8 * 1024 * 1024);
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["request_id"], "projection-smoke");
    steam_patch_protocol::validate_response(
        &response,
        response["request_id"].as_str().unwrap(),
        Some(serde_json::from_value(json!(method)).unwrap()),
    )
    .unwrap();
    response
}

fn library(base: &Path, name: &str) -> std::path::PathBuf {
    let root = base.join(name);
    fs::create_dir_all(root.join("steamapps/common/测试游戏")).unwrap();
    fs::write(root.join("steamapps/appmanifest_42.acf"),
        "\"AppState\" { \"appid\" \"42\" \"name\" \"Synthetic\" \"installdir\" \"测试游戏\" \"buildid\" \"100\" \"StateFlags\" \"4\" }").unwrap();
    root
}

#[test]
fn scan_and_preview_roundtrip_preserve_instance_identity_and_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let left = library(temp.path(), &format!("Steam {}", "库".repeat(40)));
    let right = library(temp.path(), "第二库");
    fs::write(
        left.join("steamapps/common/测试游戏/patch.dat"),
        b"installed",
    )
    .unwrap();
    let catalog = temp.path().join("catalog.json");
    let sha = format!("{:x}", Sha256::digest(b"installed"));
    fs::write(&catalog, json!({"schema_version":1,"revision":1,"releases":[{
        "patch_id":"test","release_id":"test-v1","appid":42,"name":"Test patch","version":"1",
        "language":"zh-CN","category":"content_restore","source_page":"https://example.invalid/patch",
        "support_level":"managed_zip","platforms":["windows","steamos"],"verified_buildids":["100"],
        "detection":[{"path":"patch.dat","sha256":sha}],
        "recipe":{"artifact_sha256":"0".repeat(64),"files":[{"source":"patch.dat","destination":"patch.dat","original_sha256":null,"installed_sha256":sha}]}
    }]}).to_string()).unwrap();
    let state = temp.path().join("absent-state");
    let params = json!({"roots":[left,right],"catalog":catalog,"state_dir":state});
    let response = call("scan", params.clone());
    assert_eq!(response["ok"], true, "{response}");
    let rows = response["result"]["installs"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .any(|row| row["game"]["instance_id"].as_str().unwrap().len() > 256),
        "complete Steam path identity must survive v4"
    );
    assert_ne!(
        rows[0]["game"]["instance_id"],
        rows[1]["game"]["instance_id"]
    );
    let statuses: Vec<_> = rows
        .iter()
        .map(|row| row["patches"][0]["detection"]["status"].as_str().unwrap())
        .collect();
    assert!(statuses.contains(&"installed_external"));
    assert!(statuses.contains(&"not_detected"));
    for row in rows {
        let patch = &row["patches"][0];
        assert!(patch.get("download_available").is_none());
        assert!(patch.get("auto_installable").is_none());
        assert!(patch.get("reinstall_available").is_none());
        for action in ["automatic", "local_archive", "reinstall", "overwrite"] {
            assert_eq!(patch["actions"][action]["available"], false);
            assert!(patch["actions"][action]["blockers"].is_array());
        }
        assert_eq!(patch["actions"]["primary"]["operation"], "overwrite");
        assert_eq!(patch["actions"]["primary"]["input"], "unavailable");
        assert_eq!(
            patch["actions"]["primary"]["label"],
            if patch["detection"]["status"] == "installed_external" {
                "overwrite"
            } else {
                "install"
            }
        );
    }
    let mut preview = params;
    preview["instance_id"] = rows[0]["game"]["instance_id"].clone();
    preview["release_id"] = json!("test-v1");
    let response = call("preview", preview);
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["result"]["executable"], false);
    assert!(!state.exists(), "read-only RPC must not initialize state");
    assert_eq!(
        fs::read(left.join("steamapps/common/测试游戏/patch.dat")).unwrap(),
        b"installed"
    );
    assert!(!right.join("steamapps/common/测试游戏/patch.dat").exists());
}

#[test]
fn corrupt_existing_source_state_is_visible_and_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let root = library(temp.path(), "Steam");
    let state = temp.path().join("state");
    let sources = state.join("sources/rust-v1");
    fs::create_dir_all(&sources).unwrap();
    let settings = sources.join("settings.json");
    fs::write(&settings, b"{corrupt").unwrap();
    let response = call("scan", json!({"roots":[root],"state_dir":state}));
    assert_eq!(
        response["ok"], false,
        "corrupt state must not silently disappear: {response}"
    );
    assert_eq!(fs::read(&settings).unwrap(), b"{corrupt");
    assert!(!sources.join("schema.json").exists());
    assert!(!state.join("installer").exists());
}
