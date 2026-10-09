mod maintenance;
mod runtime;

use runtime::{execute, Params};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    env,
    io::{self, BufRead, Write},
    path::PathBuf,
    process::ExitCode,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};
use steam_patch_core::catalog::Platform;

const MAX_REQUEST_BYTES: usize = 64 * 1024;
/// p1-host-contract §0.1: v3 core rejects old hosts with `unsupported_protocol`.
const PROTOCOL_VERSION: u32 = 3;
/// Upper bound for `error.detail` (p1-host-contract §3.3).
const MAX_ERROR_DETAIL_BYTES: usize = 2 * 1024;
/// Foreground lease TTL (15s, p1-host-contract §5.2); used by the task control
/// channel's in-memory lease placeholder.
const LEASE_TTL_MILLIS: u64 = 15_000;
/// Methods that must run on the `serve-task` control channel (cancel + renew +
/// foreground lease). The plain `serve` loop rejects them with `task_required`
/// so a non-task host cannot accidentally start an install (p1-host-contract §1.1).
const TASK_CHANNEL_METHODS: &[&str] = &["install_patch", "scan_sources", "refresh_sources"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    protocol_version: u32,
    request_id: String,
    method: String,
    params: Params,
}

/// `request_id` is 1–128 bytes, non-empty and free of control characters
/// (p1-host-contract §0.1).
fn valid_request_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control)
}

/// Truncates `detail` to the frozen 2 KiB ceiling on a UTF-8 boundary.
fn truncate_detail(detail: &str) -> String {
    if detail.len() <= MAX_ERROR_DETAIL_BYTES {
        return detail.to_string();
    }
    let mut end = MAX_ERROR_DETAIL_BYTES;
    while !detail.is_char_boundary(end) {
        end -= 1;
    }
    detail[..end].to_string()
}

fn error_with_detail(id: Option<&str>, code: &str, detail: Option<&str>) -> Value {
    let mut value = json!({
        "protocol_version": PROTOCOL_VERSION,
        "request_id": id,
        "ok": false,
        "error": { "code": code },
    });
    if let Some(detail) = detail {
        value["error"]["detail"] = json!(truncate_detail(detail));
    }
    value
}

fn error(id: Option<&str>, code: &str) -> Value {
    error_with_detail(id, code, None)
}

fn respond(bytes: &[u8]) -> Value {
    let cancel = AtomicBool::new(false);
    respond_with_channel(bytes, &cancel, &|_| true, &|| 0_u64, false)
}

/// `serve` and `serve-task` share this parser/executor; only `serve-task` may
/// dispatch the task-channel methods (`install_patch` and the streaming source
/// tasks), carrying the live lease closures.
fn respond_with<L: Fn(u64) -> bool, D: Fn() -> u64 + Sync>(
    bytes: &[u8],
    cancel: &AtomicBool,
    lease_active: &L,
    lease_deadline_ms: &D,
) -> Value {
    respond_with_channel(bytes, cancel, lease_active, lease_deadline_ms, true)
}

fn respond_with_channel<L: Fn(u64) -> bool, D: Fn() -> u64 + Sync>(
    bytes: &[u8],
    cancel: &AtomicBool,
    lease_active: &L,
    lease_deadline_ms: &D,
    task_channel: bool,
) -> Value {
    let request: Request = match serde_json::from_slice(bytes) {
        Ok(r) => r,
        Err(_) => return error(None, "invalid_request"),
    };
    if request.protocol_version != PROTOCOL_VERSION {
        return error_with_detail(
            Some(&request.request_id),
            "unsupported_protocol",
            Some(&format!(
                "protocol_version {} is not supported; expected {}",
                request.protocol_version, PROTOCOL_VERSION
            )),
        );
    }
    if !valid_request_id(&request.request_id) {
        return error(None, "invalid_request_id");
    }
    if !task_channel && TASK_CHANNEL_METHODS.contains(&request.method.as_str()) {
        return error_with_detail(
            Some(&request.request_id),
            "task_required",
            Some("method requires the serve-task control channel"),
        );
    }
    let mut params = request.params;
    if request.method == "install_patch" && params.request_id.is_none() {
        params.request_id = Some(request.request_id.clone());
    }
    match runtime::execute_controlled(
        &request.method,
        params,
        cancel,
        lease_active,
        lease_deadline_ms,
    ) {
        Ok(result) => {
            json!({ "protocol_version": PROTOCOL_VERSION, "request_id": request.request_id, "ok": true, "result": result })
        }
        Err(code) => {
            let (code, detail) = match code.split_once('\u{1}') {
                Some((code, detail)) => (code.to_string(), Some(detail.to_string())),
                None => {
                    let detail = (code == "unsupported_method")
                        .then(|| format!("method not in v3 whitelist: {}", request.method));
                    (code, detail)
                }
            };
            error_with_detail(Some(&request.request_id), &code, detail.as_deref())
        }
    }
}

