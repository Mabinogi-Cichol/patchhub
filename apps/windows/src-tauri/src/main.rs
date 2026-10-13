#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use steam_patch_protocol::{self as protocol, EventSequence, Method, PROTOCOL_VERSION};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

const MAX_RESPONSE: u64 = protocol::MAX_RESPONSE_BYTES as u64;
const MAX_REQUEST: usize = protocol::MAX_REQUEST_BYTES;

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Scan,
    Preview,
    ListTransactions,
    Recover,
    GetCapabilities,
    ListSources,
    ListGameSources,
    SetSourceEnabled,
    SetGameSource,
    AddSource,
    RefreshSources,
    DiscoverGame,
}

impl Operation {
    fn method(&self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Preview => "preview",
            Self::ListTransactions => "list_transactions",
            Self::Recover => "recover",
            Self::GetCapabilities => "get_capabilities",
            Self::ListSources => "list_sources",
            Self::ListGameSources => "list_game_sources",
            Self::SetSourceEnabled => "set_source_enabled",
            Self::SetGameSource => "set_game_source",
            Self::AddSource => "add_source",
            Self::RefreshSources => "refresh_sources",
            Self::DiscoverGame => "discover_game",
        }
    }
}

fn valid_id(value: &str) -> bool {
    protocol::validate_id(value, protocol::MAX_ID_BYTES)
}

fn valid_request_id(value: &str) -> bool {
    protocol::validate_id(value, protocol::MAX_REQUEST_ID_BYTES)
}

fn request_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("host-{}-{}", std::process::id(), nanos)
}

fn binary(app: &AppHandle) -> Result<PathBuf, HostError> {
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

/// Builds a `Command` for the core CLI with its console window suppressed.
///
/// The host runs under the GUI subsystem; spawning the console-subsystem
/// `steam-patch.exe` without `CREATE_NO_WINDOW` makes Windows allocate a
/// console window that flashes on every call.
fn core_command(path: &Path) -> Command {
    let mut command = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
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
    selection_id: Option<String>,
    source_id: Option<String>,
    enabled: Option<bool>,
    subscribed: Option<bool>,
    rule: Option<Value>,
    force: Option<bool>,
    name_hints: Option<Vec<String>>,
    preferred_language: Option<String>,
}

type Progress = Option<protocol::InstallProgressEvent>;

type ChildHandle = Arc<Mutex<Child>>;
struct ClosableWriter<W>(Option<W>);
impl<W: Write> Write for ClosableWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .as_mut()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "control_closed"))?
            .write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0
            .as_mut()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "control_closed"))?
            .flush()
    }
}
type StdinHandle = Arc<Mutex<ClosableWriter<ChildStdin>>>;

#[derive(Clone)]
struct ActiveJob {
    child: ChildHandle,
    stdin: StdinHandle,
    progress: Arc<Mutex<Progress>>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Clone, Default)]
struct Jobs(Arc<Mutex<Option<ActiveJob>>>);

/// Latest validated event, retained for host progress snapshots.
type SourceProgress = Option<protocol::SourceProgressEvent>;

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

fn common_params(app: &AppHandle) -> Result<Value, HostError> {
    let state_dir = app
        .path()
        .app_data_dir()
        .map_err(|_| "state_path_unavailable")?;
    Ok(json!({"host": {
        "roots": [], "platform":"windows", "state_dir":state_dir,
        "catalog": null,
        "signed_catalog": config_file(app, "signed-catalog.json"),
        "trust_keys": config_file(app, "trust-keys.json"),
    }}))
}

/// Maximum length of a forwarded `error.detail` before it is truncated
/// (p1-host-contract §3.3: optional, ≤2 KiB).
#[cfg(test)]
const MAX_DETAIL: usize = protocol::MAX_DETAIL_BYTES;

fn truncate_detail(detail: &str) -> String {
    protocol::truncate_detail(detail)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(transparent)]
