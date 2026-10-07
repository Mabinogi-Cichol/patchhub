//! `probe-worker`: the process that runs inside the LPAC + Job sandbox. It reads one
//! JSON request from stdin, performs a read-only/filesystem/network/registry/process
//! operation, and writes one JSON response line to stdout.
//!
//! A special argv mode `child-sleep <ms>` is used by the runner to exercise process-tree
//! reclamation: it just sleeps and is expected to live in the parent worker's job.

fn main() {
    #[cfg(windows)]
    win_impl::main();

    #[cfg(target_os = "linux")]
    linux_impl::main();

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        eprintln!("probe-worker: sandbox probe is only supported on Windows/Linux");
        std::process::exit(2);
    }
}

#[cfg(windows)]
mod win_impl {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{Duration, Instant};

    use serde_json::{json, Map, Value};
    use steam_patch_installer_worker::model::{WorkerRequest, WorkerResponse};
    use steam_patch_installer_worker::win::probe_os;

    const CANARY_ENV: &str = "STEAM_PATCH_PROBE_CANARY";

    pub fn main() {
        let mut args = std::env::args().skip(1);
        if let Some(mode) = args.next() {
            if mode == "child-sleep" {
                let ms: u64 = args.next().and_then(|v| v.parse().ok()).unwrap_or(3000);
                std::thread::sleep(Duration::from_millis(ms));
                std::process::exit(0);
            }
            eprintln!("probe-worker: unknown argv mode {mode}");
            std::process::exit(2);
        }

        let mut input = String::new();
        if std::io::stdin().read_to_string(&mut input).is_err() {
            fail("input", None, "failed to read stdin");
        }
        let request: WorkerRequest = match serde_json::from_str(input.trim()) {
            Ok(req) => req,
            Err(err) => fail("input", None, &format!("invalid request JSON: {err}")),
        };

        // Process-tree scenarios must report the child PID before the runner terminates
        // the job, so this one writes its response immediately and then stays alive.
        if request.scenario == "spawn_child" {
            spawn_child_and_report(&request);
        }

        let response = dispatch(&request);
        emit(&response);
    }

    fn emit(response: &WorkerResponse) {
        let line = serde_json::to_string(response).unwrap_or_else(|_| "{}".to_string());
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(line.as_bytes());
        let _ = stdout.write_all(b"\n");
        let _ = stdout.flush();
    }

    fn fail(scenario: &str, code: Option<u32>, message: &str) -> ! {
        emit(&WorkerResponse::fail(scenario, code, message));
        std::process::exit(0);
    }

    fn spawn_child_and_report(request: &WorkerRequest) -> ! {
        let exe = match &request.child_exe {
            Some(e) => e.clone(),
            None => fail("spawn_child", None, "missing child_exe"),
        };
        let ms = request.child_ms.unwrap_or(3000);
        match Command::new(&exe)
            .arg("child-sleep")
            .arg(ms.to_string())
            .spawn()
        {
            Ok(child) => {
                let pid = child.id();
                emit(
                    &WorkerResponse::new("spawn_child", true).with_data(json!({"child_pid": pid})),
                );
                // Keep the worker (and child) alive so the runner can observe the job
                // and reclaim the whole tree.
                std::thread::sleep(Duration::from_millis(8000));
                std::process::exit(0);
            }
            Err(err) => fail(
                "spawn_child",
                err.raw_os_error().map(|c| c as u32),
                &format!("spawn '{exe}' failed: {err}"),
            ),
        }
    }

    fn dispatch(request: &WorkerRequest) -> WorkerResponse {
        match request.scenario.as_str() {
            "selfcheck" => scenario_selfcheck(),
            "read_file" => scenario_read_file(request),
            "write_file" => scenario_write_file(request),
            "path_rw" => scenario_path_rw(request),
            "net_connect" => scenario_net(request),
            "registry_read" => scenario_registry_read(request),
            "registry_write" => scenario_registry_write(request),
            "handle_check" => scenario_handle_check(request),
            "alloc" => scenario_alloc(request),
            "busy" => scenario_busy(request),
            "sleep" => scenario_sleep(request),
            other => WorkerResponse::fail(other, None, format!("unknown scenario '{other}'")),
        }
    }

