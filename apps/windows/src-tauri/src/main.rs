#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

const MAX_RESPONSE: u64 = 8 * 1024 * 1024;
const MAX_REQUEST: usize = 64 * 1024;

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Scan,
    Preview,
    ListTransactions,
    Recover,
    MigrateState,
    GetCapabilities,
    ListSources,
    ListGameSources,
    SetSourceEnabled,
    SetGameSource,
    AddSource,
    RefreshSources,
    DiscoverGame,
    GetSourcePolicy,
    ResolveSourceRecord,
}

impl Operation {
    fn method(&self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Preview => "preview",
            Self::ListTransactions => "list_transactions",
            Self::Recover => "recover",
            Self::MigrateState => "migrate_state",
            Self::GetCapabilities => "get_capabilities",
            Self::ListSources => "list_sources",
            Self::ListGameSources => "list_game_sources",
            Self::SetSourceEnabled => "set_source_enabled",
            Self::SetGameSource => "set_game_source",
            Self::AddSource => "add_source",
            Self::RefreshSources => "refresh_sources",
            Self::DiscoverGame => "discover_game",
            Self::GetSourcePolicy => "get_source_policy",
            Self::ResolveSourceRecord => "resolve_source_record",
        }
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

fn request_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("host-{}-{}", std::process::id(), nanos)
}

fn binary(app: &AppHandle) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|_| "app_path_unavailable")?;
    let bundled = exe.with_file_name("steam-patch.exe");
    if bundled.is_file() {
        return Ok(bundled);
    }
    #[cfg(debug_assertions)]
    {
        let development = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../target/debug/steam-patch.exe");
        if development.is_file() {
            return Ok(development);
        }
    }
    let _ = app;
    Err("core_binary_missing".into())
}

fn config_file(app: &AppHandle, name: &str) -> Option<PathBuf> {
    let resource = app.path().resource_dir().ok()?.join("resources").join(name);
    if resource.is_file() {
        return Some(resource);
    }
    #[cfg(debug_assertions)]
    {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../catalog");
        let dev = root.join(name);
        if dev.is_file() {
            return Some(dev);
        }
        if name == "catalog.json" {
            let dev = root.join("catalog.json");
            if dev.is_file() {
                return Some(dev);
            }
        }
    }
    None
}

#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CallArguments {
    instance_id: Option<String>,
    release_id: Option<String>,
    source_id: Option<String>,
    enabled: Option<bool>,
    subscribed: Option<bool>,
    rule: Option<Value>,
    force: Option<bool>,
    record_id: Option<String>,
    name_hints: Option<Vec<String>>,
    preferred_language: Option<String>,
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct Progress {
    stage: String,
    bytes_done: u64,
    total_bytes: Option<u64>,
    files_done: usize,
    total_files: Option<usize>,
    current_file: Option<String>,
    archive_bytes: Option<u64>,
}

type ChildHandle = Arc<Mutex<Child>>;
type StdinHandle = Arc<Mutex<ChildStdin>>;

#[derive(Clone)]
struct ActiveJob {
    child: ChildHandle,
    stdin: StdinHandle,
    progress: Arc<Mutex<Progress>>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Clone, Default)]
struct Jobs(Arc<Mutex<Option<ActiveJob>>>);

/// Latest `source_progress` payload, mirrored for `source_scan_status`.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct SourceProgress {
    #[serde(default)]
    completed: usize,
    #[serde(default)]
    total: usize,
    #[serde(default)]
    running: bool,
    #[serde(default)]
    settings_revision: u64,
}

