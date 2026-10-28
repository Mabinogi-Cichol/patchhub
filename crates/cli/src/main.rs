mod maintenance;
mod runtime;

use patchhub_core::catalog::Platform;
use runtime::{execute, Params};
use serde_json::{json, Value};
use std::{
    env,
    io::{self, BufRead, Write},
    path::PathBuf,
    process::ExitCode,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use patchhub_protocol::PROTOCOL_VERSION;

/// Internal execution arguments, constructed only after the sole wire decoder.
struct Request {
    request_id: String,
    method: String,
    params: Params,
}
fn parse_request(
    bytes: &[u8],
    task_channel: bool,
) -> Result<Request, patchhub_protocol::ProtocolError> {
    let wire = patchhub_protocol::decode_request(bytes, task_channel)?;
    let params = Params::from_wire(&wire);
    Ok(Request {
        request_id: wire.request_id,
        method: wire.method.as_str().to_owned(),
        params,
    })
}

fn error_with_detail(id: Option<&str>, code: &str, detail: Option<&str>) -> Value {
    let mut value = json!({
        "protocol_version": PROTOCOL_VERSION,
        "request_id": id,
        "ok": false,
        "error": { "code": code },
    });
    if let Some(detail) = detail {
        value["error"]["detail"] = json!(detail);
    }
    value
}

fn respond(bytes: &[u8]) -> Value {
    let cancel = AtomicBool::new(false);
    respond_with_channel(bytes, &cancel, false)
}

/// `serve` and `serve-task` share this parser/executor; only `serve-task` may
/// dispatch the task-channel methods (`install_patch` and the streaming source
/// tasks).
fn respond_with(bytes: &[u8], cancel: &AtomicBool) -> Value {
    respond_with_channel(bytes, cancel, true)
}

fn respond_with_channel(bytes: &[u8], cancel: &AtomicBool, task_channel: bool) -> Value {
    let request = match parse_request(bytes, task_channel) {
        Ok(r) => r,
        Err(e) => return error_with_detail(e.request_id.as_deref(), &e.code, e.detail.as_deref()),
    };
    let params = request.params;
    match runtime::execute_controlled(&request.method, params, cancel) {
        Ok(result) => {
            json!({ "protocol_version": PROTOCOL_VERSION, "request_id": request.request_id, "ok": true, "result": result })
        }
        Err(error) => error_with_detail(
            Some(&request.request_id),
            &error.code,
            error.detail.as_deref(),
        ),
    }
}

/// Read one LF-terminated JSON line. JSON Lines framing has no line-size limit;
/// the method schema and channel rules validate the request instead.
fn read_request(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    if reader.read_until(b'\n', &mut bytes)? == 0 {
        return Ok(None);
    }
    Ok(Some(bytes))
}

fn write_line(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    writer.write_all(&patchhub_protocol::json_line(value))?;
    writer.flush()
}

fn serve(reader: &mut impl BufRead, writer: &mut impl Write) -> io::Result<()> {
    while let Some(bytes) = read_request(reader)? {
        write_line(writer, &respond(&bytes))?;
    }
    Ok(())
}

/// One cancellable task. The controller keeps stdin open and may send `cancel`
/// control lines; EOF is equivalent to `cancel` and reclaims the worker process
/// group (p1-host-contract §1.1 C, §5.3). In particular a disconnected Python
/// host cannot leave an installer running unattended.
fn serve_task() -> io::Result<()> {
    let Some(bytes) = read_request(&mut io::stdin().lock())? else {
        return Ok(());
    };
    let cancel = Arc::new(AtomicBool::new(false));
    spawn_control_thread(Arc::clone(&cancel));
    let response = run_task(&bytes, cancel, |value| {
        let mut output = io::stdout().lock();
        // A closed output pipe must not abort the rollback path.
        let _ = write_line(&mut output, &value);
    });
    let mut output = io::stdout().lock();
    write_line(&mut output, &response)
}

/// Reads `cancel` control lines from the task stdin. EOF (`None` or a read
/// error) is treated as `cancel`.
fn spawn_control_thread(cancel: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        use std::io::BufRead;
        let stdin = io::stdin();
        let mut lines = stdin.lock().lines();
        while let Some(Ok(line)) = lines.next() {
            if line.trim() == "cancel" {
                cancel.store(true, Ordering::Relaxed);
                return;
            }
        }
        cancel.store(true, Ordering::Relaxed);
    });
}