    fn scenario_selfcheck() -> WorkerResponse {
        let token = probe_os::token_info();
        let console = probe_os::console_info();
        let in_job = probe_os::self_in_job();

        let mut env = Map::new();
        let canary_present = std::env::var_os(CANARY_ENV).is_some();
        for (k, v) in std::env::vars() {
            if k == CANARY_ENV {
                env.insert(k, Value::String("<redacted>".to_string()));
            } else {
                env.insert(k, Value::String(v));
            }
        }
        let env_count = env.len();

        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let exe = std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let data = json!({
            "token": token,
            "console": console,
            "in_job": in_job,
            "cwd": cwd,
            "exe": exe,
            "env": Value::Object(env),
            "env_count": env_count,
            "canary_present": canary_present,
            "pid": std::process::id(),
        });
        WorkerResponse::new("selfcheck", true).with_data(data)
    }

    fn scenario_handle_check(request: &WorkerRequest) -> WorkerResponse {
        let raw = match request
            .value
            .as_deref()
            .and_then(|v| v.trim().parse::<usize>().ok())
        {
            Some(v) => v,
            None => {
                return WorkerResponse::fail("handle_check", None, "missing/invalid handle value")
            }
        };
        let present = probe_os::try_duplicate(raw);
        WorkerResponse::new("handle_check", present)
            .with_data(json!({"raw": raw, "present": present}))
    }

    fn scenario_read_file(request: &WorkerRequest) -> WorkerResponse {
        let path = match &request.path {
            Some(p) => p,
            None => return WorkerResponse::fail("read_file", None, "missing path"),
        };
        match std::fs::read_to_string(path) {
            Ok(content) => WorkerResponse::new("read_file", true).with_data(json!({
                "path": path,
                "content": content,
                "len": content.len(),
            })),
            Err(err) => WorkerResponse::fail(
                "read_file",
                err.raw_os_error().map(|c| c as u32),
                format!("read '{path}' failed: {err}"),
            ),
        }
    }

    fn scenario_write_file(request: &WorkerRequest) -> WorkerResponse {
        let path = match &request.path {
            Some(p) => p,
            None => return WorkerResponse::fail("write_file", None, "missing path"),
        };
        let text = request.text.clone().unwrap_or_default();
        match std::fs::write(path, text.as_bytes()) {
            Ok(()) => WorkerResponse::new("write_file", true)
                .with_data(json!({"path": path, "bytes": text.len()})),
            Err(err) => WorkerResponse::fail(
                "write_file",
                err.raw_os_error().map(|c| c as u32),
                format!("write '{path}' failed: {err}"),
            ),
        }
    }

    fn scenario_path_rw(request: &WorkerRequest) -> WorkerResponse {
        let base = match &request.path {
            Some(p) => PathBuf::from(p),
            None => return WorkerResponse::fail("path_rw", None, "missing path"),
        };
        let text = request
            .text
            .clone()
            .unwrap_or_else(|| "中文内容".to_string());
        // Unicode directory + file, plus a nested long path beyond MAX_PATH.
        let unicode_dir = base.join("中文目录_日本語_😀");
        let mut long = unicode_dir.clone();
        for i in 0..12 {
            long = long.join(format!("segment_{i:02}_abcdefghij"));
        }
        let file = long.join("文件_中文.txt");

        let mut steps = Vec::new();
        if let Err(err) = std::fs::create_dir_all(&long) {
            return WorkerResponse::fail(
                "path_rw",
                err.raw_os_error().map(|c| c as u32),
                format!("create_dir_all('{}') failed: {err}", long.display()),
            );
        }
        steps.push(format!(
            "created dir (len={})",
            long.to_string_lossy().chars().count()
        ));

        if let Err(err) = std::fs::write(&file, text.as_bytes()) {
            return WorkerResponse::fail(
                "path_rw",
                err.raw_os_error().map(|c| c as u32),
                format!("write('{}') failed: {err}", file.display()),
            );
        }
        let read_back = match std::fs::read_to_string(&file) {
            Ok(v) => v,
            Err(err) => {
                return WorkerResponse::fail(
                    "path_rw",
                    err.raw_os_error().map(|c| c as u32),
                    format!("read('{}') failed: {err}", file.display()),
                )
            }
        };
        let ok = read_back == text;
        WorkerResponse::new("path_rw", ok).with_data(json!({
            "unicode_dir": unicode_dir.to_string_lossy(),
            "long_path_len": long.to_string_lossy().chars().count(),
            "file": file.to_string_lossy(),
            "roundtrip_ok": ok,
            "steps": steps,
        }))
    }

