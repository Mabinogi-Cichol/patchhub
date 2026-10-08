//! Offline development acceptance for production launch controls; no game/Steam paths.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use steam_patch_installer_worker::{production::WorkerRuntime, SandboxConfig, TerminationReason};
fn layout() -> (PathBuf, PathBuf) {
    #[cfg(windows)]
    {
        (PathBuf::from("input/archive.exe"), PathBuf::from("output"))
    }
    #[cfg(not(windows))]
    {
        (
            PathBuf::from("/input/archive.exe"),
            PathBuf::from("/output"),
        )
    }
}
fn worker() -> Result<(), Box<dyn std::error::Error>> {
    let mut raw = Vec::new();
    std::io::stdin().read_to_end(&mut raw)?;
    let desc: Value = serde_json::from_slice(&raw)?;
    let (input, output) = layout();
    let case: Value = serde_json::from_slice(&fs::read(input)?)?;
    let scenario = case["scenario"].as_str().unwrap_or("");
    let report = match scenario {
        "sleep" => {
            fs::write(output.join("worker.pid"), std::process::id().to_string())?;
            std::thread::sleep(Duration::from_secs(20));
            json!({"slept":true})
        }
        "stdout" => {
            std::io::stdout().write_all(&vec![b'x'; 17 * 1024 * 1024])?;
            return Ok(());
        }
        "stderr" => {
            std::io::stderr().write_all(&vec![b'x'; 128 * 1024])?;
            return Ok(());
        }
        "crash" => std::process::exit(42),
        "isolation" => {
            let canary = Path::new(case["canary"].as_str().ok_or("missing canary")?);
            let input = layout().0;
            let child = std::process::Command::new(std::env::current_exe()?)
                .arg("--child")
                .status();
            let address = format!("127.0.0.1:{}", case["port"]).parse()?;
            let network = std::panic::catch_unwind(|| {
                std::net::TcpStream::connect_timeout(&address, Duration::from_millis(500))
            });
            #[cfg(windows)]
            let identity = {
                let t = steam_patch_installer_worker::win::token_info();
                t.is_app_container && t.capability_count == Some(0)
            };
            #[cfg(target_os = "linux")]
            let identity = steam_patch_installer_worker::linux::euid() != 0;
            #[cfg(not(any(windows, target_os = "linux")))]
            let identity = false;
            #[cfg(target_os = "linux")]
            let fd_leak_denied = {
                let fds: Vec<_> = fs::read_dir("/proc/self/fd")?
                    .filter_map(Result::ok)
                    .filter_map(|e| {
                        Some((
                            e.file_name().to_string_lossy().parse::<u32>().ok()?,
                            fs::read_link(e.path()).ok()?,
                        ))
                    })
                    .collect();
                let ok = fds
                    .iter()
                    .all(|(fd, p)| *fd <= 2 || p.file_name().is_some_and(|n| n == "fd"));
                if !ok {
                    eprintln!("unexpected inherited descriptors: {fds:?}");
                }
                ok
            };
            let mut report = json!({"identity":identity,"canary_denied":fs::read(canary).is_err(),
                "input_write_denied":fs::OpenOptions::new().write(true).open(input).is_err(),
                "child_denied":child.is_err(),"network_denied":!matches!(network,Ok(Ok(_))),"env_clean":std::env::var_os("P3_SECRET_CANARY").is_none()});
            #[cfg(target_os = "linux")]
            {
                report["fd_leak_denied"] = json!(fd_leak_denied);
            }
            // Windows stdio handle inheritance is enforced by the fixed HANDLE_LIST.
            #[cfg(not(target_os = "linux"))]
            let _ = &mut report;
            report
        }
        _ => return Err("unknown scenario".into()),
    };
    println!(
        "{}",
        json!({"schema_version":1,"action":"analyze","exe_sha256":desc["exe_sha256"],"report":report,"installable":false})
    );
    Ok(())
}
fn identity() -> Option<(u32, u32)> {
    #[cfg(target_os = "linux")]
    {
        if steam_patch_installer_worker::linux::euid() == 0 {
            Some((1000, 1000))
        } else {
            None
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}
fn descriptor(archive: &Path) -> Vec<u8> {
    let bytes = fs::read(archive).unwrap();
    serde_json::to_vec(&json!({"action":"analyze","exe_size":bytes.len(),"exe_sha256":format!("{:x}",Sha256::digest(bytes))})).unwrap()
}
fn worker_pid(stage: &Path) -> Option<u32> {
    for entry in fs::read_dir(stage).ok()?.filter_map(Result::ok) {
        let path = entry.path();
        if fs::read_to_string(path.join("output/worker.pid")).is_err() {
            continue;
        }
        #[cfg(windows)]
        {
            return fs::read_to_string(path.join("output/worker.pid"))
                .ok()?
                .parse()
                .ok();
        }
        #[cfg(target_os = "linux")]
        {
            // The worker reports its PID namespace PID (usually 2). Resolve the
            // host PID by the private executable inode's path, never mistake
            // namespace PID 2 for host kthreadd.
            use std::os::unix::fs::MetadataExt;
            let expected = fs::metadata(path.join("runtime/sf-worker")).ok()?;
            for process in fs::read_dir("/proc").ok()?.filter_map(Result::ok) {
                let Ok(pid) = process.file_name().to_string_lossy().parse::<u32>() else {
                    continue;
                };
                if fs::metadata(process.path().join("exe"))
                    .is_ok_and(|m| m.dev() == expected.dev() && m.ino() == expected.ino())
                {
                    return Some(pid);
                }
            }
        }
    }
    None
}
fn alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        steam_patch_installer_worker::win::process_alive(pid)
    }
    #[cfg(target_os = "linux")]
    {
        steam_patch_installer_worker::linux::process_alive(pid)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = pid;
        false
    }
}
fn parent_exit(stage: &Path, archive: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let rt = WorkerRuntime::new(std::env::current_exe()?, stage, identity())?;
    let archive = archive.to_path_buf();
    let raw = descriptor(&archive);
    std::thread::spawn(move || {
        let _ = rt.execute(&archive, &raw, &AtomicBool::new(false));
    });
    let start = Instant::now();
    while worker_pid(stage).is_none() {
        if start.elapsed() > Duration::from_secs(8) {
            return Err("parent exit worker never started".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    fs::write(
        stage.join("host-worker.pid"),
        worker_pid(stage)
            .ok_or("missing host worker PID")?
            .to_string(),
    )?;
    std::process::exit(0);
}
fn verify() -> Result<(), Box<dyn std::error::Error>> {
    let repo = std::env::current_dir()?.canonicalize()?;
    let boundary = repo.join(".artifacts/p3-production");
    fs::create_dir_all(&boundary)?;
    let root = tempfile::Builder::new()
        .prefix("controls-")
        .tempdir_in(boundary)?;
    // Linux controls must use a real POSIX filesystem: WSL DrvFs may ignore chmod.
    #[cfg(target_os = "linux")]
    let stage_holder = tempfile::tempdir()?;
    #[cfg(target_os = "linux")]
    let stage = stage_holder.path().to_path_buf();
    #[cfg(not(target_os = "linux"))]
    let stage = root.path().join("stage");
    #[cfg(not(target_os = "linux"))]
    fs::create_dir(&stage)?;
    let exe = std::env::current_exe()?;
    let rt = WorkerRuntime::new(&exe, &stage, identity())?;
    let archive = root.path().join("archive.exe");
    let mut checks = Vec::new();
    // Deny creation at the trusted staging boundary; launcher must fail closed.
    fs::write(&archive, br#"{"scenario":"crash"}"#)?;
    #[cfg(windows)]
    set_stage_acl(&stage, true)?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o500))?;
    }
    #[cfg(windows)]
    let permission_check = true;
    #[cfg(target_os = "linux")]
    let permission_check = steam_patch_installer_worker::linux::euid() != 0;
    #[cfg(not(any(windows, target_os = "linux")))]
    let permission_check = false;
    if permission_check {
        let e = rt
            .execute(&archive, &descriptor(&archive), &AtomicBool::new(false))
            .unwrap_err();
        assert_eq!(e.stage, "stage");
        checks.push(json!({"check":"stage_permission_failure","pass":true,"error":e.to_string()}));
    }
    #[cfg(windows)]
    set_stage_acl(&stage, false)?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o700))?;
        // Only this synthetic acceptance stage is delegated; never adjust real
        // installer state/game ancestors to make privilege drop work.
        if let Some((uid, gid)) = identity() {
            std::os::unix::fs::chown(&stage, Some(uid), Some(gid))?;
        }
    }
    let canary = root.path().join("host-private-canary");
    fs::write(&canary, b"host secret")?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    std::env::set_var("P3_SECRET_CANARY", "host secret");
    fs::write(
        &archive,
        serde_json::to_vec(
            &json!({"scenario":"isolation","canary":canary,"port":listener.local_addr()?.port()}),
        )?,
    )?;
    let r = rt.execute(&archive, &descriptor(&archive), &AtomicBool::new(false))?;
    for (name, value) in r.response["report"]
        .as_object()
        .ok_or("missing control report")?
    {
        if value != &Value::Bool(true) {
            return Err(format!("isolation assertion {name}: {value}").into());
        }
        checks.push(json!({"check":name,"pass":true}));
    }
    let status = std::process::Command::new(&exe)
        .arg("--janitor-only")
        .arg(&stage)
        .status()?;
    assert!(status.success());
    assert_eq!(
        fs::read_dir(&stage)?.count(),
        1,
        "other process janitor must skip live lease"
    );
    checks.push(json!({"check":"startup_janitor_skips_cross_process_lock","pass":true}));
    drop(r);
    fs::write(&archive, br#"{"scenario":"sleep"}"#)?;
    let cancel = AtomicBool::new(false);
    let start = Instant::now();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let deadline = Instant::now();
            while worker_pid(&stage).is_none() && deadline.elapsed() < Duration::from_secs(3) {
                std::thread::sleep(Duration::from_millis(20));
            }
            cancel.store(true, Ordering::Release);
        });
        let e = rt
            .execute(&archive, &descriptor(&archive), &cancel)
            .unwrap_err();
        assert_eq!(e.stage, "cancelled");
    });
    assert!(start.elapsed() < Duration::from_secs(4));
    checks
        .push(json!({"check":"cancel_running","pass":true,"wall_ms":start.elapsed().as_millis()}));
    for scenario in ["stdout", "stderr", "crash"] {
        fs::write(&archive, serde_json::to_vec(&json!({"scenario":scenario}))?)?;
        let e = rt
            .execute(&archive, &descriptor(&archive), &AtomicBool::new(false))
            .unwrap_err();
        if scenario != "crash" {
            assert_eq!(e.stage, "output-limit");
        }
        checks.push(json!({"check":scenario,"pass":true,"error":e.to_string()}));
    }
    assert_eq!(fs::read_dir(&stage)?.count(), 0, "ordinary failure cleanup");
    checks.push(json!({"check":"failure_cleanup","pass":true}));
    // Independent launcher instance exits abruptly; the OS must reclaim its worker.
    fs::write(&archive, br#"{"scenario":"sleep"}"#)?;
    let status = std::process::Command::new(&exe)
        .arg("--parent-exit")
        .arg(&stage)
        .arg(&archive)
        .status()?;
    assert!(status.success());
    let pid: u32 = fs::read_to_string(stage.join("host-worker.pid"))?.parse()?;
    fs::remove_file(stage.join("host-worker.pid"))?;
    let start = Instant::now();
    while alive(pid) && start.elapsed() < Duration::from_secs(4) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(pid), "worker {pid} survived parent exit");
    checks.push(json!({"check":"parent_exit_reclaims_worker","pass":true,"pid":pid}));
    let _reopened = WorkerRuntime::new(&exe, &stage, identity())?;
    assert_eq!(
        fs::read_dir(&stage)?.count(),
        0,
        "startup janitor must reclaim unlocked marked invocation"
    );
    checks.push(json!({"check":"startup_janitor_reclaims_stale","pass":true}));
    let unknown = stage.join("sf-unknown");
    fs::create_dir(&unknown)?;
    fs::write(unknown.join("precious.txt"), b"unknown caller data")?;
    let _again = WorkerRuntime::new(&exe, &stage, identity())?;
    assert!(unknown.join("precious.txt").exists());
    fs::remove_dir_all(unknown)?;
    checks.push(json!({"check":"startup_janitor_preserves_unknown","pass":true})); // Short timeout uses the same production flags but a 150ms development limit.
    let timeout_root = root.path().join("timeout");
    fs::create_dir(&timeout_root)?;
    let runtime = timeout_root.join("runtime");
    let input = timeout_root.join("input");
    let output = timeout_root.join("output");
    for p in [&runtime, &input, &output] {
        fs::create_dir(p)?;
    }
    let copied = runtime.join(exe.file_name().unwrap());
    fs::copy(&exe, &copied)?;
    fs::write(input.join("archive.exe"), br#"{"scenario":"sleep"}"#)?;
    let mut cfg = SandboxConfig::new(
        &copied,
        &runtime,
        &format!("steam-patch-p3-timeout-{}", std::process::id()),
    );
    cfg.production_strict = true;
    cfg.input_dir = Some(input.clone());
    cfg.output_dir = Some(output.clone());
    cfg.stdin = descriptor(&input.join("archive.exe"));
    cfg.run_as = identity();
    cfg.environment = Some(Vec::new());
    cfg.limits.timeout = Some(Duration::from_millis(150));
    cfg.limits.active_process_limit = Some(1);
    #[cfg(windows)]
    {
        use steam_patch_installer_worker::win::{ensure_profile, grant_path_access, AccessKind};
        let profile = ensure_profile(&cfg.profile_name)?;
        for p in [&timeout_root, &runtime, &input] {
            grant_path_access(p, profile.sid, AccessKind::ReadExecute)?;
        }
        grant_path_access(&output, profile.sid, AccessKind::ReadWriteDelete)?;
        cfg.working_dir = Some(timeout_root.clone());
        cfg.environment = Some(
            ["SystemRoot", "SystemDrive", "USERPROFILE", "LOCALAPPDATA"]
                .iter()
                .map(|k| (k.to_string(), std::env::var(k).unwrap_or_default()))
                .collect(),
        );
    }
    #[cfg(target_os = "linux")]
    if let Some((uid, gid)) = identity() {
        std::os::unix::fs::chown(&output, Some(uid), Some(gid))?;
    }
    let result = steam_patch_installer_worker::run_sandboxed(&cfg);
    #[cfg(windows)]
    steam_patch_installer_worker::win::delete_profile(&cfg.profile_name)?;
    let outcome = result?;
    assert_eq!(outcome.termination, TerminationReason::Timeout);
    assert!(!alive(outcome.pid));
    checks.push(json!({"check":"timeout_reclaims_worker","pass":true,"wall_ms":outcome.wall_time.as_millis()}));
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"platform":std::env::consts::OS,"checks":checks}))?
    );
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() == 1 {
        return worker();
    }
    match args[1].to_str().unwrap_or("") {
        "--child" => Ok(()),
        "--janitor-only" => {
            let _ = WorkerRuntime::new(std::env::current_exe()?, Path::new(&args[2]), identity())?;
            Ok(())
        }
        "--parent-exit" => parent_exit(Path::new(&args[2]), Path::new(&args[3])),
        "--verify" => verify(),
        #[cfg(windows)]
        "--cleanup-owned-test" => {
            let path = Path::new(&args[2]).canonicalize()?;
            let boundary = std::env::current_dir()?
                .join(".artifacts/p3-production")
                .canonicalize()?;
            if !path.starts_with(&boundary)
                || !path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("controls-")
            {
                return Err("cleanup outside synthetic acceptance boundary".into());
            }
            if path.join("stage").exists() {
                set_stage_acl(&path.join("stage"), false)?;
            }
            fs::remove_dir_all(path)?;
            Ok(())
        }
        _ => Err("invalid development mode".into()),
    }
}
#[cfg(windows)]
fn set_stage_acl(path: &Path, readonly: bool) -> Result<(), Box<dyn std::error::Error>> {
    use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows::Win32::Security::{
        SetFileSecurityW, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR,
    };
    let sid = steam_patch_installer_worker::win::appcontainer::host_sid()?;
    let rights = if readonly { "GRGX" } else { "FA" };
    let sddl =
        steam_patch_installer_worker::win::util::hstring(&format!("D:P(A;OICI;{rights};;;{sid})"));
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(&sddl, 1, &mut sd, None) }?;
    let ok = unsafe {
        SetFileSecurityW(
            &steam_patch_installer_worker::win::util::hstring(&path.to_string_lossy()),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            sd,
        )
    };
    unsafe { steam_patch_installer_worker::win::util::local_free(sd.0) };
    if !ok.as_bool() {
        return Err("readonly stage DACL failed".into());
    }
    Ok(())
}