#[derive(Clone)]
struct SourceScanJob {
    child: ChildHandle,
    stdin: StdinHandle,
    progress: Arc<Mutex<SourceProgress>>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
struct SourceScan {
    active: Option<SourceScanJob>,
    /// Final `scan_sources` result, retained after the task finishes so the
    /// frontend can snapshot it through `source_scan_status`.
    results: Option<Value>,
}

#[derive(Clone, Default)]
struct SourceScans(Arc<Mutex<SourceScan>>);

#[derive(Clone, Default)]
struct Closing(Arc<AtomicBool>);

fn common_params(app: &AppHandle) -> Result<Value, String> {
    let state_dir = app
        .path()
        .app_data_dir()
        .map_err(|_| "state_path_unavailable")?;
    std::fs::create_dir_all(&state_dir).map_err(|_| "state_path_unavailable")?;
    let mut params = json!({"platform":"windows", "state_dir":state_dir});
    if let Some(path) = config_file(app, "catalog.json") {
        params["catalog"] = json!(path);
    }
    if let Some(path) = config_file(app, "signed-catalog.json") {
        params["signed_catalog"] = json!(path);
    }
    if let Some(path) = config_file(app, "trust-keys.json") {
        params["trust_keys"] = json!(path);
    }
    Ok(params)
}

fn response_error(response: &Value) -> String {
    response["error"]["code"]
        .as_str()
        .unwrap_or("core_error")
        .to_string()
}

fn validate_response(response: Value, id: &str) -> Result<Value, String> {
    if response["protocol_version"] != 2
        || response["request_id"] != id
        || !response["ok"].is_boolean()
    {
        return Err("invalid_core_response".into());
    }
    if response["ok"] == true {
        Ok(response["result"].clone())
    } else {
        Err(response_error(&response))
    }
}

fn read_all_bounded(mut reader: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut oversized = false;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|_| "core_process_failed")?;
        if count == 0 {
            break;
        }
        if bytes.len().saturating_add(count) <= MAX_RESPONSE as usize && !oversized {
            bytes.extend_from_slice(&buffer[..count]);
        } else {
            oversized = true;
        }
    }
    if oversized {
        Err("response_too_large".into())
    } else {
        Ok(bytes)
    }
}

/// Validates the (camelCase, `deny_unknown_fields`) host arguments for a given
/// method and merges them into the fixed host-injected `common` params. Kept
/// pure so the request shape can be asserted without spawning the core.
fn build_params(method: &str, args: CallArguments, common: Value) -> Result<Value, String> {
    let CallArguments {
        instance_id,
        release_id,
        source_id,
        enabled,
        subscribed,
        rule,
        force,
        record_id,
        name_hints,
        preferred_language,
    } = args;
    for id in [&instance_id, &release_id, &source_id, &record_id]
        .into_iter()
        .flatten()
    {
        if !valid_id(id) {
            return Err("invalid_id".into());
        }
    }
    if method == "preview" && (instance_id.is_none() || release_id.is_none()) {
        return Err("instance_and_release_required".into());
    }
    if [
        "list_game_sources",
        "set_game_source",
        "discover_game",
        "resolve_source_record",
    ]
    .contains(&method)
        && instance_id.is_none()
    {
        return Err("instance_required".into());
    }
    if ["set_source_enabled", "set_game_source", "get_source_policy"].contains(&method)
        && source_id.is_none()
    {
        return Err("source_id_required".into());
    }
    if method == "set_source_enabled" && enabled.is_none() {
        return Err("enabled_required".into());
    }
    if method == "set_game_source" && subscribed.is_none() {
        return Err("subscribed_required".into());
    }
    if method == "add_source" && rule.is_none() {
        return Err("rule_required".into());
    }
    if method == "resolve_source_record" && record_id.is_none() && release_id.is_none() {
        return Err("record_required".into());
    }

    let mut params = common;
    if let Some(value) = instance_id {
        params["instance_id"] = json!(value);
    }
    if let Some(value) = release_id {
        params["release_id"] = json!(value);
    }
    if let Some(value) = source_id {
        params["source_id"] = json!(value);
    }
    if let Some(value) = enabled {
        params["enabled"] = json!(value);
    }
    if let Some(value) = subscribed {
        params["subscribed"] = json!(value);
    }
    if let Some(value) = rule {
        params["rule"] = value;
    }
    if let Some(value) = force {
        params["force"] = json!(value);
    }
    if let Some(value) = record_id {
        params["record_id"] = json!(value);
    }
    if let Some(value) = name_hints {
        params["name_hints"] = json!(value);
    }
    if let Some(value) = preferred_language {
        params["preferred_language"] = json!(value);
    }
    Ok(params)
}

fn execute_core_call(
    app: AppHandle,
    operation: Operation,
    args: CallArguments,
) -> Result<Value, String> {
    let method = operation.method();
    let params = build_params(method, args, common_params(&app)?)?;
    let id = request_id();
    let request = json!({
        "protocol_version": 2,
        "request_id": id,
        "method": method,
        "params": params,
    });
    run_one_shot(&app, request, &id)
}