/// Runs one `serve-task` request: stream methods emit their events, everything
/// else runs under `operation::run` and emits flat `install_progress` events.
/// Returns the single final response line value.
pub(crate) fn run_task(
    bytes: &[u8],
    cancel: Arc<AtomicBool>,
    mut emit: impl FnMut(Value) + 'static,
) -> Value {
    if let Ok(request) = parse_request(bytes, true) {
        if runtime::SOURCE_STREAM_METHODS.contains(&request.method.as_str()) {
            return stream_sources_task(request, &cancel, emit);
        }
        let mut cursor = runtime::ProgressCursor::new(
            PROTOCOL_VERSION,
            request.request_id.clone(),
            request.params.instance_id.clone(),
            request.params.selection_id.clone(),
        );
        return patchhub_core::operation::run(
            Arc::clone(&cancel),
            move |progress| {
                if progress.stage != "finalizing" {
                    emit(cursor.event(progress));
                }
            },
            || respond_with(bytes, cancel.as_ref()),
        );
    }
    let cancel_ref = AtomicBool::new(false);
    respond_with(bytes, &cancel_ref)
}

/// Streams a sources task: `source_progress`/`source_update` lines carrying the
/// request id and a monotonic sequence, then the single final response.
fn stream_sources_task(
    request: Request,
    cancel: &Arc<AtomicBool>,
    mut emit: impl FnMut(Value),
) -> Value {
    let Request {
        request_id,
        method,
        params,
        ..
    } = request;
    let mut sequence = 0_u64;
    // Install the operation scope so `operation::check` inside the source
    // fetchers observes this stream task's cancel flag.
    let response = patchhub_core::operation::with_cancel(cancel.clone(), || {
        runtime::execute_streaming(&method, params, cancel.as_ref(), &mut |payload| {
            let mut object = match payload {
                Value::Object(map) => map,
                other => {
                    let mut map = serde_json::Map::new();
                    map.insert("payload".to_string(), other);
                    map
                }
            };
            object.insert("protocol_version".to_string(), json!(PROTOCOL_VERSION));
            object.insert("request_id".to_string(), json!(request_id.clone()));
            object.insert("sequence".to_string(), json!(sequence));
            sequence += 1;
            emit(Value::Object(object));
        })
    });
    match response {
        Ok(result) => json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": request_id,
            "ok": true,
            "result": result,
        }),
        Err(error) => error_with_detail(Some(&request_id), &error.code, error.detail.as_deref()),
    }
}

const HELP: &str = "PatchHub Core\n\
Usage: patchhub-core scan [--steam-root PATH]... [--catalog PATH] [--platform windows|steamos]\n\
       patchhub-core preview --instance ID --release ID [scan options]\n\
       patchhub-core serve  (versioned JSON-lines on stdin/stdout)\n\
Installation requires a signed catalog. Close the game and pause Steam updates before installing.
Methods: get_capabilities scan project_instances preview install_patch list_transactions recover discard_plan cleanup_cache cache_status backup_status cleanup_backups.
Sources: list_sources list_game_sources set_source_enabled set_game_source add_source set_source_update_interval update_source match_games rematch_game.
Options: --signed-catalog --trust-keys --state-dir --archive --operation install|reinstall --request-id ID.
Source options: --source-id ID --appid N --enabled true|false --subscribed true|false --rule JSON --interval-hours N --name-hint TEXT --force --force-details --instance-id ID --match-instance ID.
Maintenance: keygen PRIVATE PUBLIC KEY_ID; sign-catalog CATALOG PRIVATE KEY_ID OUTPUT LIFETIME_SECONDS; schema OUTPUT.";