/// Read one bounded JSON line without allocating for a hostile unbounded stream.
fn read_request(reader: &mut impl BufRead) -> io::Result<Option<(Vec<u8>, bool)>> {
    let mut bytes = Vec::new();
    let mut oversized = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if bytes.is_empty() && !oversized {
                Ok(None)
            } else {
                Ok(Some((bytes, oversized)))
            };
        }
        let newline = available.iter().position(|b| *b == b'\n');
        let take = newline.map_or(available.len(), |p| p + 1);
        if bytes.len() + take <= MAX_REQUEST_BYTES && !oversized {
            bytes.extend_from_slice(&available[..take]);
        } else {
            oversized = true;
        }
        reader.consume(take);
        if newline.is_some() {
            return Ok(Some((bytes, oversized)));
        }
    }
}

fn serve(reader: &mut impl BufRead, writer: &mut impl Write) -> io::Result<()> {
    while let Some((bytes, oversized)) = read_request(reader)? {
        let response = if oversized {
            error(None, "request_too_large")
        } else {
            respond(&bytes)
        };
        serde_json::to_writer(&mut *writer, &response)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    Ok(())
}

/// One cancellable task. The controller keeps stdin open and may send `renew`
/// (refresh the foreground lease) or `cancel` control lines; EOF is equivalent
/// to `cancel` and reclaims the worker process group (p1-host-contract §1.1 C,
/// §5.2, §5.3). In particular a disconnected Python host cannot leave an
/// installer running unattended.
fn serve_task() -> io::Result<()> {
    let Some((bytes, oversized)) = read_request(&mut io::stdin().lock())? else {
        return Ok(());
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let lease = Arc::new(LeaseState::new());
    spawn_control_thread(Arc::clone(&cancel), Arc::clone(&lease));
    spawn_lease_watchdog(Arc::clone(&cancel), Arc::clone(&lease));
    let response = {
        let lease_active = Arc::clone(&lease);
        let lease_deadline = Arc::clone(&lease);
        run_task_with_lease(
            &bytes,
            oversized,
            cancel,
            move |now| lease_active.active(now),
            move || lease_deadline.deadline_ms(),
            |value| {
                let mut output = io::stdout().lock();
                // A closed output pipe must not abort the rollback path.
                let _ = serde_json::to_writer(&mut output, &value);
                let _ = output.write_all(b"\n");
                let _ = output.flush();
            },
        )
    };
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, &response)?;
    output.write_all(b"\n")?;
    output.flush()
}

/// Epoch milliseconds (foreground lease clock).
fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// In-memory foreground lease (15s). `renew` refreshes the shared deadline; the
/// watchdog and the authorization re-check treat an expired deadline as
/// inactive (p1-host-contract §5.2; p0-contracts §5.1).
struct LeaseState {
    deadline_ms: AtomicU64,
}

impl LeaseState {
    fn new() -> Self {
        Self {
            deadline_ms: AtomicU64::new(now_millis().saturating_add(LEASE_TTL_MILLIS)),
        }
    }

    fn renew(&self) -> bool {
        let now = now_millis();
        let mut previous = self.deadline_ms.load(Ordering::Acquire);
        loop {
            if now >= previous {
                return false;
            }
            match self.deadline_ms.compare_exchange_weak(
                previous,
                now.saturating_add(LEASE_TTL_MILLIS),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(current) => previous = current,
            }
        }
    }

    fn active(&self, now_seconds: u64) -> bool {
        now_seconds.saturating_mul(1000) <= self.deadline_ms.load(Ordering::Relaxed)
    }

    /// Current foreground-lease deadline (epoch ms) for `lease.json`
    /// persistence (p1-zip-orchestration §3.5).
    fn deadline_ms(&self) -> u64 {
        self.deadline_ms.load(Ordering::Relaxed)
    }
}

/// Reads `renew`/`cancel` control lines from the task stdin. EOF (`None` or a
/// read error) is treated as `cancel`.
fn spawn_control_thread(cancel: Arc<AtomicBool>, lease: Arc<LeaseState>) {
    std::thread::spawn(move || {
        use std::io::BufRead;
        let stdin = io::stdin();
        let mut lines = stdin.lock().lines();
        while let Some(Ok(line)) = lines.next() {
            match line.trim() {
                "renew" => {
                    if !lease.renew() {
                        cancel.store(true, Ordering::Release);
                        return;
                    }
                }
                "cancel" => {
                    cancel.store(true, Ordering::Relaxed);
                    return;
                }
                _ => {}
            }
        }
        cancel.store(true, Ordering::Relaxed);
    });
}

/// Expired lease watchdog: stops an unattended task after `LEASE_TTL_MILLIS`
/// without a `renew` (p1-zip-orchestration §3.10).
fn spawn_lease_watchdog(cancel: Arc<AtomicBool>, lease: Arc<LeaseState>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        if !lease.active(now_millis() / 1000) {
            cancel.store(true, Ordering::Relaxed);
            return;
        }
    });
}

