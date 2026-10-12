//! Single-task dynamic publisher ZIP orchestration
//! (p1-zip-orchestration §3).
//!
//! `install_zip` runs **inside** an `operation::run` scope (the caller wraps it)
//! and performs: request-idempotency gate → acquire → recipe → facts →
//! authorize → plan → apply, plus lease/revocation/cancellation integration.
//! It never holds `journal.lock` across the network/analysis phases: prepare and
//! apply each open their own short-lived `install::Store`.
//!
//! The signed catalog route reuses the shared helpers here for its ledger and
//! outcome plumbing.

use super::acquire::{self, DownloadRequest, Transport};
use super::authorization::{
    self, authorize, AuthorizeInput, ContentBinding, CurrentState, PolicyVersions,
};
use super::facts::{self, FactsExpectation, GameFacts};
use super::outcome::CancellationCause;
use super::resolve::ResolvedZipCandidate;
use super::store::InstallerPaths;
use super::zip_recipe;
use crate::catalog::{
    Artifact, FileMapping, Fingerprint, PatchRelease, Platform, Recipe, SupportLevel,
};
use crate::install::{self, InstallOperation, Transaction};
use crate::operation;
use crate::sources::{GameQuery, Sources};
use crate::steam::GameInstall;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Structured orchestrator failure carrying a stable host error `code`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipTaskError {
    pub code: String,
    pub detail: Option<String>,
}

impl ZipTaskError {
    pub fn code(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: None,
        }
    }

    pub fn new(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: Some(detail.into()),
        }
    }
}

impl fmt::Display for ZipTaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            Some(detail) => write!(formatter, "{}: {detail}", self.code),
            None => formatter.write_str(&self.code),
        }
    }
}

impl std::error::Error for ZipTaskError {}

pub type ZipTaskResult<T> = std::result::Result<T, ZipTaskError>;

/// Inputs for one dynamic ZIP task (p1-zip-orchestration §3.2).
pub(crate) struct ZipInstallInputs<'a> {
    pub game: &'a GameInstall,
    pub selection: &'a ResolvedZipCandidate,
    pub operation: InstallOperation,
    pub request_id: &'a str,
    pub now: u64,
    pub ttl_seconds: u64,
}

/// Injected environment for one dynamic ZIP task.
pub(crate) struct ZipInstallDeps<'a> {
    pub paths: &'a InstallerPaths,
    /// State directory used to reopen `Sources` for the apply-time re-check.
    pub state_dir: &'a Path,
    pub transport: &'a dyn Transport,
    pub cancel: &'a AtomicBool,
    pub cancel_reason: &'a (dyn Fn() -> Option<CancellationCause> + Sync),
    /// `now -> whether the foreground lease is still active` (seconds). The
    /// orchestrator supplies a *current* timestamp at every check so an expired
    /// lease is observed even when the plan/authorization was created earlier
    /// (p1-host-contract §5.2); a frozen `inputs.now` would make this always true.
    pub lease_active: &'a dyn Fn(u64) -> bool,
    /// Current host foreground-lease deadline in epoch milliseconds, re-read at
    /// each stage boundary so a `renew` is reflected in the persisted
    /// `jobs/<plan_id>/lease.json` (p1-zip-orchestration §3.5).
    pub lease_deadline_ms: &'a (dyn Fn() -> u64 + Sync),
}

fn now_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn operation_name(operation: InstallOperation) -> &'static str {
    match operation {
        InstallOperation::Install => "install",
        InstallOperation::Reinstall => "reinstall",
        InstallOperation::Overwrite => "overwrite",
    }
}

fn is_terminal(state: &str) -> bool {
    [
        "committed",
        "superseded",
        "rolled_back",
        "uninstalled",
        "needs_recovery",
    ]
    .contains(&state)
}

fn internal(error: impl fmt::Display) -> ZipTaskError {
    ZipTaskError::new("plan_failed", error.to_string())
}

/// Builds a ledger binding (p1-zip-orchestration §3.4). `content_id` starts
/// `None` and is back-filled after the plan resolves the content identity.
pub fn request_binding(
    instance_id: &str,
    selection_id: Option<&str>,
    release_id: Option<&str>,
    operation: InstallOperation,
    content_id: Option<&str>,
) -> super::requests::RequestBinding {
    super::requests::RequestBinding {
        instance_id: instance_id.to_string(),
        selection_id: selection_id.map(str::to_string),
        release_id: release_id.map(str::to_string),
        operation: operation_name(operation).to_string(),
        content_id: content_id.map(str::to_string),
    }
}

/// Idempotency gate (p1-zip-orchestration §3.4). Returns `Some(transaction)`
/// when a terminal record must be replayed, `None` when the caller should
/// proceed, and a conflict/interrupted error otherwise. On a new request it
/// inserts the binding with a null `content_id`.
pub fn begin_request(
    paths: &InstallerPaths,
    request_id: &str,
    binding: &super::requests::RequestBinding,
) -> ZipTaskResult<Option<Transaction>> {
    let ledger = super::requests::RequestLedger::open(paths).map_err(internal)?;
    let Some(record) = ledger.get(request_id).map_err(internal)? else {
        let now = now_unix();
        let record = super::requests::RequestRecord {
            request_id: request_id.to_string(),
            binding: binding.clone(),
            transaction_id: None,
            plan_id: None,
            state: None,
            error: None,
            created_at: now,
            updated_at: now,
        };
        ledger.record(&record).map_err(internal)?;
        return Ok(None);
    };
    // Compare (instance_id, operation, selection identity) first (N6 step 1).
    if record.binding.instance_id != binding.instance_id
        || record.binding.operation != binding.operation
        || record.binding.selection_id != binding.selection_id
        || record.binding.release_id != binding.release_id
    {
        return Err(ZipTaskError::code("request_id_binding_conflict"));
    }
    if let Some(plan_id) = record.plan_id.as_deref() {
        let transactions = install::list_transactions_at(paths.root()).map_err(internal)?;
        if let Some(transaction) = transactions
            .into_iter()
            .find(|item| item.plan_id == plan_id)
        {
            if is_terminal(&transaction.state) {
                return Ok(Some(transaction));
            }
        }
        return Err(ZipTaskError::code(
            "install_request_in_progress_or_interrupted",
        ));
    }
    // A failed ledger backfill must still replay its real journal transaction.
    // Only new request-derived UUIDs use this fallback; legacy random plans use
    // the original ledger plan_id above. The existing binding remains mandatory.
    let reserved = super::requests::request_plan_id(request_id);
    let transactions = install::list_transactions_at(paths.root()).map_err(internal)?;
    if let Some(transaction) = transactions.iter().find(|t| t.plan_id == reserved) {
        let plan = install::get_plan_ro(paths.root(), &reserved).map_err(internal)?;
        if plan.instance_id != binding.instance_id
            || plan.install.instance_id != binding.instance_id
            || transaction.instance_id != binding.instance_id
            || plan.release_id != transaction.release_id
            || binding.content_id.as_ref().is_some_and(|content| {
                transaction
                    .content_id
                    .as_ref()
                    .or(plan.recipe_sha256.as_ref())
                    != Some(content)
            })
            || operation_name(plan.operation) != binding.operation
            || transaction.selection_id != binding.selection_id
            || binding
                .release_id
                .as_ref()
                .is_some_and(|id| id != &transaction.release_id)
        {
            return Err(ZipTaskError::code("request_id_binding_conflict"));
        }
        if is_terminal(&transaction.state) {
            return Ok(Some(transaction.clone()));
        }
        return Err(ZipTaskError::code(
            "install_request_in_progress_or_interrupted",
        ));
    }
    // No plan yet: an unrelated non-terminal transaction for the same instance
    // means an interrupted write the host must recover first.
    let transactions = install::list_transactions_at(paths.root()).map_err(internal)?;
    if transactions
        .iter()
        .any(|item| item.instance_id == binding.instance_id && !is_terminal(&item.state))
    {
        return Err(ZipTaskError::code(
            "install_request_in_progress_or_interrupted",
        ));
    }
    Ok(None)
}

/// Whether an already-resolved `content_id` for this request conflicts with the
/// content resolved now (p1-zip-orchestration §3.4 step 3).
pub fn content_conflict(
    paths: &InstallerPaths,
    request_id: &str,
    content_id: &str,
) -> ZipTaskResult<bool> {
    let ledger = super::requests::RequestLedger::open(paths).map_err(internal)?;
    Ok(ledger
        .get(request_id)
        .map_err(internal)?
        .and_then(|record| record.binding.content_id)
        .is_some_and(|existing| existing != content_id))
}

/// Back-fills `plan_id` / `content_id` once the plan exists (step 5).
pub fn record_plan(
    paths: &InstallerPaths,
    request_id: &str,
    binding: &super::requests::RequestBinding,
    plan_id: &str,
    content_id: &str,
) -> ZipTaskResult<()> {
    let ledger = super::requests::RequestLedger::open(paths).map_err(internal)?;
    let now = now_unix();
    let mut binding = binding.clone();
    binding.content_id = Some(content_id.to_string());
    let record = super::requests::RequestRecord {
        request_id: request_id.to_string(),
        binding,
        transaction_id: None,
        plan_id: Some(plan_id.to_string()),
        state: Some("planned".to_string()),
        error: None,
        created_at: now,
        updated_at: now,
    };
    ledger.record(&record).map_err(internal)
}

/// Back-fills the terminal transaction summary (step 5).
pub fn record_terminal(
    paths: &InstallerPaths,
    request_id: &str,
    binding: &super::requests::RequestBinding,
    content_id: &str,
    transaction: &Transaction,
) -> ZipTaskResult<()> {
    let ledger = super::requests::RequestLedger::open(paths).map_err(internal)?;
    let now = now_unix();
    let mut binding = binding.clone();
    binding.content_id = Some(content_id.to_string());
    let record = super::requests::RequestRecord {
        request_id: request_id.to_string(),
        binding,
        transaction_id: Some(transaction.transaction_id.clone()),
        plan_id: Some(transaction.plan_id.clone()),
        state: Some(transaction.state.clone()),
        error: transaction.error.clone(),
        created_at: now,
        updated_at: now,
    };
    ledger.record(&record).map_err(internal)
}

