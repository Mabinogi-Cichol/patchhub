//! Publisher ZIP acquisition, static member mapping and direct extraction.
//! Mapping and streaming share one archive handle. History failures accompany
//! the real result; they never authorize or stop directory writes.

use super::acquire::{self, DownloadRequest, Transport};
use super::outcome::CancellationCause;
use super::resolve::ResolvedZipCandidate;
use super::store::InstallerPaths;
use super::{zip_direct, zip_recipe};
use crate::catalog::FileMapping;
use crate::install::{self, InstallOperation, Transaction};
use crate::operation;
use crate::steam::GameInstall;
use std::fmt;
use std::io;
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
}

/// Injected environment for one dynamic ZIP task.
pub(crate) struct ZipInstallDeps<'a> {
    pub paths: &'a InstallerPaths,
    pub transport: &'a dyn Transport,
    pub cancel: &'a AtomicBool,
    pub cancel_reason: &'a (dyn Fn() -> Option<CancellationCause> + Sync),
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
        "failed",
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
/// proceed, and a binding conflict otherwise. Callers first claim OS request
/// ownership; a non-terminal journal left by a dead owner is not an active task.
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
        // RequestClaim already proves no live owner holds this request. Retry
        // the selected archive directly; old applying state requires no recovery.
    }
    Ok(None)
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
    if [
        "insufficient_space",
        "zip_size_overflow",
        "installer_busy",
        "game_busy",
        "managed_install_exists_or_needs_recovery",
        "needs_recovery",
        "reinstall_content_identity_changed",
        "reinstall_unowned_or_incomplete",
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

/// Identity of the archive plus the resolved target mapping. The per-file
/// installed digests are produced by the single extraction pass, so they are
/// not part of this pre-write identity.
fn content_identity(
    selection: &ResolvedZipCandidate,
    artifact_sha256: &str,
    members: &[zip_recipe::Member],
) -> io::Result<String> {
    let mappings: Vec<FileMapping> = members
        .iter()
        .map(|member| FileMapping {
            source: member.source.clone(),
            destination: member.destination.clone(),
            original_sha256: None,
            installed_sha256: String::new(),
        })
        .collect();
    install::mapping_identity(
        selection.appid,
        &selection.release_id,
        &selection.language,
        artifact_sha256,
        &mappings,
    )
}

fn map_direct_error(error: io::Error) -> ZipTaskError {
    if error.to_string() == "cancelled" {
        return ZipTaskError::code("cancelled");
    }
    ZipTaskError::new("zip_extract_failed", error.to_string())
}

pub(crate) fn append_warning(transaction: &mut Transaction, warning: impl fmt::Display) {
    let warning = warning.to_string();
    match &mut transaction.error {
        Some(error) => {
            error.push_str("; ");
            error.push_str(&warning);
        }
        slot @ None => *slot = Some(warning),
    }
}
pub(crate) fn claim_request(
    paths: &InstallerPaths,
    request: &str,
    warnings: &mut Vec<String>,
) -> ZipTaskResult<Option<super::requests::RequestClaim>> {
    match super::requests::RequestClaim::acquire(paths, request) {
        Ok(claim) => Ok(Some(claim)),
        Err(error) if error.to_string() == "install_request_in_progress_or_interrupted" => {
            Err(ZipTaskError::code(error.to_string()))
        }
        Err(error) => {
            warnings.push(format!("request_history_failed: {error}"));
            Ok(None)
        }
    }
}
pub(crate) fn begin_extraction_request(
    paths: &InstallerPaths,
    request: &str,
    binding: &super::requests::RequestBinding,
    warnings: &mut Vec<String>,
) -> ZipTaskResult<Option<Transaction>> {
    match begin_request(paths, request, binding) {
        Ok(transaction) => Ok(transaction),
        Err(error)
            if matches!(
                error.code.as_str(),
                "request_id_binding_conflict" | "install_request_in_progress_or_interrupted"
            ) =>
        {
            Err(error)
        }
        Err(error) => {
            warnings.push(format!("request_history_failed: {error}"));
            Ok(None)
        }
    }
}
pub(crate) fn record_extraction_terminal(
    paths: &InstallerPaths,
    request: &str,
    binding: &super::requests::RequestBinding,
    content: &str,
    transaction: &mut Transaction,
) {
    if let Err(error) = record_terminal(paths, request, binding, content, transaction) {
        append_warning(transaction, format!("request_history_failed: {error}"));
    }
}

/// Stage-boundary gate: local cancel flag, then the live cancellation cause,
/// then the in-process operation cancel flag.
fn boundary(deps: &ZipInstallDeps<'_>) -> ZipTaskResult<()> {
    if let Some(cause) = (deps.cancel_reason)() {
        return Err(ZipTaskError::code(cause.code()));
    }
    if deps.cancel.load(Ordering::Relaxed) {
        return Err(ZipTaskError::code("cancelled"));
    }
    operation::check().map_err(|error| {
        if error.to_string() == "task_cancelled" {
            ZipTaskError::code("cancelled")
        } else {
            map_plan_error(error)
        }
    })
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
    execute_zip_acquired(inputs, deps, None)
}
pub(crate) fn execute_zip_acquired(
    inputs: ZipInstallInputs<'_>,
    deps: ZipInstallDeps<'_>,
    acquired: Option<acquire::AcquiredArchive>,
) -> ZipTaskResult<super::InstallExecution> {
    let mut warnings = Vec::new();
    let _claim = claim_request(deps.paths, inputs.request_id, &mut warnings)?;
    let binding = request_binding(
        &inputs.game.instance_id,
        Some(&inputs.selection.selection_id),
        None,
        inputs.operation,
        None,
    );
    if let Some(transaction) =
        begin_extraction_request(deps.paths, inputs.request_id, &binding, &mut warnings)?
    {
        return Ok(super::InstallExecution::replay(transaction));
    }
    boundary(&deps)?;
    operation::stage("connecting", None, None);
    let request = DownloadRequest {
        url: &inputs.selection.download_url,
        download_hosts: &inputs.selection.rule.download_hosts,
        expected_sha256: None,
        expected_size: None,
    };
    let acquired = match acquired {
        Some(archive) => archive,
        None => acquire::acquire_with(deps.transport, &request, deps.paths, deps.cancel, |_, _| {})
            .map_err(map_download_error)?,
    };
    warnings.extend(acquired.warnings);
    boundary(&deps)?;
    operation::stage("reading_archive", None, None);
    let mut zip = zip_recipe::open(&acquired.path).map_err(map_recipe_error)?;
    let members = zip_recipe::plan(&mut zip, &inputs.game.install_path, deps.cancel)
        .map_err(map_recipe_error)?;
    let identity =
        content_identity(inputs.selection, &acquired.sha256, &members).map_err(internal)?;
    let files: Vec<(String, u64)> = members
        .iter()
        .map(|m| (m.destination.clone(), m.size))
        .collect();
    let total = members.iter().try_fold(0u64, |sum, m| {
        sum.checked_add(m.size)
            .ok_or_else(|| ZipTaskError::code("zip_size_overflow"))
    })?;
    boundary(&deps)?;
    let mut plan = install::begin_direct_at(
        deps.paths.root(),
        inputs.game,
        &inputs.selection.release_id,
        Some(&inputs.selection.selection_id),
        Some(&identity),
        inputs.operation,
        &files,
        Some(&identity),
    )
    .map_err(map_plan_error)?;
    for warning in warnings {
        plan.warning(warning);
    }
    if let Err(error) = record_plan(
        deps.paths,
        inputs.request_id,
        &binding,
        &plan.plan_id,
        &identity,
    ) {
        plan.warning(format!("request_history_failed: {error}"));
    }
    let mut wrote = false;
    operation::stage("extracting", Some(total), Some(files.len()));
    let outcome = zip_direct::extract(&mut zip, &members, &plan.targets, deps.cancel, &mut wrote)
        .map_err(map_direct_error);
    let mut transaction = match outcome {
        Ok(installed) => {
            install::commit_direct_at(&mut plan, &installed).map_err(map_plan_error)?
        }
        Err(error) => {
            let cancelled = (deps.cancel_reason)().is_some()
                || deps.cancel.load(Ordering::Relaxed)
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
            install::fail_direct_at(&mut plan, &code).map_err(map_plan_error)?
        }
    };
    operation::stage("cleanup", None, None);
    record_extraction_terminal(
        deps.paths,
        inputs.request_id,
        &binding,
        &identity,
        &mut transaction,
    );
    Ok(super::InstallExecution::fresh(transaction))
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
        ArtifactFormat, CacheStatus, Capability, GameMatchRecord, GameMatchStatus, MatchBasis,
        Mirror, SourceCandidate, SourceIndex, SourceIndexEntry, SourceMatchRecord,
        SourceMatchStatus, SourceRole, SourceRule,
    };
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
            parser_fingerprint: crate::sources::index::parser_fingerprint(
                &crate::sources::test_fixtures::default_rule("kagura-cn"),
            ),
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

    /// Persists a matched publisher source, the v5 replacement for the old
    /// per-game discovery record seeded by these tests.
    pub(crate) fn save_match(sources: &Sources, appid: u32, candidate: SourceCandidate) {
        let source_id = candidate.source_id.clone();
        crate::sources::test_fixtures::save_match(
            sources.registry(),
            GameMatchRecord {
                schema_version: 1,
                appid,
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
            },
        )
        .unwrap();
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
            script: None,
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

    /// Seeds the patchmoe index snapshot and resolves an index-source candidate
    /// (`rule.discovery_only == false`).
    fn seed_and_resolve_patchmoe(state: &Path) -> crate::installer::ResolvedZipCandidate {
        let handle = Sources::open(state).unwrap();
        handle
            .store()
            .save_index(&SourceIndex {
                parser_fingerprint: crate::sources::index::parser_fingerprint(
                    &crate::sources::test_fixtures::default_rule("patchmoe"),
                ),
                schema_version: 1,
                source_id: "patchmoe".to_string(),
                revision: 1,
                complete: true,
                checked_at: 1000,
                updated_at: 1000,
                etag: None,
                modified: None,
                entries: vec![SourceIndexEntry {
                    title: "Kudos Kuppa".to_string(),
                    aliases: Vec::new(),
                    page: "https://www.patch.moe/Kudos-Kuppa".to_string(),
                    kind: "entry".to_string(),
                    appid: Some(APPID),
                    url: Some("https://download.patch.moe/Kudos_Kuppa_patch.zip".to_string()),
                    language: Some("zh-CN".to_string()),
                    date: Some("2026/6/23".to_string()),
                }],
                pending: Vec::new(),
            })
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
        _state: &Path,
        transport: &dyn Transport,
        cancel: &AtomicBool,
        request_id: &str,
        operation_kind: InstallOperation,
    ) -> ZipTaskResult<Transaction> {
        let inputs = ZipInstallInputs {
            game,
            selection,
            operation: operation_kind,
            request_id,
        };
        let deps = ZipInstallDeps {
            paths,
            transport,
            cancel,
            cancel_reason: &|| None,
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

    /// Seeds a fresh kagura-cn match record and returns the resolved candidate
    /// from the same registry.
    fn seed_and_resolve(
        state: &Path,
        selection_id: &str,
    ) -> crate::installer::ResolvedZipCandidate {
        let handle = Sources::open(state).unwrap();
        save_match(&handle, APPID, candidate());
        crate::installer::resolve_zip(handle.registry(), &query(), selection_id).unwrap()
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
    fn same_request_retries_abandoned_applying_and_then_replays_completion() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        let id = zip_selection_id(&candidate()).unwrap();
        let selection = seed_and_resolve(&state, &id);
        let binding = request_binding(
            &game.instance_id,
            Some(&id),
            None,
            InstallOperation::Overwrite,
            None,
        );
        {
            let _claim = super::super::requests::RequestClaim::acquire(&paths, "restart").unwrap();
            assert!(begin_request(&paths, "restart", &binding)
                .unwrap()
                .is_none());
            let plan = install::begin_direct_at(
                paths.root(),
                &game,
                &selection.release_id,
                Some(&id),
                Some("old-input"),
                InstallOperation::Overwrite,
                &[("data.txt".into(), 7)],
                None,
            )
            .unwrap();
            record_plan(&paths, "restart", &binding, &plan.plan_id, "old-input").unwrap();
            assert!(super::super::requests::RequestClaim::acquire(&paths, "restart").is_err());
            // Dropping these handles models process exit; persisted state is applying.
        }
        let cancel = AtomicBool::new(false);
        let transport = transport(archive);
        let run = |transport: &dyn Transport| {
            install_zip(
                ZipInstallInputs {
                    game: &game,
                    selection: &selection,
                    operation: InstallOperation::Overwrite,
                    request_id: "restart",
                },
                ZipInstallDeps {
                    paths: &paths,
                    transport,
                    cancel: &cancel,
                    cancel_reason: &|| None,
                },
            )
        };
        let transaction = run(&transport).unwrap();
        assert_eq!(transaction.state, "committed");
        assert_eq!(
            fs::read(game.install_path.join("data.txt")).unwrap(),
            b"patched"
        );
        let replay = run(&FakeTransport::new(vec![])).unwrap();
        assert_eq!(replay.transaction_id, transaction.transaction_id);
        assert_eq!(replay.state, "committed");
    }

    #[test]
    fn manual_unclassified_candidate_installs_and_records_the_target_binding() {
        let (temp, mut game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        write_manifest(&game, "", "4", None, None);
        game.buildid = None;
        let mut candidate = candidate();
        candidate.game_appid = None;
        candidate.match_basis = crate::sources::model::MatchBasis::Title;
        candidate.variant = crate::sources::model::Variant::Unknown;
        candidate.version_raw = None;
        candidate.version_identity.clear();
        candidate.languages.clear();
        let mut query = query();
        query.instance_id = Some(game.instance_id.clone());
        let id =
            super::super::manual_selection::selection_id(&candidate, &query, &candidate.mirrors[0])
                .unwrap();
        let handle = Sources::open(&state).unwrap();
        save_match(&handle, APPID, candidate);
        let selection = crate::installer::resolve_zip(handle.registry(), &query, &id).unwrap();
        let transport = transport(archive);
        let cancel = AtomicBool::new(false);
        let result = operation::run(
            Arc::new(AtomicBool::new(false)),
            |_| {},
            || {
                install_zip(
                    ZipInstallInputs {
                        game: &game,
                        selection: &selection,
                        operation: InstallOperation::Install,
                        request_id: "manual-test",
                    },
                    ZipInstallDeps {
                        paths: &paths,
                        transport: &transport,
                        cancel: &cancel,
                        cancel_reason: &|| None,
                    },
                )
            },
        )
        .unwrap();
        assert_eq!(result.state, "committed");
        assert_eq!(result.selection_id.as_deref(), Some(id.as_str()));
        assert_eq!(
            fs::read(game.install_path.join("data.txt")).unwrap(),
            b"patched"
        );
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
        let inputs = ZipInstallInputs {
            game: &game,
            selection: &selection,
            operation: InstallOperation::Install,
            request_id: "req-1",
        };
        let deps = ZipInstallDeps {
            paths: &paths,
            transport: &transport,
            cancel: &cancel,
            cancel_reason: &|| None,
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
        assert!(transaction.direct);
        // The dynamic ZIP transaction carries both the selection identity and
        // the archive/mapping content identity.
        assert_eq!(
            transaction.selection_id.as_deref(),
            Some(selection_id.as_str())
        );
        let content = transaction.content_id.clone().expect("content identity");
        let listed = install::list_transactions_at(paths.root()).unwrap();
        let listed = listed
            .iter()
            .find(|candidate| candidate.plan_id == transaction.plan_id)
            .expect("committed transaction is listed");
        assert_eq!(listed.selection_id.as_deref(), Some(selection_id.as_str()));
        assert_eq!(listed.content_id.as_deref(), Some(content.as_str()));
        let plan = install::get_plan_at(paths.root(), &transaction.plan_id).unwrap();
        assert_eq!(plan.recipe_sha256.as_deref(), Some(content.as_str()));
        assert!(plan.changes.iter().all(|change| change
            .installed_sha256
            .as_deref()
            .is_some_and(|sha| sha.len() == 64)));
        assert_eq!(
            fs::read(game.install_path.join("data.txt")).unwrap(),
            b"patched"
        );

        let stages = stages.lock().unwrap().clone();
        assert_eq!(stages.first().map(String::as_str), Some("connecting"));
        assert_eq!(stages.last().map(String::as_str), Some("cleanup"));
        for stage in ["connecting", "downloading", "extracting"] {
            assert!(
                stages.iter().any(|item| item == stage),
                "missing stage {stage}: {stages:?}"
            );
        }
        assert!(!stages.iter().any(|item| item == "writing"));
        // A direct install never writes an authorization receipt or backup.
        assert!(!paths.authorization(&transaction.plan_id).unwrap().exists());
        let job = paths.jobs_dir().join(&transaction.plan_id);
        assert!(!job.join("0.stage").exists());
        assert!(!job.join("0.backup").exists());
    }

    fn duplicate_zip() -> Vec<u8> {
        let mut archive = build_zip(&[
            ("a.bin", b"first"),
            ("b.bin", b"middle"),
            ("c.bin", b"last"),
        ]);
        // ZipWriter rejects duplicate names. Replace same-length local and
        // central names to exercise an ordinary ZIP with a,b,a records.
        for offset in 0..archive.len() - 4 {
            if &archive[offset..offset + 5] == b"c.bin" {
                archive[offset..offset + 5].copy_from_slice(b"a.bin");
            }
        }
        archive
    }

    #[test]
    fn duplicate_zip_members_stream_in_order_and_history_keeps_last_digest() {
        let (temp, game, _) = setup();
        let (state, paths) = dynamic_paths(&temp);
        let selection = manual_selection(None, None);
        let files = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&files);
        let result = operation::run(
            Arc::new(AtomicBool::new(false)),
            move |p| {
                if p.stage == "extracting" {
                    sink.lock().unwrap().push(p.files_done);
                }
            },
            || {
                install_zip(
                    ZipInstallInputs {
                        game: &game,
                        selection: &selection,
                        operation: InstallOperation::Install,
                        request_id: "duplicate",
                    },
                    ZipInstallDeps {
                        paths: &paths,
                        transport: &transport(duplicate_zip()),
                        cancel: &AtomicBool::new(false),
                        cancel_reason: &|| None,
                    },
                )
            },
        )
        .unwrap();
        assert_eq!(result.state, "committed");
        assert_eq!(fs::read(game.install_path.join("a.bin")).unwrap(), b"last");
        assert_eq!(
            fs::read(game.install_path.join("b.bin")).unwrap(),
            b"middle"
        );
        assert!(files.lock().unwrap().contains(&3));
        let plan = install::get_plan_at(paths.root(), &result.plan_id).unwrap();
        assert_eq!(plan.changes.len(), 2);
        let last = plan.changes.iter().find(|c| c.path == "a.bin").unwrap();
        assert_eq!(
            last.installed_sha256.as_deref(),
            Some(sha256_hex(b"last").as_str())
        );
        let _ = state;
    }

    #[test]
    fn corrupt_earlier_duplicate_is_not_silently_replaced_by_later_member() {
        let (temp, game, _) = setup();
        let (state, paths) = dynamic_paths(&temp);
        let selection = manual_selection(None, None);
        let mut archive = duplicate_zip();
        let first_payload = 30
            + u16::from_le_bytes([archive[26], archive[27]]) as usize
            + u16::from_le_bytes([archive[28], archive[29]]) as usize;
        archive[first_payload] ^= 0xff;
        let result = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport(archive),
            &AtomicBool::new(false),
            "duplicate-corrupt",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(result.state, "failed");
        assert!(result
            .error
            .as_deref()
            .unwrap()
            .starts_with("install_incomplete:"));
        assert!(!game.install_path.join("b.bin").exists());
        assert_ne!(fs::read(game.install_path.join("a.bin")).unwrap(), b"last");
    }

    #[test]
    fn unavailable_history_does_not_stop_remaining_members_or_change_success() {
        let (temp, game, _) = setup();
        let state = temp.path().join("blocked-state");
        let paths = InstallerPaths::new(&state);
        let bytes = build_zip(&[("a.bin", b"one"), ("b.bin", b"two")]);
        let archive = temp.path().join("input.zip");
        fs::write(&archive, &bytes).unwrap();
        fs::write(&state, b"a file occupies the auxiliary state directory").unwrap();
        let selection = manual_selection(None, None);
        let result = execute_zip_acquired(
            ZipInstallInputs {
                game: &game,
                selection: &selection,
                operation: InstallOperation::Install,
                request_id: "history-unavailable",
            },
            ZipInstallDeps {
                paths: &paths,
                transport: &FakeTransport::new(Vec::new()),
                cancel: &AtomicBool::new(false),
                cancel_reason: &|| None,
            },
            Some(acquire::AcquiredArchive {
                path: archive,
                sha256: sha256_hex(&bytes),
                size: bytes.len() as u64,
                final_url: selection.download_url.clone(),
                source: acquire::Acquisition::Downloaded,
                format: "zip".into(),
                warnings: Vec::new(),
            }),
        )
        .unwrap()
        .transaction;
        assert_eq!(result.state, "committed");
        assert!(result
            .error
            .as_deref()
            .unwrap()
            .contains("history_save_failed"));
        assert_eq!(fs::read(game.install_path.join("a.bin")).unwrap(), b"one");
        assert_eq!(fs::read(game.install_path.join("b.bin")).unwrap(), b"two");
    }

    #[cfg(windows)]
    #[test]
    fn case_aliases_follow_windows_filesystem_and_history_keeps_final_write() {
        let (temp, game, _) = setup();
        let (state, paths) = dynamic_paths(&temp);
        let selection = manual_selection(None, None);
        let result = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport(build_zip(&[("A.bin", b"first"), ("a.bin", b"last")])),
            &AtomicBool::new(false),
            "case-alias",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(result.state, "committed");
        assert_eq!(fs::read(game.install_path.join("A.bin")).unwrap(), b"last");
        let plan = install::get_plan_at(paths.root(), &result.plan_id).unwrap();
        assert_eq!(plan.changes.len(), 1);
        assert_eq!(plan.changes[0].path, "a.bin");
        assert_eq!(
            plan.changes[0].installed_sha256.as_deref(),
            Some(sha256_hex(b"last").as_str())
        );
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

        let run = |transport: &dyn Transport| {
            let inputs = ZipInstallInputs {
                game: &game,
                selection: &selection,
                operation: InstallOperation::Install,
                request_id: "req-1",
            };
            let deps = ZipInstallDeps {
                paths: &paths,
                transport,
                cancel: &cancel,
                cancel_reason: &|| None,
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
        };
        let deps = ZipInstallDeps {
            paths: &paths,
            transport: &empty,
            cancel: &cancel,
            cancel_reason: &|| None,
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
        let transport = transport(archive);
        let inputs = ZipInstallInputs {
            game: &game,
            selection: &selection,
            operation: InstallOperation::Install,
            request_id: "req-cancel",
        };
        let deps = ZipInstallDeps {
            paths: &paths,
            transport: &transport,
            cancel: &cancel,
            cancel_reason: &|| None,
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
    /// Hashes members and existing targets for the legacy prepare fixtures
    /// used by the recovery tests. Production mapping never reads content.
    fn legacy_mappings(archive: &Path, root: &Path) -> Vec<crate::catalog::FileMapping> {
        let mut zip = zip_recipe::open(archive).unwrap();
        let members = zip_recipe::plan(&mut zip, root, &AtomicBool::new(false)).unwrap();
        let mut mappings = Vec::new();
        for member in members {
            let installed = {
                let mut entry = zip.entry(member.index).unwrap();
                let mut file = entry.by_index(0).unwrap();
                let mut hasher = sha2::Sha256::new();
                std::io::copy(&mut file, &mut hasher).unwrap();
                format!("{:x}", hasher.finalize())
            };
            let original = fs::read(root.join(&member.destination))
                .ok()
                .map(|bytes| sha256_hex(&bytes));
            mappings.push(crate::catalog::FileMapping {
                source: member.source,
                destination: member.destination,
                original_sha256: original,
                installed_sha256: installed,
            });
        }
        mappings
    }

    fn legacy_release(
        selection: &ResolvedZipCandidate,
        acquired: &acquire::AcquiredArchive,
        mappings: &[crate::catalog::FileMapping],
    ) -> crate::catalog::PatchRelease {
        use crate::catalog::{Artifact, Fingerprint, Platform, Recipe, SupportLevel};
        crate::catalog::PatchRelease {
            patch_id: selection.release_id.clone(),
            release_id: selection.release_id.clone(),
            appid: selection.appid,
            name: selection.candidate.patch_title.clone(),
            version: "0".to_string(),
            language: selection.language.clone(),
            category: "publisher_zip".to_string(),
            source_page: selection.candidate.source_page.clone(),
            support_level: SupportLevel::ManagedZip,
            platforms: vec![Platform::Windows, Platform::Steamos],
            verified_buildids: Vec::new(),
            detection: mappings
                .iter()
                .map(|mapping| Fingerprint {
                    path: mapping.destination.clone(),
                    sha256: mapping.installed_sha256.clone(),
                })
                .collect(),
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

    /// Builds (without applying) a dynamic publisher ZIP plan at the P1 root.
    fn dynamic_release(
        game: &GameInstall,
        selection: &ResolvedZipCandidate,
        archive: &Path,
    ) -> crate::catalog::PatchRelease {
        let mappings = legacy_mappings(archive, &game.install_path);
        let bytes = fs::read(archive).unwrap();
        let acquired = acquire::AcquiredArchive {
            format: "zip".into(),
            warnings: Vec::new(),
            path: archive.to_path_buf(),
            sha256: sha256_hex(&bytes),
            size: bytes.len() as u64,
            final_url: selection.download_url.clone(),
            source: acquire::Acquisition::Downloaded,
        };
        let mut release = legacy_release(selection, &acquired, &mappings);
        // These legacy preparation fixtures use catalog-style admission.
        release.verified_buildids = game.buildid.clone().into_iter().collect();
        release
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
    fn dlc_requirement_does_not_block_extraction() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        write_manifest(&game, "schinese", "4", Some("100"), None);
        let selection = manual_selection(Some(vec![2_303_802]), None);
        let transport = transport(archive);
        let cancel = AtomicBool::new(false);
        let transaction = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport,
            &cancel,
            "req-dlc-required",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(transaction.state, "committed");
        assert_eq!(
            fs::read(game.install_path.join("data.txt")).unwrap(),
            b"patched"
        );
    }

    #[test]
    fn dlc_exclusion_does_not_block_extraction() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        write_manifest(&game, "schinese", "4", Some("100"), Some(2_303_999));
        let selection = manual_selection(None, Some(vec![2_303_999]));
        let transport = transport(archive);
        let cancel = AtomicBool::new(false);
        let transaction = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport,
            &cancel,
            "req-dlc-excluded",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(transaction.state, "committed");
        assert_eq!(
            fs::read(game.install_path.join("data.txt")).unwrap(),
            b"patched"
        );
    }

    #[test]
    fn zip_installs_with_different_missing_or_conflicting_manifest_languages() {
        for language in ["english", "", "conflicting"] {
            for index_source in [false, true] {
                let (temp, game, archive) = setup();
                let (state, paths) = dynamic_paths(&temp);
                write_manifest(&game, language, "4", Some("100"), None);
                let manifest = game
                    .library_path
                    .join(format!("steamapps/appmanifest_{APPID}.acf"));
                if language == "conflicting" {
                    let value = fs::read_to_string(&manifest)
                        .unwrap()
                        .replace("\"language\" \"conflicting\"", "\"language\" \"english\"");
                    fs::write(
                        &manifest,
                        value.replacen("\"language\" \"english\"", "\"language\" \"schinese\"", 1),
                    )
                    .unwrap();
                } else if language.is_empty() {
                    fs::write(
                        &manifest,
                        fs::read_to_string(&manifest)
                            .unwrap()
                            .replace("\"language\" \"\"", ""),
                    )
                    .unwrap();
                }
                let selection = if index_source {
                    seed_and_resolve_patchmoe(&state)
                } else {
                    manual_selection(None, None)
                };
                let transport = FakeTransport::new(vec![HttpResponse {
                    status: 200,
                    location: None,
                    content_type: Some("application/octet-stream".into()),
                    content_length: Some(archive.len() as u64),
                    etag: None,
                    last_modified: None,
                    final_url: selection.download_url.clone(),
                    body: Box::new(std::io::Cursor::new(archive)),
                }]);
                let result = run_install(
                    &game,
                    &selection,
                    &paths,
                    &state,
                    &transport,
                    &AtomicBool::new(false),
                    "language-free",
                    InstallOperation::Install,
                )
                .unwrap();
                assert_eq!(result.state, "committed", "{language} index={index_source}");
            }
        }
    }

    #[test]
    fn missing_manifest_does_not_block_selected_directory() {
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
        let transaction = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport,
            &cancel,
            "req-manifest",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(transaction.state, "committed");
        assert_eq!(
            fs::read(game.install_path.join("data.txt")).unwrap(),
            b"patched"
        );
    }

    #[test]
    fn changed_buildid_does_not_block_extraction() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        write_manifest(&game, "schinese", "4", None, None);
        let selection = manual_selection(None, None);
        let transport = transport(archive);
        let cancel = AtomicBool::new(false);
        let transaction = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport,
            &cancel,
            "req-build",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(transaction.state, "committed");
        assert_eq!(
            fs::read(game.install_path.join("data.txt")).unwrap(),
            b"patched"
        );
    }

    #[test]
    fn manifest_state_does_not_block_extraction() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        write_manifest(&game, "schinese", "2", Some("100"), None);
        let selection = manual_selection(None, None);
        let transport = transport(archive);
        let cancel = AtomicBool::new(false);
        let transaction = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport,
            &cancel,
            "req-not-ready",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(transaction.state, "committed");
        assert_eq!(
            fs::read(game.install_path.join("data.txt")).unwrap(),
            b"patched"
        );
    }

    #[test]
    fn cancellation_mid_download_returns_download_cancelled() {
        let (temp, game, archive) = setup();
        let (_state, paths) = dynamic_paths(&temp);
        let selection = manual_selection(None, None);
        let cancel = Arc::new(AtomicBool::new(false));
        let transport = cancelling_transport(archive, Arc::clone(&cancel));
        let inputs = ZipInstallInputs {
            game: &game,
            selection: &selection,
            operation: InstallOperation::Install,
            request_id: "req-download-cancel",
        };
        let deps = ZipInstallDeps {
            paths: &paths,
            transport: &transport,
            cancel: cancel.as_ref(),
            cancel_reason: &|| None,
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
    fn overwrites_existing_content_without_a_baseline_and_replays() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        let selection_id = zip_selection_id(&candidate()).unwrap();
        let selection = seed_and_resolve(&state, &selection_id);
        fs::write(game.install_path.join("data.txt"), b"user bytes").unwrap();
        let cancel = AtomicBool::new(false);
        let first = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport(archive),
            &cancel,
            "no-baseline",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(first.state, "committed");
        assert_eq!(
            fs::read(game.install_path.join("data.txt")).unwrap(),
            b"patched"
        );
        let replay = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &FakeTransport::new(Vec::new()),
            &cancel,
            "no-baseline",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(replay.transaction_id, first.transaction_id);
        assert_eq!(replay.state, "committed");
    }

    #[test]
    fn terminal_summary_failure_keeps_commit_and_replays_journal() {
        let (temp, game, archive) = setup();
        let (state, paths) = dynamic_paths(&temp);
        let selection_id = zip_selection_id(&candidate()).unwrap();
        let selection = seed_and_resolve(&state, &selection_id);
        let requests = paths.requests();
        let cancel = AtomicBool::new(false);
        let inputs = ZipInstallInputs {
            game: &game,
            selection: &selection,
            operation: InstallOperation::Install,
            request_id: "summary-failure",
        };
        let deps = ZipInstallDeps {
            paths: &paths,
            transport: &transport(archive),
            cancel: &cancel,
            cancel_reason: &|| None,
        };
        let t = operation::run(
            Arc::new(AtomicBool::new(false)),
            move |p| {
                if p.stage == "extracting" {
                    let db = rusqlite::Connection::open(&requests).unwrap();
                    db.execute_batch(
                        "CREATE TRIGGER IF NOT EXISTS reject_terminal_summary BEFORE UPDATE ON requests
                        WHEN NEW.state IN ('committed','failed')
                        BEGIN SELECT RAISE(FAIL,'fixture_terminal_summary_failed'); END;",
                    )
                    .unwrap();
                }
            },
            || install_zip(inputs, deps),
        )
        .unwrap();
        assert_eq!(t.state, "committed");
        let record = super::super::RequestLedger::open(&paths)
            .unwrap()
            .get("summary-failure")
            .unwrap()
            .unwrap();
        assert_eq!(record.state.as_deref(), Some("planned"));
        assert_eq!(record.plan_id.as_deref(), Some(t.plan_id.as_str()));
        assert_eq!(
            fs::read(game.install_path.join("data.txt")).unwrap(),
            b"patched"
        );
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
        assert_eq!(replay.transaction_id, t.transaction_id);
        assert_eq!(replay.state, "committed");
        assert!(t
            .error
            .as_deref()
            .is_some_and(|e| e.contains("request_history_failed")));
    }

    #[test]
    fn cancellation_during_extract_keeps_written_files() {
        let (temp, game, _) = setup();
        let archive = build_zip(&[("data.txt", b"patched"), ("next.txt", b"unwritten")]);
        let (state, paths) = dynamic_paths(&temp);
        let selection_id = zip_selection_id(&candidate()).unwrap();
        let selection = seed_and_resolve(&state, &selection_id);
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&cancel);
        let inputs = ZipInstallInputs {
            game: &game,
            selection: &selection,
            operation: InstallOperation::Install,
            request_id: "req-apply-cancel",
        };
        let deps = ZipInstallDeps {
            paths: &paths,
            transport: &transport(archive),
            cancel: cancel.as_ref(),
            cancel_reason: &|| None,
        };
        let transaction = operation::run(
            Arc::clone(&cancel),
            move |progress| {
                if progress.stage == "extracting" && progress.files_done == 1 {
                    signal.store(true, Ordering::Relaxed);
                }
            },
            || install_zip(inputs, deps),
        )
        .unwrap();
        assert_eq!(transaction.state, "failed");
        assert_eq!(transaction.error.as_deref(), Some("cancelled_incomplete"));
        // Written game files are kept; there is no rollback or staging cleanup.
        assert_eq!(
            fs::read(game.install_path.join("data.txt")).unwrap(),
            b"patched"
        );
        let record = crate::installer::RequestLedger::open(&paths)
            .unwrap()
            .get("req-apply-cancel")
            .unwrap()
            .unwrap();
        assert_eq!(record.error, transaction.error);
        let replay = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &FakeTransport::new(Vec::new()),
            &cancel,
            "req-apply-cancel",
            InstallOperation::Install,
        )
        .unwrap();
        assert_eq!(replay.transaction_id, transaction.transaction_id);
        assert_eq!(replay.state, "failed");
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
    fn reinstall_overwrites_changed_archive_bytes() {
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

        // Direct overwrite semantics: a reinstall does not require a previous
        // baseline receipt and writes the newly selected bytes.
        let changed = build_zip(&[("data.txt", b"different bytes")]);
        let second = run_install(
            &game,
            &selection,
            &paths,
            &state,
            &transport(changed),
            &cancel,
            "req-reinstall-changed",
            InstallOperation::Reinstall,
        )
        .unwrap();
        assert_eq!(second.state, "committed");
        assert_eq!(
            fs::read(game.install_path.join("data.txt")).unwrap(),
            b"different bytes"
        );
    }
    #[test]
    fn production_recovery_keeps_legacy_backups_and_latest_direct_files_read_only() {
        for replace in [false, true] {
            for old_state in ["applying", "needs_recovery"] {
                let (temp, game, archive) = setup();
                let (_state, paths) = dynamic_paths(&temp);
                if replace {
                    fs::write(game.install_path.join("data.txt"), b"original").unwrap();
                }
                let archive_path = temp.path().join("legacy.zip");
                fs::write(&archive_path, &archive).unwrap();
                let selection = manual_selection(None, None);
                let old = prepare_dynamic_plan(
                    &paths,
                    &game,
                    &selection,
                    &archive_path,
                    InstallOperation::Install,
                );
                let backup = paths.job(&old.plan_id).unwrap().join("0.backup");
                if replace {
                    fs::write(&backup, b"original").unwrap();
                }
                edit_journal(paths.root(), &old.plan_id, |job| {
                    job["transaction"]["state"] = old_state.into();
                    job["progress"][0] = "installed".into();
                });
                fs::write(game.install_path.join("data.txt"), b"patched").unwrap();
                let mut plan = install::begin_direct_at(
                    paths.root(),
                    &game,
                    "new-extraction",
                    Some("selection"),
                    Some("content"),
                    InstallOperation::Overwrite,
                    &[("data.txt".into(), 7)],
                    None,
                )
                .unwrap();
                let committed = install::commit_direct_at(
                    &mut plan,
                    &[("data.txt".into(), sha256_hex(b"patched"))],
                )
                .unwrap();
                drop(plan);
                // Also prove protection does not depend on newer auxiliary history.
                let db = rusqlite::Connection::open(paths.root().join("journal.sqlite")).unwrap();
                db.execute("DELETE FROM jobs WHERE id=?1", [&committed.plan_id])
                    .unwrap();
                drop(db);
                let result = install::recover_checked_at(paths.root(), |_| Ok(())).unwrap();
                assert!(result.is_empty());
                assert_eq!(
                    fs::read(game.install_path.join("data.txt")).unwrap(),
                    b"patched"
                );
                let old_record = install::list_transactions_at(paths.root())
                    .unwrap()
                    .into_iter()
                    .find(|t| t.plan_id == old.plan_id)
                    .unwrap();
                assert_eq!(old_record.state, old_state);
                if replace {
                    assert_eq!(fs::read(backup).unwrap(), b"original");
                }
            }
        }
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
