//! P3 production EXE install: sandbox reports, trusted facts and direct extraction.
use super::{
    acquire, exe_report, exe_resolve, facts, orchestrator as common, outcome::CancellationCause,
    space, store, task::TaskControl, InstallerPaths,
};
use crate::{
    install::{self, InstallOperation, Transaction},
    operation,
    steam::GameInstall,
};
use serde_json::json;
use std::{fs, sync::atomic::AtomicBool};
use steam_patch_installer_worker::production::WorkerRuntime;
pub type Error = common::ZipTaskError;
type Result<T> = std::result::Result<T, Error>;
fn error(e: impl std::fmt::Display) -> Error {
    let detail = e.to_string();
    if matches!(
        detail.as_str(),
        "cancelled" | "task_cancelled" | "download_cancelled"
    ) {
        return Error::code("cancelled");
    }
    if detail.starts_with("durable_revoke_failed") {
        return Error::new("needs_recovery", detail);
    }
    if !detail.is_empty()
        && detail.len() <= 128
        && detail
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Error::code(detail);
    }
    Error::new("exe_analysis_failed", detail)
}
pub(crate) struct ExeInstallInputs<'a> {
    pub game: &'a GameInstall,
    pub selection: &'a exe_resolve::ResolvedExeCandidate,
    pub operation: InstallOperation,
    pub request_id: &'a str,
}
pub(crate) struct ExeInstallDeps<'a> {
    pub paths: &'a InstallerPaths,
    pub transport: &'a dyn acquire::Transport,
    pub runtime: &'a WorkerRuntime,
    pub cancel: &'a AtomicBool,
    pub cancel_reason: &'a (dyn Fn() -> Option<CancellationCause> + Sync),
}
#[cfg(test)]
pub(crate) fn install_exe(
    inputs: ExeInstallInputs<'_>,
    deps: ExeInstallDeps<'_>,
) -> Result<Transaction> {
    execute_exe(inputs, deps).map(|run| run.transaction)
}

pub(crate) fn execute_exe(
    inputs: ExeInstallInputs<'_>,
    deps: ExeInstallDeps<'_>,
) -> Result<super::InstallExecution> {
    if let Some(code) = super::host_identity_blocker() {
        return Err(Error::code(code));
    }
    let _claim =
        super::requests::RequestClaim::acquire(deps.paths, inputs.request_id).map_err(error)?;
    let binding = common::request_binding(
        &inputs.game.instance_id,
        Some(&inputs.selection.selection_id),
        None,
        inputs.operation,
        None,
    );
    if let Some(t) = common::begin_request(deps.paths, inputs.request_id, &binding)? {
        return Ok(super::InstallExecution::replay(t));
    }
    let control = TaskControl::new(
        deps.paths,
        inputs.request_id,
        &inputs.game.instance_id,
        &inputs.selection.selection_id,
    )
    .map_err(error)?;
    control.run_with_cause(deps.cancel, deps.cancel_reason, || {
        run(&inputs, &deps, &control, &binding).map(super::InstallExecution::fresh)
    })
}
fn boundary(deps: &ExeInstallDeps<'_>, control: &TaskControl) -> Result<()> {
    if let Some(cause) = (deps.cancel_reason)() {
        control.revoke_with_cause(cause).map_err(error)?;
    }
    control
        .check_requested_with_cause(deps.cancel, deps.cancel_reason)
        .map_err(error)
}

fn worker_error(control: &TaskControl, e: steam_patch_installer_worker::IsolationError) -> Error {
    if control.cancel.load(std::sync::atomic::Ordering::Acquire)
        || e.stage == "cancelled"
        || matches!(
            e.message.as_str(),
            "cancelled" | "task_cancelled" | "download_cancelled"
        )
    {
        return Error::code("cancelled");
    }
    if e.stage == "extract" {
        return Error::new("game_write_failed", e.to_string());
    }
    if e.stage == "extract_worker" {
        if e.message.contains("payload_crc_mismatch")
            || e.message.contains("payload_size_mismatch")
            || e.message.contains("lzma2_size_mismatch")
            || e.message.contains("truncated")
            || e.message.contains("exe_payload_changed")
        {
            return Error::new("exe_payload_changed", e.to_string());
        }
        return Error::new("exe_extract_failed", e.to_string());
    }
    error(e)
}