pub(crate) fn map_download_error(error: io::Error) -> ZipTaskError {
    let code = error.to_string();
    if code == "task_cancelled" || code == "cancelled" {
        return ZipTaskError::code("download_cancelled");
    }
    if code.starts_with("insufficient_space:") {
        return ZipTaskError::new("insufficient_space", code);
    }
    if [
        "download_cancelled",
        "download_hash_mismatch",
        "download_size_limit",
        "download_size_overflow",
    ]
    .contains(&code.as_str())
    {
        ZipTaskError::code(code)
    } else {
        ZipTaskError::new("download_failed", code)
    }
}

fn map_recipe_error(error: io::Error) -> ZipTaskError {
    let code = error.to_string();
    if code == "download_cancelled" {
        return ZipTaskError::code("cancelled");
    }
    if code == "zip_metadata_limit" {
        return ZipTaskError::new("resource_limit", code);
    }
    if code == "zip_metadata_invalid" {
        return ZipTaskError::code(code);
    }
    if ["zip_size_overflow"].contains(&code.as_str()) {
        ZipTaskError::code(code)
    } else {
        ZipTaskError::new("analysis_failed", code)
    }
}

pub(crate) fn map_plan_error(error: io::Error) -> ZipTaskError {
    let code = error.to_string();
    if code.starts_with("insufficient_space:") {
        return ZipTaskError::new("insufficient_space", code);
    }
    if code == "plan_metadata_limit" {
        return ZipTaskError::new("resource_limit", code);
    }
    if [
        "insufficient_space",
        "zip_size_overflow",
        "installer_busy",
        "game_busy",
        "managed_install_exists_or_needs_recovery",
        "needs_recovery",
        "reinstall_content_identity_changed",
        "reinstall_unowned_or_incomplete",
        "plan_expired",
        "plan_has_blockers",
        "release_not_compatible",
    ]
    .contains(&code.as_str())
    {
        ZipTaskError::code(code)
    } else {
        ZipTaskError::new("plan_failed", code)
    }
}

fn map_fact_failure(failure: &facts::FactFailure) -> ZipTaskError {
    ZipTaskError::new(failure.code.clone(), failure.detail.clone())
}

pub(crate) fn build_release(
    selection: &ResolvedZipCandidate,
    acquired: &acquire::AcquiredArchive,
    mappings: &[FileMapping],
    expectation: &FactsExpectation,
) -> PatchRelease {
    let detection: Vec<Fingerprint> = mappings
        .iter()
        .map(|mapping| Fingerprint {
            path: mapping.destination.clone(),
            sha256: mapping.installed_sha256.clone(),
        })
        .collect();
    PatchRelease {
        patch_id: selection.release_id.clone(),
        release_id: selection.release_id.clone(),
        appid: selection.appid,
        name: selection.candidate.patch_title.clone(),
        version: "0".to_string(),
        language: expectation.language.clone(),
        category: "publisher_zip".to_string(),
        source_page: selection.candidate.source_page.clone(),
        // `prepare_with_operation_at` hard-rejects anything but `ManagedZip`, so
        // the dynamic publisher-ZIP routing borrows that support level here; the
        // projected `catalog::PatchMatch.support_level` remains `PublisherZip`.
        support_level: SupportLevel::ManagedZip,
        platforms: vec![Platform::Windows, Platform::Steamos],
        verified_buildids: expectation.verified_buildids.clone(),
        detection,
        recipe: Some(Recipe {
            artifact_sha256: acquired.sha256.clone(),
            files: mappings.to_vec(),
        }),
        artifact: Some(Artifact {
            url: acquired.final_url.clone(),
            size: acquired.size,
            sha256: acquired.sha256.clone(),
            allowed_redirect_hosts: selection.rule.download_hosts.clone(),
        }),
    }
}

pub(crate) fn plan_mappings(plan: &install::InstallPlan) -> Vec<FileMapping> {
    plan.changes
        .iter()
        .map(|change| FileMapping {
            source: change.source.clone().unwrap_or_else(|| change.path.clone()),
            destination: change.path.clone(),
            original_sha256: change.original_sha256.clone(),
            installed_sha256: change.installed_sha256.clone(),
        })
        .collect()
}

/// Keep the original diagnostic; the single post-plan boundary below owns recovery.
fn fail_after_plan(
    _paths: &InstallerPaths,
    _request_id: &str,
    _binding: &super::requests::RequestBinding,
    _content_id: &str,
    _plan_id: &str,
    code: &str,
    detail: impl Into<String>,
) -> ZipTaskError {
    ZipTaskError::new(code, detail)
}

/// The journal is authoritative after a plan has been bound. A summary write
/// failure must not erase a commit or rollback; replay consults that plan's journal.
pub(crate) fn record_terminal_fact(
    paths: &InstallerPaths,
    request_id: &str,
    binding: &super::requests::RequestBinding,
    content_id: &str,
    transaction: &Transaction,
) {
    let _ = record_terminal(paths, request_id, binding, content_id, transaction);
}

/// Record the recovered transaction rather than replacing its facts with an RPC error.
/// A typed live cause is used only for concrete cancellation failures, never late cancellation.
pub(crate) fn finalize_plan_failure(
    paths: &InstallerPaths,
    request_id: &str,
    binding: &super::requests::RequestBinding,
    content_id: &str,
    plan_id: &str,
    failure: &ZipTaskError,
    live_cause: Option<CancellationCause>,
) -> ZipTaskResult<Transaction> {
    operation::restoring();
    let mut t = finish_failed_plan(paths, plan_id)
        .map_err(|e| ZipTaskError::new("needs_recovery", e.to_string()))?;
    if t.state == "rolled_back" {
        let raw = t.error.as_deref();
        let cancellation_failure = matches!(
            failure.code.as_str(),
            "cancelled"
                | "task_cancelled"
                | "download_cancelled"
                | "lease_expired"
                | "installer_lease_expired"
                | "authorization_lease_inactive"
        );
        let cause = if cancellation_failure {
            live_cause.or_else(|| CancellationCause::from_code(&failure.code))
        } else {
            None
        };
        let persisted = match cause {
            Some(cause) => cause.code().to_owned(),
            None => raw
                .filter(|error| *error != "task_cancelled")
                .map(str::to_owned)
                .unwrap_or_else(|| failure.to_string()),
        };
        t = install::set_terminal_error_at(paths.root(), plan_id, &persisted).map_err(internal)?;
    }
    record_terminal_fact(paths, request_id, binding, content_id, &t);
    Ok(t)
}

/// Normalize cancellations returned by apply itself after successful rollback.
pub(crate) fn normalize_rolled_back(
    paths: &InstallerPaths,
    t: Transaction,
    cause: Option<CancellationCause>,
) -> io::Result<Transaction> {
    if t.state == "rolled_back"
        && t.error
            .as_deref()
            .and_then(CancellationCause::from_transaction_error)
            .is_some()
    {
        if let Some(cause) = cause {
            return install::set_terminal_error_at(paths.root(), &t.plan_id, cause.code());
        }
    }
    Ok(t)
}

/// Complete post-plan failure using the journal's actual state.
pub(crate) fn finish_failed_plan(paths: &InstallerPaths, id: &str) -> io::Result<Transaction> {
    let current = install::list_transactions_at(paths.root())?
        .into_iter()
        .find(|t| t.plan_id == id)
        .ok_or_else(|| super::invalid("transaction_missing"))?;
    match current.state.as_str() {
        "planned" => install::discard_plan_at(paths.root(), id),
        "committed" | "rolled_back" | "needs_recovery" => Ok(current),
        _ => install::recover_plan_at(paths.root(), id)?
            .into_iter()
            .find(|t| t.plan_id == id)
            .ok_or_else(|| super::invalid("transaction_missing")),
    }
}

/// Stage-boundary gate: local cancel flag, then the **live** foreground lease
/// (re-read from the clock), then the in-process operation cancel flag.
fn boundary(deps: &ZipInstallDeps<'_>) -> ZipTaskResult<()> {
    if let Some(cause) = (deps.cancel_reason)() {
        return Err(ZipTaskError::code(cause.code()));
    }
    if deps.cancel.load(Ordering::Relaxed) {
        return Err(ZipTaskError::code("cancelled"));
    }
    if !(deps.lease_active)(now_unix()) {
        return Err(ZipTaskError::code("installer_lease_expired"));
    }
    operation::check().map_err(map_plan_error)
}

/// Persists the current foreground-lease deadline at
/// `jobs/<plan_id>/lease.json` (`{deadline_ms, renewed_at}`, atomic write;
/// p1-zip-orchestration §3.5). Called when the plan is established and refreshed
/// at the apply-time stage boundary so a `renew` reaches disk.
fn write_lease(deps: &ZipInstallDeps<'_>, plan_id: &str) -> ZipTaskResult<()> {
    let document = serde_json::json!({
        "deadline_ms": (deps.lease_deadline_ms)(),
        // Kept in the same unit as `deadline_ms` (epoch milliseconds).
        "renewed_at": now_unix().saturating_mul(1000),
    });
    let bytes = serde_json::to_vec(&document).map_err(internal)?;
    let path = deps
        .paths
        .job(plan_id)
        .map_err(internal)?
        .join("lease.json");
    super::store::write_atomic(&path, &bytes, 4096).map_err(internal)
}

/// Orchestrates one dynamic publisher ZIP install (p1-zip-orchestration §3.3).
#[cfg(test)]
pub(crate) fn install_zip(
    inputs: ZipInstallInputs<'_>,
    deps: ZipInstallDeps<'_>,
) -> ZipTaskResult<Transaction> {
    execute_zip(inputs, deps).map(|run| run.transaction)
}

