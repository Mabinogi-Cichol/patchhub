//! `probe-runner`: host-side harness that creates synthetic unauthorized directories,
//! establishes the AppContainer profile and ACL grants, launches `probe-worker` under
//! LPAC + Job Object, and records the resulting evidence as JSON.

fn main() {
    #[cfg(windows)]
    win_impl::main();

    #[cfg(not(windows))]
    {
        eprintln!("probe-runner: LPAC probe is only supported on Windows");
        std::process::exit(2);
    }
}

#[cfg(windows)]
mod win_impl {
    use std::fs::File;
    use std::net::TcpListener;
    use std::os::windows::io::AsRawHandle;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use serde_json::{json, Value};
    use steam_patch_installer_worker::model::{
        CheckResult, CheckStatus, SandboxConfig, WorkerRequest, WorkerResponse,
    };
    use steam_patch_installer_worker::win::{
        delete_profile, ensure_profile, grant_path_access, process_alive, run_worker, self_in_job,
        set_raw_handle_inherit, AccessKind,
    };

    const CANARY_ENV: &str = "STEAM_PATCH_PROBE_CANARY";
    const FIXTURE_TEXT: &str = "PROBE_INPUT_v1_中文";

    struct Harness {
        root: PathBuf,
        worker_exe: PathBuf,
        worker_dir: PathBuf,
        input: PathBuf,
        output: PathBuf,
        game: PathBuf,
        documents: PathBuf,
        state: PathBuf,
        profile_name: String,
        profile_sid: String,
        env: Vec<(String, String)>,
    }