fn run(
    inputs: &ExeInstallInputs<'_>,
    deps: &ExeInstallDeps<'_>,
    control: &TaskControl,
    binding: &super::requests::RequestBinding,
) -> Result<Transaction> {
    boundary(deps, control)?;
    operation::stage("connecting", None, None);
    let request = acquire::DownloadRequest {
        url: &inputs.selection.download_url,
        download_hosts: &inputs.selection.rule.download_hosts,
        expected_sha256: None,
        expected_size: None,
    };
    let acquired = acquire::acquire_exe_with(
        deps.transport,
        &request,
        deps.paths,
        &control.cancel,
        |_, _| {},
    )
    .map_err(common::map_download_error)?;
    boundary(deps, control)?;
    // The cached download already exists. Analysis needs one more copy in the
    // sandbox, so require that copy plus the single metadata/margin allowance.
    let state_needed = acquired
        .size
        .checked_add(space::SPACE_MARGIN_BYTES)
        .ok_or_else(|| Error::code("exe_space_budget_overflow"))?;
    if space::inspect_volume(deps.paths.root())
        .map_err(error)?
        .available_bytes
        < state_needed
    {
        return Err(Error::code("insufficient_space"));
    }
    // One session copies the EXE once; analysis and extraction share it. The
    // worker still streams no incremental progress during analysis.
    operation::stage("analyzing_installer", None, None);
    let session = deps
        .runtime
        .open_session(
            &acquired.path,
            &acquired.sha256,
            acquired.size,
            &control.cancel,
        )
        .map_err(|e| worker_error(control, e))?;
    let descriptor =
        json!({"action":"analyze_behavior","exe_sha256":acquired.sha256,"exe_size":acquired.size});
    let analysis = session
        .analyze(
            &serde_json::to_vec(&descriptor).map_err(error)?,
            &control.cancel,
        )
        .map_err(|e| worker_error(control, e))?;
    let mut report =
        exe_report::validate_response(&analysis.response, &acquired.sha256, acquired.size)
            .map_err(error)?;
    let expectation = exe_resolve::expectation(inputs.selection);
    let game = facts::game_facts_unreviewed(inputs.game, &expectation, &control.cancel);
    // A whole-snapshot validity gate is intentionally NOT applied (design D06).
    report
        .select_payload(&game, &control.cancel)
        .map_err(error)?;
    let variant = serde_json::to_value(inputs.selection.candidate.variant)
        .map_err(error)?
        .as_str()
        .ok_or_else(|| Error::code("invalid_variant"))?
        .to_owned();
    let content = report
        .content_id(
            inputs.game.appid,
            &inputs.selection.source_id,
            &inputs.selection.source_record_id,
            &inputs.selection.language,
            &variant,
        )
        .map_err(error)?;
    if common::content_conflict(deps.paths, inputs.request_id, &content)? {
        return Err(Error::code("request_id_binding_conflict"));
    }
    // Analysis reports live under the cleanable `reports/<content_id>/` root,
    // never in the protected transaction `jobs/` root (p0-contracts §3.2).
    let reports = deps.paths.reports(&content).map_err(error)?;
    fs::create_dir_all(&reports).map_err(error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&reports, fs::Permissions::from_mode(0o700));
    }
    let snapshot = report.evaluate(&game, &control.cancel).map_err(error)?;
    if !snapshot.satisfied {
        return Err(Error::code("exe_conditions_not_satisfied"));
    }
    let sizes =
        space::original_sizes(&inputs.game.install_path, report.payload()).map_err(error)?;
    let budget = space::SpaceBudget::for_direct(acquired.size, &sizes).map_err(error)?;
    // The sandbox input copy already exists; only target growth remains.
    let evidence = budget
        .after_input_copy()
        .inspect(deps.paths.root(), &inputs.game.install_path)
        .map_err(error)?;
    store::write_atomic(
        &reports.join("space.json"),
        &serde_json::to_vec(&evidence).map_err(error)?,
        65536,
    )
    .map_err(error)?;
    let report_bytes = serde_json::to_vec(&analysis.response).map_err(error)?;
    store::write_atomic(
        &reports.join("report.json"),
        &report_bytes,
        report_bytes.len() as u64,
    )
    .map_err(error)?;
    drop(analysis);
    let files: Vec<(String, u64)> = report
        .payload()
        .iter()
        .map(|p| (p.path.clone(), p.size))
        .collect();
    let total = report.payload_bytes().map_err(error)?;
    boundary(deps, control)?;
    let plan = install::begin_direct_at(
        deps.paths.root(),
        inputs.game,
        &content,
        Some(&inputs.selection.selection_id),
        Some(&content),
        inputs.operation,
        &files,
        None,
    )
    .map_err(common::map_plan_error)?;
    let mut writer = super::direct::DirectWriter::new(report.payload(), &plan.targets);
    let mut wrote_any = false;
    let outcome = (|| {
        control
            .attach_plan(deps.paths, &plan.plan_id)
            .map_err(error)?;
        common::record_plan(
            deps.paths,
            inputs.request_id,
            binding,
            &plan.plan_id,
            &content,
        )?;
        boundary(deps, control)?;
        operation::stage("extracting", Some(total), Some(files.len()));
        let b = report.binding();
        let descriptor = json!({"action":"extract_behavior","exe_sha256":b.exe_sha256,"exe_size":b.exe_size,
            "payload":report.payload()});
        if let Err(e) = session.extract(
            &serde_json::to_vec(&descriptor).map_err(error)?,
            &control.cancel,
            &mut |chunk| writer.feed(chunk),
        ) {
            wrote_any = writer.opened_any();
            return Err(worker_error(control, e));
        }
        wrote_any = true;
        let installed = writer.finish().map_err(|e| worker_error(control, e))?;
        boundary(deps, control)?;
        install::commit_direct_at(deps.paths.root(), &plan.plan_id, &installed)
            .map_err(common::map_plan_error)
    })();
    let transaction = match outcome {
        Ok(t) => t,
        Err(e) => {
            // The journal precedes the first game write, so an interrupted
            // install is reported as incomplete. Written files stay as they are.
            let cancelled = (deps.cancel_reason)().is_some()
                || control.cancel.load(std::sync::atomic::Ordering::Acquire);
            let code = if cancelled {
                if wrote_any {
                    "cancelled_incomplete".to_owned()
                } else {
                    "task_cancelled".to_owned()
                }
            } else if wrote_any {
                format!("install_incomplete: {e}")
            } else {
                e.to_string()
            };
            let failed =
                install::fail_direct_at(deps.paths.root(), &plan.plan_id, &code).map_err(error)?;
            let _ = control.revoke_for_failure();
            common::record_terminal_fact(deps.paths, inputs.request_id, binding, &content, &failed);
            drop(session);
            return Ok(failed);
        }
    };
    drop(session);
    operation::stage("cleanup", None, None);
    common::record_terminal_fact(
        deps.paths,
        inputs.request_id,
        binding,
        &content,
        &transaction,
    );
    Ok(transaction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer::{
        acquire::{FakeTransport, HttpResponse},
        orchestrator::tests as fixture,
    };
    use crate::sources::{
        model::{
            ArtifactFormat, GameMatchRecord, GameMatchStatus, SourceMatchRecord, SourceMatchStatus,
        },
        Sources,
    };
    use serde_json::Value;
    use std::{
        fs,
        path::{Path, PathBuf},
    };
    fn golden() -> Value {
        serde_json::from_str(include_str!(
            "../../../installer-format/tests/fixtures/sf/worker-golden.json"
        ))
        .unwrap()
    }
    fn bytes() -> Vec<u8> {
        golden()["cases"][0]["data_hex"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect()
    }
    fn seed(state: &Path) -> exe_resolve::ResolvedExeCandidate {
        let mut candidate = fixture::candidate();
        candidate.mirrors[0].url = "https://kaguraserver.com/a.exe".into();
        candidate.mirrors[0].format = "exe".into();
        candidate.format_reported = ArtifactFormat::Exe;
        candidate.format_observed = ArtifactFormat::Exe;
        let selection = exe_resolve::exe_selection_id(&candidate).unwrap();
        let sources = Sources::open(state).unwrap();
        let source_id = candidate.source_id.clone();
        sources
            .store()
            .save_match(&GameMatchRecord {
                schema_version: 1,
                appid: candidate.game_appid.unwrap(),
                name: "Game".to_string(),
                localized_name: None,
                input_key: "key".to_string(),
                status: GameMatchStatus::PublisherMatch,
                sources: std::collections::BTreeMap::from([(
                    source_id,
                    SourceMatchRecord {
                        index_revision: 1,
                        status: SourceMatchStatus::Matched,
                        candidates: vec![candidate],
                        detail_pages: Vec::new(),
                        checked_at: 100,
                        diagnostics: Vec::new(),
                    },
                )]),
                fallback: None,
                updated_at: 100,
            })
            .unwrap();
        exe_resolve::resolve_exe(sources.registry(), &fixture::query(), &selection).unwrap()
    }
    fn transport() -> FakeTransport {
        let bytes = bytes();
        FakeTransport::new(vec![HttpResponse {
            status: 200,
            location: None,
            content_type: Some("application/octet-stream".into()),
            content_length: Some(bytes.len() as u64),
            etag: None,
            last_modified: None,
            final_url: "https://kaguraserver.com/a.exe".into(),
            body: Box::new(std::io::Cursor::new(bytes)),
        }])
    }
    fn runtime(stage: &Path) -> WorkerRuntime {
        let exe = std::env::var_os("SF_WORKER_TEST_BIN")
            .expect("set SF_WORKER_TEST_BIN to the built bundled worker (release profile)");
        #[cfg(target_os = "linux")]
        let uid = if steam_patch_installer_worker::linux::euid() == 0 {
            Some((1000, 1000))
        } else {
            None
        };
        #[cfg(not(target_os = "linux"))]
        let uid = None;
        WorkerRuntime::new(PathBuf::from(exe), stage, uid).unwrap()
    }
    fn task_dir_entries(paths: &InstallerPaths) -> usize {
        fs::read_dir(paths.tasks_dir())
            .map(|entries| entries.count())
            .unwrap_or(0)
    }
    #[allow(clippy::too_many_arguments)]
    fn invoke(
        _state: &Path,
        paths: &InstallerPaths,
        game: &GameInstall,
        selection: &exe_resolve::ResolvedExeCandidate,
        runtime: &WorkerRuntime,
        request: &str,
        op: InstallOperation,
        cancel: &AtomicBool,
    ) -> Result<Transaction> {
        let transport = transport();
        let inputs = ExeInstallInputs {
            game,
            selection,
            operation: op,
            request_id: request,
        };
        let deps = ExeInstallDeps {
            paths,
            transport: &transport,
            runtime,
            cancel,
            cancel_reason: &|| None,
        };
        operation::run(
            std::sync::Arc::new(AtomicBool::new(false)),
            |_| {},
            || install_exe(inputs, deps),
        )
    }
    #[test]
    #[ignore = "three existing local SF packages; release SF_WORKER_TEST_BIN and the actual sandbox required"]
    fn production_available_real_samples_admitted_and_extracted() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let stage = tempfile::tempdir().unwrap();
        let runtime = runtime(stage.path());
        assert!(runtime.probe().available);
        let cancel = AtomicBool::new(false);
        let mut evidence = Vec::new();
        for appid in [2303800u32, 3606360, 3721970] {
            let archive = root.join(format!(".artifacts/generic-exe-local/{appid}-archive.exe"));
            let exe = facts::digest_file(&archive, &cancel).unwrap();
            let session = runtime
                .open_session(&archive, &exe.sha256, exe.size, &cancel)
                .unwrap();
            let descriptor = serde_json::to_vec(&json!({"action":"analyze_behavior",
                "exe_sha256":exe.sha256,"exe_size":exe.size}))
            .unwrap();
            let analysis = session.analyze(&descriptor, &cancel).unwrap();
            let report =
                exe_report::validate_response(&analysis.response, &exe.sha256, exe.size).unwrap();
            let out = tempfile::tempdir().unwrap();
            let targets: Vec<install::DirectTarget> = report
                .payload()
                .iter()
                .map(|p| install::DirectTarget {
                    path: p.path.clone(),
                    file: out.path().join(&p.path),
                })
                .collect();
            let descriptor = serde_json::to_vec(&json!({"action":"extract_behavior",
                "exe_sha256":exe.sha256,"exe_size":exe.size,"payload":report.payload()}))
            .unwrap();
            let mut writer = super::super::direct::DirectWriter::new(report.payload(), &targets);
            session
                .extract(&descriptor, &cancel, &mut |chunk| writer.feed(chunk))
                .unwrap();
            let installed = writer.finish().unwrap();
            evidence.push(json!({"appid":appid,"exe_sha256":exe.sha256,
                "metadata_sha256":report.binding().metadata_sha256,
                "behavior_sha256":report.binding().behavior_sha256,
                "payload_files":installed.len()}));
            drop(session);
            assert_eq!(fs::read_dir(stage.path()).unwrap().count(), 0);
        }
        if let Some(path) = std::env::var_os("P3_EVIDENCE_PATH") {
            fs::write(
                path,
                serde_json::to_vec_pretty(&json!({"schema_version":1,"cases":evidence})).unwrap(),
            )
            .unwrap();
        }
    }

    #[test]
    #[ignore = "actual bundled worker fixture; set SF_WORKER_TEST_BIN (release profile)"]
    fn precancel_persists_revocation_without_worker_or_game_writes() {
        let (temp, game, _) = fixture::setup();
        fs::write(game.install_path.join("engine.exe"), b"inert").unwrap();
        fs::write(game.install_path.join("identity.dat"), [87, 164, 242, 220]).unwrap();
        let manifest = game
            .library_path
            .join("steamapps")
            .join(format!("appmanifest_{}.acf", game.appid));
        let mut text = fs::read_to_string(&manifest).unwrap();
        let end = text.rfind('}').unwrap();
        text.insert_str(end, " \"InstalledDepots\" {} ");
        fs::write(manifest, text).unwrap();
        let state = temp.path().join("state");
        let paths = InstallerPaths::new(&state);
        super::super::init_atomic(&paths).unwrap();
        let stage = temp.path().join("runtime");
        fs::create_dir(&stage).unwrap();
        let runtime = runtime(&stage);
        let selection = seed(&state);
        assert!(invoke(
            &state,
            &paths,
            &game,
            &selection,
            &runtime,
            "cancel",
            InstallOperation::Install,
            &AtomicBool::new(true)
        )
        .is_err());
        assert!(!game.install_path.join("payload.bin").exists());
        assert!(install::list_transactions_at(paths.root())
            .unwrap()
            .is_empty());
        assert_eq!(task_dir_entries(&paths), 0);
        assert_eq!(fs::read_dir(&stage).unwrap().count(), 0);
    }
}