/// Runs one `serve-task` request with the always-active placeholder lease
/// (used by tests); `serve_task` uses [`run_task_with_lease`].
#[allow(dead_code)]
pub(crate) fn run_task(
    bytes: &[u8],
    oversized: bool,
    cancel: Arc<AtomicBool>,
    emit: impl FnMut(Value) + 'static,
) -> Value {
    run_task_with_lease(bytes, oversized, cancel, |_| true, || u64::MAX, emit)
}

/// Runs one `serve-task` request: stream methods emit their events, everything
/// else runs under `operation::run` and emits flat `install_progress` events.
/// Returns the single final response line value.
pub(crate) fn run_task_with_lease<L: Fn(u64) -> bool, D: Fn() -> u64 + Sync>(
    bytes: &[u8],
    oversized: bool,
    cancel: Arc<AtomicBool>,
    lease_active: L,
    lease_deadline_ms: D,
    mut emit: impl FnMut(Value) + 'static,
) -> Value {
    let parsed = if oversized {
        None
    } else {
        serde_json::from_slice::<Request>(bytes).ok()
    };
    if let Some(request) = parsed {
        if request.protocol_version == PROTOCOL_VERSION
            && valid_request_id(&request.request_id)
            && runtime::SOURCE_STREAM_METHODS.contains(&request.method.as_str())
        {
            return stream_sources_task(request, &cancel, emit);
        }
        let mut cursor = runtime::ProgressCursor::new(
            PROTOCOL_VERSION,
            request.request_id.clone(),
            request.params.instance_id.clone(),
            request.params.selection_id.clone(),
        );
        return steam_patch_core::operation::run(
            Arc::clone(&cancel),
            move |progress| emit(cursor.event(progress)),
            || {
                if oversized {
                    error(None, "request_too_large")
                } else {
                    respond_with(bytes, cancel.as_ref(), &lease_active, &lease_deadline_ms)
                }
            },
        );
    }
    if oversized {
        error(None, "request_too_large")
    } else {
        let cancel_ref = AtomicBool::new(false);
        respond_with(bytes, &cancel_ref, &|_| true, &|| 0u64)
    }
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
    let response = runtime::execute_streaming(&method, params, cancel.as_ref(), &mut |payload| {
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
    });
    match response {
        Ok(result) => json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": request_id,
            "ok": true,
            "result": result,
        }),
        Err(code) => error(Some(&request_id), &code),
    }
}

