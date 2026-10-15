//! P3 production EXE install: sandbox reports, trusted facts, authorization and apply.
use super::{
    acquire,
    authorization::{self, AuthorizeInput, ContentBinding, CurrentState, PolicyVersions},
    exe_report::{self, ConditionSnapshot, PreparedReport, ReinstallEvidence},
    exe_resolve, facts, orchestrator as common,
    outcome::CancellationCause,
    space, store,
    task::TaskControl,
    InstallerPaths,
};
use crate::{
    catalog::FileMapping,
    install::{self, InstallOperation, Transaction},
    operation,
    sources::{GameQuery, Sources},
    steam::GameInstall,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{self, Read},
    path::Path,
    sync::atomic::AtomicBool,
};
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
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(crate) struct ExeInstallInputs<'a> {
    pub game: &'a GameInstall,
    pub selection: &'a exe_resolve::ResolvedExeCandidate,
    pub operation: InstallOperation,
    pub request_id: &'a str,
}
pub(crate) struct ExeInstallDeps<'a> {
    pub paths: &'a InstallerPaths,
    pub state_dir: &'a Path,
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
    control.run_with_cause(
        deps.cancel,
        deps.cancel_reason,
        || run(&inputs, &deps, &control, &binding).map(super::InstallExecution::fresh),
    )
}
fn boundary(deps: &ExeInstallDeps<'_>, control: &TaskControl) -> Result<()> {
    if let Some(cause) = (deps.cancel_reason)() {
        control.revoke_with_cause(cause).map_err(error)?;
    }
    control
        .check_requested_with_cause(deps.cancel, deps.cancel_reason)
        .map_err(error)
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
    // The state root holds the cached download D; analysis then makes a sandbox
    // input copy of D before the conversion budget (which needs the payload
    // size) can run. Require one known download length plus the documented
    // metadata/runtime margin here; the exact per-volume budget is re-checked
    // after analysis (design §5.2), so this is not a second fixed ceiling.
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
    operation::stage("analyzing_installer", Some(acquired.size), None);
    let descriptor =
        json!({"action":"analyze_behavior","exe_sha256":acquired.sha256,"exe_size":acquired.size});
    let analysis = deps
        .runtime
        .execute(
            &acquired.path,
            &serde_json::to_vec(&descriptor).map_err(error)?,
            &control.cancel,
        )
        .map_err(|e| {
            if control.cancel.load(std::sync::atomic::Ordering::Acquire) {
                Error::code("cancelled")
            } else {
                error(e)
            }
        })?;
    let mut report =
        exe_report::validate_response(&analysis.response, &acquired.sha256, acquired.size)
            .map_err(error)?;
    let expectation = exe_resolve::expectation(inputs.selection);
    let game = facts::game_facts_unreviewed(inputs.game, &expectation, &control.cancel);
    // A whole-snapshot validity gate is intentionally NOT applied (design D06);
    // the concrete action preconditions are evaluated later at the boundary.
    let reinstall = if inputs.operation == InstallOperation::Reinstall {
        reinstall_evidence(deps.paths, inputs.game, &report)?
    } else {
        None
    };
    if inputs.operation == InstallOperation::Reinstall && reinstall.is_none() {
        return Err(Error::code("reinstall_unowned_or_incomplete"));
    }
    report
        .select_payload_with_evidence(&game, &control.cancel, reinstall.as_ref())
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
    let snapshot = report
        .evaluate_with_reinstall(&game, &control.cancel, Some(&content), reinstall.as_ref())
        .map_err(error)?;
    if !snapshot.satisfied {
        return Err(Error::code("exe_conditions_not_satisfied"));
    }
    let report_bytes = serde_json::to_vec(&analysis.response).map_err(error)?;
    drop(analysis);
    let sizes =
        space::original_sizes(&inputs.game.install_path, report.payload()).map_err(error)?;
    let budget =
        space::SpaceBudget::for_conversion(acquired.size, report.payload_bytes().map_err(error)?, &sizes)
            .map_err(error)?;
    let evidence = budget
        .inspect(deps.paths.root(), &inputs.game.install_path)
        .map_err(error)?;
    store::write_atomic(
        &reports.join("space.json"),
        &serde_json::to_vec(&evidence).map_err(error)?,
        65536,
    )
    .map_err(error)?;
    store::write_atomic(
        &reports.join("report.json"),
        &report_bytes,
        exe_report::MAX_REPORT_BYTES as u64,
    )
    .map_err(error)?;
    operation::stage(
        "extracting",
        Some(report.payload_bytes().map_err(error)?),
        Some(report.payload().len()),
    );
    let b = report.binding();
    let conversion = json!({"action":"convert_behavior","exe_sha256":b.exe_sha256,"exe_size":b.exe_size,
        "metadata_sha256":b.metadata_sha256,"behavior_sha256":b.behavior_sha256,"payload":report.payload()});
    let converted = deps
        .runtime
        .execute(
            &acquired.path,
            &serde_json::to_vec(&conversion).map_err(error)?,
            &control.cancel,
        )
        .map_err(|e| {
            if control.cancel.load(std::sync::atomic::Ordering::Acquire) {
                Error::code("cancelled")
            } else {
                error(e)
            }
        })?;
    let artifact = converted
        .artifact_path()
        .ok_or_else(|| Error::code("exe_conversion_failed"))?
        .to_path_buf();
    let digest = validate_artifact(&artifact, &converted.response, &report, &control.cancel)
        .map_err(error)?;
    let mut mappings = Vec::new();
    for file in report.payload() {
        let target = inputs.game.install_path.join(&file.path);
        let original = if target.exists() {
            Some(
                facts::digest_file(&target, &control.cancel)
                    .map_err(error)?
                    .sha256,
            )
        } else {
            None
        };
        mappings.push(FileMapping {
            source: file.path.clone(),
            destination: file.path.clone(),
            original_sha256: original,
            installed_sha256: file.sha256.clone(),
        });
    }
    let mut selection = inputs.selection.clone();
    selection.release_id = content.clone();
    let normalized = acquire::AcquiredArchive {
        path: artifact.clone(),
        sha256: digest.sha256.clone(),
        size: digest.size,
        final_url: acquired.final_url.clone(),
        source: acquire::Acquisition::Downloaded,
    };
    let mut release = common::build_release(&selection, &normalized, &mappings, &expectation);
    release.category = "publisher_exe".into();
    boundary(deps, control)?;
    // Download and normalized ZIP already occupy disk. Prepare and apply each
    // check the remaining ZIP transaction budget, without reserving them again.
    let plan = install::prepare_for_request_at(
        deps.paths.root(),
        inputs.game,
        &release,
        &artifact,
        0,
        inputs.operation,
        Some(&inputs.selection.selection_id),
        Some(&content),
        Some(inputs.request_id),
    )
    .map_err(common::map_plan_error)?;
    // Any error from here discards/recoveries the plan before reporting a terminal result.
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
        if !plan.blockers.is_empty() {
            return Err(Error::new("plan_has_blockers", plan.blockers.join(",")));
        }
        boundary(deps, control)?;
        let content_binding = ContentBinding::publisher_exe(
            &content,
            &b.exe_sha256,
            b.exe_size,
            &b.metadata_sha256,
            &b.payload_sha256,
            &digest.sha256,
            digest.size,
            Some("sf9-audit-v3"),
            &mappings,
        );
        let auth = authorization::authorize(AuthorizeInput {
            release_id: &content,
            selection_id: Some(&inputs.selection.selection_id),
            source_id: &inputs.selection.source_id,
            source_record_id: Some(&inputs.selection.source_record_id),
            candidate_sha256: &inputs.selection.candidate_sha256,
            rule_fingerprint: &inputs.selection.rule_fingerprint,
            settings_revision: inputs.selection.settings_revision,
            conditions_sha256: &snapshot.sha256(),
            content: content_binding,
            policy: PolicyVersions {
                adapter_version: 2,
                report_schema_version: 2,
                ..PolicyVersions::default()
            },
            plan_id: &plan.plan_id,
            task_id: Some(&control.id),
            request_id: Some(inputs.request_id),
            facts: game.clone(),
            now: now(),
        })
        .map_err(error)?;
        authorization::write(deps.paths, &auth).map_err(error)?;
        store::write_atomic(
            &deps
                .paths
                .job(&plan.plan_id)
                .map_err(error)?
                .join("condition.json"),
            &serde_json::to_vec(&snapshot).map_err(error)?,
            exe_report::MAX_REPORT_BYTES as u64,
        )
        .map_err(error)?;
        let sources = Sources::open(deps.state_dir).map_err(error)?;
        let query = GameQuery {
            appid: inputs.game.appid,
            name: inputs.game.name.clone(),
            localized_name: inputs.game.localized_name.clone(),
            instance_id: Some(inputs.game.instance_id.clone()),
        };
        let fresh =
            exe_resolve::resolve_exe(sources.registry(), &query, &inputs.selection.selection_id)
                .map_err(error)?;
        let current_facts =
            facts::game_facts_unreviewed(inputs.game, &expectation, &control.cancel);
        let current_condition = report
            .evaluate_with_reinstall(
                &current_facts,
                &control.cancel,
                Some(&content),
                reinstall.as_ref(),
            )
            .map_err(error)?;
        let saved = facts::read_file(
            &reports.join("report.json"),
            exe_report::MAX_REPORT_BYTES as u64,
            &control.cancel,
        )
        .map_err(error)?
        .1;
        if saved != report_bytes {
            return Err(Error::code("authorization_report_changed"));
        }
        exe_report::validate_response(
            &serde_json::from_slice(&saved).map_err(error)?,
            &acquired.sha256,
            acquired.size,
        )
        .map_err(error)?;
        if validate_artifact(&artifact, &converted.response, &report, &control.cancel)
            .map_err(error)?
            != digest
        {
            return Err(Error::code("authorization_content_changed"));
        }
        let stored = install::get_plan_at(deps.paths.root(), &plan.plan_id).map_err(error)?;
        let current_content = ContentBinding::publisher_exe(
            &content,
            &b.exe_sha256,
            b.exe_size,
            &b.metadata_sha256,
            &b.payload_sha256,
            &digest.sha256,
            digest.size,
            Some("sf9-audit-v3"),
            &common::plan_mappings(&stored),
        );
        boundary(deps, control)?;
        let current = CurrentState {
            now: now(),
            plan_id: plan.plan_id.clone(),
            instance_id: inputs.game.instance_id.clone(),
            revoked: authorization::is_revoked(deps.paths, &plan.plan_id).map_err(error)?,
            facts: current_facts,
            settings_revision: fresh.settings_revision,
            policy: PolicyVersions {
                adapter_version: 2,
                report_schema_version: 2,
                ..PolicyVersions::default()
            },
            candidate_sha256: fresh.candidate_sha256,
            rule_fingerprint: fresh.rule_fingerprint,
            conditions_sha256: current_condition.sha256(),
            content: current_content,
        };
        authorization::validate(&auth, &current).map_err(error)?;
        install::retain_plan_archive_at(deps.paths.root(), &plan.plan_id).map_err(error)?;
        drop(converted);
        boundary(deps, control)?;
        install::apply_at(deps.paths.root(), &plan.plan_id).map_err(common::map_plan_error)
    })();
    let transaction = match outcome {
        Ok(t) => t,
        Err(e) => {
            let cause = (deps.cancel_reason)().or_else(|| control.cancellation_cause());
            control
                .revoke_for_failure()
                .map_err(|r| Error::new("needs_recovery", format!("{e}; {r}")))?;
            return common::finalize_plan_failure(
                deps.paths,
                inputs.request_id,
                binding,
                &content,
                &plan.plan_id,
                &e,
                cause,
            );
        }
    };
    let transaction = common::normalize_rolled_back(
        deps.paths,
        transaction,
        (deps.cancel_reason)().or_else(|| control.cancellation_cause()),
    )
    .map_err(error)?;
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
fn reinstall_evidence(
    paths: &InstallerPaths,
    game: &GameInstall,
    report: &PreparedReport,
) -> Result<Option<ReinstallEvidence>> {
    let transactions = install::list_transactions_at(paths.root()).map_err(error)?;
    let mut matched = None;
    for t in transactions
        .into_iter()
        .rev()
        .filter(|t| t.instance_id == game.instance_id && t.state == "committed")
    {
        let Some(auth) = authorization::load(paths, &t.plan_id).map_err(error)? else {
            continue;
        };
        if auth.content.exe_sha256.as_deref() == Some(report.binding().exe_sha256.as_str())
            && auth.content.metadata_sha256.as_deref()
                == Some(report.binding().metadata_sha256.as_str())
            && auth.content.exe_size == Some(report.binding().exe_size)
        {
            matched = Some((t, auth));
            break;
        }
    }
    let Some((t, auth)) = matched else {
        return Ok(None);
    };
    for owned in &auth.content.mappings {
        let actual = facts::digest_file(
            &game.install_path.join(&owned.destination),
            &AtomicBool::new(false),
        )
        .map_err(error)?;
        if actual.sha256 != owned.installed_sha256 {
            return Err(Error::code("reinstall_unowned_or_incomplete"));
        }
    }
    let bytes = facts::read_file(
        &paths.job(&t.plan_id).map_err(error)?.join("condition.json"),
        exe_report::MAX_REPORT_BYTES as u64,
        &AtomicBool::new(false),
    )
    .map_err(error)?
    .1;
    let snapshot: ConditionSnapshot = serde_json::from_slice(&bytes).map_err(error)?;
    Ok(Some(
        ReinstallEvidence::from_committed(&t, &auth, &snapshot).map_err(error)?,
    ))
}
pub fn validate_artifact(
    path: &Path,
    response: &Value,
    report: &PreparedReport,
    cancel: &AtomicBool,
) -> io::Result<facts::FileDigest> {
    let obj = response
        .as_object()
        .ok_or_else(|| super::invalid("invalid_conversion_response"))?;
    let expected = [
        "extractor",
        "exe_sha256",
        "metadata_sha256",
        "artifact_sha256",
        "artifact_size",
        "files",
    ];
    if obj.len() != expected.len()
        || expected.iter().any(|k| !obj.contains_key(*k))
        || response["extractor"] != "sf9-audit-v3"
        || response["exe_sha256"] != report.binding().exe_sha256
        || response["metadata_sha256"] != report.binding().metadata_sha256
        || response["files"] != serde_json::to_value(report.payload())?
    {
        return Err(super::invalid("conversion_binding_changed"));
    }
    let digest = facts::digest_file(path, cancel)?;
    if response["artifact_sha256"] != digest.sha256 || response["artifact_size"] != digest.size {
        return Err(super::invalid("conversion_artifact_changed"));
    }
    let mut archive = zip::ZipArchive::new(facts::open_nofollow(path)?)
        .map_err(|_| super::invalid("invalid_normalized_zip"))?;
    let mut entries = report.payload().to_vec();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    if archive.len() != entries.len() {
        return Err(super::invalid("normalized_payload_changed"));
    }
    use sha2::{Digest, Sha256};
    let mut buffer = [0u8; 65536];
    for (i, p) in entries.iter().enumerate() {
        let mut file = archive
            .by_index(i)
            .map_err(|_| super::invalid("invalid_normalized_zip"))?;
        if file.name() != p.path
            || file.size() != p.size
            || file.compression() != zip::CompressionMethod::Stored
            || file.is_dir()
            || file.unix_mode().is_some_and(|m| m & 0o170000 != 0o100000)
        {
            return Err(super::invalid("normalized_payload_changed"));
        }
        let mut hash = Sha256::new();
        let mut total = 0;
        loop {
            operation::check()?;
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Err(super::invalid("cancelled"));
            }
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > p.size {
                return Err(super::invalid("normalized_payload_changed"));
            }
            hash.update(&buffer[..count]);
        }
        if total != p.size || format!("{:x}", hash.finalize()) != p.sha256 {
            return Err(super::invalid("normalized_payload_changed"));
        }
    }
    Ok(digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer::{
        acquire::{FakeTransport, HttpResponse},
        orchestrator::tests as fixture,
    };
    use crate::sources::{
        model::{ArtifactFormat, DiscoveryRecord},
        registry::DiscoveryOutcome,
    };
    use std::{fs, path::PathBuf};
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
        let mut sources = Sources::open(state).unwrap();
        let rule = sources
            .registry()
            .rules()
            .iter()
            .find(|r| r.id == "kagura-cn")
            .unwrap()
            .clone();
        sources
            .registry_mut()
            .commit(
                &rule,
                candidate.game_appid.unwrap(),
                DiscoveryOutcome {
                    status: "complete".into(),
                    candidates: vec![candidate],
                    ..Default::default()
                },
                &DiscoveryRecord::default(),
                "test",
            )
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
        state: &Path,
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
            state_dir: state,
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
    fn production_available_real_samples_admitted_and_converted() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let stage = tempfile::tempdir().unwrap();
        let runtime = runtime(stage.path());
        assert!(runtime.probe().available);
        let cancel = AtomicBool::new(false);
        let mut evidence = Vec::new();
        for appid in [2303800u32, 3606360, 3721970] {
            let archive = root.join(format!(".artifacts/generic-exe-local/{appid}-archive.exe"));
            let exe = facts::digest_file(&archive, &cancel).unwrap();
            let descriptor =
                json!({"action":"analyze_behavior","exe_sha256":exe.sha256,"exe_size":exe.size});
            let analysis = runtime
                .execute(&archive, &serde_json::to_vec(&descriptor).unwrap(), &cancel)
                .unwrap();
            let report =
                exe_report::validate_response(&analysis.response, &exe.sha256, exe.size).unwrap();
            drop(analysis);
            let b = report.binding();
            let descriptor = json!({"action":"convert_behavior","exe_sha256":b.exe_sha256,"exe_size":b.exe_size,
                "metadata_sha256":b.metadata_sha256,"behavior_sha256":b.behavior_sha256,"payload":report.payload()});
            let output = runtime
                .execute(&archive, &serde_json::to_vec(&descriptor).unwrap(), &cancel)
                .unwrap();
            let artifact = validate_artifact(
                output.artifact_path().unwrap(),
                &output.response,
                &report,
                &cancel,
            )
            .unwrap();
            evidence.push(json!({"appid":appid,"exe_sha256":exe.sha256,"metadata_sha256":b.metadata_sha256,"behavior_sha256":b.behavior_sha256,
                "artifact_sha256":artifact.sha256,"artifact_size":artifact.size,"payload_files":report.payload().len()}));
            drop(output);
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
    #[ignore = "actual LPAC/bwrap fixture; set SF_WORKER_TEST_BIN (release profile)"]
    fn sandbox_to_authorization_install_retry_reinstall_and_external_edit() {
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
        assert!(runtime.probe().available);
        let selection = seed(&state);
        let cancel = AtomicBool::new(false);
        let first = invoke(
            &state,
            &paths,
            &game,
            &selection,
            &runtime,
            "exe-first",
            InstallOperation::Install,
            &cancel,
        )
        .unwrap();
        assert_eq!(first.state, "committed");
        assert!(first.release_id.starts_with("exe-content-"));
        // The dynamic EXE transaction binds both the selection identity and the
        // EXE content id (p1-host-contract §6.3).
        assert_eq!(
            first.selection_id.as_deref(),
            Some(selection.selection_id.as_str())
        );
        assert_eq!(first.content_id.as_deref(), Some(first.release_id.as_str()));
        assert_eq!(
            fs::read(game.install_path.join("payload.bin")).unwrap(),
            b"generic-content"
        );
        let replay = invoke(
            &state,
            &paths,
            &game,
            &selection,
            &runtime,
            "exe-first",
            InstallOperation::Install,
            &cancel,
        )
        .unwrap();
        assert_eq!(first.transaction_id, replay.transaction_id);
        for request in ["exe-reinstall-1", "exe-reinstall-2"] {
            let t = invoke(
                &state,
                &paths,
                &game,
                &selection,
                &runtime,
                request,
                InstallOperation::Reinstall,
                &cancel,
            )
            .unwrap();
            assert_eq!(t.state, "committed");
            assert_eq!(first.release_id, t.release_id);
        }
        // The current payload is already patched, but a real script CRC
        // condition no longer holds. Overwrite must not borrow old conditions.
        fs::write(game.install_path.join("identity.dat"), b"changed").unwrap();
        let overwrite_error = invoke(
            &state,
            &paths,
            &game,
            &selection,
            &runtime,
            "exe-overwrite-crc-mismatch",
            InstallOperation::Overwrite,
            &cancel,
        )
        .unwrap_err();
        assert_eq!(overwrite_error.code, "exe_conditions_not_satisfied");
        assert_eq!(
            fs::read(game.install_path.join("payload.bin")).unwrap(),
            b"generic-content"
        );
        fs::write(game.install_path.join("identity.dat"), [87, 164, 242, 220]).unwrap();
        fs::write(game.install_path.join("payload.bin"), b"externally edited").unwrap();
        assert!(invoke(
            &state,
            &paths,
            &game,
            &selection,
            &runtime,
            "exe-edited",
            InstallOperation::Reinstall,
            &cancel
        )
        .is_err());
        assert_eq!(
            fs::read(game.install_path.join("payload.bin")).unwrap(),
            b"externally edited"
        );
        assert_eq!(fs::read_dir(&stage).unwrap().count(), 0);
        assert_eq!(task_dir_entries(&paths), 0);
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
    #[test]
    fn normalized_response_rejects_unknown_output_and_artifact_tamper() {
        let c = golden()["cases"][0].clone();
        let response = json!({"schema_version":1,"action":"analyze_behavior","exe_sha256":c["descriptor"]["exe_sha256"],"report":c["report"],"installable":false});
        let prepared = exe_report::validate_response(
            &response,
            c["descriptor"]["exe_sha256"].as_str().unwrap(),
            c["descriptor"]["exe_size"].as_u64().unwrap(),
        )
        .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("normalized.zip");
        fs::write(&archive, b"tampered").unwrap();
        let mut conversion = c["conversion"].clone();
        assert!(
            validate_artifact(&archive, &conversion, &prepared, &AtomicBool::new(false)).is_err()
        );
        conversion["path"] = json!("/game");
        assert_eq!(
            validate_artifact(&archive, &conversion, &prepared, &AtomicBool::new(false))
                .unwrap_err()
                .to_string(),
            "conversion_binding_changed"
        );
    }
}