fn run_one_shot(app: &AppHandle, request: Value, id: &str) -> Result<Value, String> {
    let payload = serde_json::to_vec(&request).map_err(|_| "request_encoding_failed")?;
    if payload.len().saturating_add(1) > MAX_REQUEST {
        return Err("request_too_large".into());
    }
    let mut child = Command::new(binary(app)?)
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "core_process_failed")?;
    let mut stdin = child.stdin.take().ok_or("core_process_failed")?;
    let stdout = child.stdout.take().ok_or("core_process_failed")?;
    stdin
        .write_all(&payload)
        .and_then(|_| stdin.write_all(b"\n"))
        .map_err(|_| "core_process_failed")?;
    drop(stdin);
    let bytes = read_all_bounded(stdout)?;
    if !child.wait().map_err(|_| "core_process_failed")?.success() {
        return Err("core_process_failed".into());
    }
    let response: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid_core_response")?;
    validate_response(response, id)
}

#[tauri::command]
async fn core_call(
    app: AppHandle,
    operation: Operation,
    args: CallArguments,
) -> Result<Value, String> {
    one_shot_operation(app, operation, args).await
}

async fn one_shot_operation(
    app: AppHandle,
    operation: Operation,
    args: CallArguments,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || execute_core_call(app, operation, args))
        .await
        .map_err(|_| "core_task_failed".to_string())?
}

#[tauri::command]
async fn source_capabilities(app: AppHandle) -> Result<Value, String> {
    one_shot_operation(app, Operation::GetCapabilities, CallArguments::default()).await
}

#[tauri::command]
async fn list_sources(app: AppHandle) -> Result<Value, String> {
    one_shot_operation(app, Operation::ListSources, CallArguments::default()).await
}

#[tauri::command]
async fn list_game_sources(app: AppHandle, instance_id: String) -> Result<Value, String> {
    let args = CallArguments {
        instance_id: Some(instance_id),
        ..Default::default()
    };
    one_shot_operation(app, Operation::ListGameSources, args).await
}

#[tauri::command]
async fn set_source_enabled(
    app: AppHandle,
    source_id: String,
    enabled: bool,
) -> Result<Value, String> {
    let args = CallArguments {
        source_id: Some(source_id),
        enabled: Some(enabled),
        ..Default::default()
    };
    one_shot_operation(app, Operation::SetSourceEnabled, args).await
}

#[tauri::command]
async fn set_game_source(
    app: AppHandle,
    instance_id: String,
    source_id: String,
    subscribed: bool,
) -> Result<Value, String> {
    let args = CallArguments {
        instance_id: Some(instance_id),
        source_id: Some(source_id),
        subscribed: Some(subscribed),
        ..Default::default()
    };
    one_shot_operation(app, Operation::SetGameSource, args).await
}

#[tauri::command]
async fn discover_game(
    app: AppHandle,
    instance_id: String,
    force: Option<bool>,
    name_hints: Option<Vec<String>>,
    preferred_language: Option<String>,
) -> Result<Value, String> {
    let args = CallArguments {
        instance_id: Some(instance_id),
        force: Some(force.unwrap_or(false)),
        name_hints,
        preferred_language,
        ..Default::default()
    };
    one_shot_operation(app, Operation::DiscoverGame, args).await
}

#[tauri::command]
async fn refresh_sources(app: AppHandle, force: Option<bool>) -> Result<Value, String> {
    let args = CallArguments {
        force: Some(force.unwrap_or(false)),
        ..Default::default()
    };
    one_shot_operation(app, Operation::RefreshSources, args).await
}

#[tauri::command]
async fn add_source(app: AppHandle, rule: Value) -> Result<Value, String> {
    let args = CallArguments {
        rule: Some(rule),
        ..Default::default()
    };
    one_shot_operation(app, Operation::AddSource, args).await
}

#[tauri::command]
async fn get_source_policy(app: AppHandle, source_id: String) -> Result<Value, String> {
    let args = CallArguments {
        source_id: Some(source_id),
        ..Default::default()
    };
    one_shot_operation(app, Operation::GetSourcePolicy, args).await
}

#[tauri::command]
async fn resolve_source_record(
    app: AppHandle,
    instance_id: String,
    record_id: String,
) -> Result<Value, String> {
    let args = CallArguments {
        instance_id: Some(instance_id),
        record_id: Some(record_id),
        ..Default::default()
    };
    one_shot_operation(app, Operation::ResolveSourceRecord, args).await
}

fn valid_archive(value: String) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_file()
        || path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("zip"))
    {
        return Err("invalid_archive".into());
    }
    path.canonicalize().map_err(|_| "invalid_archive".into())
}

fn cancel_job(stdin: &StdinHandle, cancelled: &AtomicBool) -> Result<(), String> {
    cancelled.store(true, Ordering::SeqCst);
    let mut stdin = stdin.lock().map_err(|_| "job_state_unavailable")?;
    stdin
        .write_all(b"\n")
        .and_then(|_| stdin.flush())
        .map_err(|_| "cancel_failed".to_string())
}