const HELP: &str = "Steam patch manager Alpha\n\
Usage: steam-patch scan [--steam-root PATH]... [--catalog PATH] [--platform windows|steamos]\n\
       steam-patch preview --instance ID --release ID [scan options]\n\
       steam-patch serve  (versioned JSON-lines on stdin/stdout)\n\
Installation requires a signed catalog. Close the game and pause Steam updates before installing.
Methods: get_capabilities scan preview install_patch list_transactions recover discard_plan cleanup_cache.
Sources: list_sources list_game_sources set_source_enabled set_game_source add_source refresh_sources scan_sources discover_game.
Options: --signed-catalog --trust-keys --state-dir --archive --operation install|reinstall --request-id ID.
Source options: --source-id ID --appid N --enabled true|false --subscribed true|false --rule JSON --name-hint TEXT --preferred-language CODE --force --generation N --record-id ID.
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
        let value = args.next().ok_or_else(|| format!("missing_value:{flag}"))?;
        match flag.as_str() {
            "--signed-catalog" => params.signed_catalog = Some(PathBuf::from(value)),
            "--trust-keys" => params.trust_keys = Some(PathBuf::from(value)),
            "--state-dir" => params.state_dir = Some(PathBuf::from(value)),
            "--archive" => params.archive = Some(PathBuf::from(value)),
            "--request-id" => params.request_id = Some(value),
            "--source-id" => params.source_id = Some(value),
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
            "--preferred-language" => params.preferred_language = Some(value),
            "--generation" => {
                params.generation = Some(
                    value
                        .parse()
                        .map_err(|_| "invalid_generation".to_string())?,
                )
            }
            "--record-id" => params.record_id = Some(value),
            "--operation" => {
                params.operation = match value.as_str() {
                    "install" => steam_patch_core::install::InstallOperation::Install,
                    "reinstall" => steam_patch_core::install::InstallOperation::Reinstall,
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
    let result = execute(&command, params)?;
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
    use super::*;
    use std::io::Cursor;

    #[test]
    fn expired_lease_cannot_be_renewed() {
        let lease = LeaseState {
            deadline_ms: AtomicU64::new(now_millis().saturating_sub(1)),
        };
        let old = lease.deadline_ms();
        assert!(!lease.renew());
        assert_eq!(lease.deadline_ms(), old);
        let active = LeaseState::new();
        assert!(active.renew());
    }
    #[test]
    fn protocol_recovers_after_bad_or_oversized_line() {
        let root = tempfile::tempdir().unwrap();
        let valid = json!({"protocol_version":3,"request_id":"r1","method":"scan","params":{"roots":[root.path()]}}).to_string();
        let input = format!("broken\n{}\n{valid}\n", "x".repeat(MAX_REQUEST_BYTES + 1));
        let mut output = Vec::new();
        serve(&mut Cursor::new(input), &mut output).unwrap();
        let rows: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["error"]["code"], "invalid_request");
        assert_eq!(rows[1]["error"]["code"], "request_too_large");
        assert_eq!(rows[2]["ok"], true);
        assert_eq!(rows[2]["request_id"], "r1");
    }

    #[test]
    fn protocol_rejects_old_versions_unknown_methods_and_fields() {
        let request = |version, method| {
            json!({"protocol_version":version,"request_id":"r","method":method,"params":{}})
                .to_string()
        };
        for old in [1, 2] {
            let response = respond(request(old, "scan").as_bytes());
            assert_eq!(response["error"]["code"], "unsupported_protocol");
            assert!(response["error"]["detail"].is_string());
        }
        assert_eq!(
            respond(request(3, "uninstall").as_bytes())["error"]["code"],
            "unsupported_method"
        );
        assert_eq!(respond(br#"{"protocol_version":3,"request_id":"r","method":"scan","params":{"shell":"evil"}}"#)["error"]["code"], "invalid_request");
    }

    #[test]
    fn request_id_is_validated_for_length_and_control_characters() {
        let with_id = |id: &str| {
            json!({"protocol_version":3,"request_id":id,"method":"get_capabilities","params":{}})
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

    const V3_METHODS: &[&str] = &[
        "get_capabilities",
        "scan",
        "preview",
        "install_patch",
        "list_transactions",
        "recover",
        "discard_plan",
        "cleanup_cache",
        "list_sources",
        "list_game_sources",
        "set_source_enabled",
        "set_game_source",
        "add_source",
        "refresh_sources",
        "scan_sources",
        "discover_game",
    ];

    #[test]
    fn v3_method_whitelist_is_exact() {
        let mut expected = V3_METHODS.to_vec();
        expected.sort_unstable();
        let mut actual = runtime::METHODS.to_vec();
        actual.sort_unstable();
        assert_eq!(runtime::METHODS.len(), 16);
        assert_eq!(actual, expected);

        let capabilities = respond(
            br#"{"protocol_version":3,"request_id":"c","method":"get_capabilities","params":{}}"#,
        );
        let methods: Vec<&str> = capabilities["result"]["methods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|method| method.as_str().unwrap())
            .collect();
        assert_eq!(methods.len(), 16);
        let mut sorted = methods.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, expected);
    }

    #[test]
    fn get_capabilities_has_v3_installer_and_sources_shape() {
        let capabilities = respond(
            br#"{"protocol_version":3,"request_id":"c","method":"get_capabilities","params":{}}"#,
        );
        let result = &capabilities["result"];
        assert_eq!(result["protocol_version"], 3);
        assert_eq!(result["installer"]["schema_version"], 1);
        assert_eq!(result["installer"]["selection_version"], 1);
        assert_eq!(result["installer"]["formats"]["zip"], "supported");
        assert_eq!(result["installer"]["links"]["zip"]["available"], true);
        assert_eq!(result["installer"]["links"]["exe"]["available"], false);
        assert_eq!(
            result["installer"]["links"]["exe"]["blockers"][0]["code"],
            "exe_sandbox_unavailable"
        );
        assert_eq!(result["sources"]["parser_version"], 2);
        assert_eq!(result["sources"]["query_version"], 1);
        assert_eq!(result["sources"]["methods"].as_array().unwrap().len(), 8);
    }

    #[test]
    fn serve_channel_rejects_task_methods_with_task_required() {
        let request = |method| {
            json!({"protocol_version":3,"request_id":"r","method":method,"params":{}}).to_string()
        };
        // Task-channel methods must not run on the plain `serve` loop.
        for method in ["install_patch", "scan_sources", "refresh_sources"] {
            let response = respond(request(method).as_bytes());
            assert_eq!(response["ok"], false, "method {method}");
            assert_eq!(
                response["error"]["code"], "task_required",
                "method {method} must be refused on serve"
            );
        }
        // Serve-face methods keep working (not refused by the task guard).
        for method in ["get_capabilities", "scan", "preview"] {
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
        ] {
            let request = json!({
                "protocol_version": 3,
                "request_id": "d",
                "method": method,
                "params": {},
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
    fn archive_before_stage_emits_valid_v3_progress() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = Rc::clone(&events);
        let mut cursor =
            runtime::ProgressCursor::new(3, "signed".into(), Some("instance".into()), None);
        steam_patch_core::operation::run(
            Arc::new(AtomicBool::new(false)),
            move |progress| captured.borrow_mut().push(cursor.event(progress)),
            || {
                // Same ordering as signed download and local ZIP verification.
                steam_patch_core::operation::archive(123);
                steam_patch_core::operation::stage("validating", Some(123), Some(1));
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
        use steam_patch_core::operation::Progress;
        let mut cursor = runtime::ProgressCursor::new(
            3,
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
        assert_eq!(first["protocol_version"], 3);
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
    fn error_detail_is_optional_and_truncated_to_2kib() {
        let plain = error(None, "unsupported_method");
        assert!(plain["error"].get("detail").is_none());

        let long = "d".repeat(MAX_ERROR_DETAIL_BYTES + 500);
        let detailed = error_with_detail(Some("r"), "boom", Some(&long));
        let detail = detailed["error"]["detail"].as_str().unwrap();
        assert_eq!(detail.len(), MAX_ERROR_DETAIL_BYTES);
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
            br#"{"protocol_version":3,"request_id":"c","method":"get_capabilities","params":{}}"#,
        );
        assert_eq!(capabilities["ok"], true);
        assert!(capabilities["result"]["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|method| method == "discover_game"));
        let unknown = respond(
            br#"{"protocol_version":3,"request_id":"u","method":"source_magic","params":{}}"#,
        );
        assert_eq!(unknown["error"]["code"], "unsupported_method");
        let missing_state = respond(
            br#"{"protocol_version":3,"request_id":"s","method":"list_sources","params":{}}"#,
        );
        assert_eq!(missing_state["error"]["code"], "state_dir_required");
    }

    #[test]
    fn cleanup_cache_sweeps_cleanable_roots_and_protects_recovery_material() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let paths = steam_patch_core::installer::InstallerPaths::new(&state);
        let downloads = paths.downloads_dir();
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::write(downloads.join("artifact.zip"), b"stale bytes").unwrap();
        std::fs::create_dir_all(paths.content("c1").unwrap()).unwrap();
        std::fs::write(paths.content("c1").unwrap().join("payload"), b"12345").unwrap();
        // Recovery material must survive a cache sweep.
        let job = paths.jobs_dir().join("plan-1");
        std::fs::create_dir_all(&job).unwrap();
        std::fs::write(job.join("0.stage"), b"stage").unwrap();
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
    }
}
