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
use tauri::{AppHandle, Manager, State, WindowEvent};
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
}

impl Operation {
    fn method(&self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Preview => "preview",
            Self::ListTransactions => "list_transactions",
            Self::Recover => "recover",
            Self::MigrateState => "migrate_state",
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

fn execute_core_call(
    app: AppHandle,
    operation: Operation,
    args: CallArguments,
) -> Result<Value, String> {
    let CallArguments {
        instance_id,
        release_id,
    } = args;
    for id in [&instance_id, &release_id].into_iter().flatten() {
        if !valid_id(id) {
            return Err("invalid_id".into());
        }
    }
    let method = operation.method();
    if method == "preview" && (instance_id.is_none() || release_id.is_none()) {
        return Err("instance_and_release_required".into());
    }

    let mut params = common_params(&app)?;
    if let Some(value) = instance_id {
        params["instance_id"] = json!(value);
    }
    if let Some(value) = release_id {
        params["release_id"] = json!(value);
    }
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
    tauri::async_runtime::spawn_blocking(move || execute_core_call(app, operation, args))
        .await
        .map_err(|_| "core_task_failed".to_string())?
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

fn request_cancel(job: &ActiveJob) -> Result<(), String> {
    job.cancelled.store(true, Ordering::SeqCst);
    let mut stdin = job.stdin.lock().map_err(|_| "job_state_unavailable")?;
    stdin
        .write_all(b"\n")
        .and_then(|_| stdin.flush())
        .map_err(|_| "cancel_failed".to_string())
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
        .manage(Closing::default())
        .setup(|app| {
            // Revoke legacy planned authorizations before the UI can start an install.
            let _ = execute_core_call(
                app.handle().clone(),
                Operation::MigrateState,
                CallArguments::default(),
            );

            let jobs = app.state::<Jobs>().inner().clone();
            let closing = app.state::<Closing>().inner().clone();
            if let Some(window) = app.get_webview_window("main") {
                window.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        let active = jobs.0.lock().ok().and_then(|slot| slot.clone());
                        if let Some(job) = active {
                            api.prevent_close();
                            closing.0.store(true, Ordering::SeqCst);
                            let _ = request_cancel(&job);
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
            open_source
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Steam Patch Manager");
}