    pub fn main() {
        let args: Vec<String> = std::env::args().collect();
        let mut worker_exe: Option<PathBuf> = None;
        let mut root: Option<PathBuf> = None;
        let mut out: Option<PathBuf> = None;
        let mut profile = "steam-patch-probe".to_string();

        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--worker" => {
                    i += 1;
                    worker_exe = args.get(i).map(PathBuf::from);
                }
                "--root" => {
                    i += 1;
                    root = args.get(i).map(PathBuf::from);
                }
                "--out" => {
                    i += 1;
                    out = args.get(i).map(PathBuf::from);
                }
                "--profile" => {
                    i += 1;
                    profile = args.get(i).cloned().unwrap_or(profile);
                }
                other => {
                    eprintln!("probe-runner: ignoring unknown argument {other}");
                }
            }
            i += 1;
        }

        let worker_exe = worker_exe.unwrap_or_else(default_worker_exe);
        let root = root.unwrap_or_else(default_root);

        // Set a canary in the runner's own environment. It must not leak into the
        // worker because a clean environment block is passed explicitly.
        std::env::set_var(CANARY_ENV, "leak-me");

        let h = setup(&root, &worker_exe, &profile);
        let mut checks: Vec<CheckResult> = Vec::new();

        run_checks(&h, &mut checks);

        let passed = checks
            .iter()
            .filter(|c| c.status == CheckStatus::Pass)
            .count();
        let failed = checks
            .iter()
            .filter(|c| c.status == CheckStatus::Fail)
            .count();
        let unsupported = checks
            .iter()
            .filter(|c| c.status == CheckStatus::Unsupported)
            .count();
        let not_covered = checks
            .iter()
            .filter(|c| c.status == CheckStatus::NotCovered)
            .count();

        let evidence = json!({
            "schema": steam_patch_installer_worker::PROBE_SCHEMA,
            "generated_unix_secs": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            "os": os_info(),
            "arch": std::env::consts::ARCH,
            "host": {
                "in_job": self_in_job(),
                "runner_exe": std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
            },
            "sandbox": {
                "worker_exe": h.worker_exe.to_string_lossy(),
                "worker_dir": h.worker_dir.to_string_lossy(),
                "profile_name": h.profile_name,
                "profile_sid": h.profile_sid,
                "root": h.root.to_string_lossy(),
            },
            "summary": {
                "total": checks.len(),
                "passed": passed,
                "failed": failed,
                "unsupported": unsupported,
                "not_covered": not_covered,
            },
            "checks": checks,
        });

        // Best-effort profile cleanup; keeps evidence even if this fails.
        let _ = delete_profile(&h.profile_name);

        let text = serde_json::to_string_pretty(&evidence).unwrap_or_else(|_| "{}".to_string());
        if let Some(path) = out {
            let _ = std::fs::write(&path, &text);
        }
        println!("{text}");
    }

    fn default_worker_exe() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("probe-worker.exe")))
            .unwrap_or_else(|| PathBuf::from("probe-worker.exe"))
    }

    fn default_root() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "steam-patch-probe-{}-{}",
            std::process::id(),
            stamp
        ))
    }

    fn setup(root: &Path, source_worker_exe: &Path, profile: &str) -> Harness {
        let input = root.join("input");
        let output = root.join("output");
        let game = root.join("synthetic_game");
        let documents = root.join("synthetic_documents");
        let state = root.join("synthetic_state");
        // Private worker directory that this process owns, so we can set its DACL. On a
        // normal dev machine `target/debug` is also owner-writable, but in sandboxes the
        // build output directory may lack WRITE_DAC.
        let worker_dir = root.join("worker");
        for dir in [&input, &output, &game, &documents, &state, &worker_dir] {
            std::fs::create_dir_all(dir).expect("create probe dir");
        }

        let worker_exe = worker_dir.join(
            source_worker_exe
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new("probe-worker.exe")),
        );
        std::fs::copy(source_worker_exe, &worker_exe).expect("copy worker exe");

        std::fs::write(input.join("fixture.txt"), FIXTURE_TEXT).expect("write input fixture");
        std::fs::write(game.join("save.dat"), b"synthetic save data").expect("write game file");
        std::fs::write(documents.join("secret.doc"), b"synthetic document").expect("write doc");
        std::fs::create_dir_all(state.join("keys")).expect("create state keys");
        std::fs::write(state.join("keys").join("private.key"), b"FAKE-PRIVATE-KEY")
            .expect("write fake key");

        // Create the profile and grant ACLs to the package SID.
        let profile_handle = ensure_profile(profile).expect("ensure profile");
        let profile_sid = profile_handle.sid_string().expect("sid string");
        grant_path_access(&worker_dir, profile_handle.sid, AccessKind::ReadExecute)
            .expect("grant worker dir");
        grant_path_access(&input, profile_handle.sid, AccessKind::ReadExecute)
            .expect("grant input dir");
        grant_path_access(&output, profile_handle.sid, AccessKind::ReadWriteDelete)
            .expect("grant output dir");

        let env = build_clean_env(&output);

        Harness {
            root: root.to_path_buf(),
            worker_exe,
            worker_dir,
            input,
            output,
            game,
            documents,
            state,
            profile_name: profile.to_string(),
            profile_sid,
            env,
        }
    }

    fn build_clean_env(output: &Path) -> Vec<(String, String)> {
        let get = |k: &str| std::env::var(k).unwrap_or_default();
        let sysroot = get("SystemRoot");
        let path = format!(
            "{sysroot}\\System32;{sysroot};{sysroot}\\System32\\Wbem",
            sysroot = sysroot
        );
        let temp = output.to_string_lossy().to_string();
        vec![
            ("SystemRoot".to_string(), sysroot.clone()),
            ("windir".to_string(), sysroot.clone()),
            ("SystemDrive".to_string(), get("SystemDrive")),
            ("PATH".to_string(), path),
            ("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string()),
            ("TEMP".to_string(), temp.clone()),
            ("TMP".to_string(), temp),
            ("COMPUTERNAME".to_string(), get("COMPUTERNAME")),
            ("USERPROFILE".to_string(), get("USERPROFILE")),
            ("LOCALAPPDATA".to_string(), get("LOCALAPPDATA")),
            ("APPDATA".to_string(), get("APPDATA")),
            ("ProgramData".to_string(), get("ProgramData")),
            ("ProgramFiles".to_string(), get("ProgramFiles")),
            ("ComSpec".to_string(), get("ComSpec")),
            ("PROCESSOR_ARCHITECTURE".to_string(), "AMD64".to_string()),
            (
                "NUMBER_OF_PROCESSORS".to_string(),
                get("NUMBER_OF_PROCESSORS"),
            ),
        ]
    }

    impl Harness {
        fn base_cfg(&self) -> SandboxConfig {
            let mut cfg =
                SandboxConfig::new(&self.worker_exe, &self.worker_dir, &self.profile_name);
            cfg.input_dir = Some(self.input.clone());
            cfg.output_dir = Some(self.output.clone());
            cfg.working_dir = Some(self.worker_dir.clone());
            cfg.environment = Some(self.env.clone());
            cfg.limits.timeout = Some(Duration::from_secs(30));
            cfg
        }

        fn launch(
            &self,
            request: &WorkerRequest,
            tweak: impl FnOnce(&mut SandboxConfig),
        ) -> Result<
            (
                steam_patch_installer_worker::LaunchOutcome,
                Option<WorkerResponse>,
            ),
            String,
        > {
            let mut cfg = self.base_cfg();
            cfg.stdin = request.to_line();
            tweak(&mut cfg);
            run_worker(&cfg).map_err(|e| e.to_string())
        }
    }

    fn run_checks(h: &Harness, checks: &mut Vec<CheckResult>) {
        check_profile_lifecycle(checks);
        let selfcheck = check_selfcheck(h, checks);
        check_env_clean(h, checks, &selfcheck);
        check_handle_whitelist(h, checks);
        check_filesystem(h, checks);
        check_network(h, checks);
        check_registry(h, checks);
        check_process_tree(h, checks);
        check_limits(h, checks);
        checks.push(
            CheckResult::new("host_in_job", "宿主进程是否已在 Job 中（嵌套能力探测）")
                .expected("记录实际值（影响 JOB_LIST 原子分配）")
                .actual(format!("in_job={}", self_in_job()))
                .status(CheckStatus::Info),
        );
    }

    fn check_profile_lifecycle(checks: &mut Vec<CheckResult>) {
        let name = format!("steam-patch-probe-life-{}", std::process::id());
        let first = ensure_profile(&name);
        let second = ensure_profile(&name);
        let result = match (&first, &second) {
            (Ok(a), Ok(b)) => match (a.sid_string(), b.sid_string()) {
                (Ok(sa), Ok(sb)) => {
                    let ok = sa == sb;
                    Some((format!("create={sa}; derive={sb}"), ok))
                }
                _ => None,
            },
            _ => None,
        };
        let _ = delete_profile(&name);
        let (actual, status) = match result {
            Some((text, true)) => (text, CheckStatus::Pass),
            Some((text, false)) => (text, CheckStatus::Fail),
            None => (
                format!(
                    "create={:?}; derive={:?}",
                    first.err().map(|e| e.to_string()),
                    second.err().map(|e| e.to_string())
                ),
                CheckStatus::Fail,
            ),
        };
        checks.push(
            CheckResult::new(
                "profile_lifecycle",
                "AppContainer profile 创建/已存在派生 SID",
            )
            .expected("首次 CreateAppContainerProfile，第二次 ERROR_ALREADY_EXISTS 派生相同 SID")
            .actual(actual)
            .status(status),
        );
    }

    fn check_selfcheck(h: &Harness, checks: &mut Vec<CheckResult>) -> Option<WorkerResponse> {
        let req = WorkerRequest::new("selfcheck");
        let (outcome, resp) = match h.launch(&req, |cfg| cfg.handle_whitelist = true) {
            Ok(v) => v,
            Err(err) => {
                checks.push(
                    CheckResult::new("lpac_self_check", "LPAC 自检启动")
                        .expected("worker 在零 capability 下成功启动")
                        .actual(format!("launch failed: {err}"))
                        .status(CheckStatus::Fail),
                );
                return None;
            }
        };

        let data = resp.as_ref().map(|r| r.data.clone()).unwrap_or(Value::Null);
        let token = &data["token"];
        let console = &data["console"];
        let is_ac = token["is_app_container"].as_bool().unwrap_or(false);
        let cap = token["capability_count"].as_u64();
        let sid = token["app_container_sid"].as_str().unwrap_or("");
        let console_window = console["console_window"].as_u64().unwrap_or(u64::MAX);
        let output_console = console["std_output_is_console"].as_bool().unwrap_or(true);
        let in_job = data["in_job"].as_bool().unwrap_or(false);

        checks.push(
            CheckResult::new("lpac_self_check", "LPAC/AppContainer 自检（token 身份）")
                .expected("TokenIsAppContainer=true，TokenAppContainerSid=profile SID，capability=0")
                .actual(format!(
                    "is_app_container={is_ac}, sid={sid}, capability_count={cap:?}, api={}, exit={}, reason={:?}",
                    outcome.create_api, outcome.exit_code, outcome.termination
                ))
                .detail(format!(
                    "stdout={} bytes, stderr={} bytes: {}",
                    outcome.stdout.len(),
                    outcome.stderr.len(),
                    String::from_utf8_lossy(&outcome.stderr)
                ))
                .status(if is_ac && !sid.is_empty() && cap == Some(0) {
                    CheckStatus::Pass
                } else {
                    CheckStatus::Fail
                }),
        );
        checks.push(
            CheckResult::new("zero_capability_start", "零 capability 下启动")
                .expected("无网络/资源 capability 仍能启动 worker")
                .actual(format!(
                    "started pid={} exit={}",
                    outcome.pid, outcome.exit_code
                ))
                .status(if resp.is_some() {
                    CheckStatus::Pass
                } else {
                    CheckStatus::Fail
                }),
        );
        checks.push(
            CheckResult::new("no_console_window", "无控制台窗口")
                .expected("GetConsoleWindow()==0 且标准输出非控制台")
                .actual(format!(
                    "console_window={console_window}, std_output_is_console={output_console}"
                ))
                .status(if console_window == 0 && !output_console {
                    CheckStatus::Pass
                } else {
                    CheckStatus::Fail
                }),
        );
        checks.push(
            CheckResult::new("worker_in_job", "worker 归属 Job Object")
                .expected("JOB_LIST 原子分配，IsProcessInJob=true")
                .actual(format!("in_job={in_job}"))
                .status(if in_job {
                    CheckStatus::Pass
                } else {
                    CheckStatus::Fail
                }),
        );
        resp
    }

    fn check_env_clean(
        h: &Harness,
        checks: &mut Vec<CheckResult>,
        selfcheck: &Option<WorkerResponse>,
    ) {
        let Some(resp) = selfcheck else {
            checks.push(
                CheckResult::new("env_clean", "环境变量清洁")
                    .expected("宿主 canary 变量不出现在 worker 环境")
                    .actual("selfcheck 未产出")
                    .status(CheckStatus::NotCovered),
            );
            return;
        };
        let canary_present = resp.data["canary_present"].as_bool().unwrap_or(true);
        let env_count = resp.data["env_count"].as_u64().unwrap_or(0);
        checks.push(
            CheckResult::new("env_clean", "环境变量清洁")
                .expected("宿主 STEAM_PATCH_PROBE_CANARY 不出现；仅传入白名单环境")
                .actual(format!(
                    "canary_present={canary_present}, env_count={env_count}, passed_env={}",
                    h.env.len()
                ))
                .status(if !canary_present && env_count > 0 {
                    CheckStatus::Pass
                } else {
                    CheckStatus::Fail
                }),
        );
    }

    fn check_handle_whitelist(h: &Harness, checks: &mut Vec<CheckResult>) {
        let canary_path = h.input.join("fixture.txt");
        let canary = File::open(&canary_path).expect("open canary");
        let raw = canary.as_raw_handle() as usize;
        set_raw_handle_inherit(raw, true).expect("set canary inheritable");

        let run = |whitelist: bool, extras: Vec<usize>| -> Option<bool> {
            let req = WorkerRequest::new("handle_check").value(raw.to_string());
            let (_, resp) = h
                .launch(&req, |cfg| {
                    cfg.handle_whitelist = whitelist;
                    cfg.extra_inheritable_handles = extras.clone();
                })
                .ok()?;
            let resp = resp?;
            resp.data["present"].as_bool()
        };

        let whitelisted = run(true, Vec::new());
        let inherited = run(false, vec![raw]);

        let (actual, status) = match (whitelisted, inherited) {
            (Some(a), Some(b)) => (
                format!("raw_canary={raw}; whitelist_mode_present={a}; no_whitelist_present={b}"),
                if !a && b {
                    CheckStatus::Pass
                } else {
                    CheckStatus::Fail
                },
            ),
            _ => (
                format!(
                    "raw_canary={raw}; whitelist_mode={whitelisted:?}; no_whitelist={inherited:?}"
                ),
                CheckStatus::Fail,
            ),
        };
        let _ = set_raw_handle_inherit(raw, false);
        drop(canary);
        checks.push(
            CheckResult::new("handle_whitelist", "句柄继承白名单")
                .expected("WHITELIST 模式不继承未列句柄；无 WHITELIST 模式继承同一句柄")
                .actual(actual)
                .status(status),
        );
    }

    fn check_filesystem(h: &Harness, checks: &mut Vec<CheckResult>) {
        // Authorized read.
        let req =
            WorkerRequest::new("read_file").path(h.input.join("fixture.txt").to_string_lossy());
        push_simple(
            checks,
            h.launch(&req, |_| {}),
            "authorized_read_input",
            "授权输入目录可读",
            "读取成功且内容一致",
            |resp| {
                Ok(resp.data["content"]
                    .as_str()
                    .map(|c| c == FIXTURE_TEXT)
                    .unwrap_or(false))
            },
            |resp| resp.data["content"].as_str().unwrap_or("").to_string(),
        );

        for (id, title, path) in [
            (
                "deny_read_game",
                "未授权合成游戏目录读被拒",
                h.game.join("save.dat"),
            ),
            (
                "deny_read_documents",
                "未授权合成文档目录读被拒",
                h.documents.join("secret.doc"),
            ),
            (
                "deny_read_state_key",
                "未授权合成 state/私钥读被拒",
                h.state.join("keys").join("private.key"),
            ),
        ] {
            let req = WorkerRequest::new("read_file").path(path.to_string_lossy());
            let (outcome, resp) = match h.launch(&req, |_| {}) {
                Ok(v) => v,
                Err(err) => {
                    checks.push(
                        CheckResult::new(id, title)
                            .expected("ACCESS_DENIED (5)")
                            .actual(format!("launch failed: {err}"))
                            .status(CheckStatus::Fail),
                    );
                    continue;
                }
            };
            let code = resp
                .as_ref()
                .and_then(|r| r.error_code)
                .or(Some(outcome.exit_code));
            let denied = resp.as_ref().map(|r| !r.ok).unwrap_or(true);
            checks.push(
                CheckResult::new(id, title)
                    .expected("访问被拒 (win32 5 / 无输出)")
                    .actual(format!(
                        "ok={}, error_code={code:?}, exit={}, reason={:?}",
                        resp.as_ref().map(|r| r.ok).unwrap_or(false),
                        outcome.exit_code,
                        outcome.termination
                    ))
                    .code(code)
                    .status(if denied {
                        CheckStatus::Pass
                    } else {
                        CheckStatus::Fail
                    }),
            );
        }

        // Out-of-bounds writes.
        for (id, title, path) in [
            (
                "deny_write_game",
                "越界写合成游戏目录被拒",
                h.game.join("evil.txt"),
            ),
            (
                "deny_write_documents",
                "越界写合成文档目录被拒",
                h.documents.join("evil.txt"),
            ),
        ] {
            let req = WorkerRequest::new("write_file")
                .path(path.to_string_lossy())
                .text("evil");
            let (outcome, resp) = match h.launch(&req, |_| {}) {
                Ok(v) => v,
                Err(err) => {
                    checks.push(
                        CheckResult::new(id, title)
                            .expected("ACCESS_DENIED (5)")
                            .actual(format!("launch failed: {err}"))
                            .status(CheckStatus::Fail),
                    );
                    continue;
                }
            };
            let denied = resp.as_ref().map(|r| !r.ok).unwrap_or(true);
            let code = resp.as_ref().and_then(|r| r.error_code);
            checks.push(
                CheckResult::new(id, title)
                    .expected("访问被拒 (win32 5)")
                    .actual(format!(
                        "ok={}, error_code={code:?}, exit={}",
                        resp.as_ref().map(|r| r.ok).unwrap_or(false),
                        outcome.exit_code
                    ))
                    .code(code)
                    .status(if denied {
                        CheckStatus::Pass
                    } else {
                        CheckStatus::Fail
                    }),
            );
        }

        // Write into the read-only input directory.
        let req = WorkerRequest::new("write_file")
            .path(h.input.join("evil.txt").to_string_lossy())
            .text("evil");
        match h.launch(&req, |_| {}) {
            Ok((outcome, resp)) => {
                let denied = resp.as_ref().map(|r| !r.ok).unwrap_or(true);
                checks.push(
                    CheckResult::new("deny_write_input", "只读输入目录写被拒")
                        .expected("ACCESS_DENIED (5)")
                        .actual(format!(
                            "ok={}, error_code={:?}, exit={}",
                            resp.as_ref().map(|r| r.ok).unwrap_or(false),
                            resp.as_ref().and_then(|r| r.error_code),
                            outcome.exit_code
                        ))
                        .code(resp.as_ref().and_then(|r| r.error_code))
                        .status(if denied {
                            CheckStatus::Pass
                        } else {
                            CheckStatus::Fail
                        }),
                );
            }
            Err(err) => {
                checks.push(
                    CheckResult::new("deny_write_input", "只读输入目录写被拒")
                        .expected("ACCESS_DENIED (5)")
                        .actual(format!("launch failed: {err}"))
                        .status(CheckStatus::Fail),
                );
            }
        }

        // Authorized write.
        let out_file = h.output.join("out.txt");
        let req = WorkerRequest::new("write_file")
            .path(out_file.to_string_lossy())
            .text("hello-output");
        push_simple(
            checks,
            h.launch(&req, |_| {}),
            "allow_write_output",
            "授权输出目录可写",
            "写入成功且宿主可回读",
            |resp| {
                Ok(resp.ok
                    && std::fs::read_to_string(&out_file)
                        .map(|c| c == "hello-output")
                        .unwrap_or(false))
            },
            |_| "written".to_string(),
        );

        // Unicode + long path round trip inside the authorized output dir.
        let req = WorkerRequest::new("path_rw")
            .path(h.output.to_string_lossy())
            .text("中文内容_roundtrip");
        let launch = h.launch(&req, |_| {});
        match launch {
            Ok((_, resp)) => {
                let roundtrip = resp
                    .as_ref()
                    .map(|r| r.data["roundtrip_ok"].as_bool().unwrap_or(false))
                    .unwrap_or(false);
                let len = resp
                    .as_ref()
                    .and_then(|r| r.data["long_path_len"].as_u64())
                    .unwrap_or(0);
                checks.push(
                    CheckResult::new("unicode_long_path", "授权目录内中文/长路径读写")
                        .expected("中文文件名与 >260 字符路径读写成功")
                        .actual(format!("roundtrip_ok={roundtrip}, long_path_len={len}"))
                        .status(if roundtrip && len > 260 {
                            CheckStatus::Pass
                        } else {
                            CheckStatus::Fail
                        }),
                );
            }
            Err(err) => {
                checks.push(
                    CheckResult::new("unicode_long_path", "授权目录内中文/长路径读写")
                        .expected("中文文件名与 >260 字符路径读写成功")
                        .actual(format!("launch failed: {err}"))
                        .status(CheckStatus::Fail),
                );
            }
        }
    }

    fn check_network(h: &Harness, checks: &mut Vec<CheckResult>) {
        // Loopback: keep a listener open so a failure is caused by the sandbox, not a
        // missing peer.
        let listener = TcpListener::bind("127.0.0.1:0").ok();
        let port = listener
            .as_ref()
            .and_then(|l| l.local_addr().ok())
            .map(|a| a.port());
        let (actual, code, status) = match port {
            Some(port) => {
                let req = WorkerRequest::new("net_connect").host("127.0.0.1", port);
                match h.launch(&req, |_| {}) {
                    Ok((outcome, resp)) => {
                        let connected = resp.as_ref().map(|r| r.ok).unwrap_or(false);
                        let code = resp.as_ref().and_then(|r| r.error_code);
                        let err = resp.as_ref().and_then(|r| r.error.clone());
                        (
                            format!("connect 127.0.0.1:{port} connected={connected}, error_code={code:?}, error={err:?}, exit={}, stdout={}B, stderr={:?}", outcome.exit_code, outcome.stdout.len(), String::from_utf8_lossy(&outcome.stderr)),
                            code,
                            if connected { CheckStatus::Fail } else { CheckStatus::Pass },
                        )
                    }
                    Err(err) => (format!("launch failed: {err}"), None, CheckStatus::Fail),
                }
            }
            None => (
                "无法创建回环 listener".to_string(),
                None,
                CheckStatus::Unsupported,
            ),
        };
        drop(listener);
        checks.push(
            CheckResult::new("deny_network_loopback", "回环网络连接被拒")
                .expected("零 capability 下回环连接失败")
                .actual(actual)
                .code(code)
                .status(status),
        );

        let req = WorkerRequest::new("net_connect").host("1.1.1.1", 443);
        let (actual, code, status) = match h.launch(&req, |_| {}) {
            Ok((outcome, resp)) => {
                let connected = resp.as_ref().map(|r| r.ok).unwrap_or(false);
                let code = resp.as_ref().and_then(|r| r.error_code);
                let err = resp.as_ref().and_then(|r| r.error.clone());
                (
                    format!("connect 1.1.1.1:443 connected={connected}, error_code={code:?}, error={err:?}, exit={}, stdout={}B, stderr={:?}", outcome.exit_code, outcome.stdout.len(), String::from_utf8_lossy(&outcome.stderr)),
                    code,
                    if connected { CheckStatus::Fail } else { CheckStatus::Pass },
                )
            }
            Err(err) => (format!("launch failed: {err}"), None, CheckStatus::Fail),
        };
        checks.push(
            CheckResult::new("deny_network_external", "外网连接被拒")
                .expected("零 capability 下外网连接失败")
                .actual(actual)
                .code(code)
                .status(status),
        );
    }

    fn check_registry(h: &Harness, checks: &mut Vec<CheckResult>) {
        // Write to HKLM\SOFTWARE: expect denied.
        let req = WorkerRequest::new("registry_write")
            .key("HKLM\\SOFTWARE\\SteamPatchProbeP0")
            .value("p0");
        let (actual, code, status) = match h.launch(&req, |_| {}) {
            Ok((_, resp)) => {
                let ok = resp.as_ref().map(|r| r.ok).unwrap_or(false);
                let code = resp.as_ref().and_then(|r| r.error_code);
                (
                    format!("ok={ok}, error_code={code:?}"),
                    code,
                    if ok {
                        CheckStatus::Fail
                    } else {
                        CheckStatus::Pass
                    },
                )
            }
            Err(err) => (format!("launch failed: {err}"), None, CheckStatus::Fail),
        };
        checks.push(
            CheckResult::new("deny_registry_write", "注册表写被拒")
                .expected("HKLM\\SOFTWARE 写入被拒 (5)")
                .actual(actual)
                .code(code)
                .status(status),
        );

        // Read a protected hive key: expect denied.
        let req = WorkerRequest::new("registry_read").key("HKLM\\SAM\\SAM");
        let (actual, code, status) = match h.launch(&req, |_| {}) {
            Ok((_, resp)) => {
                let ok = resp.as_ref().map(|r| r.ok).unwrap_or(false);
                let code = resp.as_ref().and_then(|r| r.error_code);
                (
                    format!("ok={ok}, error_code={code:?}"),
                    code,
                    if ok {
                        CheckStatus::Fail
                    } else {
                        CheckStatus::Pass
                    },
                )
            }
            Err(err) => (format!("launch failed: {err}"), None, CheckStatus::Fail),
        };
        checks.push(
            CheckResult::new("deny_registry_read_protected", "受保护注册表读被拒")
                .expected("HKLM\\SAM 读取被拒")
                .actual(actual)
                .code(code)
                .status(status),
        );

        // Informational: ordinary HKLM read.
        let req = WorkerRequest::new("registry_read")
            .key("HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
        let actual = match h.launch(&req, |_| {}) {
            Ok((_, resp)) => format!(
                "ok={}, error_code={:?}",
                resp.as_ref().map(|r| r.ok).unwrap_or(false),
                resp.as_ref().and_then(|r| r.error_code)
            ),
            Err(err) => format!("launch failed: {err}"),
        };
        checks.push(
            CheckResult::new("registry_read_info", "普通注册表读（信息项）")
                .expected("记录实际结果（LPAC 默认可读资源核验）")
                .actual(actual)
                .status(CheckStatus::Info),
        );
    }

    fn check_process_tree(h: &Harness, checks: &mut Vec<CheckResult>) {
        // 1) Two LPAC processes in one job; TerminateJobObject reclaims the tree.
        let req = WorkerRequest::new("sleep").ms(8000);
        let result = h.launch(&req, |cfg| {
            cfg.tree_children = 1;
            cfg.tree_sleep_ms = 15_000;
            cfg.limits.snapshot_after = Some(Duration::from_millis(1500));
            cfg.limits.terminate_after = Some(Duration::from_millis(2500));
            cfg.limits.timeout = Some(Duration::from_secs(15));
        });
        let (actual, status) = match result {
            Ok((outcome, _)) => {
                let active = outcome
                    .snapshot
                    .as_ref()
                    .map(|s| s.active_processes)
                    .unwrap_or(0);
                let worker_dead = wait_until_dead(outcome.pid, Duration::from_secs(3));
                let tree_dead = outcome
                    .tree_pids
                    .iter()
                    .all(|p| wait_until_dead(*p, Duration::from_secs(3)));
                (
                    format!(
                        "active_processes={active}, tree_pids={:?}, worker_dead={worker_dead}, tree_dead={tree_dead}, reason={:?}",
                        outcome.tree_pids, outcome.termination
                    ),
                    if active >= 2 && worker_dead && tree_dead {
                        CheckStatus::Pass
                    } else {
                        CheckStatus::Fail
                    },
                )
            }
            Err(err) => (format!("launch failed: {err}"), CheckStatus::Fail),
        };
        checks.push(
            CheckResult::new("child_process_in_job", "同 Job 多进程归属与回收")
                .expected("ActiveProcesses>=2，TerminateJobObject 后全部消失")
                .actual(actual)
                .status(status),
        );

        // 2) Sandboxed worker child-process creation is controlled (denied).
        let req = WorkerRequest::new("spawn_child").child(h.worker_exe.to_string_lossy(), 3000);
        let (actual, code, status) = match h.launch(&req, |cfg| {
            cfg.limits.timeout = Some(Duration::from_secs(10));
        }) {
            Ok((_, resp)) => {
                let blocked = resp.as_ref().map(|r| !r.ok).unwrap_or(true);
                let code = resp.as_ref().and_then(|r| r.error_code);
                (
                    format!(
                        "spawn_ok={}, error_code={code:?}, error={:?}",
                        resp.as_ref().map(|r| r.ok).unwrap_or(false),
                        resp.as_ref().and_then(|r| r.error.clone())
                    ),
                    code,
                    if blocked {
                        CheckStatus::Pass
                    } else {
                        CheckStatus::Fail
                    },
                )
            }
            Err(err) => (format!("launch failed: {err}"), None, CheckStatus::Fail),
        };
        checks.push(
            CheckResult::new("child_process_controlled", "worker 创建子进程受控")
                .expected("LPAC worker 直接创建子进程被拒 (ACCESS_DENIED 5)")
                .actual(actual)
                .code(code)
                .status(status),
        );

        // 3) KILL_ON_JOB_CLOSE.
        let req = WorkerRequest::new("sleep").ms(8000);
        let result = h.launch(&req, |cfg| {
            cfg.tree_children = 1;
            cfg.tree_sleep_ms = 15_000;
            cfg.limits.close_job_after = Some(Duration::from_millis(2000));
            cfg.limits.timeout = Some(Duration::from_secs(15));
        });
        let (actual, status) = match result {
            Ok((outcome, _)) => {
                let worker_dead = wait_until_dead(outcome.pid, Duration::from_secs(3));
                let tree_dead = outcome
                    .tree_pids
                    .iter()
                    .all(|p| wait_until_dead(*p, Duration::from_secs(3)));
                (
                    format!(
                        "reason={:?}, worker_dead={worker_dead}, tree_dead={tree_dead}",
                        outcome.termination
                    ),
                    if worker_dead && tree_dead {
                        CheckStatus::Pass
                    } else {
                        CheckStatus::Fail
                    },
                )
            }
            Err(err) => (format!("launch failed: {err}"), CheckStatus::Fail),
        };
        checks.push(
            CheckResult::new("kill_on_job_close", "父退出/KILL_ON_JOB_CLOSE 回收进程树")
                .expected("关闭 Job 句柄后 worker 与 tree 子进程均终止")
                .actual(actual)
                .status(status),
        );

        // 3) Wall-clock timeout.
        let req = WorkerRequest::new("sleep").ms(10_000);
        let result = h.launch(&req, |cfg| {
            cfg.limits.timeout = Some(Duration::from_millis(1500));
        });
        let (actual, status) = match result {
            Ok((outcome, _)) => {
                let dead = wait_until_dead(outcome.pid, Duration::from_secs(3));
                (
                    format!(
                        "reason={:?}, wall_ms={}, dead={dead}, exit={}",
                        outcome.termination,
                        outcome.wall_time.as_millis(),
                        outcome.exit_code
                    ),
                    if outcome.termination
                        == steam_patch_installer_worker::TerminationReason::Timeout
                        && dead
                    {
                        CheckStatus::Pass
                    } else {
                        CheckStatus::Fail
                    },
                )
            }
            Err(err) => (format!("launch failed: {err}"), CheckStatus::Fail),
        };
        checks.push(
            CheckResult::new("wall_clock_timeout", "宿主超时终止并回收")
                .expected("1.5s 超时后 Timeout 终止且进程消失")
                .actual(actual)
                .status(status),
        );

        // 4) Active process limit.
        let req = WorkerRequest::new("spawn_child").child(h.worker_exe.to_string_lossy(), 3000);
        let result = h.launch(&req, |cfg| {
            cfg.limits.active_process_limit = Some(1);
            cfg.limits.timeout = Some(Duration::from_secs(8));
        });
        let actual = match result {
            Ok((outcome, resp)) => format!(
                "spawn_ok={}, error_code={:?}, reason={:?}, exit={}",
                resp.as_ref().map(|r| r.ok).unwrap_or(false),
                resp.as_ref().and_then(|r| r.error_code),
                outcome.termination,
                outcome.exit_code
            ),
            Err(err) => format!("launch failed: {err}"),
        };
        checks.push(
            CheckResult::new("active_process_limit", "进程数限额")
                .expected("ActiveProcessLimit=1 时子进程创建受控（失败或被回收）")
                .actual(actual)
                .status(CheckStatus::Info),
        );
    }

    fn check_limits(h: &Harness, checks: &mut Vec<CheckResult>) {
        // Memory: 128 MiB process limit vs a 256 MiB commit.
        let req = WorkerRequest::new("alloc").mb(256);
        let (actual, code, status) = match h.launch(&req, |cfg| {
            cfg.limits.process_memory_bytes = Some(128 * 1024 * 1024);
        }) {
            Ok((_, resp)) => {
                let ok = resp.as_ref().map(|r| r.ok).unwrap_or(false);
                let code = resp.as_ref().and_then(|r| r.error_code);
                (
                    format!("alloc_ok={ok}, error_code={code:?}"),
                    code,
                    if ok {
                        CheckStatus::Fail
                    } else {
                        CheckStatus::Pass
                    },
                )
            }
            Err(err) => (format!("launch failed: {err}"), None, CheckStatus::Fail),
        };
        checks.push(
            CheckResult::new("memory_limit", "内存限额")
                .expected("PROCESS_MEMORY=128MiB 时 256MiB commit 失败")
                .actual(actual)
                .code(code)
                .status(status),
        );

        // Control: larger limit, small commit succeeds.
        let req = WorkerRequest::new("alloc").mb(32);
        let (actual, status) = match h.launch(&req, |cfg| {
            cfg.limits.process_memory_bytes = Some(1024 * 1024 * 1024);
        }) {
            Ok((_, resp)) => {
                let ok = resp.as_ref().map(|r| r.ok).unwrap_or(false);
                (
                    format!("alloc_ok={ok}"),
                    if ok {
                        CheckStatus::Pass
                    } else {
                        CheckStatus::Fail
                    },
                )
            }
            Err(err) => (format!("launch failed: {err}"), CheckStatus::Fail),
        };
        checks.push(
            CheckResult::new("memory_limit_control", "内存限额对照")
                .expected("PROCESS_MEMORY=1GiB 时 32MiB commit 成功")
                .actual(actual)
                .status(status),
        );

        // CPU hard cap.
        let req = WorkerRequest::new("busy").ms(3000);
        let result = h.launch(&req, |cfg| {
            cfg.limits.cpu_rate_percent = Some(10);
            cfg.limits.snapshot_after = Some(Duration::from_millis(2500));
            cfg.limits.timeout = Some(Duration::from_secs(12));
        });
        let (actual, status, detail) = match result {
            Ok((outcome, _)) => {
                let cpu_ms = outcome
                    .snapshot
                    .as_ref()
                    .map(|s| (s.total_user_time_100ns + s.total_kernel_time_100ns) / 10_000)
                    .unwrap_or(u64::MAX);
                let cfg_text = outcome
                    .cpu_rate_control
                    .map(|(flags, rate)| format!("flags=0x{flags:X}, cpu_rate={rate}"))
                    .unwrap_or_else(|| "unavailable".to_string());
                let nested = self_in_job();
                let status = if cpu_ms != u64::MAX && cpu_ms <= 1500 {
                    CheckStatus::Pass
                } else if nested {
                    CheckStatus::NotCovered
                } else {
                    CheckStatus::Fail
                };
                (
                    format!(
                        "cpu_ms={cpu_ms}, wall_ms={}, job_cpu_rate=({cfg_text})",
                        outcome.wall_time.as_millis()
                    ),
                    status,
                    Some(format!(
                        "runner_in_job={nested}; Windows does not enforce CPU rate control for nested jobs, which explains no observed throttling"
                    )),
                )
            }
            Err(err) => (format!("launch failed: {err}"), CheckStatus::Fail, None),
        };
        checks.push(
            CheckResult::new("cpu_hard_cap", "CPU 硬上限（JobObjectCpuRateControl）")
                .expected("10% HARD_CAP 下 3s busy 的 CPU 时间 <= ~1.5s")
                .actual(actual)
                .detail(detail.unwrap_or_default())
                .status(status),
        );

        // Per-job user time limit.
        let req = WorkerRequest::new("busy").ms(10_000);
        let result = h.launch(&req, |cfg| {
            cfg.limits.per_job_user_time_100ns = Some(10_000_000); // 1s
            cfg.limits.timeout = Some(Duration::from_secs(15));
        });
        let (actual, status) = match result {
            Ok((outcome, _)) => {
                let wall = outcome.wall_time.as_millis();
                (
                    format!(
                        "wall_ms={wall}, reason={:?}, exit={:#010X}",
                        outcome.termination, outcome.exit_code
                    ),
                    // Terminated before the 10s busy loop could complete.
                    if wall < 9500 {
                        CheckStatus::Pass
                    } else {
                        CheckStatus::Fail
                    },
                )
            }
            Err(err) => (format!("launch failed: {err}"), CheckStatus::Fail),
        };
        checks.push(
            CheckResult::new("job_time_limit", "Job 用户态时间限额")
                .expected("PerJobUserTimeLimit=1s 时 10s busy 提前被终止（<9.5s）")
                .actual(actual)
                .status(status),
        );
    }

    fn wait_until_dead(pid: u32, timeout: Duration) -> bool {
        let start = Instant::now();
        loop {
            if !process_alive(pid) {
                return true;
            }
            if start.elapsed() >= timeout {
                return false;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn push_simple<F, G>(
        checks: &mut Vec<CheckResult>,
        result: Result<
            (
                steam_patch_installer_worker::LaunchOutcome,
                Option<WorkerResponse>,
            ),
            String,
        >,
        id: &str,
        title: &str,
        expected: &str,
        predicate: F,
        describe: G,
    ) where
        F: Fn(&WorkerResponse) -> Result<bool, String>,
        G: Fn(&WorkerResponse) -> String,
    {
        match result {
            Ok((_, Some(resp))) => {
                let (pass, actual) = match predicate(&resp) {
                    Ok(true) => (CheckStatus::Pass, describe(&resp)),
                    Ok(false) => (CheckStatus::Fail, describe(&resp)),
                    Err(err) => (CheckStatus::Fail, err),
                };
                checks.push(
                    CheckResult::new(id, title)
                        .expected(expected)
                        .actual(actual)
                        .code(resp.error_code)
                        .status(pass),
                );
            }
            Ok((_, None)) => checks.push(
                CheckResult::new(id, title)
                    .expected(expected)
                    .actual("worker 无 JSON 响应")
                    .status(CheckStatus::Fail),
            ),
            Err(err) => checks.push(
                CheckResult::new(id, title)
                    .expected(expected)
                    .actual(format!("launch failed: {err}"))
                    .status(CheckStatus::Fail),
            ),
        }
    }

    /// Minimal Windows version info read from the registry so the evidence records the
    /// actual host build without requiring a manifest.
    fn os_info() -> Value {
        use winreg::enums::HKEY_LOCAL_MACHINE;
        use winreg::RegKey;
        let key = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
        match key {
            Ok(k) => {
                let s = |name: &str| k.get_value::<String, _>(name).ok();
                let d = |name: &str| k.get_value::<u32, _>(name).ok();
                json!({
                    "product_name": s("ProductName"),
                    "display_version": s("DisplayVersion").or_else(|| s("ReleaseId")),
                    "current_build": s("CurrentBuildNumber"),
                    "ubr": d("UBR"),
                    "major": d("CurrentMajorVersionNumber"),
                    "minor": d("CurrentMinorVersionNumber"),
                })
            }
            Err(err) => json!({"error": err.to_string()}),
        }
    }
}
