//! Host-owned fixed SF launcher. Paths, arguments and environment never come from a frontend.
use crate::{sf, IsolationError, LaunchOutcome, SandboxConfig, TerminationReason};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tempfile::TempDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxAvailability {
    pub available: bool,
    pub reason: String,
}
/// Constructed by the trusted host with its bundled executable and staging root.
#[derive(Debug, Clone)]
pub struct WorkerRuntime {
    worker_exe: PathBuf,
    staging_root: PathBuf,
    run_as: Option<(u32, u32)>,
}
/// One completed buffered worker invocation.
#[derive(Debug)]
pub struct WorkerExecution {
    pub response: Value,
}
/// One sandbox session over a single staged EXE. Analysis and extraction reuse
/// the same input copy; the directory is released when the session drops.
pub struct WorkerSession {
    #[cfg(windows)]
    _profile: ProfileGuard,
    _input_lock: File,
    cfg: SandboxConfig,
    _directory: InvocationDirectory,
}
fn err(stage: &str, message: impl Into<String>) -> IsolationError {
    IsolationError::new(stage, None, message)
}
fn ioerr(stage: &str, e: std::io::Error) -> IsolationError {
    IsolationError::new(stage, e.raw_os_error().map(|n| n as u32), e.to_string())
}
fn no_links(path: &Path) -> Result<(), IsolationError> {
    if !path.is_absolute() {
        return Err(err("path", "host paths must be absolute"));
    }
    for p in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        let m = fs::symlink_metadata(p).map_err(|e| ioerr("path", e))?;
        if m.file_type().is_symlink() {
            return Err(err("path", "linked sandbox path"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if m.file_attributes() & 0x400 != 0 {
                return Err(err("path", "reparse sandbox path"));
            }
        }
    }
    Ok(())
}
impl WorkerRuntime {
    pub fn new(
        worker_exe: impl Into<PathBuf>,
        staging_root: impl Into<PathBuf>,
        run_as: Option<(u32, u32)>,
    ) -> Result<Self, IsolationError> {
        let worker_exe = worker_exe.into();
        let staging_root = staging_root.into();
        no_links(&worker_exe)?;
        no_links(&staging_root)?;
        if !worker_exe.is_file() || !staging_root.is_dir() {
            return Err(err("path", "missing bundled worker or stage root"));
        }
        #[cfg(target_os = "linux")]
        if crate::linux::euid() == 0 && !matches!(run_as,Some((uid,gid)) if uid != 0 && gid != 0) {
            return Err(err(
                "identity",
                "root host requires explicit non-root uid/gid",
            ));
        }
        cleanup_stale(&staging_root)?;
        Ok(Self {
            worker_exe: worker_exe.canonicalize().map_err(|e| ioerr("path", e))?,
            staging_root: staging_root.canonicalize().map_err(|e| ioerr("path", e))?,
            run_as,
        })
    }
    /// Actually invokes the bundled SF worker in the production sandbox. The inert
    /// invalid bytes must reach the parser; executable/bwrap existence is insufficient.
    pub fn probe(&self) -> SandboxAvailability {
        let result = (|| {
            let sample = b"steam-patch inert sandbox capability probe";
            let dir = self.directory()?;
            let archive = dir.path().join("probe.exe");
            fs::write(&archive, sample).map_err(|e| ioerr("probe", e))?;
            let raw = serde_json::to_vec(
                &serde_json::json!({"action":"analyze","exe_size":sample.len(),
                "exe_sha256":format!("{:x}",Sha256::digest(sample))}),
            )
            .map_err(|e| err("probe", e.to_string()))?;
            let session = self.session_in(
                dir,
                &archive,
                &format!("{:x}", Sha256::digest(sample)),
                sample.len() as u64,
                &AtomicBool::new(false),
            )?;
            let outcome = session.run(&raw, &AtomicBool::new(false), true)?;
            let response = parse_response(&outcome)?;
            if response.as_object().is_some_and(|o| {
                o.len() == 1 && o.get("error").and_then(Value::as_str) == Some("not_pe")
            }) {
                Ok(())
            } else {
                Err(err(
                    "probe",
                    format!("unexpected inert parser response: {response}"),
                ))
            }
        })();
        match result {
            Ok(()) => SandboxAvailability {
                available: true,
                reason: "fixed SF worker executed in production sandbox".into(),
            },
            Err(e) => SandboxAvailability {
                available: false,
                reason: e.to_string(),
            },
        }
    }
    /// Buffered analysis used by probe/diagnostic callers and tests.
    pub fn execute(
        &self,
        archive: &Path,
        descriptor: &[u8],
        cancel: &AtomicBool,
    ) -> Result<WorkerExecution, IsolationError> {
        cancelled(cancel)?;
        let parsed = sf::parse_descriptor(descriptor).map_err(|e| err("descriptor", e.0))?;
        let (hash, size) = match &parsed {
            sf::Descriptor::Analyze {
                exe_sha256,
                exe_size,
            }
            | sf::Descriptor::AnalyzeBehavior {
                exe_sha256,
                exe_size,
            } => (exe_sha256, *exe_size),
            sf::Descriptor::ExtractBehavior { .. } => {
                return Err(err("descriptor", "extraction requires a streaming session"))
            }
        };
        let directory = self.directory()?;
        let session = self.session_in(directory, archive, hash, size, cancel)?;
        session.analyze(descriptor, cancel)
    }
    /// Open one staging session over `archive`, copying the sandbox input once.
    pub fn open_session(
        &self,
        archive: &Path,
        exe_sha256: &str,
        exe_size: u64,
        cancel: &AtomicBool,
    ) -> Result<WorkerSession, IsolationError> {
        cancelled(cancel)?;
        let directory = self.directory()?;
        self.session_in(directory, archive, exe_sha256, exe_size, cancel)
    }
    fn directory(&self) -> Result<InvocationDirectory, IsolationError> {
        no_links(&self.staging_root)?;
        let d = tempfile::Builder::new()
            .prefix("sf-")
            .tempdir_in(&self.staging_root)
            .map_err(|e| ioerr("stage", e))?;
        #[cfg(windows)]
        crate::win::appcontainer::make_private(d.path())?;
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(d.path(), fs::Permissions::from_mode(0o700))
                .map_err(|e| ioerr("stage", e))?;
            if fs::metadata(d.path())
                .map_err(|e| ioerr("stage", e))?
                .permissions()
                .mode()
                & 0o777
                != 0o700
            {
                return Err(err(
                    "acl",
                    "staging filesystem did not enforce private POSIX mode",
                ));
            }
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(d.path().join("owner.lock"))
            .map_err(|e| ioerr("stage", e))?;
        lock.try_lock().map_err(|e| err("stage", e.to_string()))?;
        let marker = InvocationMarker {
            schema: "steam-patch/worker-stage/v1".into(),
            owner: host_owner()?,
            directory: d.path().file_name().unwrap().to_string_lossy().into_owned(),
        };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(d.path().join("owner.json"))
            .map_err(|e| ioerr("stage", e))?;
        file.write_all(&serde_json::to_vec(&marker).map_err(|e| err("stage", e.to_string()))?)
            .and_then(|_| file.sync_all())
            .map_err(|e| ioerr("stage", e))?;
        Ok(InvocationDirectory {
            directory: d,
            _lock: lock,
        })
    }
    fn session_in(
        &self,
        directory: InvocationDirectory,
        archive: &Path,
        exe_sha256: &str,
        exe_size: u64,
        cancel: &AtomicBool,
    ) -> Result<WorkerSession, IsolationError> {
        no_links(archive)?;
        no_links(&self.worker_exe)?;
        let root = directory.path();
        let runtime = root.join("runtime");
        let input = root.join("input");
        let output = root.join("output");
        for p in [&runtime, &input, &output] {
            fs::create_dir(p).map_err(|e| ioerr("stage", e))?;
        }
        #[cfg(windows)]
        let exe = runtime.join("sf-worker.exe");
        #[cfg(not(windows))]
        let exe = runtime.join("sf-worker");
        fs::copy(&self.worker_exe, &exe).map_err(|e| ioerr("runtime", e))?;
        let staged_input = input.join("archive.exe");
        copy_bound(archive, &staged_input, exe_sha256, exe_size, cancel)?;
        let mut cfg = SandboxConfig::new(
            &exe,
            &runtime,
            &format!(
                "steam-patch-{}",
                root.file_name().unwrap().to_string_lossy()
            ),
        );
        cfg.production_strict = true;
        cfg.input_dir = Some(input.clone());
        cfg.output_dir = Some(output.clone());
        cfg.run_as = self.run_as;
        cfg.environment = Some(Vec::new());
        cfg.limits.process_memory_bytes = Some(3 * 1024 * 1024 * 1024);
        cfg.limits.job_memory_bytes = cfg.limits.process_memory_bytes;
        // File-backed mappings consume address space but are not a whole-file
        // heap allocation. Linux RLIMIT_AS must include the actual input map.
        cfg.limits.address_space_bytes = Some(
            exe_size
                .checked_add(3 * 1024 * 1024 * 1024)
                .ok_or_else(|| err("limits", "input address-space overflow"))?,
        );
        cfg.limits.active_process_limit = Some(1);
        cfg.limits.file_size_bytes = None;
        #[cfg(windows)]
        let profile = ProfileGuard::prepare(&mut cfg, root, &runtime, &input, &output)?;
        #[cfg(target_os = "linux")]
        prepare_linux(
            root,
            &runtime,
            &exe,
            &input,
            &staged_input,
            &output,
            self.run_as,
        )?;
        // Windows denies write/delete opens throughout the invocation. Linux exposes
        // a private copied inode through a read-only bind, never the cache inode.
        let input_lock = sf::open_input(&staged_input).map_err(|e| err("input", e.0))?;
        cancelled(cancel)?;
        Ok(WorkerSession {
            cfg,
            _input_lock: input_lock,
            #[cfg(windows)]
            _profile: profile,
            _directory: directory,
        })
    }
}
impl WorkerSession {
    /// Buffered analysis response. The caller validates report structure.
    pub fn analyze(
        &self,
        descriptor: &[u8],
        cancel: &AtomicBool,
    ) -> Result<WorkerExecution, IsolationError> {
        let outcome = self.run(descriptor, cancel, false)?;
        let response = parse_response(&outcome)?;
        if !response.is_object() {
            return Err(err("response", "worker response must be one JSON object"));
        }
        validate_response(&response, descriptor)?;
        Ok(WorkerExecution { response })
    }
    /// Run the extraction descriptor and feed raw stdout chunks to `consume` on
    /// the calling thread. The callback writes decoded member bytes; framing is
    /// parsed by the caller. No stdout is retained.
    pub fn extract(
        &self,
        descriptor: &[u8],
        cancel: &AtomicBool,
        consume: &mut dyn FnMut(&[u8]) -> Result<(), IsolationError>,
    ) -> Result<(), IsolationError> {
        cancelled(cancel)?;
        let mut cfg = self.cfg.clone();
        cfg.stdin = descriptor.to_vec();
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(8);
        std::thread::scope(|scope| {
            let launched = scope.spawn(|| run_stream(&cfg, cancel, tx));
            let mut consume_error = None;
            while let Ok(chunk) = rx.recv() {
                if let Err(e) = consume(&chunk) {
                    consume_error = Some(e);
                    break;
                }
            }
            drop(rx);
            let outcome = launched
                .join()
                .map_err(|_| err("worker", "worker supervisor panicked"))?;
            if let Some(e) = consume_error {
                return Err(e);
            }
            let outcome = outcome?;
            if outcome.termination != TerminationReason::Completed {
                return Err(err(
                    if outcome.termination == TerminationReason::Cancelled {
                        "cancelled"
                    } else {
                        "timeout"
                    },
                    "sandbox worker terminated",
                ));
            }
            if outcome.exit_code != 0 {
                return Err(err(
                    "extract_worker",
                    format!(
                        "sandbox worker failed (exit={}): {}",
                        outcome.exit_code,
                        String::from_utf8_lossy(&outcome.stderr).trim()
                    ),
                ));
            }
            Ok(())
        })
    }
    fn run(
        &self,
        descriptor: &[u8],
        cancel: &AtomicBool,
        probe: bool,
    ) -> Result<LaunchOutcome, IsolationError> {
        cancelled(cancel)?;
        let mut cfg = self.cfg.clone();
        cfg.stdin = descriptor.to_vec();
        if probe {
            cfg.limits.timeout = Some(Duration::from_secs(10));
        }
        let outcome = run_capture(&cfg, cancel)?;
        if outcome.termination != TerminationReason::Completed {
            return Err(err(
                if outcome.termination == TerminationReason::Cancelled {
                    "cancelled"
                } else {
                    "timeout"
                },
                "sandbox worker terminated",
            ));
        }
        if probe {
            if outcome.exit_code != 1 {
                return Err(err("probe", "inert invalid input must exit 1"));
            }
        } else if outcome.exit_code != 0 {
            let response = parse_response(&outcome)?;
            return Err(err(
                "worker",
                response
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("sandbox worker failed"),
            ));
        }
        cancelled(cancel)?;
        Ok(outcome)
    }
}
fn parse_response(outcome: &LaunchOutcome) -> Result<Value, IsolationError> {
    serde_json::from_slice::<StrictValue>(&outcome.stdout)
        .map(|value| value.0)
        .map_err(|e| {
            err(
                "response",
                format!(
                    "{e}; exit={}; stderr={}",
                    outcome.exit_code,
                    String::from_utf8_lossy(&outcome.stderr)
                ),
            )
        })
}
fn cancelled(cancel: &AtomicBool) -> Result<(), IsolationError> {
    if cancel.load(Ordering::Acquire) {
        Err(err("cancelled", "worker cancellation requested"))
    } else {
        Ok(())
    }
}
#[cfg(windows)]
fn run_capture(cfg: &SandboxConfig, cancel: &AtomicBool) -> Result<LaunchOutcome, IsolationError> {
    crate::win::launch::run_sandboxed_cancellable(cfg, cancel)
}
#[cfg(windows)]
fn run_stream(
    cfg: &SandboxConfig,
    cancel: &AtomicBool,
    tx: std::sync::mpsc::SyncSender<Vec<u8>>,
) -> Result<LaunchOutcome, IsolationError> {
    crate::win::launch::run_sandboxed_streaming(cfg, cancel, tx)
}
#[cfg(target_os = "linux")]
fn run_capture(cfg: &SandboxConfig, cancel: &AtomicBool) -> Result<LaunchOutcome, IsolationError> {
    crate::linux::launch::run_sandboxed_cancellable(cfg, cancel)
}
#[cfg(target_os = "linux")]
fn run_stream(
    cfg: &SandboxConfig,
    cancel: &AtomicBool,
    tx: std::sync::mpsc::SyncSender<Vec<u8>>,
) -> Result<LaunchOutcome, IsolationError> {
    crate::linux::launch::run_sandboxed_streaming(cfg, cancel, tx)
}
#[cfg(not(any(windows, target_os = "linux")))]
fn run_capture(
    _cfg: &SandboxConfig,
    _cancel: &AtomicBool,
) -> Result<LaunchOutcome, IsolationError> {
    Err(err("sandbox", "worker platform unsupported"))
}
#[cfg(not(any(windows, target_os = "linux")))]
fn run_stream(
    _cfg: &SandboxConfig,
    _cancel: &AtomicBool,
    _tx: std::sync::mpsc::SyncSender<Vec<u8>>,
) -> Result<LaunchOutcome, IsolationError> {
    Err(err("sandbox", "worker platform unsupported"))
}

fn copy_bound(
    source: &Path,
    dest: &Path,
    expected: &str,
    size: u64,
    cancel: &AtomicBool,
) -> Result<(), IsolationError> {
    let mut src = sf::open_input(source).map_err(|e| err("input", e.0))?;
    if src.metadata().map_err(|e| ioerr("input", e))?.len() != size {
        return Err(err("input", "input size mismatch"));
    }
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)
        .map_err(|e| ioerr("input", e))?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        cancelled(cancel)?;
        let n = src.read(&mut buf).map_err(|e| ioerr("input", e))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > size {
            return Err(err("input", "input grew during copy"));
        }
        hash.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| ioerr("input", e))?;
    }
    if total != size || format!("{:x}", hash.finalize()) != expected {
        return Err(err("input", "input identity mismatch"));
    }
    out.sync_all().map_err(|e| ioerr("input", e))?;
    Ok(())
}
#[cfg(windows)]
struct ProfileGuard {
    name: String,
    active: bool,
}
#[cfg(windows)]
impl ProfileGuard {
    fn close(&mut self) -> Result<(), IsolationError> {
        if self.active {
            crate::win::delete_profile(&self.name)?;
            self.active = false;
        }
        Ok(())
    }
    fn prepare(
        cfg: &mut SandboxConfig,
        root: &Path,
        runtime: &Path,
        input: &Path,
        output: &Path,
    ) -> Result<Self, IsolationError> {
        use crate::win::{ensure_profile, grant_path_access, AccessKind};
        let mut guard = Self {
            name: cfg.profile_name.clone(),
            active: false,
        };
        let profile = ensure_profile(&guard.name)?;
        guard.active = true;
        for p in [root, runtime, input] {
            grant_path_access(p, profile.sid, AccessKind::ReadExecute)?;
        }
        grant_path_access(output, profile.sid, AccessKind::ReadWriteDelete)?;
        cfg.working_dir = Some(root.to_path_buf());
        let system = std::env::var("SystemRoot")
            .map_err(|_| err("environment", "missing trusted SystemRoot"))?;
        cfg.environment = Some(vec![
            ("SystemRoot".into(), system.clone()),
            (
                "SystemDrive".into(),
                Path::new(&system)
                    .components()
                    .next()
                    .ok_or_else(|| err("environment", "invalid SystemRoot"))?
                    .as_os_str()
                    .to_string_lossy()
                    .into_owned(),
            ),
            ("USERPROFILE".into(), output.to_string_lossy().into_owned()),
            ("LOCALAPPDATA".into(), output.to_string_lossy().into_owned()),
            ("TEMP".into(), output.to_string_lossy().into_owned()),
            ("TMP".into(), output.to_string_lossy().into_owned()),
        ]);
        Ok(guard)
    }
}
#[cfg(windows)]
impl Drop for ProfileGuard {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
#[cfg(target_os = "linux")]
fn prepare_linux(
    root: &Path,
    runtime: &Path,
    exe: &Path,
    input: &Path,
    archive: &Path,
    output: &Path,
    run_as: Option<(u32, u32)>,
) -> Result<(), IsolationError> {
    use std::os::unix::fs::{chown, PermissionsExt};
    for (p, mode) in [
        (root, 0o700),
        (runtime, 0o500),
        (exe, 0o500),
        (input, 0o500),
        (archive, 0o400),
        (output, 0o700),
    ] {
        fs::set_permissions(p, fs::Permissions::from_mode(mode)).map_err(|e| ioerr("acl", e))?;
        if fs::metadata(p)
            .map_err(|e| ioerr("acl", e))?
            .permissions()
            .mode()
            & 0o777
            != mode
        {
            return Err(err(
                "acl",
                "staging filesystem did not enforce POSIX permissions",
            ));
        }
        if crate::linux::euid() == 0 {
            let (uid, gid) =
                run_as.ok_or_else(|| err("identity", "missing explicit non-root uid/gid"))?;
            chown(p, Some(uid), Some(gid)).map_err(|e| ioerr("acl", e))?;
        }
    }
    Ok(())
}
#[derive(Debug)]
struct InvocationDirectory {
    _lock: File,
    directory: TempDir,
}
impl InvocationDirectory {
    fn path(&self) -> &Path {
        self.directory.path()
    }
}
impl Drop for InvocationDirectory {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::PermissionsExt;
            // Restore owner directory write access so TempDir can unlink the
            // read-only runtime/input. Never follow a link during cleanup.
            for name in ["", "runtime", "input", "output"] {
                let p = self.directory.path().join(name);
                if fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
                {
                    let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o700));
                }
            }
        }
    }
}
fn validate_response(v: &Value, raw: &[u8]) -> Result<(), IsolationError> {
    let o = v
        .as_object()
        .ok_or_else(|| err("response", "invalid response object"))?;
    let d = sf::parse_descriptor(raw).map_err(|e| err("descriptor", e.0))?;
    let (action, hash) = match &d {
        sf::Descriptor::Analyze { exe_sha256, .. } => ("analyze", exe_sha256),
        sf::Descriptor::AnalyzeBehavior { exe_sha256, .. } => ("analyze_behavior", exe_sha256),
        sf::Descriptor::ExtractBehavior { .. } => {
            return Err(err("response", "extraction has no buffered response"))
        }
    };
    let keys = [
        "schema_version",
        "action",
        "exe_sha256",
        "report",
        "installable",
    ];
    if o.len() != keys.len()
        || keys.iter().any(|k| !o.contains_key(*k))
        || v["exe_sha256"].as_str() != Some(hash)
        || v["schema_version"] != 1
        || v["action"] != action
        || v["installable"] != false
        || !v["report"].is_object()
    {
        return Err(err(
            "response",
            "worker response fields/input binding mismatch",
        ));
    }
    Ok(())
}
/// serde_json::Value accepts duplicate object keys. IPC must reject them at
/// every depth rather than silently taking the last untrusted report binding.
struct StrictValue(Value);
impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = StrictValue;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON without duplicate keys")
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Bool(v)))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| StrictValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("nonfinite JSON"))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut v = Vec::new();
                while let Some(item) = seq.next_element::<StrictValue>()? {
                    v.push(item.0);
                }
                Ok(StrictValue(Value::Array(v)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut v = serde_json::Map::new();
                while let Some((key, item)) = map.next_entry::<String, StrictValue>()? {
                    if v.insert(key, item.0).is_some() {
                        return Err(serde::de::Error::custom("duplicate JSON key"));
                    }
                }
                Ok(StrictValue(Value::Object(v)))
            }
        }
        d.deserialize_any(Visitor)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InvocationMarker {
    schema: String,
    owner: String,
    directory: String,
}
fn host_owner() -> Result<String, IsolationError> {
    #[cfg(windows)]
    {
        crate::win::appcontainer::host_sid()
    }
    #[cfg(target_os = "linux")]
    {
        Ok(format!("uid:{}", crate::linux::euid()))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err(err("identity", "unsupported platform"))
    }
}
/// Reclaim only current-host-owned, marked, unlocked invocation directories.
/// Unknown entries and live output leases are deliberately retained.
fn cleanup_stale(root: &Path) -> Result<(), IsolationError> {
    let owner = host_owner()?;
    for entry in fs::read_dir(root).map_err(|e| ioerr("cleanup", e))? {
        let entry = entry.map_err(|e| ioerr("cleanup", e))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.len() != 9
            || !name.starts_with("sf-")
            || !name.as_bytes()[3..].iter().all(u8::is_ascii_alphanumeric)
            || no_links(&path).is_err()
            || !path.is_dir()
        {
            continue;
        }
        let marker_path = path.join("owner.json");
        let lock_path = path.join("owner.lock");
        if no_links(&marker_path).is_err() || no_links(&lock_path).is_err() {
            continue;
        }
        let Ok(metadata) = fs::metadata(&marker_path) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > 4096 {
            continue;
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.uid() != crate::linux::euid() {
                continue;
            }
        }
        #[cfg(windows)]
        if crate::win::appcontainer::path_owner_sid(&path)
            .ok()
            .as_deref()
            != Some(owner.as_str())
            || crate::win::appcontainer::path_owner_sid(&marker_path)
                .ok()
                .as_deref()
                != Some(owner.as_str())
        {
            continue;
        }
        let Ok(lock_metadata) = fs::metadata(&lock_path) else {
            continue;
        };
        if !lock_metadata.is_file() {
            continue;
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            if lock_metadata.uid() != crate::linux::euid()
                || lock_metadata.nlink() != 1
                || metadata.nlink() != 1
            {
                continue;
            }
        }
        let Ok(lock) = OpenOptions::new().read(true).write(true).open(&lock_path) else {
            continue;
        };
        if lock.try_lock().is_err() {
            continue;
        }
        let Ok(file) = File::open(&marker_path) else {
            continue;
        };
        let mut raw = Vec::new();
        if file.take(4097).read_to_end(&mut raw).is_err() || raw.len() > 4096 {
            continue;
        }
        let Ok(marker) = serde_json::from_slice::<InvocationMarker>(&raw) else {
            continue;
        };
        if marker.schema != "steam-patch/worker-stage/v1"
            || marker.owner != owner
            || marker.directory != name
        {
            continue;
        }
        #[cfg(windows)]
        {
            match crate::win::delete_profile(&format!("steam-patch-{name}")) {
                Ok(()) => {}
                Err(e) if matches!(e.code, Some(2 | 3 | 1168)) => {}
                Err(e) => return Err(e),
            }
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::PermissionsExt;
            for p in [
                &path,
                &path.join("runtime"),
                &path.join("input"),
                &path.join("output"),
            ] {
                if fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
                {
                    fs::set_permissions(p, fs::Permissions::from_mode(0o700))
                        .map_err(|e| ioerr("cleanup", e))?;
                }
            }
        }
        fs::remove_dir_all(&path).map_err(|e| ioerr("cleanup", e))?;
        drop(lock);
    }
    Ok(())
}