    fn scenario_net(request: &WorkerRequest) -> WorkerResponse {
        // Probe Winsock explicitly: under LPAC the stack may be unavailable, and Rust
        // std's lazy WSAStartup would otherwise assert and panic before we can report.
        let mut wsa = windows::Win32::Networking::WinSock::WSADATA::default();
        let rc = unsafe { windows::Win32::Networking::WinSock::WSAStartup(0x0202, &mut wsa) };
        if rc != 0 {
            return WorkerResponse::fail(
                "net_connect",
                Some(rc as u32),
                "WSAStartup failed: Winsock unavailable under LPAC",
            );
        }

        let host = request
            .host
            .clone()
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let port = request.port.unwrap_or(80);
        let addr: SocketAddr = match format!("{host}:{port}").parse() {
            Ok(a) => a,
            Err(err) => {
                return WorkerResponse::fail("net_connect", None, format!("bad addr: {err}"))
            }
        };
        // Plain connect: `connect_timeout` panics inside std's winsock layer when the
        // AppContainer blocks socket option setup (observed WSA error 10107).
        match TcpStream::connect(addr) {
            Ok(stream) => {
                let local = stream.local_addr().ok().map(|a| a.to_string());
                WorkerResponse::new("net_connect", true).with_data(json!({
                    "target": addr.to_string(),
                    "connected": true,
                    "local": local,
                }))
            }
            Err(err) => WorkerResponse::fail(
                "net_connect",
                err.raw_os_error().map(|c| c as u32),
                format!("connect {addr} failed: {err}"),
            ),
        }
    }

    fn scenario_registry_read(request: &WorkerRequest) -> WorkerResponse {
        let key = match &request.key {
            Some(k) => k.clone(),
            None => return WorkerResponse::fail("registry_read", None, "missing key"),
        };
        match split_hive(&key) {
            Some((hive, sub)) => match hive.open_subkey(&sub) {
                Ok(k) => {
                    let value = k.get_value::<String, _>("").ok();
                    WorkerResponse::new("registry_read", true).with_data(json!({
                        "key": key,
                        "default_value": value,
                    }))
                }
                Err(err) => WorkerResponse::fail(
                    "registry_read",
                    err.raw_os_error().map(|c| c as u32),
                    format!("open '{key}' failed: {err}"),
                ),
            },
            None => WorkerResponse::fail("registry_read", None, format!("unsupported hive: {key}")),
        }
    }

    fn scenario_registry_write(request: &WorkerRequest) -> WorkerResponse {
        let key = match &request.key {
            Some(k) => k.clone(),
            None => return WorkerResponse::fail("registry_write", None, "missing key"),
        };
        let value = request.value.clone().unwrap_or_else(|| "p0".to_string());
        match split_hive(&key) {
            Some((hive, sub)) => match hive.create_subkey(&sub) {
                Ok((k, _)) => match k.set_value("probe", &value) {
                    Ok(()) => WorkerResponse::new("registry_write", true)
                        .with_data(json!({"key": key, "value": value})),
                    Err(err) => WorkerResponse::fail(
                        "registry_write",
                        err.raw_os_error().map(|c| c as u32),
                        format!("set_value '{key}' failed: {err}"),
                    ),
                },
                Err(err) => WorkerResponse::fail(
                    "registry_write",
                    err.raw_os_error().map(|c| c as u32),
                    format!("create '{key}' failed: {err}"),
                ),
            },
            None => {
                WorkerResponse::fail("registry_write", None, format!("unsupported hive: {key}"))
            }
        }
    }