pub(crate) fn execute_zip(
    inputs: ZipInstallInputs<'_>,
    deps: ZipInstallDeps<'_>,
) -> ZipTaskResult<super::InstallExecution> {
    if let Some(code) = super::host_identity_blocker() {
        return Err(ZipTaskError::code(code));
    }
    let _claim =
        super::requests::RequestClaim::acquire(deps.paths, inputs.request_id).map_err(internal)?;
    let control = super::task::TaskControl::new(
        deps.paths,
        inputs.request_id,
        &inputs.game.instance_id,
        &inputs.selection.selection_id,
        (deps.lease_deadline_ms)(),
    )
    .map_err(internal)?;
    if !(deps.lease_active)(now_unix()) {
        control
            .revoke_with_cause(CancellationCause::LeaseExpired)
            .map_err(internal)?;
        return Err(ZipTaskError::code("cancelled"));
    }
    let requested = deps.cancel;
    control.run_with_cause(
        requested,
        deps.lease_deadline_ms,
        deps.cancel_reason,
        || {
            let deps = ZipInstallDeps {
                cancel: &control.cancel,
                ..deps
            };
            install_zip_inner(inputs, deps, &control, requested)
        },
    )
}

fn install_zip_inner(
    inputs: ZipInstallInputs<'_>,
    deps: ZipInstallDeps<'_>,
    control: &super::task::TaskControl,
    requested: &AtomicBool,
) -> ZipTaskResult<super::InstallExecution> {
    let binding = request_binding(
        &inputs.game.instance_id,
        Some(&inputs.selection.selection_id),
        None,
        inputs.operation,
        None,
    );
    if let Some(transaction) = begin_request(deps.paths, inputs.request_id, &binding)? {
        return Ok(super::InstallExecution::replay(transaction));
    }

    control
        .check_requested_with_cause(requested, deps.lease_deadline_ms, deps.cancel_reason)
        .map_err(|_| ZipTaskError::code("cancelled"))?;
    // Step 3: connecting + acquire (no journal lock held).
    operation::stage("connecting", None, None);
    boundary(&deps)?;
    let request = DownloadRequest {
        url: &inputs.selection.download_url,
        download_hosts: &inputs.selection.rule.download_hosts,
        expected_sha256: None,
        expected_size: None,
    };
    let acquired =
        acquire::acquire_with(deps.transport, &request, deps.paths, deps.cancel, |_, _| {})
            .map_err(map_download_error)?;

    control
        .check_requested_with_cause(requested, deps.lease_deadline_ms, deps.cancel_reason)
        .map_err(|_| ZipTaskError::code("download_cancelled"))?;
    // Step 4: recipe.
    let mappings = zip_recipe::build_recipe(&acquired.path, &inputs.game.install_path, deps.cancel)
        .map_err(map_recipe_error)?;

    // Step 5: facts.
    let buildid = inputs.game.buildid.as_deref();
    let mut expectation = inputs.selection.expectation(buildid);
    if expectation.language.is_empty() {
        if let Some(language) = facts::current_language(inputs.game, deps.cancel) {
            expectation.language = language;
        }
    }
    let facts = facts::game_facts(inputs.game, &expectation, deps.cancel);
    if !facts.is_valid() {
        return Err(match facts.failures().first() {
            Some(failure) => map_fact_failure(failure),
            None => ZipTaskError::code("game_not_ready"),
        });
    }

    // Step 6: dynamic release (in memory, never a signed catalog).
    let release = build_release(inputs.selection, &acquired, &mappings, &expectation);
    let identity = install::recipe_identity(&release).map_err(internal)?;
    let conditions_sha256 = inputs.selection.conditions_sha256(buildid);
    if content_conflict(deps.paths, inputs.request_id, &identity)? {
        return Err(ZipTaskError::code("request_id_binding_conflict"));
    }

    control
        .check_requested_with_cause(requested, deps.lease_deadline_ms, deps.cancel_reason)
        .map_err(|_| ZipTaskError::code("cancelled"))?;
    // Step 7: plan + authorize + re-check (journal.lock held only inside prepare).
    boundary(&deps)?;
    let plan = install::prepare_for_request_at(
        deps.paths.root(),
        inputs.game,
        &release,
        &acquired.path,
        0,
        inputs.operation,
        Some(&inputs.selection.selection_id),
        None,
        Some(inputs.request_id),
    )
    .map_err(map_plan_error)?;
    let outcome = (|| {
        if let Err(e) = control.attach_plan(deps.paths, &plan.plan_id) {
            return Err(internal(e));
        }
        if !plan.blockers.is_empty() {
            return Err(ZipTaskError::new(
                "plan_has_blockers",
                plan.blockers.join(","),
            ));
        }
        record_plan(
            deps.paths,
            inputs.request_id,
            &binding,
            &plan.plan_id,
            &identity,
        )?;
        // The plan exists: persist the foreground lease for recovery/audit
        // (p1-zip-orchestration §3.5).
        if let Err(error) = write_lease(&deps, &plan.plan_id) {
            return Err(fail_after_plan(
                deps.paths,
                inputs.request_id,
                &binding,
                &identity,
                &plan.plan_id,
                "plan_failed",
                error.to_string(),
            ));
        }

        let content = ContentBinding::publisher_zip(&acquired.sha256, acquired.size, &mappings);
        let auth = authorize(AuthorizeInput {
            release_id: &release.release_id,
            selection_id: Some(&inputs.selection.selection_id),
            source_id: &inputs.selection.source_id,
            source_record_id: Some(&inputs.selection.source_record_id),
            candidate_sha256: &inputs.selection.candidate_sha256,
            rule_fingerprint: &inputs.selection.rule_fingerprint,
            settings_revision: inputs.selection.settings_revision,
            conditions_sha256: &conditions_sha256,
            content,
            policy: PolicyVersions::default(),
            plan_id: &plan.plan_id,
            task_id: None,
            request_id: Some(inputs.request_id),
            facts: facts.clone(),
            now: inputs.now,
            ttl_seconds: inputs.ttl_seconds,
        })
        .map_err(|invalidation| {
            fail_after_plan(
                deps.paths,
                inputs.request_id,
                &binding,
                &identity,
                &plan.plan_id,
                &invalidation.code,
                invalidation.to_string(),
            )
        })?;
        if let Err(error) = authorization::write(deps.paths, &auth) {
            return Err(fail_after_plan(
                deps.paths,
                inputs.request_id,
                &binding,
                &identity,
                &plan.plan_id,
                "plan_failed",
                error.to_string(),
            ));
        }

        let current = recheck_authorization(&deps, &inputs, &plan, &acquired, &expectation)?;
        if let Err(invalidation) = authorization::validate(&auth, &current) {
            if invalidation.code == "authorization_lease_inactive"
                && !(deps.lease_active)(now_unix())
            {
                control
                    .revoke_with_cause(CancellationCause::LeaseExpired)
                    .map_err(internal)?;
            }
            return Err(fail_after_plan(
                deps.paths,
                inputs.request_id,
                &binding,
                &identity,
                &plan.plan_id,
                &invalidation.code,
                invalidation.to_string(),
            ));
        }

        // Stage boundary after the plan: refresh the persisted lease so a `renew`
        // reaches disk, then re-check cancel/lease/op before apply.
        if let Err(error) = write_lease(&deps, &plan.plan_id) {
            return Err(fail_after_plan(
                deps.paths,
                inputs.request_id,
                &binding,
                &identity,
                &plan.plan_id,
                "plan_failed",
                error.to_string(),
            ));
        }
        let _ = control.check_requested_with_cause(
            requested,
            deps.lease_deadline_ms,
            deps.cancel_reason,
        );
        // The post-plan boundary recovers and persists the concrete cause.
        if deps.cancel.load(Ordering::Relaxed)
            || !(deps.lease_active)(now_unix())
            || operation::check().is_err()
        {
            let cause = (deps.cancel_reason)()
                .or_else(|| control.cancellation_cause())
                .or_else(|| {
                    (!(deps.lease_active)(now_unix())).then_some(CancellationCause::LeaseExpired)
                });
            if let Some(cause) = cause {
                control.revoke_with_cause(cause).map_err(internal)?;
            }
            return Err(ZipTaskError::code(
                cause.map(CancellationCause::code).unwrap_or("cancelled"),
            ));
        }
        // The download must still hash to the authorized artifact before apply.
        let current_size = fs::metadata(&acquired.path).map_err(internal)?.len();
        let artifact =
            facts::digest_file(&acquired.path, current_size, deps.cancel).map_err(internal)?;
        if artifact.sha256 != acquired.sha256 || artifact.size != acquired.size {
            return Err(fail_after_plan(
                deps.paths,
                inputs.request_id,
                &binding,
                &identity,
                &plan.plan_id,
                "authorization_content_changed",
                "download artifact changed after authorization",
            ));
        }
        let transaction =
            install::apply_at(deps.paths.root(), &plan.plan_id).map_err(map_plan_error)?;
        let transaction = normalize_rolled_back(
            deps.paths,
            transaction,
            (deps.cancel_reason)().or_else(|| control.cancellation_cause()),
        )
        .map_err(internal)?;

        operation::stage("cleanup", None, None);
        record_terminal_fact(
            deps.paths,
            inputs.request_id,
            &binding,
            &identity,
            &transaction,
        );
        Ok(transaction)
    })();
    match outcome {
        Ok(t) => Ok(t),
        Err(error) => {
            let cause = (deps.cancel_reason)().or_else(|| control.cancellation_cause());
            control
                .revoke_for_failure()
                .map_err(|e| ZipTaskError::new("needs_recovery", e.to_string()))?;
            finalize_plan_failure(
                deps.paths,
                inputs.request_id,
                &binding,
                &identity,
                &plan.plan_id,
                &error,
                cause,
            )
        }
    }
    .map(super::InstallExecution::fresh)
}

