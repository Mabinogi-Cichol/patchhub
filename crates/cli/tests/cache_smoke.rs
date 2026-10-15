//! Actual v4 subprocess policy and cache-result integration, only temporary state.
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
};
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

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

fn call(method: &str, id: &str, params: Value) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_steam-patch"))
        .arg(if method == "install_patch" {
            "serve-task"
        } else {
            "serve"
        })
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    writeln!(
        input,
        "{}",
        json!({"protocol_version":4,"request_id":id,"method":method,"params":wire_params(method, params)})
    )
    .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut response = None;
    let mut consumed = 0;
    for line in stdout.by_ref().lines() {
        let line = line.unwrap();
        consumed += line.len();
        assert!(consumed <= 8 * 1024 * 1024);
        let value: Value = serde_json::from_str(&line).unwrap();
        if value.get("ok").is_some() {
            response = Some(value);
            break;
        }
    }
    // Keep stdin open through the terminal response; EOF would cancel the task.
    drop(input);
    drop(stdout);
    let output = child.wait_with_output().unwrap();
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
    let response = response.unwrap();
    assert_eq!(response["request_id"], id);
    steam_patch_protocol::validate_response(
        &response,
        response["request_id"].as_str().unwrap(),
        Some(serde_json::from_value(json!(method)).unwrap()),
    )
    .unwrap();
    response
}
#[test]
fn cli_after_commit_policy_reaches_core_and_replay_does_not_delete_new_cache() {
    if steam_patch_core::installer::host_identity_blocker().is_some() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let library = temp.path().join("Steam");
    let game = library.join("steamapps/common/Game");
    fs::create_dir_all(&game).unwrap();
    fs::write(library.join("steamapps/appmanifest_42.acf"), r#""AppState" { "appid" "42" "name" "Game" "installdir" "Game" "buildid" "100" "StateFlags" "4" "UserConfig" { "language" "english" } }"#).unwrap();
    let archive = temp.path().join("archive.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    zip.start_file(
        "patch.dat",
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
    )
    .unwrap();
    zip.write_all(b"patched").unwrap();
    zip.finish().unwrap();
    let now = steam_patch_core::trust::now().unwrap();
    let payload = serde_json::to_vec(&json!({"catalog":{"schema_version":1,"revision":1,"releases":[{
        "patch_id":"fixture","release_id":"fixture-v1","appid":42,"name":"Fixture","version":"1","language":"en","category":"content_restore",
        "source_page":"https://publisher.example/patch","support_level":"managed_zip","platforms":["windows","steamos"],"verified_buildids":["100"],
        "detection":[{"path":"patch.dat","sha256":hash(b"patched")}],
        "recipe":{"artifact_sha256":hash(&fs::read(&archive).unwrap()),"files":[{"source":"patch.dat","destination":"patch.dat","original_sha256":null,"installed_sha256":hash(b"patched")}]}
    }]},"generated_at":now-1,"expires_at":now+3600,"revoked_release_ids":[]})).unwrap();
    let signing = SigningKey::from_bytes(&[19; 32]);
    let signed = temp.path().join("signed.json");
    let keys = temp.path().join("keys.json");
    fs::write(&signed, json!({"schema_version":1,"key_id":"fixture","payload":STANDARD.encode(&payload),"signature":STANDARD.encode(signing.sign(&payload).to_bytes())}).to_string()).unwrap();
    fs::write(&keys, json!({"schema_version":1,"keys":[{"key_id":"fixture","public_key":STANDARD.encode(signing.verifying_key().to_bytes()),"revoked":false}]}).to_string()).unwrap();
    let state = temp.path().join("state");
    let paths = steam_patch_core::installer::InstallerPaths::new(&state);
    fs::create_dir_all(paths.downloads_dir()).unwrap();
    let cache = paths.downloads_dir().join("discard-me");
    fs::write(&cache, b"cache").unwrap();
    let id = steam_patch_core::steam::scan(std::slice::from_ref(&library))
        .installs
        .remove(0)
        .instance_id;
    let params = json!({"roots":[library],"state_dir":state,"instance_id":id,"signed_catalog":signed,"trust_keys":keys,"release_id":"fixture-v1","archive":archive,"cleanup_policy":"after_commit"});
    let result = call("install_patch", "cli-cleanup", params.clone());
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["result"]["state"], "committed");
    assert_eq!(result["result"]["cleanup"]["status"], "completed");
    assert_eq!(result["result"]["cleanup"]["removed_bytes"], 5);
    assert!(!cache.exists());
    fs::write(&cache, b"new").unwrap();
    let replay = call("install_patch", "cli-cleanup", params);
    assert_eq!(replay["ok"], true, "{replay}");
    assert_eq!(replay["result"]["plan_id"], result["result"]["plan_id"]);
    assert_eq!(replay["result"]["cleanup"]["status"], "not_requested");
    assert_eq!(fs::read(cache).unwrap(), b"new");
    let manual = call(
        "cleanup_cache",
        "manual",
        json!({"roots":[temp.path().join("Steam")],"state_dir":state}),
    );
    assert_eq!(manual["ok"], true, "{manual}");
    assert_eq!(manual["result"]["status"], "completed");
    assert_eq!(manual["result"]["removed_bytes"], 3);
}