    fn scenario_alloc(request: &WorkerRequest) -> WorkerResponse {
        let mb = request.mb.unwrap_or(256);
        match probe_os::try_commit(mb) {
            Ok(()) => WorkerResponse::new("alloc", true)
                .with_data(json!({"requested_mb": mb, "committed": true})),
            Err(code) => {
                WorkerResponse::fail("alloc", Some(code), format!("commit {mb} MiB failed"))
            }
        }
    }

    fn scenario_busy(request: &WorkerRequest) -> WorkerResponse {
        let ms = request.ms.unwrap_or(1000);
        let start = Instant::now();
        let mut counter: u64 = 0;
        while start.elapsed() < Duration::from_millis(ms) {
            // Black-box-ish churn to consume user-mode CPU.
            counter = counter.wrapping_mul(1664525).wrapping_add(1013904223);
            std::hint::black_box(counter);
        }
        WorkerResponse::new("busy", true).with_data(json!({
            "requested_ms": ms,
            "wall_ms": start.elapsed().as_millis() as u64,
            "iterations": counter,
        }))
    }

    fn scenario_sleep(request: &WorkerRequest) -> WorkerResponse {
        let ms = request.ms.unwrap_or(1000);
        std::thread::sleep(Duration::from_millis(ms));
        WorkerResponse::new("sleep", true).with_data(json!({"requested_ms": ms}))
    }

    fn split_hive(key: &str) -> Option<(winreg::RegKey, String)> {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        use winreg::RegKey;
        if let Some(sub) = key.strip_prefix("HKLM\\") {
            Some((RegKey::predef(HKEY_LOCAL_MACHINE), sub.to_string()))
        } else {
            key.strip_prefix("HKCU\\")
                .map(|sub| (RegKey::predef(HKEY_CURRENT_USER), sub.to_string()))
        }
    }

    #[allow(dead_code)]
    fn path_exists(p: &Path) -> bool {
        p.exists()
    }
}

#[cfg(target_os = "linux")]
mod linux_impl {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::os::unix::io::FromRawFd;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{Duration, Instant};

    use serde_json::{json, Map, Value};
    use steam_patch_installer_worker::linux::probe_os;
    use steam_patch_installer_worker::model::{WorkerRequest, WorkerResponse};

    const CANARY_ENV: &str = "STEAM_PATCH_PROBE_CANARY";

    pub fn main() {
        let mut args = std::env::args().skip(1);
        if let Some(mode) = args.next() {
            if mode == "child-sleep" {
                let ms: u64 = args.next().and_then(|v| v.parse().ok()).unwrap_or(3000);
                std::thread::sleep(Duration::from_millis(ms));
                std::process::exit(0);
            }
            eprintln!("probe-worker: unknown argv mode {mode}");
            std::process::exit(2);
        }

        let mut input = String::new();
        if std::io::stdin().read_to_string(&mut input).is_err() {
            fail("input", None, "failed to read stdin");
        }
        let request: WorkerRequest = match serde_json::from_str(input.trim()) {
            Ok(req) => req,
            Err(err) => fail("input", None, &format!("invalid request JSON: {err}")),
        };

        // `spawn_child` reports the child PID and then stays alive so the runner
        // can terminate the sandbox and check process-tree reclamation.
        if request.scenario == "spawn_child" {
            spawn_child_and_report(&request);
        }

        let response = dispatch(&request);
        emit(&response);
    }

    fn emit(response: &WorkerResponse) {
        let line = serde_json::to_string(response).unwrap_or_else(|_| "{}".to_string());
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(line.as_bytes());
        let _ = stdout.write_all(b"\n");
        let _ = stdout.flush();
    }

    fn fail(scenario: &str, code: Option<u32>, message: &str) -> ! {
        emit(&WorkerResponse::fail(scenario, code, message));
        std::process::exit(0);
    }