#[cfg(test)]
mod production_tests {
    use super::*;
    #[test]
    fn ipc_rejects_duplicate_keys_at_all_depths_and_trailing_objects() {
        for raw in [
            br#"{"exe_sha256":"a","exe_sha256":"b"}"#.as_slice(),
            br#"{"report":{"binding":1,"binding":2}}"#.as_slice(),
            br#"{}{}"#.as_slice(),
        ] {
            assert!(serde_json::from_slice::<StrictValue>(raw).is_err());
        }
        assert!(serde_json::from_slice::<StrictValue>(br#"{"report":{"binding":1}}"#).is_ok());
    }
    #[test]
    fn response_cannot_authorize_or_change_action_and_input_binding() {
        let raw = serde_json::to_vec(
            &serde_json::json!({"action":"analyze","exe_size":1,"exe_sha256":"a".repeat(64)}),
        )
        .unwrap();
        let good = serde_json::json!({"schema_version":1,"action":"analyze","exe_sha256":"a".repeat(64),"report":{},"installable":false});
        assert!(validate_response(&good, &raw).is_ok());
        for (key, value) in [
            ("installable", serde_json::json!(true)),
            ("exe_sha256", serde_json::json!("b".repeat(64))),
            ("action", serde_json::json!("convert_behavior")),
            ("extra_path", serde_json::json!("/game")),
        ] {
            let mut changed = good.clone();
            changed[key] = value;
            assert!(validate_response(&changed, &raw).is_err());
        }
    }
}