fn recheck_authorization(
    deps: &ZipInstallDeps<'_>,
    inputs: &ZipInstallInputs<'_>,
    plan: &install::InstallPlan,
    acquired: &acquire::AcquiredArchive,
    expectation: &FactsExpectation,
) -> ZipTaskResult<CurrentState> {
    let sources = Sources::open(deps.state_dir).map_err(internal)?;
    let query = GameQuery {
        appid: inputs.game.appid,
        name: inputs.game.name.clone(),
        localized_name: inputs.game.localized_name.clone(),
        instance_id: Some(inputs.game.instance_id.clone()),
    };
    let fresh =
        super::resolve::resolve_zip(sources.registry(), &query, &inputs.selection.selection_id)
            .map_err(|error| {
                ZipTaskError::new(
                    "authorization_candidate_changed",
                    format!("{}: {}", error.code, error),
                )
            })?;
    let facts: GameFacts = facts::game_facts(inputs.game, expectation, deps.cancel);
    let stored = install::get_plan_at(deps.paths.root(), &plan.plan_id).map_err(internal)?;
    let mappings = plan_mappings(&stored);
    let content = ContentBinding::publisher_zip(&acquired.sha256, acquired.size, &mappings);
    let conditions_sha256 = fresh.conditions_sha256(inputs.game.buildid.as_deref());
    Ok(CurrentState {
        now: inputs.now,
        plan_id: plan.plan_id.clone(),
        instance_id: inputs.game.instance_id.clone(),
        revoked: authorization::is_revoked(deps.paths, &plan.plan_id).map_err(internal)?,
        lease_active: (deps.lease_active)(now_unix()),
        facts,
        settings_revision: fresh.settings_revision,
        policy: PolicyVersions::default(),
        candidate_sha256: fresh.candidate_sha256,
        rule_fingerprint: fresh.rule_fingerprint,
        conditions_sha256,
        content,
    })
}