    fn spawn_child_and_report(request: &WorkerRequest) -> ! {
        let exe = match &request.child_exe {
            Some(e) => e.clone(),
            None => fail("spawn_child", None, "missing child_exe"),
        };
        let ms = request.child_ms.unwrap_or(3000);
        match Command::new(&exe)
            .arg("child-sleep")
            .arg(ms.to_string())
            .spawn()
        {
            Ok(child) => {
                emit(
                    &WorkerResponse::new("spawn_child", true)
                        .with_data(json!({"child_pid": child.id()})),
                );
                std::thread::sleep(Duration::from_millis(8000));
                std::process::exit(0);
            }
            Err(err) => fail(
                "spawn_child",
                err.raw_os_error().map(|c| c as u32),
                &format!("spawn '{exe}' failed: {err}"),
            ),
        }
    }

    fn dispatch(request: &WorkerRequest) -> WorkerResponse {
        match request.scenario.as_str() {
            "selfcheck" => scenario_selfcheck(),
            "read_file" => scenario_read_file(request),
            "write_file" => scenario_write_file(request),
            "path_rw" => scenario_path_rw(request),
            "net_connect" => scenario_net(request),
            "alloc" => scenario_alloc(request),
            "busy" => scenario_busy(request),
            "sleep" => scenario_sleep(request),
            "rlimit" => scenario_rlimit(),
            "big_output" => scenario_big_stream(request, false),
            "big_stderr" => scenario_big_stream(request, true),
            other => WorkerResponse::fail(other, None, format!("unsupported scenario '{other}'")),
        }
    }

    fn scenario_selfcheck() -> WorkerResponse {
        let mut env = Map::new();
        let canary_present = std::env::var_os(CANARY_ENV).is_some();
        for (k, v) in std::env::vars() {
            if k == CANARY_ENV {
                env.insert(k, Value::String("<redacted>".to_string()));
            } else {
                env.insert(k, Value::String(v));
            }
        }
        let env_count = env.len();

        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let exe = std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        WorkerResponse::new("selfcheck", true).with_data(json!({
            "uid": probe_os::euid(),
            "cwd": cwd,
            "exe": exe,
            "env": Value::Object(env),
            "env_count": env_count,
            "canary_present": canary_present,
            "pid": std::process::id(),
        }))
    }

    fn scenario_rlimit() -> WorkerResponse {
        WorkerResponse::new("rlimit", true).with_data(json!({
            "limits": probe_os::rlimits(),
            "uid": probe_os::euid(),
        }))
    }

    /// Writes `mb` MiB to stdout/stderr to exercise the launcher's fail-closed
    /// output limit. Raw file descriptors bypass libstd buffering.
    fn scenario_big_stream(request: &WorkerRequest, stderr: bool) -> WorkerResponse {
        let mb = request.mb.unwrap_or(17);
        let size = (mb as usize).saturating_mul(1024 * 1024);
        let fd = if stderr { 2 } else { 1 };
        let chunk = vec![b'x'; 64 * 1024];
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        let mut written = 0usize;
        while written < size {
            if file.write_all(&chunk).is_err() {
                break;
            }
            written += chunk.len();
        }
        let _ = file.flush();
        std::mem::forget(file);
        WorkerResponse::new(if stderr { "big_stderr" } else { "big_output" }, true)
            .with_data(json!({"requested_mb": mb, "written": written}))
    }

    fn scenario_read_file(request: &WorkerRequest) -> WorkerResponse {
        let path = match &request.path {
            Some(p) => p,
            None => return WorkerResponse::fail("read_file", None, "missing path"),
        };
        match std::fs::read_to_string(path) {
            Ok(content) => WorkerResponse::new("read_file", true).with_data(json!({
                "path": path,
                "content": content,
                "len": content.len(),
            })),
            Err(err) => WorkerResponse::fail(
                "read_file",
                err.raw_os_error().map(|c| c as u32),
                format!("read '{path}' failed: {err}"),
            ),
        }
    }

    fn scenario_write_file(request: &WorkerRequest) -> WorkerResponse {
        let path = match &request.path {
            Some(p) => p,
            None => return WorkerResponse::fail("write_file", None, "missing path"),
        };
        let text = request.text.clone().unwrap_or_default();
        match std::fs::write(path, text.as_bytes()) {
            Ok(()) => WorkerResponse::new("write_file", true)
                .with_data(json!({"path": path, "bytes": text.len()})),
            Err(err) => WorkerResponse::fail(
                "write_file",
                err.raw_os_error().map(|c| c as u32),
                format!("write '{path}' failed: {err}"),
            ),
        }
    }

