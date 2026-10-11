//! Actual JSONL subprocess coverage for the projection adapter (current v3).
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn call(method: &str, params: Value) -> Value {
    let mut process = Command::new(env!("CARGO_BIN_EXE_steam-patch"))
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let request = json!({"protocol_version":3,"request_id":"projection-smoke","method":method,"params":params});
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
    let left = library(temp.path(), "Steam 中文");
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