/// Convenience used by the CLI to classify an unexpected internal error.
pub fn installer_error(error: impl fmt::Display) -> ZipTaskError {
    ZipTaskError::new("plan_failed", error.to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::installer::acquire::{FakeTransport, HttpResponse};
    use crate::installer::zip_selection_id;
    use crate::sources::model::{
        ArtifactFormat, CacheStatus, Capability, DiscoveryRecord, MatchBasis, Mirror,
        SourceCandidate, SourceRole, SourceRule,
    };
    use crate::sources::registry::DiscoveryOutcome;
    use crate::sources::{GameQuery, Sources};
    use sha2::Digest;
    use std::fs;
    use std::io::{Read, Write};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    const APPID: u32 = 42;

    fn manifest() -> String {
        format!(
            "\"AppState\" {{ \"appid\" \"{APPID}\" \"name\" \"Test\" \"installdir\" \"Test\" \
             \"buildid\" \"100\" \"StateFlags\" \"4\" \
             \"UserConfig\" {{ \"language\" \"schinese\" }} \
             \"MountedConfig\" {{ \"language\" \"schinese\" }} }}"
        )
    }

    fn build_zip(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut buffer);
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            for (name, data) in members {
                writer.start_file(*name, options).unwrap();
                writer.write_all(data).unwrap();
            }
            writer.finish().unwrap();
        }
        buffer.into_inner()
    }

    pub(crate) fn setup() -> (TempDir, GameInstall, Vec<u8>) {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("Steam");
        let install = library.join("steamapps").join("common").join("Test");
        fs::create_dir_all(&install).unwrap();
        fs::write(
            library
                .join("steamapps")
                .join(format!("appmanifest_{APPID}.acf")),
            manifest(),
        )
        .unwrap();
        let mut report = crate::steam::scan(std::slice::from_ref(&library));
        let game = report.installs.remove(0);
        let archive = build_zip(&[("data.txt", b"patched")]);
        (temp, game, archive)
    }

    pub(crate) fn candidate() -> SourceCandidate {
        SourceCandidate {
            candidate_id: "candidate-000000000000000000000000".to_string(),
            source_id: "kagura-cn".to_string(),
            source_record_id: "https://kagurafan.com/patches/game".to_string(),
            game_appid: Some(APPID),
            parent_appid: None,
            aliases: Vec::new(),
            game_title: "Test".to_string(),
            patch_title: "Test patch".to_string(),
            variant: crate::sources::model::Variant::Base,
            requires_dlc: None,
            excludes_dlc: None,
            platforms: vec!["steam".to_string()],
            patch_kind: "adult_content".to_string(),
            languages: vec!["zh-CN".to_string()],
            version_raw: Some("1".to_string()),
            version_evidence: Vec::new(),
            version_identity: "1".to_string(),
            published_at: None,
            patch_updated_at: None,
            source_page: "https://kagurafan.com/patches/game".to_string(),
            upstream_page: None,
            mirrors: vec![Mirror {
                url: "https://kaguraserver.com/a.zip".to_string(),
                label: None,
                format: "zip".to_string(),
            }],
            format_reported: ArtifactFormat::Zip,
            format_observed: ArtifactFormat::Zip,
            capability: Capability::DiscoverOnly,
            identity_evidence: Vec::new(),
            provenance: Vec::new(),
            match_basis: MatchBasis::Appid,
            observed_at: "2026-01-01T00:00:00+00:00".to_string(),
            cache_status: CacheStatus::Cached,
            warnings: Vec::new(),
            fingerprint: "f".repeat(64),
            related_index_pages: Vec::new(),
        }
    }

    pub(crate) fn query() -> GameQuery {
        GameQuery {
            appid: APPID,
            name: "Test".to_string(),
            localized_name: None,
            instance_id: None,
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        format!("{:x}", sha2::Sha256::digest(bytes))
    }

    /// A source rule with a download host, for a hand-built selection that does
    /// not require a seeded registry (facts/acquire/reinstall-error paths).
    fn source_rule() -> SourceRule {
        SourceRule {
            id: "kagura-cn".to_string(),
            name: "Kagura".to_string(),
            adapter: "kagura_wordpress".to_string(),
            role: SourceRole::Publisher,
            discovery_only: true,
            priority: 10,
            enabled: true,
            pages: vec!["https://kagurafan.com/".to_string()],
            page_hosts: vec!["kagurafan.com".to_string()],
            download_hosts: vec!["kaguraserver.com".to_string()],
            root_layout: None,
            columns: None,
            table_id: None,
            language: None,
            url: None,
        }
    }

    fn manual_selection(
        requires: Option<Vec<u32>>,
        excludes: Option<Vec<u32>>,
    ) -> ResolvedZipCandidate {
        let mut candidate = candidate();
        candidate.requires_dlc = requires;
        candidate.excludes_dlc = excludes;
        ResolvedZipCandidate {
            selection_id: zip_selection_id(&candidate).unwrap(),
            release_id: format!("source-kagura-cn-{APPID}"),
            appid: APPID,
            source_id: "kagura-cn".to_string(),
            source_record_id: candidate.source_record_id.clone(),
            candidate,
            rule: source_rule(),
            download_url: "https://kaguraserver.com/a.zip".to_string(),
            language: "zh-CN".to_string(),
            candidate_sha256: "f".repeat(64),
            rule_fingerprint: "a".repeat(64),
            settings_revision: 0,
        }
    }

    /// Seeds the patchmoe index cache and resolves an index-source candidate
    /// (`rule.discovery_only == false`, language recorded as evidence only).
    fn seed_and_resolve_patchmoe(state: &Path) -> crate::installer::ResolvedZipCandidate {
        let handle = Sources::open(state).unwrap();
        handle
            .store()
            .save_cache(
                "patchmoe",
                &crate::sources::model::SourceCache {
                    entries: vec![crate::sources::model::SourceEntry {
                        title: "Kudos Kuppa".to_string(),
                        aliases: Vec::new(),
                        url: "https://download.patch.moe/Kudos_Kuppa_patch.zip".to_string(),
                        source_page: "https://www.patch.moe/Kudos-Kuppa".to_string(),
                        language: "zh-CN".to_string(),
                        date: Some("2026/6/23".to_string()),
                        appid: Some(APPID),
                    }],
                    etag: None,
                    modified: None,
                    cached_at: Some(1000),
                    checked_at: Some(1000),
                },
            )
            .unwrap();
        drop(handle);
        let sources = Sources::open(state).unwrap();
        let candidate = sources
            .registry()
            .index_candidates(&query())
            .into_iter()
            .next()
            .unwrap();
        let selection_id = zip_selection_id(&candidate).unwrap();
        crate::installer::resolve_zip(sources.registry(), &query(), &selection_id).unwrap()
    }

    fn write_manifest(
        game: &GameInstall,
        language: &str,
        flags: &str,
        buildid: Option<&str>,
        dlc: Option<u32>,
    ) {
        let build = buildid
            .map(|build| format!("\"buildid\" \"{build}\" "))
            .unwrap_or_default();
        let dlc = dlc
            .map(|dlc| {
                format!("\"InstalledDepots\" {{ \"2303801\" {{ \"dlcappid\" \"{dlc}\" }} }}")
            })
            .unwrap_or_default();
        let body = format!(
            "\"AppState\" {{ \"appid\" \"{APPID}\" \"name\" \"Test\" \"installdir\" \"Test\" \
             {build}\"StateFlags\" \"{flags}\" \
             \"UserConfig\" {{ \"language\" \"{language}\" }} \
             \"MountedConfig\" {{ \"language\" \"{language}\" }} {dlc} }}"
        );
        fs::write(
            game.library_path
                .join("steamapps")
                .join(format!("appmanifest_{APPID}.acf")),
            body,
        )
        .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn run_install(
        game: &GameInstall,
        selection: &ResolvedZipCandidate,
        paths: &InstallerPaths,
        state: &Path,
        transport: &dyn Transport,
        cancel: &AtomicBool,
        request_id: &str,
        operation_kind: InstallOperation,
    ) -> ZipTaskResult<Transaction> {
        let lease = |_now: u64| true;
        let lease_deadline = || u64::MAX;
        let inputs = ZipInstallInputs {
            game,
            selection,
            operation: operation_kind,
            request_id,
            now: 1_000,
            ttl_seconds: 900,
        };
        let deps = ZipInstallDeps {
            paths,
            state_dir: state,
            transport,
            cancel,
            cancel_reason: &|| None,
            lease_active: &lease,
            lease_deadline_ms: &lease_deadline,
        };
        operation::run(
            Arc::new(AtomicBool::new(false)),
            |_| {},
            || install_zip(inputs, deps),
        )
    }

    fn dynamic_paths(temp: &TempDir) -> (std::path::PathBuf, InstallerPaths) {
        let state = temp.path().join("state");
        let paths = InstallerPaths::new(&state);
        crate::installer::init_atomic(&paths).unwrap();
        (state, paths)
    }

    /// Seeds a fresh kagura-cn discovery record and returns the resolved
    /// candidate from the same registry.
    fn seed_and_resolve(
        state: &Path,
        selection_id: &str,
    ) -> crate::installer::ResolvedZipCandidate {
        let mut handle = Sources::open(state).unwrap();
        let rule = handle
            .registry()
            .rules()
            .iter()
            .find(|rule| rule.id == "kagura-cn")
            .cloned()
            .unwrap();
        handle
            .registry_mut()
            .commit(
                &rule,
                APPID,
                DiscoveryOutcome {
                    status: "complete".to_string(),
                    candidates: vec![candidate()],
                    ..DiscoveryOutcome::default()
                },
                &DiscoveryRecord::default(),
                "key",
            )
            .unwrap();
        let selection =
            crate::installer::resolve_zip(handle.registry(), &query(), selection_id).unwrap();
        selection
    }

    fn transport(archive: Vec<u8>) -> FakeTransport {
        FakeTransport::new(vec![HttpResponse {
            status: 200,
            location: None,
            content_type: Some("application/octet-stream".to_string()),
            content_length: Some(archive.len() as u64),
            etag: None,
            last_modified: None,
            final_url: "https://kaguraserver.com/a.zip".to_string(),
            body: Box::new(std::io::Cursor::new(archive)),
        }])
    }

    #[test]
    fn metadata_limits_and_invalid_zip_have_distinct_errors() {
        let resource = map_recipe_error(io::Error::other("zip_metadata_limit"));
        assert_eq!(resource.code, "resource_limit");
        assert_eq!(resource.detail.as_deref(), Some("zip_metadata_limit"));
        let invalid = map_recipe_error(io::Error::other("zip_metadata_invalid"));
        assert_eq!(invalid.code, "zip_metadata_invalid");
        let plan = map_plan_error(io::Error::other("plan_metadata_limit"));
        assert_eq!(plan.code, "resource_limit");
        assert_eq!(plan.detail.as_deref(), Some("plan_metadata_limit"));
    }

    #[test]
    fn install_zip_commits_with_the_frozen_stage_sequence() {
        let (temp, game, archive) = setup();
        let state = temp.path().join("state");
        let paths = InstallerPaths::new(&state);
        crate::installer::init_atomic(&paths).unwrap();
        let candidate = candidate();
        let selection_id = zip_selection_id(&candidate).unwrap();
        let selection = seed_and_resolve(&state, &selection_id);
        assert_eq!(selection.release_id, "source-kagura-cn-42");

        let transport = transport(archive);
        let cancel = AtomicBool::new(false);
        let lease = |_now: u64| true;
        let lease_deadline = || u64::MAX;
        let inputs = ZipInstallInputs {
            game: &game,
            selection: &selection,
            operation: InstallOperation::Install,
            request_id: "req-1",
            now: 1_000,
            ttl_seconds: 900,
        };
        let deps = ZipInstallDeps {
            paths: &paths,
            state_dir: &state,
            transport: &transport,
            cancel: &cancel,
            cancel_reason: &|| None,
            lease_active: &lease,
            lease_deadline_ms: &lease_deadline,
        };
        let stages = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&stages);
        let transaction = operation::run(
            Arc::new(AtomicBool::new(false)),
            move |progress| sink.lock().unwrap().push(progress.stage.clone()),
            || install_zip(inputs, deps),
        )
        .unwrap();

        assert_eq!(transaction.state, "committed");
        assert_eq!(transaction.release_id, "source-kagura-cn-42");
        // The dynamic ZIP transaction carries its selection identity but no EXE
        // content id, and the binding is visible through list_transactions.
        assert_eq!(
            transaction.selection_id.as_deref(),
            Some(selection_id.as_str())
        );
        assert_eq!(transaction.content_id, None);
        let listed = install::list_transactions_at(paths.root()).unwrap();
        let listed = listed
            .iter()
            .find(|candidate| candidate.plan_id == transaction.plan_id)
            .expect("committed transaction is listed");
        assert_eq!(listed.selection_id.as_deref(), Some(selection_id.as_str()));
        assert_eq!(listed.content_id, None);
        assert_eq!(
            fs::read(game.install_path.join("data.txt")).unwrap(),
            b"patched"
        );

        let stages = stages.lock().unwrap().clone();
        assert_eq!(stages.first().map(String::as_str), Some("connecting"));
        assert_eq!(stages.last().map(String::as_str), Some("cleanup"));
        for stage in [
            "connecting",
            "downloading",
            "validating",
            "planning",
            "writing",
            "committing",
        ] {
            assert!(
                stages.iter().any(|item| item == stage),
                "missing stage {stage}: {stages:?}"
            );
        }
        assert!(!stages.iter().any(|item| item == "analyzing_installer"));
        assert!(!stages.iter().any(|item| item == "extracting"));

        // The authorization receipt was persisted under the protected job dir.
        assert!(
            crate::installer::authorization_path(&paths, &transaction.plan_id)
                .unwrap()
                .exists()
        );
        assert!(!crate::installer::is_revoked(&paths, &transaction.plan_id).unwrap());
        // The plan-time foreground lease is persisted for recovery/audit.
        assert!(paths
            .job(&transaction.plan_id)
            .unwrap()
            .join("lease.json")
            .exists());
    }

    #[test]
    fn same_request_replays_terminal_transaction() {
        let (temp, game, archive) = setup();
        let state = temp.path().join("state");
        let paths = InstallerPaths::new(&state);
        crate::installer::init_atomic(&paths).unwrap();
        let selection_id = zip_selection_id(&candidate()).unwrap();
        let selection = seed_and_resolve(&state, &selection_id);
        let cancel = AtomicBool::new(false);
        let lease = |_now: u64| true;
        let lease_deadline = || u64::MAX;

        let run = |transport: &dyn Transport| {
            let inputs = ZipInstallInputs {
                game: &game,
                selection: &selection,
                operation: InstallOperation::Install,
                request_id: "req-1",
                now: 1_000,
                ttl_seconds: 900,
            };
            let deps = ZipInstallDeps {
                paths: &paths,
                state_dir: &state,
                transport,
                cancel: &cancel,
                cancel_reason: &|| None,
                lease_active: &lease,
                lease_deadline_ms: &lease_deadline,
            };
            operation::run(
                Arc::new(AtomicBool::new(false)),
                |_| {},
                || install_zip(inputs, deps),
            )
        };
        let first = run(&transport(archive)).unwrap();
        // No responses left: a replay must not touch the transport.
        let empty = FakeTransport::new(Vec::new());
        let second = run(&empty).unwrap();
        assert_eq!(first.transaction_id, second.transaction_id);
        assert_eq!(second.state, "committed");

        // A different operation for the same request id is a binding conflict.
        let inputs = ZipInstallInputs {
            game: &game,
            selection: &selection,
            operation: InstallOperation::Reinstall,
            request_id: "req-1",
            now: 1_000,
            ttl_seconds: 900,
        };
        let deps = ZipInstallDeps {
            paths: &paths,
            state_dir: &state,
            transport: &empty,
            cancel: &cancel,
            cancel_reason: &|| None,
            lease_active: &lease,
            lease_deadline_ms: &lease_deadline,
        };
        let error = operation::run(
            Arc::new(AtomicBool::new(false)),
            |_| {},
            || install_zip(inputs, deps),
        )
        .unwrap_err();
        assert_eq!(error.code, "request_id_binding_conflict");
    }

    #[test]
    fn cancellation_before_a_plan_returns_cancelled_without_a_transaction() {
        let (temp, game, archive) = setup();
        let state = temp.path().join("state");
        let paths = InstallerPaths::new(&state);
        crate::installer::init_atomic(&paths).unwrap();
        let selection_id = zip_selection_id(&candidate()).unwrap();
        let selection = seed_and_resolve(&state, &selection_id);
        let cancel = AtomicBool::new(true);
        let lease = |_now: u64| true;
        let lease_deadline = || u64::MAX;
        let transport = transport(archive);
        let inputs = ZipInstallInputs {
            game: &game,
            selection: &selection,
            operation: InstallOperation::Install,
            request_id: "req-cancel",
            now: 1_000,
            ttl_seconds: 900,
        };
        let deps = ZipInstallDeps {
            paths: &paths,
            state_dir: &state,
            transport: &transport,
            cancel: &cancel,
            cancel_reason: &|| None,
            lease_active: &lease,
            lease_deadline_ms: &lease_deadline,
        };
        let error = operation::run(
            Arc::new(AtomicBool::new(false)),
            |_| {},
            || install_zip(inputs, deps),
        )
        .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert!(install::list_transactions_at(paths.root())
            .unwrap()
            .is_empty());
    }

    struct FailingTransport;

    impl Transport for FailingTransport {
        fn get(
            &self,
            _url: &str,
            _etag: Option<&str>,
            _last_modified: Option<&str>,
        ) -> std::io::Result<HttpResponse> {
            Err(std::io::Error::other("download_http_error"))
        }
    }

    /// A download body that returns one byte per read and flips `cancel` on the
    /// first read, so cancellation is observed mid-download.
    struct OneByteCancelReader {
        data: Vec<u8>,
        offset: usize,
        cancel: Arc<AtomicBool>,
    }

    impl Read for OneByteCancelReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.offset >= self.data.len() || buffer.is_empty() {
                return Ok(0);
            }
            self.cancel.store(true, Ordering::Relaxed);
            buffer[0] = self.data[self.offset];
            self.offset += 1;
            Ok(1)
        }
    }

    struct BodyTransport {
        response: Mutex<Option<HttpResponse>>,
    }

    impl Transport for BodyTransport {
        fn get(
            &self,
            _url: &str,
            _etag: Option<&str>,
            _last_modified: Option<&str>,
        ) -> std::io::Result<HttpResponse> {
            self.response
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| std::io::Error::other("no_response"))
        }
    }

    fn cancelling_transport(data: Vec<u8>, cancel: Arc<AtomicBool>) -> BodyTransport {
        BodyTransport {
            response: Mutex::new(Some(HttpResponse {
                status: 200,
                location: None,
                content_type: Some("application/octet-stream".to_string()),
                content_length: Some(data.len() as u64),
                etag: None,
                last_modified: None,
                final_url: "https://kaguraserver.com/a.zip".to_string(),
                body: Box::new(OneByteCancelReader {
                    data,
                    offset: 0,
                    cancel,
                }),
            })),
        }
    }

    fn edit_journal(root: &Path, plan_id: &str, edit: impl FnOnce(&mut serde_json::Value)) {
        let db = rusqlite::Connection::open(root.join("journal.sqlite")).unwrap();
        let text: String = db
            .query_row("SELECT document FROM jobs WHERE id=?1", [plan_id], |row| {
                row.get(0)
            })
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        edit(&mut value);
        db.execute(
            "UPDATE jobs SET document=?2 WHERE id=?1",
            rusqlite::params![plan_id, value.to_string()],
        )
        .unwrap();
    }

    /// Builds (without preparing) the in-memory dynamic release for a ZIP
    /// selection, mirroring the production orchestrator.
    fn dynamic_release(
        game: &GameInstall,
        selection: &ResolvedZipCandidate,
        archive: &Path,
    ) -> crate::catalog::PatchRelease {
        let cancel = AtomicBool::new(false);
        let mappings = zip_recipe::build_recipe(archive, &game.install_path, &cancel).unwrap();
        let expectation = selection.expectation(game.buildid.as_deref());
        let bytes = fs::read(archive).unwrap();
        let acquired = acquire::AcquiredArchive {
            path: archive.to_path_buf(),
            sha256: sha256_hex(&bytes),
            size: bytes.len() as u64,
            final_url: selection.download_url.clone(),
            source: acquire::Acquisition::Downloaded,
        };
        build_release(selection, &acquired, &mappings, &expectation)
    }

    /// Builds (without applying) a dynamic publisher ZIP plan at the P1 root.
    fn prepare_dynamic_plan(
        paths: &InstallerPaths,
        game: &GameInstall,
        selection: &ResolvedZipCandidate,
        archive: &Path,
        operation_kind: InstallOperation,
    ) -> install::InstallPlan {
        let release = dynamic_release(game, selection, archive);
        install::prepare_with_operation_at(paths.root(), game, &release, archive, 0, operation_kind)
            .unwrap()
    }

    /// The signed/legacy prepare entry point leaves both dynamic binding fields
    /// unset on the persisted transaction.
    #[test]
    fn prepare_with_operation_leaves_dynamic_binding_unset() {
        let (temp, game, archive) = setup();
        let (_state, paths) = dynamic_paths(&temp);
        let archive_path = temp.path().join("dynamic.zip");
        fs::write(&archive_path, &archive).unwrap();
        let selection = manual_selection(None, None);
        let release = dynamic_release(&game, &selection, &archive_path);
        let plan = install::prepare_with_operation_at(
            paths.root(),
            &game,
            &release,
            &archive_path,
            0,
            InstallOperation::Install,
        )
        .unwrap();
        let transactions = install::list_transactions_at(paths.root()).unwrap();
        let transaction = transactions
            .iter()
            .find(|candidate| candidate.plan_id == plan.plan_id)
            .expect("planning writes a transaction");
        assert_eq!(transaction.selection_id, None);
        assert_eq!(transaction.content_id, None);
    }

    /// `prepare_with_binding_at` persists `selection_id`/`content_id` supplied by
    /// a dynamic orchestrator (zip here; the EXE flow uses the same entry point).
    #[test]
    fn prepare_with_binding_records_selection_and_content() {
        let (temp, game, archive) = setup();
        let (_state, paths) = dynamic_paths(&temp);
        let archive_path = temp.path().join("dynamic.zip");
        fs::write(&archive_path, &archive).unwrap();
        let selection = manual_selection(None, None);
        let release = dynamic_release(&game, &selection, &archive_path);
        let plan = install::prepare_with_binding_at(
            paths.root(),
            &game,
            &release,
            &archive_path,
            0,
            InstallOperation::Install,
            Some(&selection.selection_id),
            Some("exe-content-000000000000000000000000"),
        )
        .unwrap();
        let transactions = install::list_transactions_at(paths.root()).unwrap();
        let transaction = transactions
            .iter()
            .find(|candidate| candidate.plan_id == plan.plan_id)
            .expect("planning writes a transaction");
        assert_eq!(
            transaction.selection_id.as_deref(),
            Some(selection.selection_id.as_str())
        );
        assert_eq!(
            transaction.content_id.as_deref(),
            Some("exe-content-000000000000000000000000")
        );
    }

    fn assert_no_transactions(paths: &InstallerPaths) {
        assert!(
            install::list_transactions_at(paths.root())
                .unwrap()
                .is_empty(),
            "a plan-phase failure must not write a transaction"
        );
    }

    #[test]
    fn facts_requires_dlc_missing_blocks_without_a_transaction() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        write_manifest(&game, "schinese", "4", Some("100"), None);
        let selection = manual_selection(Some(vec![2_303_802]), None);
        let transport = transport(archive);
        let cancel = AtomicBool::new(false);
        let error = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport,
            &cancel,
            "req-dlc-required",
            InstallOperation::Install,
        )
        .unwrap_err();
        assert_eq!(error.code, "exe_dlc_required");
        assert_no_transactions(&paths);
    }

    #[test]
    fn facts_excludes_dlc_hit_blocks_without_a_transaction() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        write_manifest(&game, "schinese", "4", Some("100"), Some(2_303_999));
        let selection = manual_selection(None, Some(vec![2_303_999]));
        let transport = transport(archive);
        let cancel = AtomicBool::new(false);
        let error = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport,
            &cancel,
            "req-dlc-excluded",
            InstallOperation::Install,
        )
        .unwrap_err();
        assert_eq!(error.code, "exe_dlc_excluded");
        assert_no_transactions(&paths);
    }

    #[test]
    fn facts_language_mismatch_blocks_without_a_transaction() {
        // Publisher sources only: `rule.discovery_only == true` keeps the
        // language gate (index sources record language as evidence instead).
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        write_manifest(&game, "english", "4", Some("100"), None);
        let selection = manual_selection(None, None);
        let transport = transport(archive);
        let cancel = AtomicBool::new(false);
        let error = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport,
            &cancel,
            "req-language",
            InstallOperation::Install,
        )
        .unwrap_err();
        assert_eq!(error.code, "exe_language_mismatch");
        assert_no_transactions(&paths);
    }

    #[test]
    fn index_source_language_is_not_a_gate() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        // Manifest is English while the advertised patch language is zh-CN.
        write_manifest(&game, "english", "4", Some("100"), None);
        let cancel = AtomicBool::new(false);

        // Index source (patchmoe): language is evidence only, so the install
        // commits despite the manifest mismatch.
        let index = seed_and_resolve_patchmoe(&state);
        let index_transport = FakeTransport::new(vec![HttpResponse {
            status: 200,
            location: None,
            content_type: Some("application/octet-stream".to_string()),
            content_length: Some(archive.len() as u64),
            etag: None,
            last_modified: None,
            final_url: index.download_url.clone(),
            body: Box::new(std::io::Cursor::new(archive.clone())),
        }]);
        let transaction = run_install(
            &game,
            &index,
            &paths,
            &state,
            &index_transport,
            &cancel,
            "req-index-language",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(transaction.state, "committed");

        // Publisher source (kagura) against the same manifest still blocks.
        let publisher_transport = transport(archive);
        let publisher = manual_selection(None, None);
        let error = run_install(
            &game,
            &publisher,
            &paths,
            &state,
            &publisher_transport,
            &cancel,
            "req-publisher-language",
            InstallOperation::Install,
        )
        .unwrap_err();
        assert_eq!(error.code, "exe_language_mismatch");
    }

    #[test]
    fn facts_missing_manifest_blocks_without_a_transaction() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        fs::remove_file(
            game.library_path
                .join("steamapps")
                .join(format!("appmanifest_{APPID}.acf")),
        )
        .unwrap();
        let selection = manual_selection(None, None);
        let transport = transport(archive);
        let cancel = AtomicBool::new(false);
        let error = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport,
            &cancel,
            "req-manifest",
            InstallOperation::Install,
        )
        .unwrap_err();
        assert_eq!(error.code, "exe_manifest_unavailable");
        assert_no_transactions(&paths);
    }

    #[test]
    fn facts_missing_buildid_blocks_build_not_verified() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        write_manifest(&game, "schinese", "4", None, None);
        let selection = manual_selection(None, None);
        let transport = transport(archive);
        let cancel = AtomicBool::new(false);
        let error = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport,
            &cancel,
            "req-build",
            InstallOperation::Install,
        )
        .unwrap_err();
        assert_eq!(error.code, "build_not_verified");
        assert_no_transactions(&paths);
    }

    #[test]
    fn facts_not_ready_blocks_game_not_ready() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        write_manifest(&game, "schinese", "2", Some("100"), None);
        let selection = manual_selection(None, None);
        let transport = transport(archive);
        let cancel = AtomicBool::new(false);
        let error = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport,
            &cancel,
            "req-not-ready",
            InstallOperation::Install,
        )
        .unwrap_err();
        assert_eq!(error.code, "game_not_ready");
        assert_no_transactions(&paths);
    }

    #[test]
    fn cancellation_mid_download_returns_download_cancelled() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        let selection = manual_selection(None, None);
        let cancel = Arc::new(AtomicBool::new(false));
        let transport = cancelling_transport(archive, Arc::clone(&cancel));
        let lease = |_now: u64| true;
        let lease_deadline = || u64::MAX;
        let inputs = ZipInstallInputs {
            game: &game,
            selection: &selection,
            operation: InstallOperation::Install,
            request_id: "req-download-cancel",
            now: 1_000,
            ttl_seconds: 900,
        };
        let deps = ZipInstallDeps {
            paths: &paths,
            state_dir: &state,
            transport: &transport,
            cancel: cancel.as_ref(),
            cancel_reason: &|| None,
            lease_active: &lease,
            lease_deadline_ms: &lease_deadline,
        };
        let error = operation::run(
            Arc::new(AtomicBool::new(false)),
            |_| {},
            || install_zip(inputs, deps),
        )
        .unwrap_err();
        assert_eq!(error.code, "download_cancelled");
        assert_no_transactions(&paths);
    }

    #[test]
    fn pre_plan_transport_failure_can_retry_with_the_same_request_id() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        let selection_id = zip_selection_id(&candidate()).unwrap();
        let selection = seed_and_resolve(&state, &selection_id);
        let cancel = AtomicBool::new(false);

        let first = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &FailingTransport,
            &cancel,
            "req-retry",
            InstallOperation::Install,
        )
        .unwrap_err();
        assert_eq!(first.code, "download_failed");
        // A pre-plan failure must not persist `error` (the binding stays retryable).
        let record = crate::installer::RequestLedger::open(&paths)
            .unwrap()
            .get("req-retry")
            .unwrap()
            .unwrap();
        assert!(record.plan_id.is_none());
        assert!(record.error.is_none());

        let transport = transport(archive);
        let transaction = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport,
            &cancel,
            "req-retry",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(transaction.state, "committed");
    }

    #[test]
    fn post_plan_domain_failure_returns_transaction_and_replays_without_transport() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        let selection_id = zip_selection_id(&candidate()).unwrap();
        let selection = seed_and_resolve(&state, &selection_id);
        let transport = transport(archive);
        let cancel = AtomicBool::new(false);
        let destination = game.install_path.join("data.txt");
        let changed = destination.clone();
        let deps = ZipInstallDeps {
            paths: &paths,
            state_dir: &state,
            transport: &transport,
            cancel: &cancel,
            cancel_reason: &|| None,
            lease_active: &|_| true,
            lease_deadline_ms: &|| u64::MAX,
        };
        let inputs = ZipInstallInputs {
            game: &game,
            selection: &selection,
            operation: InstallOperation::Install,
            request_id: "domain-replay",
            now: 1000,
            ttl_seconds: 900,
        };
        let t = operation::run(
            Arc::new(AtomicBool::new(false)),
            move |p| {
                if p.stage == "planning" {
                    fs::write(&changed, b"external bytes").unwrap();
                }
            },
            || install_zip(inputs, deps),
        )
        .unwrap();
        assert_eq!(t.state, "rolled_back");
        assert_ne!(t.error.as_deref(), Some("task_cancelled"));
        let out = super::super::outcome::InstallerOutcome::from_transaction(&t, None, None);
        assert_eq!(out.disposition, super::super::outcome::Disposition::Failed);
        assert_eq!(
            out.cause.unwrap().kind,
            super::super::outcome::FailureKind::DomainFailure
        );
        assert_eq!(fs::read(destination).unwrap(), b"external bytes");
        let record = super::super::RequestLedger::open(&paths)
            .unwrap()
            .get("domain-replay")
            .unwrap()
            .unwrap();
        assert_eq!(record.error, t.error);
        let replay = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &FakeTransport::new(Vec::new()),
            &cancel,
            "domain-replay",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&replay).unwrap(),
            serde_json::to_value(&t).unwrap()
        );
    }

    #[test]
    fn typed_causes_and_cancel_named_domain_failure_are_durable() {
        for (failure, live, expected, disposition) in [
            (
                ZipTaskError::code("cancelled"),
                Some(CancellationCause::UserCancelled),
                "cancelled",
                super::super::outcome::Disposition::Cancelled,
            ),
            (
                ZipTaskError::code("cancelled"),
                Some(CancellationCause::LeaseExpired),
                "lease_expired",
                super::super::outcome::Disposition::Cancelled,
            ),
            (
                ZipTaskError::new("cannot_cancel_write", "disk denied"),
                Some(CancellationCause::UserCancelled),
                "cannot_cancel_write: disk denied",
                super::super::outcome::Disposition::Failed,
            ),
            (
                ZipTaskError::new("plan_has_blockers", "baseline_conflict:data.txt"),
                None,
                "plan_has_blockers: baseline_conflict:data.txt",
                super::super::outcome::Disposition::Failed,
            ),
            (
                ZipTaskError::code("authorization_revoked"),
                Some(CancellationCause::UserCancelled),
                "authorization_revoked",
                super::super::outcome::Disposition::Failed,
            ),
        ] {
            let (temp, game, archive) = setup();
            let (state, paths) = dynamic_paths(&temp);
            let selection = manual_selection(None, None);
            let transport = transport(archive);
            let cancel = AtomicBool::new(false);
            let request = DownloadRequest {
                url: &selection.download_url,
                download_hosts: &selection.rule.download_hosts,
                expected_sha256: None,
                expected_size: None,
            };
            let acquired =
                acquire::acquire_with(&transport, &request, &paths, &cancel, |_, _| {}).unwrap();
            let mappings =
                zip_recipe::build_recipe(&acquired.path, &game.install_path, &cancel).unwrap();
            let release = build_release(
                &selection,
                &acquired,
                &mappings,
                &selection.expectation(game.buildid.as_deref()),
            );
            let plan = install::prepare_with_binding_at(
                paths.root(),
                &game,
                &release,
                &acquired.path,
                0,
                InstallOperation::Install,
                Some(&selection.selection_id),
                None,
            )
            .unwrap();
            assert_eq!(
                install::set_terminal_error_at(paths.root(), &plan.plan_id, "cancelled")
                    .unwrap_err()
                    .to_string(),
                "transaction_not_terminal"
            );
            let binding = request_binding(
                &game.instance_id,
                Some(&selection.selection_id),
                None,
                InstallOperation::Install,
                None,
            );
            let content = install::recipe_identity(&release).unwrap();
            begin_request(&paths, "typed-replay", &binding).unwrap();
            record_plan(&paths, "typed-replay", &binding, &plan.plan_id, &content).unwrap();
            let t = finalize_plan_failure(
                &paths,
                "typed-replay",
                &binding,
                &content,
                &plan.plan_id,
                &failure,
                live,
            )
            .unwrap();
            assert_eq!(t.state, "rolled_back");
            assert_eq!(t.error.as_deref(), Some(expected));
            let record = super::super::RequestLedger::open(&paths)
                .unwrap()
                .get("typed-replay")
                .unwrap()
                .unwrap();
            assert_eq!(record.error, t.error);
            let replay = begin_request(&paths, "typed-replay", &binding)
                .unwrap()
                .unwrap();
            assert_eq!(
                serde_json::to_value(&replay).unwrap(),
                serde_json::to_value(&t).unwrap()
            );
            let out =
                super::super::outcome::InstallerOutcome::from_transaction(&replay, None, None);
            assert_eq!(out.disposition, disposition);
            assert_eq!(out.cleanup, super::super::outcome::CleanupResult::default());
            assert!(!game.install_path.join("data.txt").exists());
            drop(state);
        }
    }

    #[test]
    fn terminal_summary_failure_keeps_zip_commit_or_rollback_and_replays_journal() {
        for ordinary_failure in [false, true] {
            let (temp, game, archive) = setup();
            let (state, paths) = dynamic_paths(&temp);
            let selection_id = zip_selection_id(&candidate()).unwrap();
            let selection = seed_and_resolve(&state, &selection_id);
            let transport = transport(archive);
            let cancel = AtomicBool::new(false);
            let requests = paths.requests();
            let destination = game.install_path.join("data.txt");
            let changed = destination.clone();
            let inputs = ZipInstallInputs {
                game: &game,
                selection: &selection,
                operation: InstallOperation::Install,
                request_id: "summary-failure",
                now: 1000,
                ttl_seconds: 900,
            };
            let deps = ZipInstallDeps {
                paths: &paths,
                state_dir: &state,
                transport: &transport,
                cancel: &cancel,
                cancel_reason: &|| None,
                lease_active: &|_| true,
                lease_deadline_ms: &|| u64::MAX,
            };
            let t = operation::run(
                Arc::new(AtomicBool::new(false)),
                move |p| {
                    if p.stage == "planning" {
                        let db = rusqlite::Connection::open(&requests).unwrap();
                        db.execute_batch(
                            "CREATE TRIGGER reject_terminal_summary BEFORE UPDATE ON requests
                        WHEN NEW.state IN ('committed','rolled_back')
                        BEGIN SELECT RAISE(FAIL,'fixture_terminal_summary_failed'); END;",
                        )
                        .unwrap();
                        if ordinary_failure {
                            fs::write(&changed, b"external bytes").unwrap();
                        }
                    }
                },
                || install_zip(inputs, deps),
            )
            .unwrap();
            assert_eq!(
                t.state,
                if ordinary_failure {
                    "rolled_back"
                } else {
                    "committed"
                }
            );
            let out = super::super::outcome::InstallerOutcome::from_transaction(&t, None, None);
            assert_eq!(
                out.disposition,
                if ordinary_failure {
                    super::super::outcome::Disposition::Failed
                } else {
                    super::super::outcome::Disposition::Completed
                }
            );
            let record = super::super::RequestLedger::open(&paths)
                .unwrap()
                .get("summary-failure")
                .unwrap()
                .unwrap();
            assert_eq!(record.state.as_deref(), Some("planned"));
            assert_eq!(record.plan_id.as_deref(), Some(t.plan_id.as_str()));
            let replay = run_install(
                &game,
                &selection,
                &paths,
                &state,
                &FakeTransport::new(Vec::new()),
                &cancel,
                "summary-failure",
                InstallOperation::Install,
            )
            .unwrap();
            assert_eq!(
                serde_json::to_value(&replay).unwrap(),
                serde_json::to_value(&t).unwrap()
            );
            assert_eq!(
                fs::read(destination).unwrap(),
                if ordinary_failure {
                    b"external bytes".as_slice()
                } else {
                    b"patched".as_slice()
                }
            );
        }
    }

    #[test]
    fn cancellation_during_apply_restores_and_rolls_back() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        let selection_id = zip_selection_id(&candidate()).unwrap();
        let selection = seed_and_resolve(&state, &selection_id);
        let transport = transport(archive);
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&cancel);
        let lease = |_now: u64| true;
        let lease_deadline = || u64::MAX;
        let inputs = ZipInstallInputs {
            game: &game,
            selection: &selection,
            operation: InstallOperation::Install,
            request_id: "req-apply-cancel",
            now: 1_000,
            ttl_seconds: 900,
        };
        let deps = ZipInstallDeps {
            paths: &paths,
            state_dir: &state,
            transport: &transport,
            cancel: cancel.as_ref(),
            cancel_reason: &|| None,
            lease_active: &lease,
            lease_deadline_ms: &lease_deadline,
        };
        let transaction = operation::run(
            Arc::clone(&cancel),
            move |progress| {
                if progress.stage == "writing" && progress.files_done == 1 {
                    signal.store(true, Ordering::Relaxed);
                }
            },
            || install_zip(inputs, deps),
        )
        .unwrap();
        assert_eq!(transaction.state, "rolled_back");
        assert_eq!(transaction.error.as_deref(), Some("cancelled"));
        let record = crate::installer::RequestLedger::open(&paths)
            .unwrap()
            .get("req-apply-cancel")
            .unwrap()
            .unwrap();
        assert_eq!(record.error, transaction.error);
        assert!(!game.install_path.join("data.txt").exists());
    }

    #[test]
    fn late_cancellation_after_commit_replays_committed() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        let selection_id = zip_selection_id(&candidate()).unwrap();
        let selection = seed_and_resolve(&state, &selection_id);
        let no_cancel = AtomicBool::new(false);
        let transport = transport(archive);
        let first = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport,
            &no_cancel,
            "req-late",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(first.state, "committed");

        let late = AtomicBool::new(true);
        let empty = FakeTransport::new(Vec::new());
        let replay = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &empty,
            &late,
            "req-late",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(replay.state, "committed");
        assert_eq!(replay.transaction_id, first.transaction_id);
    }

    #[test]
    fn lease_expiry_after_plan_persists_revocation() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        let selection_id = zip_selection_id(&candidate()).unwrap();
        let selection = seed_and_resolve(&state, &selection_id);
        let transport = transport(archive);
        let active = Arc::new(AtomicBool::new(true));
        let signal = Arc::clone(&active);
        let lease = move |_now: u64| active.load(Ordering::Relaxed);
        let lease_deadline = || u64::MAX;
        let cancel = AtomicBool::new(false);
        let inputs = ZipInstallInputs {
            game: &game,
            selection: &selection,
            operation: InstallOperation::Install,
            request_id: "req-lease",
            now: 1_000,
            ttl_seconds: 900,
        };
        let deps = ZipInstallDeps {
            paths: &paths,
            state_dir: &state,
            transport: &transport,
            cancel: &cancel,
            cancel_reason: &|| None,
            lease_active: &lease,
            lease_deadline_ms: &lease_deadline,
        };
        // An expired foreground lease after the plan is a durable rollback
        // (`InstallOutcome.state="rolled_back"`), not an RPC failure
        // (p1-host-contract §2.3): the plan is revoked and the terminal
        // transaction is recorded.
        let transaction = operation::run(
            Arc::new(AtomicBool::new(false)),
            move |progress| {
                if progress.stage == "planning" {
                    signal.store(false, Ordering::Relaxed);
                }
            },
            || install_zip(inputs, deps),
        )
        .unwrap();
        assert_eq!(transaction.state, "rolled_back");

        let transactions = install::list_transactions_at(paths.root()).unwrap();
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].state, "rolled_back");
        assert!(crate::installer::is_revoked(&paths, &transactions[0].plan_id).unwrap());
        // The lease deadline is on disk for both the plan and the rollback.
        assert!(paths
            .job(&transactions[0].plan_id)
            .unwrap()
            .join("lease.json")
            .exists());

        // A replay of the same request returns the same terminal shape
        // (idempotent, §5.1), without touching the transport.
        let replay_lease = |_now: u64| true;
        let replay_deadline = || u64::MAX;
        let replay_inputs = ZipInstallInputs {
            game: &game,
            selection: &selection,
            operation: InstallOperation::Install,
            request_id: "req-lease",
            now: 1_000,
            ttl_seconds: 900,
        };
        let replay_deps = ZipInstallDeps {
            paths: &paths,
            state_dir: &state,
            transport: &FakeTransport::new(Vec::new()),
            cancel: &cancel,
            cancel_reason: &|| None,
            lease_active: &replay_lease,
            lease_deadline_ms: &replay_deadline,
        };
        let replayed = operation::run(
            Arc::new(AtomicBool::new(false)),
            |_| {},
            || install_zip(replay_inputs, replay_deps),
        )
        .unwrap();
        assert_eq!(replayed.state, "rolled_back");
        assert_eq!(replayed.transaction_id, transaction.transaction_id);
        assert_eq!(transaction.error.as_deref(), Some("lease_expired"));
        assert_eq!(replayed.error, transaction.error);
        let record = crate::installer::RequestLedger::open(&paths)
            .unwrap()
            .get("req-lease")
            .unwrap()
            .unwrap();
        assert_eq!(record.error, transaction.error);
    }

    /// The foreground lease is re-read at every boundary from the injected clock
    /// (F4): a frozen `inputs.now` would keep it "active" forever. With a
    /// time-sensitive predicate the boundary must observe the current time.
    #[test]
    fn expired_foreground_lease_uses_the_current_time_before_plan() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        let selection_id = zip_selection_id(&candidate()).unwrap();
        let selection = seed_and_resolve(&state, &selection_id);
        let transport = transport(archive);
        let cancel = AtomicBool::new(false);
        // `inputs.now == 1_000`; a boundary using that frozen value would report
        // the lease active. The live clock (epoch seconds) is far past it.
        let lease = |now: u64| now < 1_050;
        let lease_deadline = || 1_050_000_u64;
        let inputs = ZipInstallInputs {
            game: &game,
            selection: &selection,
            operation: InstallOperation::Install,
            request_id: "req-lease-expired",
            now: 1_000,
            ttl_seconds: 900,
        };
        let deps = ZipInstallDeps {
            paths: &paths,
            state_dir: &state,
            transport: &transport,
            cancel: &cancel,
            cancel_reason: &|| None,
            lease_active: &lease,
            lease_deadline_ms: &lease_deadline,
        };
        let error = operation::run(
            Arc::new(AtomicBool::new(false)),
            |_| {},
            || install_zip(inputs, deps),
        )
        .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert_no_transactions(&paths);
    }

    #[test]
    fn same_content_reinstall_commits() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        let selection_id = zip_selection_id(&candidate()).unwrap();
        let selection = seed_and_resolve(&state, &selection_id);
        let cancel = AtomicBool::new(false);
        let first = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport(archive.clone()),
            &cancel,
            "req-install",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(first.state, "committed");

        let second = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport(archive),
            &cancel,
            "req-reinstall",
            InstallOperation::Reinstall,
        )
        .unwrap();
        assert_eq!(second.state, "committed");
        assert_ne!(second.transaction_id, first.transaction_id);
    }

    #[test]
    fn reinstall_with_changed_bytes_is_rejected() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        let selection_id = zip_selection_id(&candidate()).unwrap();
        let selection = seed_and_resolve(&state, &selection_id);
        let cancel = AtomicBool::new(false);
        let first = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport(archive),
            &cancel,
            "req-install",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(first.state, "committed");

        let changed = build_zip(&[("data.txt", b"different bytes")]);
        let error = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport(changed),
            &cancel,
            "req-reinstall-changed",
            InstallOperation::Reinstall,
        )
        .unwrap_err();
        assert_eq!(error.code, "reinstall_content_identity_changed");
    }

    #[test]
    fn reinstall_without_an_owned_target_is_rejected() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        fs::write(game.install_path.join("data.txt"), b"original").unwrap();
        let selection = manual_selection(None, None);
        let cancel = AtomicBool::new(false);
        let error = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport(archive),
            &cancel,
            "req-unowned",
            InstallOperation::Reinstall,
        )
        .unwrap_err();
        assert_eq!(error.code, "reinstall_unowned_or_incomplete");
    }

    #[test]
    fn recover_of_an_interrupted_add_plan_rolls_back() {
        let (temp, game, archive) = setup();
        let (_state, paths) = dynamic_paths(&temp);
        let archive_path = temp.path().join("dynamic.zip");
        fs::write(&archive_path, &archive).unwrap();
        let selection = manual_selection(None, None);
        let plan = prepare_dynamic_plan(
            &paths,
            &game,
            &selection,
            &archive_path,
            InstallOperation::Install,
        );
        edit_journal(paths.root(), &plan.plan_id, |job| {
            job["transaction"]["state"] = "applying".into();
            job["progress"][0] = "installed".into();
        });
        fs::write(game.install_path.join("data.txt"), b"patched").unwrap();

        let recovered = install::recover_at(paths.root()).unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].state, "rolled_back");
        assert!(!game.install_path.join("data.txt").exists());
    }

    #[test]
    fn recover_of_an_interrupted_replace_plan_reports_needs_recovery() {
        let (temp, game, archive) = setup();
        let (_state, paths) = dynamic_paths(&temp);
        fs::write(game.install_path.join("data.txt"), b"original").unwrap();
        let archive_path = temp.path().join("dynamic.zip");
        fs::write(&archive_path, &archive).unwrap();
        let selection = manual_selection(None, None);
        let plan = prepare_dynamic_plan(
            &paths,
            &game,
            &selection,
            &archive_path,
            InstallOperation::Install,
        );
        edit_journal(paths.root(), &plan.plan_id, |job| {
            job["transaction"]["state"] = "applying".into();
            job["progress"][0] = "installed".into();
        });
        fs::write(game.install_path.join("data.txt"), b"patched").unwrap();

        let recovered = install::recover_at(paths.root()).unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].state, "needs_recovery");
        assert!(recovered[0].error.is_some());
    }
}