    fn scenario_path_rw(request: &WorkerRequest) -> WorkerResponse {
        let base = match &request.path {
            Some(p) => PathBuf::from(p),
            None => return WorkerResponse::fail("path_rw", None, "missing path"),
        };
        let text = request
            .text
            .clone()
            .unwrap_or_else(|| "中文内容".to_string());
        let unicode_dir = base.join("中文目录_日本語_😀");
        let mut long = unicode_dir.clone();
        for i in 0..12 {
            long = long.join(format!("segment_{i:02}_abcdefghij"));
        }
        let file = long.join("文件_中文.txt");

        if let Err(err) = std::fs::create_dir_all(&long) {
            return WorkerResponse::fail(
                "path_rw",
                err.raw_os_error().map(|c| c as u32),
                format!("create_dir_all('{}') failed: {err}", long.display()),
            );
        }
        if let Err(err) = std::fs::write(&file, text.as_bytes()) {
            return WorkerResponse::fail(
                "path_rw",
                err.raw_os_error().map(|c| c as u32),
                format!("write('{}') failed: {err}", file.display()),
            );
        }
        let read_back = match std::fs::read_to_string(&file) {
            Ok(v) => v,
            Err(err) => {
                return WorkerResponse::fail(
                    "path_rw",
                    err.raw_os_error().map(|c| c as u32),
                    format!("read('{}') failed: {err}", file.display()),
                )
            }
        };
        let ok = read_back == text;
        WorkerResponse::new("path_rw", ok).with_data(json!({
            "unicode_dir": unicode_dir.to_string_lossy(),
            "long_path_len": long.to_string_lossy().chars().count(),
            "file": file.to_string_lossy(),
            "roundtrip_ok": ok,
        }))
    }

    fn scenario_net(request: &WorkerRequest) -> WorkerResponse {
        let host = request
            .host
            .clone()
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let port = request.port.unwrap_or(80);
        let addr: SocketAddr = match format!("{host}:{port}").parse() {
            Ok(a) => a,
            Err(err) => {
                return WorkerResponse::fail("net_connect", None, format!("bad addr: {err}"))
            }
        };
        match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
            Ok(stream) => WorkerResponse::new("net_connect", true).with_data(json!({
                "target": addr.to_string(),
                "connected": true,
                "local": stream.local_addr().ok().map(|a| a.to_string()),
            })),
            Err(err) => WorkerResponse::fail(
                "net_connect",
                err.raw_os_error().map(|c| c as u32),
                format!("connect {addr} failed: {err}"),
            ),
        }
    }

    fn scenario_alloc(request: &WorkerRequest) -> WorkerResponse {
        let mb = request.mb.unwrap_or(256);
        let size = (mb as usize).saturating_mul(1024 * 1024);
        let mut buffer: Vec<u8> = Vec::new();
        match buffer.try_reserve_exact(size) {
            Ok(()) => {
                buffer.resize(size, 0);
                WorkerResponse::new("alloc", true)
                    .with_data(json!({"requested_mb": mb, "allocated": true}))
            }
            Err(err) => WorkerResponse::fail(
                "alloc",
                None,
                format!("allocate {mb} MiB failed under RLIMIT_AS: {err}"),
            ),
        }
    }

    fn scenario_busy(request: &WorkerRequest) -> WorkerResponse {
        let ms = request.ms.unwrap_or(1000);
        let start = Instant::now();
        let mut counter: u64 = 0;
        while start.elapsed() < Duration::from_millis(ms) {
            counter = counter.wrapping_mul(1664525).wrapping_add(1013904223);
            std::hint::black_box(counter);
        }
        WorkerResponse::new("busy", true).with_data(json!({
            "requested_ms": ms,
            "wall_ms": start.elapsed().as_millis() as u64,
        }))
    }

    fn scenario_sleep(request: &WorkerRequest) -> WorkerResponse {
        let ms = request.ms.unwrap_or(1000);
        std::thread::sleep(Duration::from_millis(ms));
        WorkerResponse::new("sleep", true).with_data(json!({"requested_ms": ms}))
    }
}