fn request_cancel(job: &ActiveJob) -> Result<(), String> {
    cancel_job(&job.stdin, &job.cancelled)
}

fn request_source_cancel(job: &SourceScanJob) -> Result<(), String> {
    cancel_job(&job.stdin, &job.cancelled)
}

/// Recognizes the streaming source events the CLI sends before the single final
/// response; anything else is treated as the terminal response.
fn source_event_name(value: &Value) -> Option<&'static str> {
    match value.get("event").and_then(Value::as_str) {
        Some("source_progress") => Some("source_progress"),
        Some("source_update") => Some("source_update"),
        _ => None,
    }
}

fn read_bounded_line(reader: &mut impl BufRead) -> std::io::Result<Option<(Vec<u8>, bool)>> {
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
        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(available.len(), |position| position + 1);
        if !oversized && bytes.len().saturating_add(take) <= MAX_RESPONSE as usize {
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

fn read_task_output(
    stdout: impl Read,
    request_id: String,
    progress: Arc<Mutex<Progress>>,
) -> Result<Value, String> {
    let mut reader = BufReader::new(stdout);
    let mut final_response = None;
    let mut stream_error = None;
    loop {
        let Some((line, oversized)) =
            read_bounded_line(&mut reader).map_err(|_| "core_process_failed")?
        else {
            break;
        };
        if oversized {
            stream_error.get_or_insert("response_too_large".to_string());
            continue;
        }
        let value: Value = match serde_json::from_slice(&line) {
            Ok(value) => value,
            Err(_) => {
                stream_error.get_or_insert("invalid_core_response".to_string());
                continue;
            }
        };
        if value["event"] == "progress" {
            match serde_json::from_value::<Progress>(value["progress"].clone()) {
                Ok(value) => match progress.lock() {
                    Ok(mut current) => *current = value,
                    Err(_) => {
                        stream_error.get_or_insert("job_state_unavailable".to_string());
                    }
                },
                Err(_) => {
                    stream_error.get_or_insert("invalid_core_response".to_string());
                }
            }
        } else if final_response.is_some() {
            stream_error.get_or_insert("invalid_core_response".to_string());
        } else {
            final_response = Some(value);
        }
    }
    if let Some(error) = stream_error {
        return Err(error);
    }
    final_response
        .ok_or_else(|| "core_process_failed".into())
        .and_then(|response| validate_response(response, &request_id))
}

/// Reads a `serve-task` `scan_sources` stream: forwards `source_progress` /
/// `source_update` to the frontend, mirrors progress for `source_scan_status`,
/// and returns the single final aggregated response.
fn read_source_task_output(
    stdout: impl Read,
    request_id: String,
    app: AppHandle,
    progress: Arc<Mutex<SourceProgress>>,
) -> Result<Value, String> {
    let mut reader = BufReader::new(stdout);
    let mut final_response = None;
    let mut stream_error = None;
    loop {
        let Some((line, oversized)) =
            read_bounded_line(&mut reader).map_err(|_| "core_process_failed")?
        else {
            break;
        };
        if oversized {
            stream_error.get_or_insert("response_too_large".to_string());
            continue;
        }
        let value: Value = match serde_json::from_slice(&line) {
            Ok(value) => value,
            Err(_) => {
                stream_error.get_or_insert("invalid_core_response".to_string());
                continue;
            }
        };
        match source_event_name(&value) {
            Some("source_progress") => {
                if let Ok(parsed) = serde_json::from_value::<SourceProgress>(value.clone()) {
                    if let Ok(mut current) = progress.lock() {
                        *current = parsed;
                    }
                }
                // A closed event pipe must not abort the task reader.
                let _ = app.emit("source_progress", value);
            }
            Some("source_update") => {
                let _ = app.emit("source_update", value);
            }
            _ => {
                if final_response.is_some() {
                    stream_error.get_or_insert("invalid_core_response".to_string());
                } else {
                    final_response = Some(value);
                }
            }
        }
    }
    if let Some(error) = stream_error {
        return Err(error);
    }
    final_response
        .ok_or_else(|| "core_process_failed".into())
        .and_then(|response| validate_response(response, &request_id))
}

#[tauri::command]
async fn start_install(
    app: AppHandle,
    jobs: State<'_, Jobs>,
    instance_id: String,
    release_id: String,
    operation: String,
    request_id: String,
    archive: Option<String>,
) -> Result<Value, String> {
    if !valid_id(&instance_id) || !valid_id(&release_id) {
        return Err("invalid_id".into());
    }
    if !matches!(operation.as_str(), "install" | "reinstall") {
        return Err("invalid_operation".into());
    }
    if !valid_request_id(&request_id) {
        return Err("invalid_request_id".into());
    }
    let archive = archive.map(valid_archive).transpose()?;

    let mut params = common_params(&app)?;
    params["instance_id"] = json!(instance_id);
    params["release_id"] = json!(release_id);
    params["operation"] = json!(operation);
    params["request_id"] = json!(request_id);
    if let Some(path) = archive {
        params["archive"] = json!(path);
    }
    let request = json!({
        "protocol_version": 2,
        "request_id": request_id,
        "method": "install_patch",
        "params": params,
    });
    let payload = serde_json::to_vec(&request).map_err(|_| "request_encoding_failed")?;
    if payload.len().saturating_add(1) > MAX_REQUEST {
        return Err("request_too_large".into());
    }

    let jobs = jobs.inner().clone();
    let closing = app.state::<Closing>().inner().clone();
    // Keep the mutex guard inside this block so no non-Send guard is held across the `.await` below.
    let (child_handle, progress, stdout) = {
        let mut slot = jobs.0.lock().map_err(|_| "job_state_unavailable")?;
        if slot.is_some() {
            return Err("install_in_progress".into());
        }
        let mut child = Command::new(binary(&app)?)
            .arg("serve-task")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "core_process_failed")?;
        let mut stdin = child.stdin.take().ok_or("core_process_failed")?;
        let stdout = child.stdout.take().ok_or("core_process_failed")?;
        stdin
            .write_all(&payload)
            .and_then(|_| stdin.write_all(b"\n"))
            .map_err(|_| "core_process_failed")?;
        stdin.flush().map_err(|_| "core_process_failed")?;

        let active = ActiveJob {
            child: Arc::new(Mutex::new(child)),
            stdin: Arc::new(Mutex::new(stdin)),
            progress: Arc::new(Mutex::new(Progress::default())),
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let child_handle = active.child.clone();
        let progress = active.progress.clone();
        *slot = Some(active);
        (child_handle, progress, stdout)
    };

    let app_for_task = app.clone();
    let response_id = request_id.clone();
    let task = tauri::async_runtime::spawn_blocking(move || {
        let reader = thread::spawn(move || read_task_output(stdout, response_id, progress));
        let status = match child_handle.lock() {
            Ok(mut child) => child.wait().map_err(|_| "core_process_failed".to_string()),
            Err(_) => Err("core_process_failed".to_string()),
        };
        let response = match reader.join() {
            Ok(response) => response,
            Err(_) => Err("core_process_failed".to_string()),
        };
        let result = match status {
            Ok(status) if status.success() => response,
            Ok(_) => Err("core_process_failed".into()),
            Err(error) => Err(error),
        };
        if let Ok(mut current) = jobs.0.lock() {
            current.take();
        }
        if closing.0.load(Ordering::SeqCst) {
            let _ = app_for_task.exit(0);
        }
        result
    });
    task.await.map_err(|_| "core_task_failed".to_string())?
}

#[tauri::command]
fn install_status(jobs: State<'_, Jobs>) -> Result<Value, String> {
    let active = jobs.0.lock().map_err(|_| "job_state_unavailable")?;
    let Some(job) = active.as_ref() else {
        return Ok(json!({"running":false,"cancelled":false,"progress":null}));
    };
    let progress = job
        .progress
        .lock()
        .map_err(|_| "job_state_unavailable")?
        .clone();
    Ok(json!({
        "running": true,
        "cancelled": job.cancelled.load(Ordering::SeqCst),
        "progress": progress,
    }))
}

#[tauri::command]
fn cancel_install(jobs: State<'_, Jobs>) -> Result<bool, String> {
    let active = jobs.0.lock().map_err(|_| "job_state_unavailable")?;
    if let Some(job) = active.as_ref() {
        request_cancel(job)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Builds the `serve-task` `scan_sources` request and enforces the request-size
/// bound. Pure so tests can assert the wire shape without spawning the core.
fn source_scan_request(id: &str, common: Value, force: bool) -> Result<Value, String> {
    let mut params = common;
    params["force"] = json!(force);
    let request = json!({
        "protocol_version": 2,
        "request_id": id,
        "method": "scan_sources",
        "params": params,
    });
    let payload = serde_json::to_vec(&request).map_err(|_| "request_encoding_failed")?;
    if payload.len().saturating_add(1) > MAX_REQUEST {
        return Err("request_too_large".into());
    }
    Ok(request)
}

#[tauri::command]
async fn start_source_scan(
    app: AppHandle,
    scans: State<'_, SourceScans>,
    force: Option<bool>,
) -> Result<Value, String> {
    let id = request_id();
    let request = source_scan_request(&id, common_params(&app)?, force.unwrap_or(false))?;
    let payload = serde_json::to_vec(&request).map_err(|_| "request_encoding_failed")?;

    let scans = scans.inner().clone();
    let closing = app.state::<Closing>().inner().clone();
    let jobs = app.state::<Jobs>().inner().clone();
    // Keep the mutex guard inside this block so no non-Send guard is held across the `.await` below.
    let (child_handle, progress, stdout) = {
        let mut slot = scans.0.lock().map_err(|_| "job_state_unavailable")?;
        if slot.active.is_some() {
            return Err("source_scan_in_progress".into());
        }
        slot.results = None;
        let mut child = Command::new(binary(&app)?)
            .arg("serve-task")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "core_process_failed")?;
        let mut stdin = child.stdin.take().ok_or("core_process_failed")?;
        let stdout = child.stdout.take().ok_or("core_process_failed")?;
        stdin
            .write_all(&payload)
            .and_then(|_| stdin.write_all(b"\n"))
            .map_err(|_| "core_process_failed")?;
        stdin.flush().map_err(|_| "core_process_failed")?;

        let active = SourceScanJob {
            child: Arc::new(Mutex::new(child)),
            stdin: Arc::new(Mutex::new(stdin)),
            progress: Arc::new(Mutex::new(SourceProgress::default())),
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let child_handle = active.child.clone();
        let progress = active.progress.clone();
        slot.active = Some(active);
        (child_handle, progress, stdout)
    };

    let app_for_task = app.clone();
    let app_for_reader = app.clone();
    let scans_for_task = scans.clone();
    let response_id = id.clone();
    let task = tauri::async_runtime::spawn_blocking(move || {
        let reader = thread::spawn(move || {
            read_source_task_output(stdout, response_id, app_for_reader, progress)
        });
        let status = match child_handle.lock() {
            Ok(mut child) => child.wait().map_err(|_| "core_process_failed".to_string()),
            Err(_) => Err("core_process_failed".to_string()),
        };
        let response = match reader.join() {
            Ok(response) => response,
            Err(_) => Err("core_process_failed".to_string()),
        };
        let result = match status {
            Ok(status) if status.success() => response,
            Ok(_) => Err("core_process_failed".into()),
            Err(error) => Err(error),
        };
        if let Ok(mut slot) = scans_for_task.0.lock() {
            slot.active = None;
            if let Ok(value) = &result {
                slot.results = Some(value.clone());
            }
        }
        if closing.0.load(Ordering::SeqCst) {
            // Do not exit out from under a committed install rollback path.
            let install_active = jobs.0.lock().map(|slot| slot.is_some()).unwrap_or(true);
            if !install_active {
                let _ = app_for_task.exit(0);
            }
        }
        result
    });
    task.await.map_err(|_| "core_task_failed".to_string())?
}

#[tauri::command]
fn source_scan_status(scans: State<'_, SourceScans>) -> Result<Value, String> {
    let slot = scans.0.lock().map_err(|_| "job_state_unavailable")?;
    let results = slot.results.clone();
    if let Some(job) = slot.active.as_ref() {
        let progress = job
            .progress
            .lock()
            .map_err(|_| "job_state_unavailable")?
            .clone();
        Ok(json!({
            "running": true,
            "cancelled": job.cancelled.load(Ordering::SeqCst),
            "progress": progress,
            "results": results,
        }))
    } else {
        Ok(json!({
            "running": false,
            "cancelled": false,
            "progress": null,
            "results": results,
        }))
    }
}

#[tauri::command]
fn cancel_source_scan(scans: State<'_, SourceScans>) -> Result<bool, String> {
    let slot = scans.0.lock().map_err(|_| "job_state_unavailable")?;
    if let Some(job) = slot.active.as_ref() {
        request_source_cancel(job)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
fn choose_archive(app: AppHandle) -> Result<Option<String>, String> {
    let file = app
        .dialog()
        .file()
        .add_filter("ZIP 补丁包", &["zip"])
        .blocking_pick_file()
        .and_then(|file| file.into_path().ok());
    file.map(|path| {
        if !path.is_file()
            || path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_none_or(|extension| !extension.eq_ignore_ascii_case("zip"))
        {
            return Err("invalid_archive".into());
        }
        path.canonicalize()
            .map(|path| path.display().to_string())
            .map_err(|_| "invalid_archive".into())
    })
    .transpose()
}

#[tauri::command]
fn open_source(app: AppHandle, url: String) -> Result<(), String> {
    if !url.starts_with("https://") || url.chars().any(char::is_control) {
        return Err("invalid_source_url".into());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|_| "browser_open_failed".into())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Jobs::default())
        .manage(SourceScans::default())
        .manage(Closing::default())
        .setup(|app| {
            // Revoke legacy planned authorizations before the UI can start an install.
            let _ = execute_core_call(
                app.handle().clone(),
                Operation::MigrateState,
                CallArguments::default(),
            );

            let jobs = app.state::<Jobs>().inner().clone();
            let scans = app.state::<SourceScans>().inner().clone();
            let closing = app.state::<Closing>().inner().clone();
            if let Some(window) = app.get_webview_window("main") {
                window.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        let active = jobs.0.lock().ok().and_then(|slot| slot.clone());
                        let scan = scans.0.lock().ok().and_then(|slot| slot.active.clone());
                        if active.is_some() || scan.is_some() {
                            api.prevent_close();
                            closing.0.store(true, Ordering::SeqCst);
                            if let Some(job) = active {
                                let _ = request_cancel(&job);
                            }
                            if let Some(job) = scan {
                                let _ = request_source_cancel(&job);
                            }
                        }
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core_call,
            start_install,
            install_status,
            cancel_install,
            choose_archive,
            open_source,
            source_capabilities,
            list_sources,
            list_game_sources,
            set_source_enabled,
            set_game_source,
            discover_game,
            refresh_sources,
            add_source,
            start_source_scan,
            source_scan_status,
            cancel_source_scan,
            get_source_policy,
            resolve_source_record
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Steam Patch Manager");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn common() -> Value {
        json!({"platform": "windows", "state_dir": "C:/state"})
    }

    #[test]
    fn source_operation_method_names_are_snake_case() {
        assert_eq!(Operation::GetCapabilities.method(), "get_capabilities");
        assert_eq!(Operation::ListSources.method(), "list_sources");
        assert_eq!(Operation::ListGameSources.method(), "list_game_sources");
        assert_eq!(Operation::SetSourceEnabled.method(), "set_source_enabled");
        assert_eq!(Operation::SetGameSource.method(), "set_game_source");
        assert_eq!(Operation::AddSource.method(), "add_source");
        assert_eq!(Operation::RefreshSources.method(), "refresh_sources");
        assert_eq!(Operation::DiscoverGame.method(), "discover_game");
        assert_eq!(Operation::GetSourcePolicy.method(), "get_source_policy");
        assert_eq!(
            Operation::ResolveSourceRecord.method(),
            "resolve_source_record"
        );
    }

    #[test]
    fn set_game_source_merges_common_and_arguments() {
        let params = build_params(
            "set_game_source",
            CallArguments {
                instance_id: Some("inst-1".into()),
                source_id: Some("kagura-cn".into()),
                subscribed: Some(true),
                ..Default::default()
            },
            common(),
        )
        .unwrap();
        assert_eq!(params["state_dir"], "C:/state");
        assert_eq!(params["platform"], "windows");
        assert_eq!(params["instance_id"], "inst-1");
        assert_eq!(params["source_id"], "kagura-cn");
        assert_eq!(params["subscribed"], true);
        // Unset optional arguments must not leak into the strict core params.
        assert!(params.get("enabled").is_none());
        assert!(params.get("rule").is_none());
        assert!(params.get("force").is_none());
        assert!(params.get("record_id").is_none());
    }

    #[test]
    fn add_source_passes_rule_value_through() {
        let rule = json!({"id": "custom", "adapter_id": "table"});
        let params = build_params(
            "add_source",
            CallArguments {
                rule: Some(rule.clone()),
                ..Default::default()
            },
            common(),
        )
        .unwrap();
        assert_eq!(params["rule"], rule);
    }

    #[test]
    fn discover_game_passes_hints_language_and_force() {
        let params = build_params(
            "discover_game",
            CallArguments {
                instance_id: Some("inst-1".into()),
                force: Some(false),
                name_hints: Some(vec!["别名".into()]),
                preferred_language: Some("zh-CN".into()),
                ..Default::default()
            },
            common(),
        )
        .unwrap();
        assert_eq!(params["instance_id"], "inst-1");
        assert_eq!(params["force"], false);
        assert_eq!(params["name_hints"][0], "别名");
        assert_eq!(params["preferred_language"], "zh-CN");
    }

    #[test]
    fn resolve_source_record_accepts_record_or_release() {
        let by_record = build_params(
            "resolve_source_record",
            CallArguments {
                instance_id: Some("inst-1".into()),
                record_id: Some("candidate-1".into()),
                ..Default::default()
            },
            common(),
        )
        .unwrap();
        assert_eq!(by_record["record_id"], "candidate-1");
        let by_release = build_params(
            "resolve_source_record",
            CallArguments {
                instance_id: Some("inst-1".into()),
                release_id: Some("release-1".into()),
                ..Default::default()
            },
            common(),
        )
        .unwrap();
        assert_eq!(by_release["release_id"], "release-1");
        assert!(by_release.get("record_id").is_none());
    }

    #[test]
    fn required_source_arguments_are_rejected() {
        assert_eq!(
            build_params("list_game_sources", CallArguments::default(), common()).unwrap_err(),
            "instance_required"
        );
        assert_eq!(
            build_params(
                "set_source_enabled",
                CallArguments {
                    source_id: Some("kagura-cn".into()),
                    ..Default::default()
                },
                common(),
            )
            .unwrap_err(),
            "enabled_required"
        );
        assert_eq!(
            build_params(
                "set_game_source",
                CallArguments {
                    instance_id: Some("inst-1".into()),
                    source_id: Some("kagura-cn".into()),
                    ..Default::default()
                },
                common(),
            )
            .unwrap_err(),
            "subscribed_required"
        );
        assert_eq!(
            build_params("add_source", CallArguments::default(), common()).unwrap_err(),
            "rule_required"
        );
        assert_eq!(
            build_params(
                "resolve_source_record",
                CallArguments {
                    instance_id: Some("inst-1".into()),
                    ..Default::default()
                },
                common(),
            )
            .unwrap_err(),
            "record_required"
        );
        assert_eq!(
            build_params(
                "get_source_policy",
                CallArguments::default(),
                common(),
            )
            .unwrap_err(),
            "source_id_required"
        );
    }

    #[test]
    fn call_arguments_are_strict_and_ids_are_validated() {
        let unknown = serde_json::from_str::<CallArguments>(r#"{"instanceId":"i","shell":"evil"}"#);
        assert!(unknown.is_err());

        let camel = serde_json::from_str::<CallArguments>(
            r#"{"instanceId":"i","sourceId":"s","preferredLanguage":"zh-CN"}"#,
        )
        .unwrap();
        assert_eq!(camel.instance_id.as_deref(), Some("i"));
        assert_eq!(camel.source_id.as_deref(), Some("s"));
        assert_eq!(camel.preferred_language.as_deref(), Some("zh-CN"));

        let control = CallArguments {
            instance_id: Some("bad\u{1}".into()),
            ..Default::default()
        };
        assert_eq!(
            build_params("list_game_sources", control, common()).unwrap_err(),
            "invalid_id"
        );
    }

    #[test]
    fn source_scan_request_shape_and_force() {
        let request = source_scan_request("host-1-2", common(), false).unwrap();
        assert_eq!(request["protocol_version"], 2);
        assert_eq!(request["request_id"], "host-1-2");
        assert_eq!(request["method"], "scan_sources");
        assert_eq!(request["params"]["state_dir"], "C:/state");
        assert_eq!(request["params"]["force"], false);
    }

    #[test]
    fn source_stream_separates_events_from_final_response() {
        let progress = json!({
            "protocol_version": 2,
            "request_id": "r",
            "sequence": 0,
            "event": "source_progress",
            "completed": 1,
            "total": 2,
            "running": true,
            "settings_revision": 5,
        });
        let update = json!({
            "protocol_version": 2,
            "request_id": "r",
            "sequence": 1,
            "event": "source_update",
            "appid": 42,
            "index": 0,
            "settings_revision": 5,
            "discovery": {},
        });
        let terminal = json!({
            "protocol_version": 2,
            "request_id": "r",
            "ok": true,
            "result": {"settings_revision": 5, "results": []},
        });
        assert_eq!(source_event_name(&progress), Some("source_progress"));
        assert_eq!(source_event_name(&update), Some("source_update"));
        assert_eq!(source_event_name(&terminal), None);

        let parsed: SourceProgress = serde_json::from_value(progress).unwrap();
        assert_eq!(parsed.completed, 1);
        assert_eq!(parsed.total, 2);
        assert!(parsed.running);
        assert_eq!(parsed.settings_revision, 5);
    }
}