struct HostError(protocol::DomainError);
impl std::ops::Deref for HostError {
    type Target = protocol::DomainError;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl From<&str> for HostError {
    fn from(code: &str) -> Self {
        Self(protocol::DomainError::code(code))
    }
}
impl From<String> for HostError {
    fn from(code: String) -> Self {
        Self(protocol::DomainError::code(code))
    }
}
#[cfg(test)]
impl PartialEq<&str> for HostError {
    fn eq(&self, value: &&str) -> bool {
        &self.code == value
    }
}
fn response_error(response: &Value) -> HostError {
    HostError(
        serde_json::from_value(response["error"].clone())
            .unwrap_or_else(|_| protocol::DomainError::code("invalid_core_response")),
    )
}

fn validate_response(response: Value, id: &str) -> Result<Value, HostError> {
    protocol::validate_response(&response, id, None).map_err(protocol_error)?;
    if response["ok"] == true {
        Ok(response["result"].clone())
    } else {
        Err(response_error(&response))
    }
}

fn protocol_error(error: protocol::ProtocolError) -> HostError {
    HostError(protocol::DomainError {
        code: error.code,
        detail: error
            .detail
            .filter(|detail| !detail.is_empty())
            .map(|detail| truncate_detail(&detail)),
    })
}

/// Probe the fixed binary for every newly spawned core lifecycle. No write
/// request, lease renewal, or state-directory creation precedes this gate.
fn require_capabilities(app: &AppHandle) -> Result<(), HostError> {
    let id = request_id();
    let request = json!({"protocol_version":PROTOCOL_VERSION,"request_id":id,
        "method":"get_capabilities","params":common_params(app)?});
    let capabilities = run_one_shot(app, request, &id)?;
    protocol::validate_capabilities(&capabilities).map_err(protocol_error)?;
    Ok(())
}

fn read_all_bounded(mut reader: impl Read) -> Result<Vec<u8>, HostError> {
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
fn build_params(method: &str, args: CallArguments, common: Value) -> Result<Value, HostError> {
    let CallArguments {
        instance_id,
        release_id,
        selection_id,
        source_id,
        enabled,
        subscribed,
        rule,
        force,
        name_hints,
        preferred_language,
    } = args;
    if instance_id
        .as_ref()
        .is_some_and(|id| !protocol::validate_id(id, protocol::MAX_INSTANCE_ID_BYTES))
    {
        return Err("invalid_id".into());
    }
    for id in [&release_id, &selection_id, &source_id]
        .into_iter()
        .flatten()
    {
        if !valid_id(id) {
            return Err("invalid_id".into());
        }
    }
    if method == "preview" {
        if instance_id.is_none() {
            return Err("instance_required".into());
        }
        // `selection_id` and `release_id` are mutually exclusive; exactly one is
        // required (p1-host-contract §1.1 A / §1.4).
        match (&selection_id, &release_id) {
            (Some(_), Some(_)) => return Err("invalid_request".into()),
            (None, None) => return Err("selection_or_release_required".into()),
            _ => {}
        }
    }
    if ["list_game_sources", "set_game_source", "discover_game"].contains(&method)
        && instance_id.is_none()
    {
        return Err("instance_required".into());
    }
    if ["set_source_enabled", "set_game_source"].contains(&method) && source_id.is_none() {
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

    let mut params = common;
    if let Some(value) = instance_id {
        params["instance_id"] = json!(value);
    }
    if let Some(value) = release_id {
        params["release_id"] = json!(value);
    }
    if let Some(value) = selection_id {
        params["selection_id"] = json!(value);
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
    if let Some(value) = name_hints {
        params["name_hints"] = json!(value);
    }
    if let Some(value) = preferred_language {
        params["preferred_language"] = json!(value);
    }
    let probe = json!({"protocol_version":PROTOCOL_VERSION,"request_id":"host-validation","method":method,"params":params});
    let mut bytes = serde_json::to_vec(&probe).map_err(|_| "request_encoding_failed")?;
    bytes.push(b'\n');
    protocol::decode_request(&bytes, false).map_err(protocol_error)?;
    Ok(params)
}

fn execute_core_call(
    app: AppHandle,
    operation: Operation,
    args: CallArguments,
) -> Result<Value, HostError> {
    let method = operation.method();
    // A timed-out writer retains its slot until actual exit. Recovery also
    // mutates game files and must not bypass that pending shutdown guard.
    if method == "recover"
        && app
            .state::<Jobs>()
            .0
            .lock()
            .map_err(|_| "job_state_unavailable")?
            .is_some()
    {
        return Err("install_in_progress".into());
    }
    if method != "get_capabilities" {
        require_capabilities(&app)?;
    }
    let params = build_params(method, args, common_params(&app)?)?;
    let id = request_id();
    let request = json!({
        "protocol_version": PROTOCOL_VERSION,
        "request_id": id,
        "method": method,
        "params": params,
    });
    run_one_shot(&app, request, &id)
}

fn run_one_shot(app: &AppHandle, request: Value, id: &str) -> Result<Value, HostError> {
    let payload = serde_json::to_vec(&request).map_err(|_| "request_encoding_failed")?;
    let mut request_line = payload.clone();
    request_line.push(b'\n');
    protocol::decode_request(
        &request_line,
        request["method"]
            .as_str()
            .is_some_and(|m| matches!(m, "install_patch" | "scan_sources" | "refresh_sources")),
    )
    .map_err(protocol_error)?;
    if payload.len().saturating_add(1) > MAX_REQUEST {
        return Err("request_too_large".into());
    }
    let mut child = core_command(&binary(app)?)
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
    if bytes.last() != Some(&b'\n') || bytes[..bytes.len() - 1].contains(&b'\n') {
        return Err("invalid_core_response".into());
    }
    let response: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid_core_response")?;
    let method: Method =
        serde_json::from_value(request["method"].clone()).map_err(|_| "invalid_request")?;
    protocol::validate_response(&response, id, Some(method)).map_err(protocol_error)?;
    let result = validate_response(response, id)?;
    if method.as_str() == "get_capabilities" {
        protocol::validate_capabilities(&result).map_err(protocol_error)?;
    }
    Ok(result)
}

#[tauri::command]
async fn core_call(
    app: AppHandle,
    operation: Operation,
    args: CallArguments,
) -> Result<Value, HostError> {
    one_shot_operation(app, operation, args).await
}

async fn one_shot_operation(
    app: AppHandle,
    operation: Operation,
    args: CallArguments,
) -> Result<Value, HostError> {
    tauri::async_runtime::spawn_blocking(move || execute_core_call(app, operation, args))
        .await
        .map_err(|_| HostError::from("core_task_failed"))?
}

#[tauri::command]
async fn get_capabilities(app: AppHandle) -> Result<Value, HostError> {
    one_shot_operation(app, Operation::GetCapabilities, CallArguments::default()).await
}

#[tauri::command]
async fn list_sources(app: AppHandle) -> Result<Value, HostError> {
    one_shot_operation(app, Operation::ListSources, CallArguments::default()).await
}

#[tauri::command]
async fn list_game_sources(app: AppHandle, instance_id: String) -> Result<Value, HostError> {
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
) -> Result<Value, HostError> {
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
) -> Result<Value, HostError> {
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
) -> Result<Value, HostError> {
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
async fn refresh_sources(
    app: AppHandle,
    scans: State<'_, SourceScans>,
    force: Option<bool>,
) -> Result<Value, HostError> {
    // `refresh_sources` is a task-channel method (p1-host-contract §1.1 B): it
    // must run on the `serve-task` control channel (renew/cancel/EOF) exactly
    // like `scan_sources`. The one-shot `serve` face is rejected by the core
    // with `task_required`, which is the failure this command previously hit.
    require_capabilities(&app)?;
    let id = request_id();
    let request = source_refresh_request(&id, common_params(&app)?, force.unwrap_or(false))?;
    let payload = serde_json::to_vec(&request).map_err(|_| "request_encoding_failed")?;
    let mut request_line = payload.clone();
    request_line.push(b'\n');
    protocol::decode_request(
        &request_line,
        request["method"]
            .as_str()
            .is_some_and(|m| matches!(m, "install_patch" | "scan_sources" | "refresh_sources")),
    )
    .map_err(protocol_error)?;

    let scans = scans.inner().clone();
    let closing = app.state::<Closing>().inner().clone();
    let jobs = app.state::<Jobs>().inner().clone();
    // Keep the mutex guard inside this block so no non-Send guard is held across the `.await` below.
    let (child_handle, progress, stdout, renew_handle, renew_stop) = {
        let mut slot = scans.0.lock().map_err(|_| "job_state_unavailable")?;
        // A refresh shares the single source-task slot with `scan_sources`: the
        // core permits one task per `serve-task` child and both methods stream
        // over the same channel, so serializing them prevents two concurrent
        // source tasks mutating the same state directory.
        if slot.active.is_some() {
            return Err("source_scan_in_progress".into());
        }
        let mut child = core_command(&binary(&app)?)
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

        let stdin_handle = Arc::new(Mutex::new(ClosableWriter(Some(stdin))));
        let cancelled = Arc::new(AtomicBool::new(false));
        let renew_stop = Arc::new(AtomicBool::new(false));
        // Refreshes hold the same 15s foreground lease as installs/scans, so the
        // host must forward `renew` here too (p1-host-contract §5.2).
        let renew_handle =
            spawn_renew_forwarder(stdin_handle.clone(), cancelled.clone(), renew_stop.clone());
        let active = SourceScanJob {
            child: Arc::new(Mutex::new(child)),
            stdin: stdin_handle,
            progress: Arc::new(Mutex::new(SourceProgress::default())),
            cancelled,
        };
        let child_handle = active.child.clone();
        let progress = active.progress.clone();
        slot.active = Some(active);
        (child_handle, progress, stdout, renew_handle, renew_stop)
    };

    let app_for_task = app.clone();
    let app_for_reader = app.clone();
    let scans_for_task = scans.clone();
    let response_id = id.clone();
    let task = tauri::async_runtime::spawn_blocking(move || {
        let rejection_child = child_handle.clone();
        let rejection_stop = renew_stop.clone();
        let reader = thread::spawn(move || {
            let result = read_refresh_task_output(stdout, response_id, app_for_reader, progress);
            if result.is_err() {
                rejection_stop.store(true, Ordering::SeqCst);
                // This source lookup child owns no installer game writes or
                // rollback. Installer rejection uses cancel/EOF instead.
                if let Ok(mut child) = rejection_child.lock() {
                    let _ = child.kill();
                }
            }
            result
        });
        let status = wait_core_child(&child_handle);
        let response = match reader.join() {
            Ok(response) => response,
            Err(_) => Err(HostError::from("core_process_failed")),
        };
        // A validated terminal is authoritative even if the process fails
        // during teardown. Incomplete or corrupt streams already return Err.
        let result = task_terminal_result(response, status);
        // Stop lease forwarding, then drop the job so its stdin is closed
        // (EOF is equivalent to `cancel`, p1-host-contract §1.1 C/§5.3).
        renew_stop.store(true, Ordering::SeqCst);
        renew_handle.thread().unpark();
        let _ = renew_handle.join();
        if let Ok(mut slot) = scans_for_task.0.lock() {
            slot.active = None;
            // `refresh_sources` returns `{settings_revision, statuses}`, a
            // different shape than `scan_sources` `results`; it must not
            // overwrite the retained scan snapshot read by `source_scan_status`.
        }
        if closing.0.load(Ordering::SeqCst) {
            // Do not exit out from under a committed install rollback path.
            let install_active = jobs.0.lock().map(|slot| slot.is_some()).unwrap_or(true);
            if !install_active {
                app_for_task.exit(0);
            }
        }
        result
    });
    task.await
        .map_err(|_| HostError::from("core_task_failed"))?
}

#[tauri::command]
async fn add_source(app: AppHandle, rule: Value) -> Result<Value, HostError> {
    let args = CallArguments {
        rule: Some(rule),
        ..Default::default()
    };
    one_shot_operation(app, Operation::AddSource, args).await
}

fn valid_archive(value: String) -> Result<PathBuf, HostError> {
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

/// Writes one newline-terminated control line to the `serve-task` stdin channel
/// (`renew`/`cancel`, p1-host-contract §1.1 C). Generic over the writer so the
/// wire shape can be asserted without spawning the core.
fn task_terminal_result(
    response: Result<Value, HostError>,
    exit: Result<std::process::ExitStatus, HostError>,
) -> Result<Value, HostError> {
    if response.is_ok() && !exit.as_ref().is_ok_and(|status| status.success()) {
        eprintln!("core_process_failed_after_terminal");
    }
    response
}

fn wait_core_child(child: &ChildHandle) -> Result<std::process::ExitStatus, HostError> {
    loop {
        if let Some(status) = child
            .lock()
            .map_err(|_| "core_process_failed")?
            .try_wait()
            .map_err(|_| "core_process_failed")?
        {
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn write_control_line(writer: &mut impl Write, line: &[u8]) -> Result<(), HostError> {
    writer
        .write_all(line)
        .and_then(|_| writer.flush())
        .map_err(|_| HostError::from("control_write_failed"))
}

/// Interval between forwarded `renew` control lines. Kept well below the 15s
/// foreground lease TTL (p1-host-contract §5.2).
const RENEW_INTERVAL: Duration = Duration::from_secs(3);

/// Whether the renew loop should still emit another control line: forwarding
/// stops once the task is terminal (`stop`) or the user cancelled it. A late
/// `renew` after `cancel` must therefore have no effect (p1-host-contract §5.2).
fn should_forward_renew(stop: &AtomicBool, cancelled: &AtomicBool) -> bool {
    !stop.load(Ordering::SeqCst) && !cancelled.load(Ordering::SeqCst)
}

/// Forwards `renew` to the task stdin every `RENEW_INTERVAL` until the task
/// reaches a terminal state, is cancelled, or the host exits (p1-host-contract
/// §5.2). Returns promptly on `stop`/`cancel` via an `unpark` wake-up.
fn spawn_renew_forwarder(
    stdin: StdinHandle,
    cancelled: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    spawn_renew_forwarder_with_interval(stdin, cancelled, stop, RENEW_INTERVAL)
}

/// Generic/injectable variant of [`spawn_renew_forwarder`]: the writer and
/// interval are parameters so the start/stop behaviour can be asserted without
/// spawning the core or waiting out the production 3s cadence.
fn spawn_renew_forwarder_with_interval<W>(
    stdin: Arc<Mutex<W>>,
    cancelled: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    interval: Duration,
) -> thread::JoinHandle<()>
where
    W: Write + Send + 'static,
{
    thread::spawn(move || loop {
        thread::park_timeout(interval);
        if !should_forward_renew(&stop, &cancelled) {
            return;
        }
        let Ok(mut stdin) = stdin.lock() else {
            return;
        };
        if write_control_line(&mut *stdin, b"renew\n").is_err() {
            return;
        }
    })
}

fn cancel_job(stdin: &StdinHandle, cancelled: &AtomicBool) -> Result<(), HostError> {
    cancelled.store(true, Ordering::SeqCst);
    let mut stdin = stdin.lock().map_err(|_| "job_state_unavailable")?;
    write_control_line(&mut *stdin, b"cancel\n")
}

/// Wire rejection revokes the lease and closes the control pipe; it never
/// terminates the writer process while it may be applying or rolling back.
fn cancel_and_close<W: Write>(
    stdin: &Arc<Mutex<ClosableWriter<W>>>,
    stop: &AtomicBool,
    cancelled: &AtomicBool,
) {
    stop.store(true, Ordering::SeqCst);
    cancelled.store(true, Ordering::SeqCst);
    if let Ok(mut control) = stdin.lock() {
        let _ = write_control_line(&mut *control, b"cancel\n");
        control.0.take();
    }
}

const INSTALL_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
const INSTALL_DRAIN_BYTES: usize = 64 * 1024 * 1024;
enum InstallReaderMessage {
    Rejected(HostError),
    Terminal(Value),
    Finished(Result<Value, HostError>),
}

fn wait_core_until(
    child: &ChildHandle,
    deadline: std::time::Instant,
) -> Result<std::process::ExitStatus, HostError> {
    loop {
        if let Some(status) = child
            .lock()
            .map_err(|_| "core_process_failed")?
            .try_wait()
            .map_err(|_| "core_process_failed")?
        {
            return Ok(status);
        }
        if std::time::Instant::now() >= deadline {
            return Err("core_process_shutdown_pending".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn drain_install_terminal(reader: &mut impl BufRead, request_id: &str) {
    let mut consumed = 0usize;
    while let Ok(Some((line, oversized))) = read_bounded_line(reader) {
        consumed = consumed.saturating_add(line.len());
        if oversized || consumed > INSTALL_DRAIN_BYTES {
            return;
        }
        if let Ok(value) = serde_json::from_slice::<Value>(&line) {
            if value.get("event").is_none()
                && protocol::validate_response(&value, request_id, Some(Method::InstallPatch))
                    .is_ok()
            {
                return;
            }
        }
    }
}

/// Late telemetry is untrusted teardown noise. Extra terminal frames are
/// rejected diagnostically, while the first receipt remains authoritative.
fn drain_terminal_tail(reader: &mut impl BufRead) -> usize {
    let mut consumed = 0usize;
    let mut rejected = 0usize;
    while let Ok(Some((line, oversized))) = read_bounded_line(reader) {
        consumed = consumed.saturating_add(line.len());
        if oversized || consumed > INSTALL_DRAIN_BYTES {
            eprintln!("response_too_large_after_terminal");
            return rejected;
        }
        match serde_json::from_slice::<Value>(&line) {
            Ok(value) if value.get("event").is_some() => {}
            _ => {
                rejected += 1;
                eprintln!("invalid_core_response_after_terminal");
            }
        }
    }
    rejected
}

fn close_control<W: Write>(stdin: &Arc<Mutex<ClosableWriter<W>>>) {
    if let Ok(mut control) = stdin.lock() {
        control.0.take();
    }
}

fn read_install_task_output(
    stdout: impl Read,
    request_id: &str,
    progress: Arc<Mutex<Progress>>,
    mut reject: impl FnMut(&HostError),
    mut terminal: impl FnMut(&Value),
) -> Result<Value, HostError> {
    let mut reader = BufReader::new(stdout);
    let result = read_core_stream(&mut reader, request_id, Method::InstallPatch, |_, value| {
        let parsed =
            serde_json::from_value::<Progress>(value).map_err(|_| "invalid_core_response")?;
        *progress.lock().map_err(|_| "job_state_unavailable")? = parsed;
        Ok(())
    });
    if let Err(ref error) = result {
        reject(error);
        drain_install_terminal(&mut reader, request_id);
    } else if let Ok(ref value) = result {
        terminal(value);
        drain_terminal_tail(&mut reader);
    }
    result
}

fn request_cancel(job: &ActiveJob) -> Result<(), HostError> {
    cancel_job(&job.stdin, &job.cancelled)
}

fn request_source_cancel(job: &SourceScanJob) -> Result<(), HostError> {
    cancel_job(&job.stdin, &job.cancelled)
}

/// Recognizes the streaming source events the CLI sends before the single final
/// response; anything else is treated as the terminal response.
#[cfg(test)]
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

/// One bounded parser for all task channels. Shared protocol validators own
/// event fields and ordering; callbacks only apply accepted host projections.
fn read_core_stream(
    reader: &mut impl BufRead,
    request_id: &str,
    method: Method,
    mut event_handler: impl FnMut(&str, Value) -> Result<(), HostError>,
) -> Result<Value, HostError> {
    let mut sequence = EventSequence::new(request_id.to_string());
    while let Some((line, oversized)) =
        read_bounded_line(reader).map_err(|_| "core_process_failed")?
    {
        if oversized {
            return Err("response_too_large".into());
        }
        if line.last() != Some(&b'\n') {
            return Err("invalid_core_response".into());
        }
        let value: Value = serde_json::from_slice(&line).map_err(|_| "invalid_core_response")?;
        if value.get("event").is_some() {
            // Only telemetry preceding the first validated terminal is
            // authoritative. The terminal ends this parser lifecycle.
            protocol::validate_event(&value, request_id).map_err(protocol_error)?;
            if !sequence.accept(&value).map_err(protocol_error)? {
                continue;
            }
            let event = value["event"]
                .as_str()
                .ok_or("invalid_core_response")?
                .to_owned();
            let allowed = match method.as_str() {
                "install_patch" => event == "install_progress",
                "scan_sources" => {
                    event == "source_progress"
                        || (event == "source_update" && value.get("discovery").is_some())
                }
                "refresh_sources" => event == "source_update" && value.get("source_id").is_some(),
                _ => false,
            };
            if !allowed {
                return Err("invalid_core_response".into());
            }
            event_handler(&event, value)?;
        } else {
            protocol::validate_response(&value, request_id, Some(method))
                .map_err(protocol_error)?;
            sequence.finish();
            return validate_response(value, request_id);
        }
    }
    Err("core_process_failed".into())
}

#[cfg(test)]
fn read_task_output(
    stdout: impl Read,
    request_id: String,
    progress: Arc<Mutex<Progress>>,
) -> Result<Value, HostError> {
    read_core_stream(
        &mut BufReader::new(stdout),
        &request_id,
        Method::InstallPatch,
        |_, value| {
            let parsed =
                serde_json::from_value::<Progress>(value).map_err(|_| "invalid_core_response")?;
            *progress.lock().map_err(|_| "job_state_unavailable")? = parsed;
            Ok(())
        },
    )
}

fn read_source_task_output(
    stdout: impl Read,
    request_id: String,
    app: AppHandle,
    progress: Arc<Mutex<SourceProgress>>,
) -> Result<Value, HostError> {
    read_core_stream(
        &mut BufReader::new(stdout),
        &request_id,
        Method::ScanSources,
        |name, value| {
            if name == "source_progress" {
                let parsed = serde_json::from_value::<SourceProgress>(value.clone())
                    .map_err(|_| "invalid_core_response")?;
                *progress.lock().map_err(|_| "job_state_unavailable")? = parsed;
            }
            let _ = app.emit(name, value);
            Ok(())
        },
    )
}

fn read_refresh_stream(
    stdout: impl Read,
    request_id: &str,
    mut emit: impl FnMut(&'static str, Value),
    _progress: &Arc<Mutex<SourceProgress>>,
) -> Result<Value, HostError> {
    read_core_stream(
        &mut BufReader::new(stdout),
        request_id,
        Method::RefreshSources,
        |_, value| {
            emit("source_update", value);
            Ok(())
        },
    )
}

/// Reads a `serve-task` `refresh_sources` stream and forwards each accepted
/// event to the frontend. The mirror is kept for parity with the shared source
/// task slot even though `refresh_sources` emits no `source_progress`.
fn read_refresh_task_output(
    stdout: impl Read,
    request_id: String,
    app: AppHandle,
    progress: Arc<Mutex<SourceProgress>>,
) -> Result<Value, HostError> {
    read_refresh_stream(
        stdout,
        &request_id,
        move |name, value| {
            // A closed event pipe must not abort the task reader.
            let _ = app.emit(name, value);
        },
        &progress,
    )
}

/// Validates the `start_install` arguments and builds the `install_patch`
/// request envelope. Pure so tests can assert `selection_id`/`release_id`
/// mutual exclusion and the `archive` route restriction without spawning the
/// core (p1-host-contract §1.1 A, §6.1).
fn build_install_request(
    instance_id: &str,
    release_id: Option<&str>,
    selection_id: Option<&str>,
    operation: &str,
    request_id: &str,
    archive: Option<PathBuf>,
    common: Value,
) -> Result<Value, HostError> {
    if !protocol::validate_id(instance_id, protocol::MAX_INSTANCE_ID_BYTES) {
        return Err("invalid_id".into());
    }
    for id in [release_id, selection_id].into_iter().flatten() {
        if !valid_id(id) {
            return Err("invalid_id".into());
        }
    }
    if !matches!(operation, "install" | "reinstall" | "overwrite") {
        return Err("invalid_operation".into());
    }
    if !valid_request_id(request_id) {
        return Err("invalid_request_id".into());
    }
    match (selection_id, release_id) {
        (Some(_), Some(_)) => return Err("invalid_request".into()),
        (None, None) => return Err("selection_or_release_required".into()),
        _ => {}
    }
    // `archive` is only valid on the signed `release_id` route; a dynamic
    // selection must never accept a caller-supplied archive (p1-host-contract
    // §1.1 A). Rejected locally, never sent to the core.
    if selection_id.is_some() && archive.is_some() {
        return Err("archive_not_allowed_for_selection".into());
    }

    let mut params = common;
    params["instance_id"] = json!(instance_id);
    if let Some(value) = release_id {
        params["release_id"] = json!(value);
    }
    if let Some(value) = selection_id {
        params["selection_id"] = json!(value);
    }
    params["operation"] = json!(operation);
    params["cleanup_policy"] = json!("disabled");
    if let Some(path) = archive {
        params["archive"] = json!(path);
    }
    let request = json!({
        "protocol_version": PROTOCOL_VERSION,
        "request_id": request_id,
        "method": "install_patch",
        "params": params,
    });
    let payload = serde_json::to_vec(&request).map_err(|_| "request_encoding_failed")?;
    let mut request_line = payload.clone();
    request_line.push(b'\n');
    protocol::decode_request(
        &request_line,
        request["method"]
            .as_str()
            .is_some_and(|m| matches!(m, "install_patch" | "scan_sources" | "refresh_sources")),
    )
    .map_err(protocol_error)?;
    if payload.len().saturating_add(1) > MAX_REQUEST {
        return Err("request_too_large".into());
    }
    Ok(request)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn start_install(
    app: AppHandle,
    jobs: State<'_, Jobs>,
    instance_id: String,
    release_id: Option<String>,
    selection_id: Option<String>,
    operation: String,
    request_id: String,
    archive: Option<String>,
) -> Result<Value, HostError> {
    let archive = archive.map(valid_archive).transpose()?;
    require_capabilities(&app)?;
    let request = build_install_request(
        &instance_id,
        release_id.as_deref(),
        selection_id.as_deref(),
        &operation,
        &request_id,
        archive,
        common_params(&app)?,
    )?;
    let payload = serde_json::to_vec(&request).map_err(|_| "request_encoding_failed")?;
    let mut request_line = payload.clone();
    request_line.push(b'\n');
    protocol::decode_request(
        &request_line,
        request["method"]
            .as_str()
            .is_some_and(|m| matches!(m, "install_patch" | "scan_sources" | "refresh_sources")),
    )
    .map_err(protocol_error)?;

    let jobs = jobs.inner().clone();
    let closing = app.state::<Closing>().inner().clone();
    // Keep the mutex guard inside this block so no non-Send guard is held across the `.await` below.
    let (child_handle, control_stdin, cancelled, progress, stdout, renew_handle, renew_stop) = {
        let mut slot = jobs.0.lock().map_err(|_| "job_state_unavailable")?;
        if slot.is_some() {
            return Err("install_in_progress".into());
        }
        let mut child = core_command(&binary(&app)?)
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

        let stdin_handle = Arc::new(Mutex::new(ClosableWriter(Some(stdin))));
        let cancelled = Arc::new(AtomicBool::new(false));
        let renew_stop = Arc::new(AtomicBool::new(false));
        let renew_handle =
            spawn_renew_forwarder(stdin_handle.clone(), cancelled.clone(), renew_stop.clone());
        let active = ActiveJob {
            child: Arc::new(Mutex::new(child)),
            stdin: stdin_handle,
            progress: Arc::new(Mutex::new(Progress::default())),
            cancelled,
        };
        let child_handle = active.child.clone();
        let progress = active.progress.clone();
        let control_stdin = active.stdin.clone();
        let cancelled = active.cancelled.clone();
        *slot = Some(active);
        (
            child_handle,
            control_stdin,
            cancelled,
            progress,
            stdout,
            renew_handle,
            renew_stop,
        )
    };

    let app_for_task = app.clone();
    let response_id = request_id.clone();
    let task = tauri::async_runtime::spawn_blocking(move || {
        let (sender, messages) = std::sync::mpsc::channel();
        let rejection_stop = renew_stop.clone();
        let rejection_stdin = control_stdin.clone();
        let rejection_cancelled = cancelled.clone();
        let terminal_sender = sender.clone();
        let terminal_stdin = control_stdin.clone();
        let terminal_stop = renew_stop.clone();
        let reader = thread::spawn(move || {
            let result = read_install_task_output(
                stdout,
                &response_id,
                progress,
                |error| {
                    cancel_and_close(&rejection_stdin, &rejection_stop, &rejection_cancelled);
                    let _ = sender.send(InstallReaderMessage::Rejected(error.clone()));
                },
                |value| {
                    terminal_stop.store(true, Ordering::SeqCst);
                    close_control(&terminal_stdin);
                    let _ = terminal_sender.send(InstallReaderMessage::Terminal(value.clone()));
                },
            );
            let _ = sender.send(InstallReaderMessage::Finished(result));
        });
        let mut rejection = None;
        let mut accepted_terminal = None;
        let mut deadline = None;
        let response = loop {
            match messages.recv_timeout(Duration::from_millis(20)) {
                Ok(InstallReaderMessage::Rejected(error)) => {
                    rejection = Some(error);
                    deadline = Some(std::time::Instant::now() + INSTALL_DRAIN_TIMEOUT);
                }
                Ok(InstallReaderMessage::Terminal(value)) => {
                    accepted_terminal = Some(value);
                    deadline = Some(std::time::Instant::now() + INSTALL_DRAIN_TIMEOUT);
                }
                Ok(InstallReaderMessage::Finished(result)) => break result,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    break accepted_terminal.clone().ok_or_else(|| {
                        rejection
                            .clone()
                            .unwrap_or_else(|| "core_process_failed".into())
                    });
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if deadline.is_some_and(|until| std::time::Instant::now() >= until) {
                        break accepted_terminal.clone().ok_or_else(|| {
                            rejection
                                .clone()
                                .unwrap_or_else(|| "core_process_failed".into())
                        });
                    }
                }
            }
        };
        if response.is_err() {
            cancel_and_close(&control_stdin, &renew_stop, &cancelled);
        }
        let status = wait_core_until(
            &child_handle,
            deadline.unwrap_or_else(|| std::time::Instant::now() + INSTALL_DRAIN_TIMEOUT),
        );
        let exited = status.is_ok();
        if reader.is_finished() {
            let _ = reader.join();
        }
        // A validated terminal is authoritative even if the process fails
        // during teardown. Incomplete or corrupt streams already return Err.
        let result = task_terminal_result(response, status);
        // Stop lease forwarding, then drop the job so its stdin is closed
        // (EOF is equivalent to `cancel`, p1-host-contract §1.1 C/§5.3).
        renew_stop.store(true, Ordering::SeqCst);
        renew_handle.thread().unpark();
        let _ = renew_handle.join();
        if exited {
            if let Ok(mut current) = jobs.0.lock() {
                current.take();
            }
            if closing.0.load(Ordering::SeqCst) {
                app_for_task.exit(0);
            }
        } else {
            // Keep the active slot occupied until the writer really exits. A
            // timeout is diagnostic, never permission to start a second write.
            thread::spawn(move || {
                if wait_core_child(&child_handle).is_ok() {
                    if let Ok(mut current) = jobs.0.lock() {
                        current.take();
                    }
                    if closing.0.load(Ordering::SeqCst) {
                        app_for_task.exit(0);
                    }
                } else {
                    eprintln!("core_process_shutdown_pending");
                }
            });
        }
        result
    });
    task.await
        .map_err(|_| HostError::from("core_task_failed"))?
}

#[tauri::command]
fn install_status(jobs: State<'_, Jobs>) -> Result<Value, HostError> {
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
fn cancel_install(jobs: State<'_, Jobs>) -> Result<bool, HostError> {
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
fn source_scan_request(id: &str, common: Value, force: bool) -> Result<Value, HostError> {
    let mut params = common;
    params["force"] = json!(force);
    let request = json!({
        "protocol_version": PROTOCOL_VERSION,
        "request_id": id,
        "method": "scan_sources",
        "params": params,
    });
    let payload = serde_json::to_vec(&request).map_err(|_| "request_encoding_failed")?;
    let mut request_line = payload.clone();
    request_line.push(b'\n');
    protocol::decode_request(
        &request_line,
        request["method"]
            .as_str()
            .is_some_and(|m| matches!(m, "install_patch" | "scan_sources" | "refresh_sources")),
    )
    .map_err(protocol_error)?;
    if payload.len().saturating_add(1) > MAX_REQUEST {
        return Err("request_too_large".into());
    }
    Ok(request)
}

/// Builds the `serve-task` `refresh_sources` request and enforces the
/// request-size bound. `refresh_sources` is a task-channel method
/// (p1-host-contract §1.1 B), so it must reuse the `serve-task` control channel
/// exactly like [`source_scan_request`]; routing it through one-shot `serve` is
/// rejected by the core with `task_required`. Pure so tests can assert the wire
/// shape without spawning the core.
fn source_refresh_request(id: &str, common: Value, force: bool) -> Result<Value, HostError> {
    let mut params = common;
    params["force"] = json!(force);
    let request = json!({
        "protocol_version": PROTOCOL_VERSION,
        "request_id": id,
        "method": "refresh_sources",
        "params": params,
    });
    let payload = serde_json::to_vec(&request).map_err(|_| "request_encoding_failed")?;
    let mut request_line = payload.clone();
    request_line.push(b'\n');
    protocol::decode_request(
        &request_line,
        request["method"]
            .as_str()
            .is_some_and(|m| matches!(m, "install_patch" | "scan_sources" | "refresh_sources")),
    )
    .map_err(protocol_error)?;
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
) -> Result<Value, HostError> {
    require_capabilities(&app)?;
    let id = request_id();
    let request = source_scan_request(&id, common_params(&app)?, force.unwrap_or(false))?;
    let payload = serde_json::to_vec(&request).map_err(|_| "request_encoding_failed")?;
    let mut request_line = payload.clone();
    request_line.push(b'\n');
    protocol::decode_request(
        &request_line,
        request["method"]
            .as_str()
            .is_some_and(|m| matches!(m, "install_patch" | "scan_sources" | "refresh_sources")),
    )
    .map_err(protocol_error)?;

    let scans = scans.inner().clone();
    let closing = app.state::<Closing>().inner().clone();
    let jobs = app.state::<Jobs>().inner().clone();
    // Keep the mutex guard inside this block so no non-Send guard is held across the `.await` below.
    let (child_handle, progress, stdout, renew_handle, renew_stop) = {
        let mut slot = scans.0.lock().map_err(|_| "job_state_unavailable")?;
        if slot.active.is_some() {
            return Err("source_scan_in_progress".into());
        }
        slot.results = None;
        let mut child = core_command(&binary(&app)?)
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

        let stdin_handle = Arc::new(Mutex::new(ClosableWriter(Some(stdin))));
        let cancelled = Arc::new(AtomicBool::new(false));
        let renew_stop = Arc::new(AtomicBool::new(false));
        // Source scans run on the same 15s foreground lease as installs, so the
        // host must forward `renew` here too (p1-host-contract §5.2).
        let renew_handle =
            spawn_renew_forwarder(stdin_handle.clone(), cancelled.clone(), renew_stop.clone());
        let active = SourceScanJob {
            child: Arc::new(Mutex::new(child)),
            stdin: stdin_handle,
            progress: Arc::new(Mutex::new(SourceProgress::default())),
            cancelled,
        };
        let child_handle = active.child.clone();
        let progress = active.progress.clone();
        slot.active = Some(active);
        (child_handle, progress, stdout, renew_handle, renew_stop)
    };

    let app_for_task = app.clone();
    let app_for_reader = app.clone();
    let scans_for_task = scans.clone();
    let response_id = id.clone();
    let task = tauri::async_runtime::spawn_blocking(move || {
        let rejection_child = child_handle.clone();
        let rejection_stop = renew_stop.clone();
        let reader = thread::spawn(move || {
            let result = read_source_task_output(stdout, response_id, app_for_reader, progress);
            if result.is_err() {
                rejection_stop.store(true, Ordering::SeqCst);
                // This source lookup child owns no installer game writes or
                // rollback. Installer rejection uses cancel/EOF instead.
                if let Ok(mut child) = rejection_child.lock() {
                    let _ = child.kill();
                }
            }
            result
        });
        let status = wait_core_child(&child_handle);
        let response = match reader.join() {
            Ok(response) => response,
            Err(_) => Err(HostError::from("core_process_failed")),
        };
        // A validated terminal is authoritative even if the process fails
        // during teardown. Incomplete or corrupt streams already return Err.
        let result = task_terminal_result(response, status);
        // Stop lease forwarding, then drop the job so its stdin is closed
        // (EOF is equivalent to `cancel`, p1-host-contract §1.1 C/§5.3).
        renew_stop.store(true, Ordering::SeqCst);
        renew_handle.thread().unpark();
        let _ = renew_handle.join();
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
                app_for_task.exit(0);
            }
        }
        result
    });
    task.await
        .map_err(|_| HostError::from("core_task_failed"))?
}

#[tauri::command]
fn source_scan_status(scans: State<'_, SourceScans>) -> Result<Value, HostError> {
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
fn cancel_source_scan(scans: State<'_, SourceScans>) -> Result<bool, HostError> {
    let slot = scans.0.lock().map_err(|_| "job_state_unavailable")?;
    if let Some(job) = slot.active.as_ref() {
        request_source_cancel(job)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
fn choose_archive(app: AppHandle) -> Result<Option<String>, HostError> {
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
fn open_source(app: AppHandle, url: String) -> Result<(), HostError> {
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
            get_capabilities,
            list_sources,
            list_game_sources,
            set_source_enabled,
            set_game_source,
            discover_game,
            refresh_sources,
            add_source,
            start_source_scan,
            source_scan_status,
            cancel_source_scan
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Steam Patch Manager");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn common() -> Value {
        json!({"host":{"roots":[],"catalog":null,"signed_catalog":null,"trust_keys":null,"platform":"windows","state_dir":"C:/state"}})
    }

    #[test]
    fn operation_method_names_are_snake_case_and_deleted_ones_are_absent() {
        let methods = [
            Operation::Scan.method(),
            Operation::Preview.method(),
            Operation::ListTransactions.method(),
            Operation::Recover.method(),
            Operation::GetCapabilities.method(),
            Operation::ListSources.method(),
            Operation::ListGameSources.method(),
            Operation::SetSourceEnabled.method(),
            Operation::SetGameSource.method(),
            Operation::AddSource.method(),
            Operation::RefreshSources.method(),
            Operation::DiscoverGame.method(),
        ];
        let expected = [
            "scan",
            "preview",
            "list_transactions",
            "recover",
            "get_capabilities",
            "list_sources",
            "list_game_sources",
            "set_source_enabled",
            "set_game_source",
            "add_source",
            "refresh_sources",
            "discover_game",
        ];
        assert_eq!(methods, expected);
        // Deleted v2 operations must not deserialize or be advertised.
        for deleted in [
            "migrate_state",
            "get_source_policy",
            "resolve_source_record",
            "list_source_matches",
        ] {
            assert!(!methods.contains(&deleted));
            assert!(serde_json::from_str::<Operation>(&format!("\"{deleted}\"")).is_err());
        }
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
        assert_eq!(params["host"]["state_dir"], "C:/state");
        assert_eq!(params["host"]["platform"], "windows");
        assert_eq!(params["instance_id"], "inst-1");
        assert_eq!(params["source_id"], "kagura-cn");
        assert_eq!(params["subscribed"], true);
        // Unset optional arguments must not leak into the strict core params.
        assert!(params.get("enabled").is_none());
        assert!(params.get("rule").is_none());
        assert!(params.get("force").is_none());
        assert!(params.get("selection_id").is_none());
    }

    #[test]
    fn add_source_passes_rule_value_through() {
        let rule = json!({"id":"custom","name":"Custom","adapter":"table","enabled":true,"pages":[],"page_hosts":[]});
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
    fn preview_requires_exactly_one_of_selection_or_release() {
        let by_release = build_params(
            "preview",
            CallArguments {
                instance_id: Some("inst-1".into()),
                release_id: Some("release-1".into()),
                ..Default::default()
            },
            common(),
        )
        .unwrap();
        assert_eq!(by_release["release_id"], "release-1");
        assert!(by_release.get("selection_id").is_none());

        let by_selection = build_params(
            "preview",
            CallArguments {
                instance_id: Some("inst-1".into()),
                selection_id: Some("zip-selection-0123456789abcdef01234567-10".into()),
                ..Default::default()
            },
            common(),
        )
        .unwrap();
        assert_eq!(
            by_selection["selection_id"],
            "zip-selection-0123456789abcdef01234567-10"
        );
        assert!(by_selection.get("release_id").is_none());

        // Both provided -> invalid_request.
        assert_eq!(
            build_params(
                "preview",
                CallArguments {
                    instance_id: Some("inst-1".into()),
                    release_id: Some("release-1".into()),
                    selection_id: Some("zip-selection-0123456789abcdef01234567-10".into()),
                    ..Default::default()
                },
                common(),
            )
            .unwrap_err(),
            "invalid_request"
        );
        // Neither provided -> selection_or_release_required.
        assert_eq!(
            build_params(
                "preview",
                CallArguments {
                    instance_id: Some("inst-1".into()),
                    ..Default::default()
                },
                common(),
            )
            .unwrap_err(),
            "selection_or_release_required"
        );
        // Missing instance -> instance_required.
        assert_eq!(
            build_params(
                "preview",
                CallArguments {
                    release_id: Some("release-1".into()),
                    ..Default::default()
                },
                common(),
            )
            .unwrap_err(),
            "instance_required"
        );
    }

    #[test]
    fn install_request_routes_selection_or_release() {
        let signed = build_install_request(
            "inst-1",
            Some("release-1"),
            None,
            "install",
            "req-1",
            None,
            common(),
        )
        .unwrap();
        assert_eq!(signed["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(signed["method"], "install_patch");
        assert_eq!(signed["request_id"], "req-1");
        assert_eq!(signed["params"]["instance_id"], "inst-1");
        assert_eq!(signed["params"]["release_id"], "release-1");
        assert_eq!(signed["params"]["operation"], "install");
        assert_eq!(signed["params"]["cleanup_policy"], "disabled");
        assert!(signed["params"].get("request_id").is_none());
        assert!(signed["params"].get("selection_id").is_none());

        let dynamic = build_install_request(
            "inst-1",
            None,
            Some("zip-selection-0123456789abcdef01234567-10"),
            "reinstall",
            "req-2",
            None,
            common(),
        )
        .unwrap();
        assert_eq!(
            dynamic["params"]["selection_id"],
            "zip-selection-0123456789abcdef01234567-10"
        );
        assert_eq!(dynamic["params"]["operation"], "reinstall");
        assert!(dynamic["params"].get("release_id").is_none());

        let overwrite = build_install_request(
            "inst-1",
            None,
            Some("exe-selection-0123456789abcdef01234567-10"),
            "overwrite",
            "req-3",
            None,
            common(),
        )
        .unwrap();
        assert_eq!(overwrite["params"]["operation"], "overwrite");
        assert!(overwrite["params"].get("request_id").is_none());
    }

    #[test]
    fn install_request_rejects_mutual_exclusion_and_archive_route_violations() {
        assert_eq!(
            build_install_request(
                "inst-1",
                Some("release-1"),
                Some("zip-selection-0123456789abcdef01234567-10"),
                "install",
                "req-1",
                None,
                common(),
            )
            .unwrap_err(),
            "invalid_request"
        );
        assert_eq!(
            build_install_request("inst-1", None, None, "install", "req-1", None, common())
                .unwrap_err(),
            "selection_or_release_required"
        );
        assert_eq!(
            build_install_request(
                "inst-1",
                None,
                Some("zip-selection-0123456789abcdef01234567-10"),
                "install",
                "req-1",
                Some(PathBuf::from("C:/patches/patch.zip")),
                common(),
            )
            .unwrap_err(),
            "archive_not_allowed_for_selection"
        );
        assert_eq!(
            build_install_request(
                "inst-1",
                Some("release-1"),
                None,
                "uninstall",
                "req-1",
                None,
                common()
            )
            .unwrap_err(),
            "invalid_operation"
        );
        // `archive` remains valid on the signed route.
        let signed = build_install_request(
            "inst-1",
            Some("release-1"),
            None,
            "install",
            "req-1",
            Some(PathBuf::from("C:/patches/patch.zip")),
            common(),
        )
        .unwrap();
        assert_eq!(signed["params"]["archive"], "C:/patches/patch.zip");
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
    }

    #[test]
    fn call_arguments_are_strict_and_ids_are_validated() {
        let unknown = serde_json::from_str::<CallArguments>(r#"{"instanceId":"i","shell":"evil"}"#);
        assert!(unknown.is_err());

        let camel = serde_json::from_str::<CallArguments>(
            r#"{"instanceId":"i","sourceId":"s","selectionId":"zip-selection-0123456789abcdef01234567-10","preferredLanguage":"zh-CN"}"#,
        )
        .unwrap();
        assert_eq!(camel.instance_id.as_deref(), Some("i"));
        assert_eq!(camel.source_id.as_deref(), Some("s"));
        assert_eq!(
            camel.selection_id.as_deref(),
            Some("zip-selection-0123456789abcdef01234567-10")
        );
        assert_eq!(camel.preferred_language.as_deref(), Some("zh-CN"));

        // `recordId` belonged to the deleted `resolve_source_record` command.
        assert!(serde_json::from_str::<CallArguments>(r#"{"recordId":"candidate-1"}"#).is_err());

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
        assert_eq!(request["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(request["request_id"], "host-1-2");
        assert_eq!(request["method"], "scan_sources");
        assert_eq!(request["params"]["host"]["state_dir"], "C:/state");
        assert_eq!(request["params"]["force"], false);
    }

    #[test]
    fn source_refresh_request_shape_is_serve_task_and_carries_force() {
        let request = source_refresh_request("host-9-1", common(), true).unwrap();
        // Mirrors `source_scan_request`: same v4 envelope, same host-injected
        // params, task-channel method name, and the `force` flag.
        assert_eq!(request["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(request["request_id"], "host-9-1");
        assert_eq!(request["method"], "refresh_sources");
        assert_eq!(request["params"]["host"]["state_dir"], "C:/state");
        assert_eq!(request["params"]["host"]["platform"], "windows");
        assert_eq!(request["params"]["force"], true);
        // The command must not be assembled as a one-shot `serve` method; that
        // route is what the core rejected with `task_required`.
        assert_ne!(request["method"], "scan_sources");
    }

    #[test]
    fn refresh_stream_deduplicates_and_returns_statuses() {
        fn update(request_id: &str, sequence: u64, source_id: &str) -> Value {
            json!({
                "protocol_version": PROTOCOL_VERSION,
                "request_id": request_id,
                "sequence": sequence,
                "event": "source_update",
                "source_id": source_id,
                "status": {"code":"source_refreshed","checked_at":0,"cached_at":null,"entry_count":0},
                "settings_revision": 7,
            })
        }
        let terminal = json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": "r",
            "ok": true,
            "result": {"settings_revision": 7, "statuses": [{"code":"source_refreshed","checked_at":0,"cached_at":null,"entry_count":0}]},
        });
        protocol::validate_event(&update("r", 0, "kagura"), "r").unwrap();
        protocol::validate_response(&terminal, "r", Some(Method::RefreshSources)).unwrap();
        protocol::validate_response(&terminal, "r", None).unwrap();
        let input = task_stream(&[
            update("r", 0, "kagura"),
            // Duplicate sequence for the same task: dropped.
            update("r", 0, "kagura"),
            update("r", 1, "moe"),
            terminal,
            // Events after the terminal line are ignored.
            update("r", 2, "late"),
        ]);
        let progress = Arc::new(Mutex::new(SourceProgress::default()));
        let mut forwarded: Vec<Value> = Vec::new();
        let result = read_refresh_stream(
            std::io::Cursor::new(input),
            "r",
            |_name, value| forwarded.push(value),
            &progress,
        )
        .unwrap();
        assert_eq!(result["settings_revision"], 7);
        assert_eq!(result["statuses"][0]["code"], "source_refreshed");
        assert_eq!(forwarded.len(), 2);
        assert_eq!(forwarded[0]["source_id"], "kagura");
        assert_eq!(forwarded[1]["source_id"], "moe");
    }

    #[test]
    fn refresh_stream_rejects_malformed_sequence() {
        fn update(request_id: &str, sequence: u64) -> Value {
            json!({
                "protocol_version": PROTOCOL_VERSION,
                "request_id": request_id,
                "sequence": sequence,
                "event": "source_update",
                "source_id": "kagura",
                "status": {"code":"source_refreshed","checked_at":0,"cached_at":null,"entry_count":0},
                "settings_revision": 7,
            })
        }
        let mut malformed = update("r", 0);
        malformed.as_object_mut().unwrap().remove("sequence");
        let terminal = json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": "r",
            "ok": true,
            "result": {"settings_revision": 7, "statuses": []},
        });
        let input = task_stream(&[malformed, terminal]);
        let progress = Arc::new(Mutex::new(SourceProgress::default()));
        assert_eq!(
            read_refresh_stream(std::io::Cursor::new(input), "r", |_, _| {}, &progress)
                .unwrap_err(),
            "invalid_core_response"
        );
    }

    #[test]
    fn source_stream_separates_events_from_final_response() {
        let progress = json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": "r",
            "sequence": 0,
            "event": "source_progress",
            "completed": 1,
            "total": 2,
            "running": true,
            "settings_revision": 5,
        });
        let update = json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": "r",
            "sequence": 1,
            "event": "source_update",
            "appid": 42,
            "index": 0,
            "settings_revision": 5,
            "discovery": {},
        });
        let terminal = json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": "r",
            "ok": true,
            "result": {"settings_revision": 5, "results": []},
        });
        assert_eq!(source_event_name(&progress), Some("source_progress"));
        assert_eq!(source_event_name(&update), Some("source_update"));
        assert_eq!(source_event_name(&terminal), None);

        let parsed = serde_json::from_value::<SourceProgress>(progress)
            .unwrap()
            .unwrap();
        assert_eq!(parsed.completed, 1);
        assert_eq!(parsed.total, 2);
        assert!(parsed.running);
        assert_eq!(parsed.settings_revision, 5);
    }

    #[test]
    fn validate_response_requires_v4_and_preserves_error_detail() {
        let ok = json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": "r",
            "ok": true,
            "result": [],
        });
        assert_eq!(validate_response(ok, "r").unwrap(), json!([]));

        let v2 = json!({
            "protocol_version": 2,
            "request_id": "r",
            "ok": true,
            "result": {},
        });
        assert_eq!(
            validate_response(v2, "r").unwrap_err(),
            "unsupported_protocol"
        );

        let mismatched_id = json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": "other",
            "ok": true,
            "result": {},
        });
        assert_eq!(
            validate_response(mismatched_id, "r").unwrap_err(),
            "invalid_core_response"
        );

        let coded = json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": "r",
            "ok": false,
            "error": {"code": "needs_recovery"},
        });
        assert_eq!(validate_response(coded, "r").unwrap_err(), "needs_recovery");

        let detailed = json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": "r",
            "ok": false,
            "error": {"code": "download_failed", "detail": "HTTP 404"},
        });
        let error = validate_response(detailed, "r").unwrap_err();
        assert_eq!(error.code, "download_failed");
        assert_eq!(error.detail.as_deref(), Some("HTTP 404"));

        let empty_detail = json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": "r",
            "ok": false,
            "error": {"code": "cancelled", "detail": ""},
        });
        assert_eq!(
            validate_response(empty_detail, "r").unwrap_err(),
            "cancelled"
        );
    }

    #[test]
    fn error_detail_is_truncated_to_the_frozen_bound() {
        let long = "x".repeat(MAX_DETAIL + 500);
        let rendered = truncate_detail(&long);
        // The bounded wire detail has no presentation ellipsis.
        assert!(rendered.len() <= MAX_DETAIL);
        assert!(!rendered.ends_with('…'));
        assert_eq!(rendered.len(), MAX_DETAIL);
        assert_eq!(truncate_detail("short"), "short");

        // Multi-byte boundary: truncation must land on a char boundary and the
        // rendered value must still stay within the frozen bound.
        let multibyte = "补".repeat(MAX_DETAIL); // 3 bytes per code point
        let rendered = truncate_detail(&multibyte);
        assert!(rendered.len() <= MAX_DETAIL);
        assert!(!rendered.ends_with('…'));
        assert!(rendered.is_char_boundary(rendered.len()));
    }

    #[test]
    fn control_lines_write_renew_and_cancel() {
        let mut renew = Vec::new();
        write_control_line(&mut renew, b"renew\n").unwrap();
        assert_eq!(renew, b"renew\n");

        let mut cancel = Vec::new();
        write_control_line(&mut cancel, b"cancel\n").unwrap();
        assert_eq!(cancel, b"cancel\n");
    }

    #[test]
    fn renew_forwarding_stops_on_stop_or_cancel() {
        let stop = AtomicBool::new(false);
        let cancelled = AtomicBool::new(false);
        assert!(should_forward_renew(&stop, &cancelled));

        // A cancelled task must not accept a late `renew`.
        cancelled.store(true, Ordering::SeqCst);
        assert!(!should_forward_renew(&stop, &cancelled));

        // Terminal/stop signal also halts forwarding.
        cancelled.store(false, Ordering::SeqCst);
        stop.store(true, Ordering::SeqCst);
        assert!(!should_forward_renew(&stop, &cancelled));
    }

    /// Waits until `writer` has at least `len` bytes, panicking past the deadline.
    fn wait_for_bytes(writer: &Arc<Mutex<Vec<u8>>>, len: usize) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while writer.lock().unwrap().len() < len {
            assert!(
                std::time::Instant::now() < deadline,
                "renew line was not forwarded in time"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn renew_forwarder_writes_until_stopped() {
        let writer = Arc::new(Mutex::new(Vec::<u8>::new()));
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = spawn_renew_forwarder_with_interval(
            writer.clone(),
            cancelled.clone(),
            stop.clone(),
            Duration::from_millis(2),
        );
        wait_for_bytes(&writer, b"renew\nrenew\n".len());

        stop.store(true, Ordering::SeqCst);
        handle.thread().unpark();
        handle.join().unwrap();

        let after_stop = writer.lock().unwrap().len();
        thread::sleep(Duration::from_millis(20));
        assert_eq!(
            writer.lock().unwrap().len(),
            after_stop,
            "forwarding continued after the stop signal"
        );
    }

    #[test]
    fn renew_forwarder_ignores_renew_after_cancel() {
        let writer = Arc::new(Mutex::new(Vec::<u8>::new()));
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = spawn_renew_forwarder_with_interval(
            writer.clone(),
            cancelled.clone(),
            stop.clone(),
            Duration::from_millis(2),
        );
        wait_for_bytes(&writer, b"renew\n".len());

        cancelled.store(true, Ordering::SeqCst);
        handle.thread().unpark();
        handle.join().unwrap();

        let after_cancel = writer.lock().unwrap().len();
        thread::sleep(Duration::from_millis(20));
        assert_eq!(
            writer.lock().unwrap().len(),
            after_cancel,
            "a cancelled task kept forwarding renew"
        );
    }

    fn task_stream(lines: &[Value]) -> Vec<u8> {
        let mut input = Vec::new();
        for line in lines {
            serde_json::to_writer(&mut input, line).unwrap();
            input.push(b'\n');
        }
        input
    }

    fn install_progress_event(request_id: &str, sequence: u64, stage: &str) -> Value {
        json!({
            "event": "install_progress",
            "protocol_version": PROTOCOL_VERSION,
            "request_id": request_id,
            "sequence": sequence,
            "instance_id": "inst-1",
            "selection_id": "zip-selection-0123456789abcdef01234567-10",
            "stage": stage,
            "substage": null,
            "bytes_done": sequence * 10,
            "total_bytes": null,
            "files_done": sequence as usize,
            "total_files": null,
            "current_file": null,
            "archive_bytes": 0,
        })
    }

    fn shared_fixtures() -> Value {
        serde_json::from_str(
            &std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../packages/contracts/fixtures/v4/cases.json"
            ))
            .unwrap(),
        )
        .unwrap()
    }
    fn terminal_response(request_id: &str) -> Value {
        let cases = shared_fixtures();
        let mut response = cases["responses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == "install_patch-success")
            .unwrap()["response"]
            .clone();
        response["request_id"] = json!(request_id);
        response
    }

    #[test]
    fn shared_generated_responses_events_and_capability_gates() {
        let cases = shared_fixtures();
        for case in cases["requests"].as_array().unwrap() {
            let mut bytes = serde_json::to_vec(&case["request"]).unwrap();
            bytes.push(b'\n');
            let actual = protocol::decode_request(&bytes, case["task_channel"].as_bool().unwrap());
            if let Some(error) = case["error"].as_str() {
                assert_eq!(actual.unwrap_err().code, error, "{}", case["name"]);
            } else {
                actual.unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
            }
        }
        for case in cases["sequences"].as_array().unwrap() {
            let mut sequence = EventSequence::new("fixture".into());
            for (index, (event, expected)) in case["events"]
                .as_array()
                .unwrap()
                .iter()
                .zip(case["expected"].as_array().unwrap())
                .enumerate()
            {
                if case["terminal_after"].as_u64() == Some(index as u64) {
                    sequence.finish();
                }
                let actual = sequence.accept(event);
                match expected.as_str().unwrap() {
                    "accept" => assert!(actual.unwrap()),
                    "drop" => assert!(!actual.unwrap()),
                    "error" => assert_eq!(actual.unwrap_err().code, "invalid_core_response"),
                    _ => panic!("unknown fixture expectation"),
                }
            }
        }
        for case in cases["responses"].as_array().unwrap() {
            let method: Method = serde_json::from_value(case["method"].clone()).unwrap();
            let response = &case["response"];
            let actual = protocol::validate_response(response, "fixture", Some(method));
            if let Some(error) = case["error"].as_str() {
                assert_eq!(actual.unwrap_err().code, error, "{}", case["name"]);
            } else {
                actual.unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
            }
        }
        for case in cases["events"].as_array().unwrap() {
            let event = &case["event"];
            let actual = protocol::validate_event(event, "fixture");
            if let Some(error) = case["error"].as_str() {
                assert_eq!(actual.unwrap_err().code, error, "{}", case["name"]);
            } else {
                actual.unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
            }
        }
        let mut caps = cases["responses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == "get_capabilities-success")
            .unwrap()["response"]["result"]
            .clone();
        protocol::validate_capabilities(&caps).unwrap();
        let original = caps.clone();
        caps.as_object_mut().unwrap().remove("contracts");
        assert_eq!(
            protocol::validate_capabilities(&caps).unwrap_err().code,
            "capability_mismatch"
        );
        caps = original.clone();
        caps["methods"] = json!([]);
        assert_eq!(
            protocol::validate_capabilities(&caps).unwrap_err().code,
            "capability_mismatch"
        );
        caps = original;
        caps["installer"]["schema_version"] = json!(2);
        assert_eq!(
            protocol::validate_capabilities(&caps).unwrap_err().code,
            "installer_schema_unsupported"
        );
    }

    #[test]
    fn source_scan_stream_shares_event_sequence_and_ignores_late_events() {
        let cases = shared_fixtures();
        let event = |name: &str| {
            cases["events"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["name"] == name)
                .unwrap()["event"]
                .clone()
        };
        let first = event("source_progress");
        let mut second = event("source_update-game");
        second["sequence"] = json!(1);
        let mut late = event("source_progress");
        late["sequence"] = json!(2);
        let terminal = cases["responses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "scan_sources-success")
            .unwrap()["response"]
            .clone();
        let mut forwarded = Vec::new();
        read_core_stream(
            &mut std::io::Cursor::new(task_stream(&[
                first.clone(),
                second.clone(),
                first,
                terminal,
                late,
            ])),
            "fixture",
            Method::ScanSources,
            |name, _| {
                forwarded.push(name.to_string());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(forwarded, ["source_progress", "source_update"]);
        second["sequence"] = json!(0);
        second["request_id"] = json!("other");
        assert_eq!(
            read_core_stream(
                &mut std::io::Cursor::new(task_stream(&[second])),
                "fixture",
                Method::ScanSources,
                |_, _| Ok(())
            )
            .unwrap_err(),
            "invalid_core_response"
        );
    }

    #[test]
    fn install_progress_is_parsed_flat_and_sequence_deduplicated() {
        let progress = Arc::new(Mutex::new(Progress::default()));
        let input = task_stream(&[
            install_progress_event("req-1", 0, "downloading"),
            // Out-of-order stale event: must be dropped.
            install_progress_event("req-1", 1, "validating"),
            install_progress_event("req-1", 1, "applying"),
            // Newest event wins.
            install_progress_event("req-1", 2, "cleanup"),
            terminal_response("req-1"),
            // Events after the terminal line are ignored.
            install_progress_event("req-1", 4, "rolling_back"),
        ]);
        let result = read_task_output(
            std::io::Cursor::new(input),
            "req-1".to_string(),
            progress.clone(),
        )
        .unwrap();
        assert_eq!(result["state"], "committed");
        let mirror = progress.lock().unwrap().clone().unwrap();
        assert_eq!(mirror.stage, protocol::InstallStage::Cleanup);
        assert_eq!(mirror.bytes_done, 20);
        assert_eq!(mirror.files_done, 2);
    }

    #[test]
    fn install_progress_rejects_malformed_sequence() {
        let progress = Arc::new(Mutex::new(Progress::default()));
        let mut event = install_progress_event("req-1", 0, "downloading");
        event.as_object_mut().unwrap().remove("sequence");
        let input = task_stream(&[event, terminal_response("req-1")]);
        assert_eq!(
            read_task_output(
                std::io::Cursor::new(input),
                "req-1".to_string(),
                progress.clone(),
            )
            .unwrap_err(),
            "invalid_core_response"
        );
    }

    #[test]
    fn install_progress_captures_substage_and_unknown_totals() {
        let progress = Arc::new(Mutex::new(Progress::default()));
        let mut applying = install_progress_event("req-1", 0, "applying");
        applying["substage"] = json!("writing");
        let input = task_stream(&[applying, terminal_response("req-1")]);
        read_task_output(
            std::io::Cursor::new(input),
            "req-1".to_string(),
            progress.clone(),
        )
        .unwrap();
        let mirror = progress.lock().unwrap().clone().unwrap();
        assert_eq!(mirror.stage, protocol::InstallStage::Applying);
        assert_eq!(mirror.substage, Some(protocol::InstallSubstage::Writing));
        assert!(mirror.total_bytes.is_none());
        assert!(mirror.total_files.is_none());
    }
    #[test]
    fn mixed_version_and_mismatched_id_never_change_progress() {
        for (mut event, expected) in [
            (
                install_progress_event("req-1", 0, "applying"),
                "unsupported_protocol",
            ),
            (
                install_progress_event("other", 0, "applying"),
                "invalid_core_response",
            ),
        ] {
            if expected == "unsupported_protocol" {
                event["protocol_version"] = json!(3);
            }
            let progress = Arc::new(Mutex::new(Progress::default()));
            assert_eq!(
                read_task_output(
                    std::io::Cursor::new(task_stream(&[event])),
                    "req-1".into(),
                    progress.clone()
                )
                .unwrap_err(),
                expected
            );
            assert!(progress.lock().unwrap().is_none());
        }
    }

    #[test]
    fn first_event_must_start_at_zero_and_transport_ends_with_lf() {
        let progress = Arc::new(Mutex::new(Progress::default()));
        let event = install_progress_event("req-1", 1, "applying");
        assert_eq!(
            read_task_output(
                std::io::Cursor::new(task_stream(&[event])),
                "req-1".into(),
                progress.clone()
            )
            .unwrap_err(),
            "invalid_core_response"
        );
        let mut input = task_stream(&[terminal_response("req-1")]);
        input.pop();
        assert_eq!(
            read_task_output(std::io::Cursor::new(input), "req-1".into(), progress).unwrap_err(),
            "invalid_core_response"
        );
    }

    #[test]
    fn line_limit_includes_lf_and_rejects_one_extra_byte() {
        let exact = vec![b' '; MAX_RESPONSE as usize - 1];
        let mut input = exact.clone();
        input.push(b'\n');
        let (line, oversized) = read_bounded_line(&mut std::io::Cursor::new(input))
            .unwrap()
            .unwrap();
        assert!(!oversized);
        assert_eq!(line.len(), MAX_RESPONSE as usize);
        let mut oversized_line = exact;
        oversized_line.extend_from_slice(b" \n");
        assert!(
            read_bounded_line(&mut std::io::Cursor::new(oversized_line))
                .unwrap()
                .unwrap()
                .1
        );
    }

    #[test]
    fn host_builder_rejects_method_extraneous_fields() {
        assert_eq!(
            build_params(
                "scan",
                CallArguments {
                    force: Some(true),
                    ..Default::default()
                },
                common()
            )
            .unwrap_err(),
            "invalid_request"
        );
        let mut old_host = common();
        old_host["state_dir"] = json!("C:/other");
        assert_eq!(
            build_params("scan", CallArguments::default(), old_host).unwrap_err(),
            "invalid_request"
        );
    }

    #[test]
    fn validated_commit_survives_process_teardown_error() {
        let progress = Arc::new(Mutex::new(Progress::default()));
        let validated = read_task_output(
            std::io::Cursor::new(task_stream(&[terminal_response("req-1")])),
            "req-1".into(),
            progress,
        )
        .unwrap();
        let result =
            task_terminal_result(Ok(validated), Err("core_process_failed".into())).unwrap();
        assert_eq!(result["state"], "committed");
        assert_eq!(result["disposition"], "completed");
        assert_eq!(
            task_terminal_result(
                Err("invalid_core_response".into()),
                Err("core_process_failed".into())
            )
            .unwrap_err(),
            "invalid_core_response"
        );
    }

    #[test]
    fn long_instance_identity_has_a_dedicated_bound() {
        let instance = format!("10@C:/{}", "SteamLibrary/".repeat(40));
        assert!(instance.len() > protocol::MAX_ID_BYTES);
        let params = build_params(
            "list_game_sources",
            CallArguments {
                instance_id: Some(instance.clone()),
                ..Default::default()
            },
            common(),
        )
        .unwrap();
        assert_eq!(params["instance_id"], instance);
        build_install_request(
            &instance,
            Some("release-1"),
            None,
            "overwrite",
            "long-instance",
            None,
            common(),
        )
        .unwrap();
        assert_eq!(
            build_install_request(
                &instance,
                Some(&"r".repeat(protocol::MAX_ID_BYTES + 1)),
                None,
                "overwrite",
                "long-instance",
                None,
                common()
            )
            .unwrap_err(),
            "invalid_id"
        );
    }

    #[test]
    fn malformed_late_telemetry_and_duplicate_terminal_do_not_replace_receipt() {
        let first = terminal_response("req-1");
        let malformed_late = json!({"event":"install_progress"});
        let mut old_late = install_progress_event("other", 8, "cleanup");
        old_late["protocol_version"] = json!(3);
        let mut duplicate = first.clone();
        duplicate["result"]["state"] = json!("needs_recovery");
        let mut input =
            std::io::Cursor::new(task_stream(&[first, malformed_late, old_late, duplicate]));
        let result = read_core_stream(&mut input, "req-1", Method::InstallPatch, |_, _| {
            panic!("late telemetry cannot apply")
        })
        .unwrap();
        assert_eq!(
            drain_terminal_tail(&mut input),
            1,
            "duplicate terminal is rejected independently"
        );
        assert_eq!(result["state"], "committed");
        assert_eq!(result["disposition"], "completed");
    }

    #[cfg(windows)]
    #[test]
    fn rejected_install_stream_cancels_closes_stdin_and_allows_safe_core_terminal() {
        let cases = shared_fixtures();
        for name in ["install_patch-success", "outcome-rolled_back-cancelled"] {
            let mut receipt = cases["responses"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["name"] == name)
                .unwrap()["response"]
                .clone();
            receipt["request_id"] = json!("req-1");
            let marker = std::env::temp_dir().join(format!(
                "steam-patch-host-control-{}-{}.json",
                std::process::id(),
                request_id()
            ));
            let literal = |text: String| format!("'{}'", text.replace('\'', "''"));
            let script = format!("[Console]::WriteLine({}); [Console]::WriteLine('{}'); $control=[Console]::ReadLine(); if($control -ne 'cancel') {{ exit 3 }}; $eof=[Console]::ReadLine(); if($null -ne $eof) {{ exit 4 }}; [System.IO.File]::WriteAllText({},{}); [Console]::WriteLine({}); exit 0",
                literal(install_progress_event("req-1",0,"applying").to_string()),
                r#"{"protocol_version":4,"request_id":"req-1","ok":true,"result":{}}"#,
                literal(marker.display().to_string()),literal(receipt.to_string()),literal(receipt.to_string()));
            let mut child = core_command(Path::new("powershell.exe"))
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &script,
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let stdin = Arc::new(Mutex::new(ClosableWriter(Some(
                child.stdin.take().unwrap(),
            ))));
            let stdout = child.stdout.take().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let cancelled = Arc::new(AtomicBool::new(false));
            let progress = Arc::new(Mutex::new(Progress::default()));
            let error = read_install_task_output(
                stdout,
                "req-1",
                progress.clone(),
                |_| cancel_and_close(&stdin, &stop, &cancelled),
                |_| panic!("invalid envelope must retain transport error"),
            )
            .unwrap_err();
            assert_eq!(error, "invalid_core_response");
            assert!(stop.load(Ordering::SeqCst));
            assert!(stdin.lock().unwrap().0.is_none());
            assert_eq!(
                progress.lock().unwrap().as_ref().unwrap().stage,
                protocol::InstallStage::Applying
            );
            assert!(
                child.wait().unwrap().success(),
                "mock writer must be allowed to finish after cancel/EOF"
            );
            let stored: Value =
                serde_json::from_str(&std::fs::read_to_string(&marker).unwrap()).unwrap();
            assert_eq!(
                stored, receipt,
                "host must not rewrite the core's true receipt disposition"
            );
            std::fs::remove_file(marker).unwrap();
        }
    }

    #[cfg(windows)]
    #[test]
    fn bounded_shutdown_wait_does_not_terminate_a_writer() {
        let marker = std::env::temp_dir().join(format!(
            "steam-patch-host-timeout-{}-{}.txt",
            std::process::id(),
            request_id()
        ));
        let script = format!("Start-Sleep -Milliseconds 150; [System.IO.File]::WriteAllText('{}','safe_terminal'); exit 0",marker.display().to_string().replace('\'',"''"));
        let child = core_command(Path::new("powershell.exe"))
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &script,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let child = Arc::new(Mutex::new(child));
        assert_eq!(
            wait_core_until(&child, std::time::Instant::now()).unwrap_err(),
            "core_process_shutdown_pending"
        );
        assert!(child.lock().unwrap().try_wait().unwrap().is_none());
        assert!(wait_core_child(&child).unwrap().success());
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "safe_terminal");
        std::fs::remove_file(marker).unwrap();
    }

    #[test]
    fn response_limit_accepts_exact_boundary_and_rejects_the_next_byte() {
        assert_eq!(
            read_all_bounded(std::io::Cursor::new(vec![b'x'; MAX_RESPONSE as usize]))
                .unwrap()
                .len(),
            MAX_RESPONSE as usize
        );
        assert_eq!(
            read_all_bounded(std::io::Cursor::new(vec![b'x'; MAX_RESPONSE as usize + 1]))
                .unwrap_err(),
            "response_too_large"
        );
    }
}
