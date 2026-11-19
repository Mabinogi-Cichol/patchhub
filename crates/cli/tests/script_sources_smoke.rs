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
    params["host"] = json!({"roots":[],"state_dir":state,"platform":if cfg!(windows) {"windows"} else {"steamos"}});
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
fn sample_script(body: &str, extra_comment: &str) -> String {
    format!(
        "-- {extra_comment}\nreturn {{\n  meta = {{\n    name = '脚本原名',\n    pages = {{'https://example.com/'}},\n    page_hosts = {{'example.com'}},\n    download_hosts = {{'download.example.com'}}\n  }},\n  {body}\n}}\n"
    )
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
    let code = sample_script(
        "collect_index = function() while true do end end",
        &"中文\n-- ".repeat(12000),
    );
    assert!(code.len() > 32768);
    let imported = call(
        temp.path(),
        "import_script_source",
        json!({"api_version":1,"code":code}),
    );
    assert_eq!(imported["ok"], true, "{imported}");
    assert_eq!(
        imported["result"]["rule"]["id"], "example-com",
        "{imported}"
    );
    assert_eq!(imported["result"]["name"], "脚本原名", "{imported}");
    assert_eq!(
        imported["result"]["rule"]["script"]["sha256"],
        format!("{:x}", Sha256::digest(code.as_bytes()))
    );
    let before = files(temp.path());
    let document = call(
        temp.path(),
        "get_script_source",
        json!({"source_id":"example-com"}),
    );
    assert_eq!(document["result"]["source_id"], "example-com");
    assert_eq!(document["result"]["name"], "脚本原名");
    assert_eq!(document["result"]["code"], code);
    assert_eq!(
        files(temp.path()),
        before,
        "reading must not write source state"
    );
    // Omitting source_id on a second import allocates a distinct suffixed ID.
    let second = call(
        temp.path(),
        "import_script_source",
        json!({"api_version":1,"code":code}),
    );
    assert_eq!(second["ok"], true, "{second}");
    assert_eq!(second["result"]["rule"]["id"], "example-com-2", "{second}");
    assert_eq!(
        call(
            temp.path(),
            "remove_source",
            json!({"source_id":"example-com-2"})
        )["ok"],
        true
    );
    for (method, params) in [
        (
            "set_source_enabled",
            json!({"source_id":"example-com","enabled":false}),
        ),
        (
            "set_source_name",
            json!({"source_id":"example-com","name":"用户名称"}),
        ),
        (
            "set_source_update_interval",
            json!({"source_id":"example-com","interval_hours":72}),
        ),
        (
            "set_game_source",
            json!({"source_id":"example-com","appid":42,"subscribed":false}),
        ),
    ] {
        assert_eq!(call(temp.path(), method, params)["ok"], true);
    }
    let before = files(temp.path());
    for (api, bad_code, error) in [
        (1, "return {", "script_source_syntax_error"),
        (1, "return {}", "invalid_source_meta"),
        (
            1,
            "while true do end\nreturn { meta = { pages = {'https://example.com/'} } }",
            "script_source_init_limit_exceeded",
        ),
        (2, "return {}", "unsupported_script_api_version"),
    ] {
        let result = call(
            temp.path(),
            "import_script_source",
            json!({"source_id":"example-com","api_version":api,"code":bad_code}),
        );
        assert_eq!(result["error"]["code"], error, "{result}");
        assert_eq!(
            files(temp.path()),
            before,
            "failed replacement changed saved code"
        );
    }
    let replacement_code = sample_script(
        "collect_index = function() error('only on execution') end",
        "replaced",
    );
    let replaced = call(
        temp.path(),
        "import_script_source",
        json!({"source_id":"example-com","api_version":1,"code":replacement_code}),
    );
    assert_eq!(replaced["ok"], true, "{replaced}");
    let sources = call(temp.path(), "list_sources", json!({}));
    let source = sources["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "example-com")
        .unwrap();
    assert_eq!(source["enabled"], false);
    assert_eq!(source["name"], "用户名称");
    assert_eq!(source["interval_hours"], 72);
    let subscriptions = call(temp.path(), "list_game_sources", json!({"appid":42}));
    let source = subscriptions["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "example-com")
        .unwrap();
    assert_eq!(source["subscribed"], false);
    let reopened = call(
        temp.path(),
        "get_script_source",
        json!({"source_id":"example-com"}),
    );
    assert_eq!(reopened["result"]["name"], "用户名称");
    assert_eq!(reopened["result"]["code"], replacement_code);
}

#[test]
fn read_missing_state_is_read_only_and_builtin_scripts_are_read_only() {
    let temp = tempfile::tempdir().unwrap();
    let absent = temp.path().join("absent");
    let prompt = call(&absent, "get_script_source_prompt", json!({}));
    assert_eq!(prompt["ok"], true, "{prompt}");
    assert!(prompt["result"]["prompt"]
        .as_str()
        .unwrap()
        .contains("docs/custom-subscription-sources.md"));
    assert!(!absent.exists(), "reading prompt must not create state");
    let response = call(&absent, "get_script_source", json!({"source_id":"missing"}));
    assert_eq!(response["error"]["code"], "unknown_source");
    assert!(!absent.exists());
    let document = call(
        &absent,
        "get_script_source",
        json!({"source_id":"patchmoe"}),
    );
    assert_eq!(document["ok"], true);
    assert_eq!(document["result"]["rule"]["adapter"], "lua");
    assert!(document["result"]["code"]
        .as_str()
        .unwrap()
        .contains("form_zh_cn"));
    assert!(
        !absent.exists(),
        "reading bundled text must not create state"
    );
    let code = sample_script(
        "collect_index = function() return {entries = ctx.array(), complete = true} end",
        "builtin",
    );
    let response = call(
        temp.path(),
        "import_script_source",
        json!({"source_id":"patchmoe","api_version":1,"code":code}),
    );
    assert_eq!(response["error"]["code"], "source_builtin_readonly");
    assert_eq!(
        call(
            temp.path(),
            "remove_source",
            json!({"source_id":"patchmoe"})
        )["error"]["code"],
        "source_builtin_readonly"
    );
    assert!(call(
        temp.path(),
        "get_script_source",
        json!({"source_id":"patchmoe"})
    )["result"]["code"]
        .as_str()
        .unwrap()
        .contains("form_zh_cn"));

    // A user source can be created, deleted and re-created in the same state.
    let created = call(
        temp.path(),
        "import_script_source",
        json!({"api_version":1,"code":code}),
    );
    assert_eq!(created["ok"], true, "{created}");
    assert_eq!(created["result"]["rule"]["id"], "example-com");
    let removed = call(
        temp.path(),
        "remove_source",
        json!({"source_id":"example-com"}),
    );
    assert_eq!(removed["ok"], true, "{removed}");
    assert!(removed["result"]["settings_revision"].is_number());
    let sources = call(temp.path(), "list_sources", json!({}));
    assert!(
        !sources["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|source| source["id"] == "example-com"),
        "{sources}"
    );
    assert_eq!(
        call(
            temp.path(),
            "remove_source",
            json!({"source_id":"example-com"})
        )["error"]["code"],
        "unknown_source"
    );
    assert_eq!(
        call(&absent, "remove_source", json!({"source_id":"missing"}))["error"]["code"],
        "unknown_source"
    );
    let recreated = call(
        temp.path(),
        "import_script_source",
        json!({"api_version":1,"code":code}),
    );
    assert_eq!(recreated["ok"], true, "{recreated}");
    assert_eq!(recreated["result"]["rule"]["id"], "example-com");
}

#[test]
fn update_task_roundtrips_browser_requests_without_restarting() {
    use std::io::{BufRead, BufReader};

    let state = tempfile::tempdir().unwrap();
    let code = r#"return {
        meta = {
            name = '浏览器示例',
            pages = {'https://example.com/'},
            page_hosts = {'example.com'},
            download_hosts = {'download.example.com'}
        },
        collect_index = function(ctx, rule)
            local page = rule.pages[1]
            local token = ctx.browser_get(page).text
            local response = ctx.browser_post(page, 'application/x-www-form-urlencoded', 'token=' .. token)
            assert(response.text == 'done')
            return {entries = ctx.array(), complete = true}
        end
    }"#;
    let imported = call(
        state.path(),
        "import_script_source",
        json!({"api_version":1,"code":code}),
    );
    assert_eq!(imported["ok"], true, "{imported}");
    let source_id = imported["result"]["rule"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut child = Command::new(env!("CARGO_BIN_EXE_patchhub-core"))
        .arg("serve-task")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    writeln!(
        input,
        "{}",
        json!({
            "protocol_version":5, "request_id":"browser-smoke", "method":"update_source",
            "params":{"source_id":source_id,"host":{"roots":[],"state_dir":state.path(),
              "platform":if cfg!(windows) {"windows"} else {"steamos"}}}
        })
    )
    .unwrap();

    let mut methods = Vec::new();
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let message: Value = serde_json::from_str(&line.unwrap()).unwrap();
        if let Some(request) = message.get("browser_request") {
            let method = request["method"].as_str().unwrap();
            assert_eq!(request["page_hosts"], json!(["example.com"]));
            let text = if method == "GET" {
                assert!(request["body"].is_null());
                "live"
            } else {
                assert_eq!(request["body"], "token=live");
                "done"
            };
            methods.push(method.to_string());
            writeln!(
                input,
                "{}",
                json!({"browser_response":{
                    "url":request["url"],"response_url":request["url"],"method":method,
                    "body":request["body"],"status":200,"text":text
                }})
            )
            .unwrap();
        } else if message.get("ok").is_some() {
            assert_eq!(message["ok"], true, "{message}");
            assert_eq!(message["result"]["complete"], true, "{message}");
        }
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(methods, ["GET", "POST"]);
}
