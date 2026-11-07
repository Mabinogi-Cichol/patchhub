//! Static EXE member extraction into the selected target directory.
use super::{
    acquire, exe_resolve, orchestrator as common, outcome::CancellationCause, InstallerPaths,
};
use crate::{
    install::{self, InstallOperation},
    operation,
    steam::GameInstall,
};
use patchhub_installer_worker::{production::WorkerRuntime, sf::Member, WorkerError};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
pub type Error = common::ZipTaskError;
type Result<T> = std::result::Result<T, Error>;
fn error(error: impl std::fmt::Display) -> Error {
    Error::new("exe_extract_failed", error.to_string())
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
pub(crate) fn execute_exe(
    inputs: ExeInstallInputs<'_>,
    deps: ExeInstallDeps<'_>,
) -> Result<super::InstallExecution> {
    execute_exe_acquired(inputs, deps, None)
}
fn boundary(deps: &ExeInstallDeps<'_>) -> Result<()> {
    if deps.cancel.load(Ordering::Acquire) || (deps.cancel_reason)().is_some() {
        return Err(Error::code("cancelled"));
    }
    operation::check().map_err(|error| {
        if error.to_string() == "task_cancelled" {
            Error::code("cancelled")
        } else {
            common::map_plan_error(error)
        }
    })
}
fn worker_error(error: WorkerError) -> Error {
    if error.stage == "cancelled"
        || matches!(error.message.as_str(), "cancelled" | "task_cancelled")
    {
        return Error::code("cancelled");
    }
    Error::new(
        if error.stage == "extract" {
            "game_write_failed"
        } else {
            "exe_extract_failed"
        },
        error.to_string(),
    )
}

pub(crate) fn execute_exe_acquired(
    inputs: ExeInstallInputs<'_>,
    deps: ExeInstallDeps<'_>,
    acquired: Option<acquire::AcquiredArchive>,
) -> Result<super::InstallExecution> {
    let mut warnings = Vec::new();
    let _claim = common::claim_request(deps.paths, inputs.request_id, &mut warnings)?;
    let binding = common::request_binding(
        &inputs.game.instance_id,
        Some(&inputs.selection.selection_id),
        None,
        inputs.operation,
        None,
    );
    if let Some(transaction) =
        common::begin_extraction_request(deps.paths, inputs.request_id, &binding, &mut warnings)?
    {
        return Ok(super::InstallExecution::replay(transaction));
    }
    boundary(&deps)?;
    operation::stage("connecting", None, None);
    let request = acquire::DownloadRequest {
        url: &inputs.selection.download_url,
        download_hosts: &inputs.selection.rule.download_hosts,
        expected_sha256: None,
        expected_size: None,
    };
    let acquired = match acquired {
        Some(archive) => archive,
        None => {
            acquire::acquire_exe_with(deps.transport, &request, deps.paths, deps.cancel, |_, _| {})
                .map_err(common::map_download_error)?
        }
    };
    warnings.extend(acquired.warnings);
    boundary(&deps)?;
    operation::stage("reading_archive", None, None);
    let session = deps
        .runtime
        .open_session(&acquired.path, &acquired.sha256, acquired.size, deps.cancel)
        .map_err(worker_error)?;
    let descriptor =
        json!({"action":"list_members", "exe_sha256":acquired.sha256,"exe_size":acquired.size});
    let response = session
        .list_members(
            &serde_json::to_vec(&descriptor).map_err(error)?,
            deps.cancel,
        )
        .map_err(worker_error)?
        .response;
    let members: Vec<Member> = serde_json::from_value(
        response
            .get("members")
            .cloned()
            .ok_or_else(|| error("missing archive members"))?,
    )
    .map_err(error)?;
    if members.is_empty() {
        return Err(error("empty_archive"));
    }
    let files: Vec<(String, u64)> = members
        .iter()
        .map(|member| (member.path.clone(), member.size))
        .collect();
    let total = members.iter().try_fold(0u64, |sum, m| {
        sum.checked_add(m.size)
            .ok_or_else(|| error("archive_size_overflow"))
    })?;
    // Input identity describes history only; it never binds game facts or scripts.
    let content = format!("exe-content-{}", acquired.sha256);
    boundary(&deps)?;
    let mut plan = install::begin_direct_at(
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
    for warning in warnings {
        plan.warning(warning);
    }
    if let Err(error) = common::record_plan(
        deps.paths,
        inputs.request_id,
        &binding,
        &plan.plan_id,
        &content,
    ) {
        plan.warning(format!("request_history_failed: {error}"));
    }
    let mut writer = super::direct::DirectWriter::new(&members, &plan.targets);
    let mut wrote = false;
    let outcome: Result<Vec<(String, String)>> = (|| {
        boundary(&deps)?;
        operation::stage("extracting", Some(total), Some(files.len()));
        let descriptor = json!({"action":"extract_members", "exe_sha256":acquired.sha256,"exe_size":acquired.size});
        let result = session.extract(
            &serde_json::to_vec(&descriptor).map_err(error)?,
            deps.cancel,
            &mut |chunk| writer.feed(chunk),
        );
        wrote = writer.opened_any();
        result.map_err(worker_error)?;
        writer.finish().map_err(worker_error)
    })();
    let mut transaction = match outcome {
        Ok(installed) => {
            install::commit_direct_at(&mut plan, &installed).map_err(common::map_plan_error)?
        }
        Err(error) => {
            let cancelled = deps.cancel.load(Ordering::Acquire)
                || (deps.cancel_reason)().is_some()
                || error.code == "cancelled";
            let code = if cancelled {
                if wrote {
                    "cancelled_incomplete"
                } else {
                    "task_cancelled"
                }
                .to_owned()
            } else if wrote {
                format!("install_incomplete: {error}")
            } else {
                error.to_string()
            };
            install::fail_direct_at(&mut plan, &code).map_err(common::map_plan_error)?
        }
    };
    operation::stage("cleanup", None, None);
    common::record_extraction_terminal(
        deps.paths,
        inputs.request_id,
        &binding,
        &content,
        &mut transaction,
    );
    Ok(super::InstallExecution::fresh(transaction))
}
