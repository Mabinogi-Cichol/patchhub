#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

const MAX_RESPONSE: u64 = 8 * 1024 * 1024;

#[derive(Default)]
struct Selection(Mutex<Option<PathBuf>>);

type ChildHandle = Arc<Mutex<Child>>;
#[derive(Clone, Default)]
struct Downloads(Arc<Mutex<Option<ChildHandle>>>);

struct ProcessGuard {
    child: ChildHandle,
    downloads: Option<Downloads>,
}
impl Drop for ProcessGuard {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        if let Some(downloads) = &self.downloads {
            if let Ok(mut active) = downloads.0.lock() {
                if active
                    .as_ref()
                    .is_some_and(|child| Arc::ptr_eq(child, &self.child))
                {
                    *active = None;
                }
            }
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Scan,
    Preview,
    PlanInstall,
    FetchArtifact,
    StartInstall,
    ListTransactions,
    Uninstall,
    Recover,
}

impl Operation {
    fn method(&self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Preview => "preview",
            Self::PlanInstall => "plan_install",
            Self::FetchArtifact => "fetch_artifact",
            Self::StartInstall => "start_install",
            Self::ListTransactions => "list_transactions",
            Self::Uninstall => "uninstall",
            Self::Recover => "recover",
        }
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
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
    plan_id: Option<String>,
    transaction_id: Option<String>,
    #[serde(default)]
    confirmed_game_closed: bool,
}

fn execute(
    app: AppHandle,
    operation: Operation,
    args: CallArguments,
    archive: Option<PathBuf>,
    downloads: Downloads,
) -> Result<Value, String> {
    let CallArguments {
        instance_id,
        release_id,
        plan_id,
        transaction_id,
        confirmed_game_closed,
    } = args;
    let method = operation.method();
    for id in [&instance_id, &release_id, &plan_id, &transaction_id]
        .into_iter()
        .flatten()
    {
        if !valid_id(id) {
            return Err("invalid_id".into());
        }
    }
    if matches!(method, "preview" | "plan_install" | "fetch_artifact")
        && (instance_id.is_none() || release_id.is_none())
    {
        return Err("instance_and_release_required".into());
    }
    if method == "start_install" && plan_id.is_none() {
        return Err("plan_required".into());
    }
    if method == "uninstall" && transaction_id.is_none() {
        return Err("transaction_required".into());
    }

    let state_dir = app
        .path()
        .app_data_dir()
        .map_err(|_| "state_path_unavailable")?;
    std::fs::create_dir_all(&state_dir).map_err(|_| "state_path_unavailable")?;
    let mut params = json!({"platform":"windows", "state_dir":state_dir, "confirmed_game_closed":confirmed_game_closed});
    if let Some(path) = config_file(&app, "catalog.json") {
        params["catalog"] = json!(path);
    }
    if let Some(path) = config_file(&app, "signed-catalog.json") {
        params["signed_catalog"] = json!(path);
    }
    if let Some(path) = config_file(&app, "trust-keys.json") {
        params["trust_keys"] = json!(path);
    }
    if let Some(value) = instance_id {
        params["instance_id"] = json!(value);
    }
    if let Some(value) = release_id {
        params["release_id"] = json!(value);
    }
    if let Some(value) = plan_id {
        params["plan_id"] = json!(value);
    }
    if let Some(value) = transaction_id {
        params["transaction_id"] = json!(value);
    }
    if let Some(path) = archive {
        params["archive"] = json!(path);
    }
    let request_id = format!("host-{}", std::process::id());
    let request =
        json!({"protocol_version":1,"request_id":request_id,"method":method,"params":params});
    let payload = serde_json::to_vec(&request).map_err(|_| "request_encoding_failed")?;
    if payload.len() >= 64 * 1024 {
        return Err("request_too_large".into());
    }
    let mut child = Command::new(binary(&app)?)
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "core_process_failed")?;
    let stdin = child.stdin.take().ok_or("core_process_failed")?;
    let stdout = child.stdout.take().ok_or("core_process_failed")?;
    let child = Arc::new(Mutex::new(child));
    let guard = ProcessGuard {
        child: child.clone(),
        downloads: if method == "fetch_artifact" {
            Some(downloads.clone())
        } else {
            None
        },
    };
    if method == "fetch_artifact" {
        let mut active = downloads
            .0
            .lock()
            .map_err(|_| "download_state_unavailable")?;
        if active.is_some() {
            return Err("download_in_progress".into());
        }
        *active = Some(child.clone());
    }
    let mut stdin = stdin;
    stdin
        .write_all(&payload)
        .and_then(|_| stdin.write_all(b"\n"))
        .map_err(|_| "core_process_failed")?;
    drop(stdin);
    let mut bytes = Vec::new();
    stdout
        .take(MAX_RESPONSE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "core_process_failed")?;
    if bytes.len() as u64 > MAX_RESPONSE {
        return Err("response_too_large".into());
    }
    if !child
        .lock()
        .map_err(|_| "core_process_failed")?
        .wait()
        .map_err(|_| "core_process_failed")?
        .success()
    {
        return Err(if method == "fetch_artifact" {
            "download_canceled_or_failed"
        } else {
            "core_process_failed"
        }
        .into());
    }
    drop(guard);
    let response: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid_core_response")?;
    if response["protocol_version"] != 1 || response["request_id"] != request_id {
        return Err("invalid_core_response".into());
    }
    if response["ok"] == true {
        Ok(response["result"].clone())
    } else {
        Err(response["error"]["code"]
            .as_str()
            .unwrap_or("core_error")
            .to_string())
    }
}

