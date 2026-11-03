//! Actual v4 subprocess policy and cache-result integration, only temporary state.
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
};
fn wire_params(method: &str, mut legacy: Value) -> Value {
    let object = legacy.as_object_mut().unwrap();
    let mut host = serde_json::Map::new();
    for key in ["roots", "state_dir", "platform"] {
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
        "preview" => &["instance_id", "selection_id"],
        "install_patch" => &[
            "instance_id",
            "selection_id",
            "operation",
            "cleanup_policy",
            "manual_download",
        ],
        "import_script_source" => &["rule", "api_version", "code"],
        "set_source_enabled" => &["source_id", "enabled"],
        "update_source" => &["source_id"],
        "rematch_game" => &["appid"],
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

fn call(binary: &std::path::Path, method: &str, id: &str, params: Value) -> Value {
    let mut child = Command::new(binary)
        .arg(
            if ["install_patch", "update_source", "rematch_game"].contains(&method) {
                "serve-task"
            } else {
                "serve"
            },
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    writeln!(
        input,
        "{}",
        json!({"protocol_version":5,"request_id":id,"method":method,"params":wire_params(method, params)})
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
    patchhub_protocol::validate_response(
        &response,
        response["request_id"].as_str().unwrap(),
        Some(serde_json::from_value(json!(method)).unwrap()),
    )
    .unwrap();
    response
}
#[test]
fn clean_app_queries_matches_installs_and_replays_without_a_public_catalog() {
    if patchhub_core::installer::host_identity_blocker().is_some() {
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
    let state = temp.path().join("state");
    let app = temp.path().join("application");
    fs::create_dir(&app).unwrap();
    let binary = app.join(if cfg!(windows) {
        "patchhub-core.exe"
    } else {
        "patchhub-core"
    });
    fs::copy(env!("CARGO_BIN_EXE_patchhub-core"), &binary).unwrap();
    let clean = call(
        &binary,
        "scan",
        "clean",
        json!({"roots":[library],"state_dir":state}),
    );
    assert_eq!(clean["ok"], true, "{clean}");
    assert!(clean["result"].get("catalog_error").is_none());
    assert!(clean["result"]["installs"][0]["patches"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!state.exists());
    let defaults = call(
        &binary,
        "list_sources",
        "defaults",
        json!({"state_dir":state}),
    );
    for source in defaults["result"].as_array().unwrap() {
        assert_eq!(source["origin"], "builtin");
        let result = call(
            &binary,
            "set_source_enabled",
            "disable",
            json!({"state_dir":state,"source_id":source["id"],"enabled":false}),
        );
        assert_eq!(result["ok"], true, "{result}");
    }
    let code = r#"return {
        collect_index=function() return {entries={{title='Game',page='https://example.com/game',kind='product'}},complete=true} end,
        resolve_page=function(ctx,rule,page) return {title='Game',page=page,kind='product',steam_ids={42},text='English patch',mirrors={{url='https://download.example.com/fixture.zip',format='zip'}}} end
    }"#;
    let imported = call(
        &binary,
        "import_script_source",
        "import",
        json!({"state_dir":state,"rule":{"id":"custom","name":"Fixture","adapter":"lua","enabled":true,"pages":["https://example.com/list"],"page_hosts":["example.com"],"download_hosts":["download.example.com"],"root_layout":"game_root"},"api_version":1,"code":code}),
    );
    assert_eq!(imported["ok"], true, "{imported}");
    let updated = call(
        &binary,
        "update_source",
        "update",
        json!({"state_dir":state,"source_id":"custom"}),
    );
    assert_eq!(updated["ok"], true, "{updated}");
    let matched = call(
        &binary,
        "rematch_game",
        "match",
        json!({"roots":[library],"state_dir":state,"appid":42}),
    );
    assert_eq!(matched["ok"], true, "{matched}");
    let candidate = &matched["result"]["results"][0]["candidates"][0];
    assert_eq!(candidate["source_id"], "custom", "{matched}");
    let projected = call(
        &binary,
        "scan",
        "project",
        json!({"roots":[library],"state_dir":state}),
    );
    assert_eq!(
        projected["result"]["installs"][0]["patches"][0]["source_id"], "custom",
        "{projected}"
    );
    assert!(
        projected["result"]["installs"][0]["patches"][0]["selection_id"]
            .as_str()
            .unwrap()
            .starts_with("zip-selection-")
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/fixture.zip", listener.local_addr().unwrap());
    let bytes = fs::read(&archive).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(&mut stream);
        let mut line = String::new();
        loop {
            line.clear();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
        }
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).unwrap();
        stream.write_all(&bytes).unwrap();
    });
    let paths = patchhub_core::installer::InstallerPaths::new(&state);
    fs::create_dir_all(paths.downloads_dir()).unwrap();
    let cache = paths.downloads_dir().join("discard-me");
    fs::write(&cache, b"cache").unwrap();
    let id = patchhub_core::steam::scan(std::slice::from_ref(&library))
        .installs
        .remove(0)
        .instance_id;
    let params = json!({"roots":[library],"state_dir":state,"instance_id":id,"manual_download":{"source_id":"custom","candidate_id":candidate["candidate_id"],"url":url},"cleanup_policy":"after_commit"});
    let result = call(&binary, "install_patch", "cli-cleanup", params.clone());
    assert_eq!(result["ok"], true, "{result}");
    server.join().unwrap();
    assert_eq!(result["result"]["state"], "committed");
    assert_eq!(result["result"]["cleanup"]["status"], "completed");
    assert!(
        result["result"]["cleanup"]["removed_bytes"]
            .as_u64()
            .unwrap()
            >= 5
    );
    assert!(!cache.exists());
    fs::write(&cache, b"new").unwrap();
    let replay = call(&binary, "install_patch", "cli-cleanup", params);
    assert_eq!(replay["ok"], true, "{replay}");
    assert_eq!(replay["result"]["plan_id"], result["result"]["plan_id"]);
    assert_eq!(replay["result"]["cleanup"]["status"], "not_requested");
    assert_eq!(fs::read(cache).unwrap(), b"new");
    let manual = call(
        &binary,
        "cleanup_cache",
        "manual",
        json!({"roots":[temp.path().join("Steam")],"state_dir":state}),
    );
    assert_eq!(manual["ok"], true, "{manual}");
    assert_eq!(manual["result"]["status"], "completed");
    assert_eq!(manual["result"]["removed_bytes"], 3);
}