fn parse_bool(value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("invalid_boolean".into()),
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        println!("{HELP}");
        return Ok(());
    };
    if ["--help", "-h", "help"].contains(&command.as_str()) {
        println!("{HELP}");
        return Ok(());
    }
    if command == "serve-task" {
        if args.next().is_some() {
            return Err("serve_accepts_no_arguments".into());
        }
        return serve_task().map_err(|e| e.to_string());
    }
    if command == "serve" {
        if args.next().is_some() {
            return Err("serve_accepts_no_arguments".into());
        }
        return serve(&mut io::stdin().lock(), &mut io::stdout().lock()).map_err(|e| e.to_string());
    }
    if ["keygen", "sign-catalog", "schema"].contains(&command.as_str()) {
        return maintenance::run(&command, &args.collect::<Vec<_>>());
    }
    let mut params = Params::default();
    while let Some(flag) = args.next() {
        if flag == "--force" {
            params.force = true;
            continue;
        }
        if flag == "--force-details" {
            params.force_details = Some(true);
            continue;
        }
        let value = args.next().ok_or_else(|| format!("missing_value:{flag}"))?;
        match flag.as_str() {
            "--signed-catalog" => params.signed_catalog = Some(PathBuf::from(value)),
            "--trust-keys" => params.trust_keys = Some(PathBuf::from(value)),
            "--state-dir" => params.state_dir = Some(PathBuf::from(value)),
            "--archive" => params.archive = Some(PathBuf::from(value)),
            "--request-id" => params.request_id = Some(value),
            "--source-id" => params.source_id = Some(value),
            "--interval-hours" => {
                params.interval_hours = Some(
                    value
                        .parse()
                        .map_err(|_| "invalid_interval_hours".to_string())?,
                )
            }
            "--instance-id" => params.instance_ids.push(value),
            "--match-instance" => params.instances.push(value),
            "--appid" => {
                params.appid = Some(value.parse().map_err(|_| "invalid_appid".to_string())?)
            }
            "--enabled" => params.enabled = Some(parse_bool(&value)?),
            "--subscribed" => params.subscribed = Some(parse_bool(&value)?),
            "--rule" => {
                params.rule = Some(
                    serde_json::from_str(&value).map_err(|_| "invalid_rule_json".to_string())?,
                )
            }
            "--name-hint" => params.name_hints.push(value),
            "--operation" => {
                params.operation = match value.as_str() {
                    "install" => patchhub_core::install::InstallOperation::Install,
                    "reinstall" => patchhub_core::install::InstallOperation::Reinstall,
                    "overwrite" => patchhub_core::install::InstallOperation::Overwrite,
                    _ => return Err("invalid_operation".into()),
                }
            }
            "--steam-root" => params.roots.push(PathBuf::from(value)),
            "--catalog" if params.catalog.is_none() => params.catalog = Some(PathBuf::from(value)),
            "--instance" if params.instance_id.is_none() => params.instance_id = Some(value),
            "--release" if params.release_id.is_none() => params.release_id = Some(value),
            "--platform" if params.platform.is_none() => {
                params.platform = Some(match value.as_str() {
                    "windows" => Platform::Windows,
                    "steamos" => Platform::Steamos,
                    _ => return Err("invalid_platform".into()),
                })
            }
            _ => return Err(format!("unknown_or_duplicate_option:{flag}")),
        }
    }
    let result = execute(&command, params).map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    pub(crate) fn wire_params(method: &str, mut legacy: Value) -> Value {
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

    use super::*;
    use std::io::Cursor;

    #[test]
    fn protocol_recovers_after_a_bad_line_larger_than_the_removed_cap() {
        let root = tempfile::tempdir().unwrap();
        let valid = json!({"protocol_version":5,"request_id":"r1","method":"scan","params":{"host":{"roots":[root.path()],"catalog":null,"signed_catalog":null,"trust_keys":null,"state_dir":null,"platform":"windows"}}}).to_string();
        // A line past the old 64 KiB cap is framed and parsed like any other.
        let input = format!("{}\nbroken\n{valid}\n", "x".repeat(70_000));
        let mut output = Vec::new();
        serve(&mut Cursor::new(input), &mut output).unwrap();
        let rows: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["error"]["code"], "invalid_request");
        assert_eq!(rows[1]["error"]["code"], "invalid_request");
        assert_eq!(rows[2]["ok"], true);
        assert_eq!(rows[2]["request_id"], "r1");
    }

    #[test]
    fn large_valid_request_is_parsed_without_a_line_cap() {
        let hints: Vec<String> = (0..40)
            .map(|index| format!("{index}-{}", "界".repeat(2000)))
            .collect();
        let request = json!({
            "protocol_version":5,
            "request_id":"big",
            "method":"match_games",
            "params":{"host":{"roots":[],"catalog":null,"signed_catalog":null,"trust_keys":null,"state_dir":null,"platform":"windows"},"name_hints":hints},
        });
        let bytes = serde_json::to_vec(&request).unwrap();
        assert!(bytes.len() > 64 * 1024);
        assert_eq!(parse_request(&bytes, true).unwrap().method, "match_games");
    }

    #[test]
    fn protocol_rejects_old_versions_unknown_methods_and_fields() {
        let request = |version, method| {
            json!({"protocol_version":version,"request_id":"r","method":method,"params":{"host":{"roots":[],"catalog":null,"signed_catalog":null,"trust_keys":null,"state_dir":null,"platform":"windows"}}})
                .to_string()
        };
        for old in [1, 2, 3, 4] {
            let response = respond(request(old, "scan").as_bytes());
            assert_eq!(response["error"]["code"], "unsupported_protocol");
        }
        assert_eq!(
            respond(request(5, "uninstall").as_bytes())["error"]["code"],
            "unsupported_method"
        );
        assert_eq!(respond(br#"{"protocol_version":5,"request_id":"r","method":"scan","params":{"host":{"roots":[],"catalog":null,"signed_catalog":null,"trust_keys":null,"state_dir":null,"platform":"windows"},"shell":"evil"}}"#)["error"]["code"], "invalid_request");
    }

    #[test]
    fn request_id_is_validated_for_length_and_control_characters() {
        let with_id = |id: &str| {
            json!({"protocol_version":5,"request_id":id,"method":"get_capabilities","params":{"host":{"roots":[],"catalog":null,"signed_catalog":null,"trust_keys":null,"state_dir":null,"platform":"windows"}}})
                .to_string()
        };
        assert_eq!(
            respond(with_id("").as_bytes())["error"]["code"],
            "invalid_request_id"
        );
        assert_eq!(
            respond(with_id(&"a".repeat(129)).as_bytes())["error"]["code"],
            "invalid_request_id"
        );
        assert_eq!(
            respond(with_id("bad\u{1}id").as_bytes())["error"]["code"],
            "invalid_request_id"
        );
        let accepted = respond(with_id(&"a".repeat(128)).as_bytes());
        assert_eq!(accepted["ok"], true);
    }

    const V5_METHODS: &[&str] = &[
        "get_capabilities",
        "scan",
        "project_instances",
        "preview",
        "install_patch",
        "list_transactions",
        "recover",
        "discard_plan",
        "cleanup_cache",
        "cache_status",
        "backup_status",
        "cleanup_backups",
        "list_sources",
        "list_game_sources",
        "set_source_enabled",
        "set_game_source",
        "add_source",
        "set_source_update_interval",
        "update_source",
        "match_games",
        "rematch_game",
    ];

    #[test]
    fn v5_method_whitelist_is_exact() {
        let mut expected = V5_METHODS.to_vec();
        expected.sort_unstable();
        let mut actual = runtime::METHODS.to_vec();
        actual.sort_unstable();
        assert_eq!(runtime::METHODS.len(), 21);
        assert_eq!(actual, expected);

        let capabilities = respond(
            br#"{"protocol_version":5,"request_id":"c","method":"get_capabilities","params":{"host":{"roots":[],"catalog":null,"signed_catalog":null,"trust_keys":null,"state_dir":null,"platform":"windows"}}}"#,
        );
        let methods: Vec<&str> = capabilities["result"]["methods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|method| method.as_str().unwrap())
            .collect();
        assert_eq!(methods.len(), 21);
        let mut sorted = methods.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, expected);
    }

    #[test]
    fn get_capabilities_has_v5_installer_and_sources_shape() {
        let capabilities = respond(
            br#"{"protocol_version":5,"request_id":"c","method":"get_capabilities","params":{"host":{"roots":[],"catalog":null,"signed_catalog":null,"trust_keys":null,"state_dir":null,"platform":"windows"}}}"#,
        );
        let result = &capabilities["result"];
        assert_eq!(result["protocol_version"], 5);
        assert_eq!(result["installer"]["schema_version"], 1);
        assert_eq!(result["installer"]["selection_version"], 1);
        assert_eq!(result["installer"]["formats"]["zip"], "supported");
        assert_eq!(result["installer"]["links"]["zip"]["available"], true);
        assert_eq!(result["installer"]["links"]["exe"]["available"], true);
        assert_eq!(
            result["installer"]["links"]["exe"]["blockers"],
            serde_json::json!([])
        );
        assert_eq!(result["sources"]["index_schema_version"], 1);
        assert_eq!(result["sources"]["detail_schema_version"], 1);
        assert_eq!(result["sources"]["match_schema_version"], 1);
        assert_eq!(result["sources"]["schedule_schema_version"], 1);
        assert_eq!(result["sources"]["parser_version"], 2);
        assert_eq!(result["sources"]["encoding_version"], 1);
        assert_eq!(result["sources"]["methods"].as_array().unwrap().len(), 9);
    }

    #[test]
    fn serve_channel_rejects_task_methods_with_task_required() {
        let request = |method| {
            json!({"protocol_version":5,"request_id":"r","method":method,"params":{"host":{"roots":[],"catalog":null,"signed_catalog":null,"trust_keys":null,"state_dir":null,"platform":"windows"}}}).to_string()
        };
        // Task-channel methods must not run on the plain `serve` loop.
        for method in [
            "install_patch",
            "update_source",
            "match_games",
            "rematch_game",
        ] {
            let response = respond(request(method).as_bytes());
            assert_eq!(response["ok"], false, "method {method}");
            assert_eq!(
                response["error"]["code"], "task_required",
                "method {method} must be refused on serve"
            );
        }
        // Serve-face methods keep working (not refused by the task guard).
        for method in ["get_capabilities", "scan", "preview", "project_instances"] {
            let response = respond(request(method).as_bytes());
            assert_ne!(
                response["error"]["code"], "task_required",
                "method {method} must remain a serve method"
            );
        }
    }

    #[test]
    fn deleted_methods_return_unsupported_method() {
        for method in [
            "plan_install",
            "start_install",
            "fetch_artifact",
            "migrate_state",
            "get_source_state",
            "get_source_settings",
            "list_source_matches",
            "get_source_policy",
            "resolve_source_record",
            "refresh_sources",
            "scan_sources",
            "discover_game",
        ] {
            let request = json!({
                "protocol_version": 5,
                "request_id": "d",
                "method": method,
                "params": {"host":{"roots":[],"catalog":null,"signed_catalog":null,"trust_keys":null,"state_dir":null,"platform":"windows"}},
            })
            .to_string();
            assert_eq!(
                respond(request.as_bytes())["error"]["code"],
                "unsupported_method",
                "deleted method {method} must be rejected"
            );
        }
    }

    #[test]
    fn archive_before_stage_emits_valid_v5_progress() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = Rc::clone(&events);
        let mut cursor =
            runtime::ProgressCursor::new(5, "signed".into(), Some("instance".into()), None);
        patchhub_core::operation::run(
            Arc::new(AtomicBool::new(false)),
            move |progress| captured.borrow_mut().push(cursor.event(progress)),
            || {
                // Same ordering as signed download and local ZIP verification.
                patchhub_core::operation::archive(123);
                patchhub_core::operation::stage("validating", Some(123), Some(1));
            },
        );
        let rows = events.borrow();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["stage"], "connecting");
        assert_eq!(rows[0]["archive_bytes"], 123);
        assert_eq!(rows[0]["sequence"], 0);
        assert_eq!(rows[0]["request_id"], "signed");
        assert_eq!(rows[0]["instance_id"], "instance");
        assert!(rows[0]["selection_id"].is_null());
        assert_eq!(rows[1]["stage"], "validating");
        assert_eq!(rows[1]["sequence"], 1);
    }

    #[test]
    fn install_progress_fields_and_monotonic_sequence() {
        use patchhub_core::operation::Progress;
        let mut cursor = runtime::ProgressCursor::new(
            5,
            "req".to_string(),
            Some("inst".to_string()),
            Some("sel".to_string()),
        );
        let first = cursor.event(&Progress {
            stage: "writing".to_string(),
            bytes_done: 5,
            total_bytes: None,
            files_done: 1,
            total_files: None,
            current_file: Some("a.dat".to_string()),
            archive_bytes: Some(99),
        });
        assert_eq!(first["event"], "install_progress");
        assert_eq!(first["protocol_version"], 5);
        assert_eq!(first["request_id"], "req");
        assert_eq!(first["sequence"], 0);
        assert_eq!(first["instance_id"], "inst");
        assert_eq!(first["selection_id"], "sel");
        assert_eq!(first["stage"], "applying");
        assert_eq!(first["substage"], "writing");
        assert_eq!(first["bytes_done"], 5);
        assert!(first["total_bytes"].is_null());
        assert_eq!(first["files_done"], 1);
        assert!(first["total_files"].is_null());
        assert_eq!(first["current_file"], "a.dat");
        assert_eq!(first["archive_bytes"], 99);

        let second = cursor.event(&Progress {
            stage: "cleanup".to_string(),
            ..Default::default()
        });
        assert_eq!(second["sequence"], 1);
        assert!(second["substage"].is_null());
        assert!(second["current_file"].is_null());
        assert_eq!(cursor.sequence, 2);
    }

    #[test]
    fn error_detail_is_optional_and_not_truncated() {
        let plain = error_with_detail(None, "unsupported_method", None);
        assert!(plain["error"].get("detail").is_none());

        let long = "补".repeat(5000);
        let detailed = error_with_detail(Some("r"), "boom", Some(&long));
        assert_eq!(detailed["error"]["detail"].as_str().unwrap(), long);
    }

    #[test]
    fn response_beyond_8mib_is_emitted_complete_and_keeps_the_channel_usable() {
        let id = "big";
        let payload = 9 * 1024 * 1024;
        let mut response = json!({"protocol_version":5,"request_id":id,"ok":true,"result":""});
        response["result"] = json!("a".repeat(payload));
        let line = patchhub_protocol::json_line(&response);
        assert!(line.len() > 8 * 1024 * 1024);
        assert!(line.ends_with(b"\n"));
        let decoded: Value = serde_json::from_slice(&line).unwrap();
        assert_eq!(decoded["ok"], true);
        assert_eq!(decoded["result"].as_str().unwrap().len(), payload);
        let next = patchhub_protocol::json_line(&error_with_detail(
            Some("next"),
            "unknown_future_code",
            None,
        ));
        assert_eq!(
            serde_json::from_slice::<Value>(&next).unwrap()["request_id"],
            "next"
        );
    }

    #[test]
    fn committed_outcome_with_maximum_escaped_cleanup_diagnostics_is_complete() {
        let mut result = patchhub_protocol::fixtures::outcome("committed", "completed");
        result["cleanup"] = json!({"status":"partial","performed":true,"removed_bytes":0,
            "failures": vec![json!({"code":"cleanup_failed","detail":"\u{1}".repeat(2048)});512]});
        let value = json!({"protocol_version":5,"request_id":"fixture","ok":true,"result":result});
        patchhub_protocol::validate_response(
            &value,
            "fixture",
            Some(patchhub_protocol::Method::InstallPatch),
        )
        .unwrap();
        let line = patchhub_protocol::json_line(&value);
        let decoded: Value = serde_json::from_slice(&line).unwrap();
        assert_eq!(decoded["result"]["disposition"], "completed");
        assert_eq!(
            decoded["result"]["cleanup"]["failures"]
                .as_array()
                .unwrap()
                .len(),
            512
        );
    }

    #[test]
    fn v5_capabilities_pass_the_shared_response_validator() {
        let value = respond(json!({"protocol_version":5,"request_id":"cap","method":"get_capabilities","params":{"host":{"roots":[],"catalog":null,"signed_catalog":null,"trust_keys":null,"state_dir":null,"platform":"windows"}}}).to_string().as_bytes());
        patchhub_protocol::validate_response(
            &value,
            "cap",
            Some(patchhub_protocol::Method::GetCapabilities),
        )
        .unwrap();
    }

    #[test]
    fn scan_to_preview_preserves_instance_and_is_read_only() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Steam");
        std::fs::create_dir_all(root.join("steamapps/common/Test/game")).unwrap();
        std::fs::write(root.join("steamapps/appmanifest_42.acf"),
            "\"AppState\" { \"appid\" \"42\" \"name\" \"Synthetic\" \"installdir\" \"Test\" \"buildid\" \"100\" \"StateFlags\" \"4\" }").unwrap();
        let hash = "0".repeat(64);
        let catalog_path = temp.path().join("catalog.json");
        let data = json!({"schema_version":1,"revision":1,"releases":[{
            "patch_id":"test","release_id":"test-v1","appid":42,"name":"Synthetic patch",
            "version":"1","language":"zh-CN","category":"content_restore",
            "source_page":"https://example.invalid/patch","support_level":"managed_zip",
            "platforms":["windows"],"verified_buildids":["100"],
            "detection":[{"path":"game/patch.rpa","sha256":hash}],
            "recipe":{"artifact_sha256":hash,"files":[{"source":"patch.rpa","destination":"game/patch.rpa",
                "original_sha256":null,"installed_sha256":hash}]}
        }]});
        std::fs::write(&catalog_path, data.to_string()).unwrap();
        let params = |instance| Params {
            roots: vec![root.clone()],
            catalog: Some(catalog_path.clone()),
            platform: Some(Platform::Windows),
            instance_id: instance,
            release_id: None,
            ..Default::default()
        };
        let scan = execute("scan", params(None)).unwrap();
        assert_eq!(scan["installs"][0]["match_status"], "matched");
        assert_eq!(
            scan["installs"][0]["patches"][0]["detection"]["status"],
            "not_detected"
        );
        let id = scan["installs"][0]["game"]["instance_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut preview_params = params(Some(id));
        preview_params.release_id = Some("test-v1".into());
        let result = execute("preview", preview_params).unwrap();
        assert_eq!(result["executable"], false);
        assert_eq!(result["changes"][0]["baseline_matches"], true);
        assert!(!root.join("steamapps/common/Test/game/patch.rpa").exists());
    }

    #[test]
    fn protocol_exposes_sources_capabilities_and_rejects_unknown() {
        let capabilities = respond(
            br#"{"protocol_version":5,"request_id":"c","method":"get_capabilities","params":{"host":{"roots":[],"catalog":null,"signed_catalog":null,"trust_keys":null,"state_dir":null,"platform":"windows"}}}"#,
        );
        assert_eq!(capabilities["ok"], true);
        assert!(capabilities["result"]["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|method| method == "match_games"));
        let unknown = respond(
            br#"{"protocol_version":5,"request_id":"u","method":"source_magic","params":{"host":{"roots":[],"catalog":null,"signed_catalog":null,"trust_keys":null,"state_dir":null,"platform":"windows"}}}"#,
        );
        assert_eq!(unknown["error"]["code"], "unsupported_method");
        let missing_state = respond(
            br#"{"protocol_version":5,"request_id":"s","method":"list_sources","params":{"host":{"roots":[],"catalog":null,"signed_catalog":null,"trust_keys":null,"state_dir":null,"platform":"windows"}}}"#,
        );
        assert_eq!(missing_state["error"]["code"], "state_dir_required");
    }

    #[test]
    fn cleanup_cache_sweeps_cleanable_roots_and_protects_recovery_material() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let paths = patchhub_core::installer::InstallerPaths::new(&state);
        let downloads = paths.downloads_dir();
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::write(downloads.join("artifact.zip"), b"stale bytes").unwrap();
        std::fs::create_dir_all(paths.content("c1").unwrap()).unwrap();
        std::fs::write(paths.content("c1").unwrap().join("payload"), b"12345").unwrap();
        // Recovery material must survive a cache sweep.
        let job = paths.jobs_dir().join("plan-1");
        std::fs::create_dir_all(&job).unwrap();
        std::fs::write(job.join("0.stage"), b"stage").unwrap();
        let host = json!({"roots": [], "catalog": null, "signed_catalog": null, "trust_keys": null, "state_dir": state, "platform": if cfg!(windows) {"windows"} else {"steamos"}});
        let request = |method: &str| {
            respond(serde_json::to_vec(&json!({"protocol_version":5,"request_id":"cache-rpc","method":method,"params":{"host":host}})).unwrap().as_slice())
        };
        let inventory = request("cache_status");
        assert_eq!(inventory["ok"], true, "{inventory}");
        assert_eq!(inventory["result"]["total_bytes"], 16);
        assert_eq!(inventory["result"]["entries"], 3);
        assert_eq!(inventory["result"]["diagnostics"], json!([]));
        assert!(downloads.join("artifact.zip").exists());
        let result = execute(
            "cleanup_cache",
            Params {
                state_dir: Some(state.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result["performed"], true);
        assert_eq!(result["removed_bytes"], 16);
        assert!(!downloads.join("artifact.zip").exists());
        assert!(!paths.content("c1").unwrap().exists());
        assert!(job.join("0.stage").exists());
        let inventory = request("cache_status");
        assert_eq!(inventory["ok"], true, "{inventory}");
        assert_eq!(inventory["result"]["total_bytes"], 0);
        assert_eq!(inventory["result"]["entries"], 0);
    }

    #[test]
    fn early_errors_match_the_generated_error_envelope() {
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(
            "../../../packages/contracts/fixtures/v5/cases.json"
        ))
        .unwrap();
        let cases = fixtures["early_errors"].as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let raw = if let Some(json) = case["json"].as_str() {
                json.as_bytes().to_vec()
            } else {
                case["hex"]
                    .as_str()
                    .unwrap()
                    .as_bytes()
                    .chunks_exact(2)
                    .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                    .collect()
            };
            assert_eq!(respond(&raw), case["response"], "{}", case["name"]);
        }
    }
    #[test]
    fn backup_rpc_inventory_cleanup_and_unknown_path_are_contract_checked() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let paths = patchhub_core::installer::InstallerPaths::new(&state);
        let job = paths.jobs_dir().join("orphan");
        std::fs::create_dir_all(&job).unwrap();
        std::fs::write(job.join("0.backup"), b"original").unwrap();
        std::fs::write(job.join("0.stage"), b"stage").unwrap();
        let host = json!({"state_dir":state,"roots":[dir.path().join("no-steam")],"catalog":null,"signed_catalog":null,"trust_keys":null,"platform":if cfg!(windows) {"windows"} else {"steamos"}});
        let request = |method: &str, params: Value| {
            respond(serde_json::to_vec(&json!({"protocol_version":5,"request_id":"backup-rpc","method":method,"params":params})).unwrap().as_slice())
        };
        let inventory = request("backup_status", json!({"host":host}));
        assert_eq!(inventory["ok"], true);
        assert_eq!(inventory["result"]["files"], 1);
        let rejected = request("cleanup_backups", json!({"host":host,"path":dir.path()}));
        assert_eq!(rejected["ok"], false);
        assert!(job.join("0.backup").exists());
        let result = request("cleanup_backups", json!({"host":host}));
        assert_eq!(result["ok"], true);
        assert_eq!(result["result"]["removed_files"], 1);
        assert_eq!(result["result"]["removed_bytes"], 8);
        assert!(job.join("0.stage").exists());
    }
}