#[tauri::command]
async fn core_call(
    app: AppHandle,
    selection: State<'_, Selection>,
    downloads: State<'_, Downloads>,
    operation: Operation,
    args: CallArguments,
) -> Result<Value, String> {
    let archive = if matches!(operation, Operation::PlanInstall) {
        selection
            .0
            .lock()
            .map_err(|_| "selection_unavailable")?
            .clone()
    } else {
        None
    };
    let downloads = downloads.inner().clone();
    let fetching = matches!(operation, Operation::FetchArtifact);
    let result = tauri::async_runtime::spawn_blocking(move || {
        execute(app, operation, args, archive, downloads)
    })
    .await
    .map_err(|_| "core_task_failed".to_string())??;
    if fetching {
        if let Some(path) = result["archive"].as_str() {
            *selection.0.lock().map_err(|_| "selection_unavailable")? = Some(PathBuf::from(path));
        }
    }
    Ok(result)
}

#[tauri::command]
fn cancel_download(downloads: State<'_, Downloads>) -> Result<bool, String> {
    let active = downloads
        .0
        .lock()
        .map_err(|_| "download_state_unavailable")?
        .clone();
    if let Some(child) = active {
        let mut child = child.lock().map_err(|_| "download_state_unavailable")?;
        if child
            .try_wait()
            .map_err(|_| "core_process_failed")?
            .is_none()
        {
            child.kill().map_err(|_| "download_cancel_failed")?;
            return Ok(true);
        }
    }
    Ok(false)
}

#[tauri::command]
async fn choose_archive(
    app: AppHandle,
    selection: State<'_, Selection>,
) -> Result<Option<String>, String> {
    let file = app
        .dialog()
        .file()
        .add_filter("ZIP 补丁包", &["zip"])
        .blocking_pick_file()
        .and_then(|f| f.into_path().ok());
    let checked = match file {
        Some(path) => {
            if !path.is_file()
                || path
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_none_or(|s| !s.eq_ignore_ascii_case("zip"))
            {
                return Err("invalid_archive".into());
            }
            Some(path.canonicalize().map_err(|_| "invalid_archive")?)
        }
        None => None,
    };
    *selection.0.lock().map_err(|_| "selection_unavailable")? = checked.clone();
    Ok(checked.map(|p| p.display().to_string()))
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
        .manage(Selection::default())
        .manage(Downloads::default())
        .invoke_handler(tauri::generate_handler![
            core_call,
            cancel_download,
            choose_archive,
            open_source
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Steam Patch Manager");
}
