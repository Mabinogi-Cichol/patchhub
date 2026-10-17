//! Signed managed-ZIP installation. One trusted catalog verification and one
//! archive-identity check at the entry, then the shared direct-overwrite
//! extraction used by publisher ZIP installs.
use super::{
    facts, orchestrator, outcome::CancellationCause, space, task::TaskControl, zip_direct,
    zip_recipe, InstallerPaths, Transport,
};
use crate::{
    catalog::{Artifact, Platform, SupportLevel},
    install::{self, InstallOperation, Transaction},
    operation,
    projection::DomainError,
    steam::GameInstall,
    trust::{
        revision::{self, TrustRevisionStore, TrustedCatalogProvider},
        VerifiedCatalog,
    },
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub(crate) struct SignedInstallInputs<'a> {
    pub game: &'a GameInstall,
    pub release_id: &'a str,
    pub archive: Option<&'a Path>,
    pub operation: InstallOperation,
    pub request_id: &'a str,
    pub now: u64,
}

pub(crate) struct SignedInstallDeps<'a> {
    pub paths: &'a InstallerPaths,
    pub provider: &'a dyn TrustedCatalogProvider,
    pub revision_store: &'a dyn TrustRevisionStore,
    pub platform: Platform,
    pub transport: &'a dyn Transport,
    pub cancel: &'a AtomicBool,
    pub cancel_reason: &'a (dyn Fn() -> Option<CancellationCause> + Sync),
}

fn io_error(error: io::Error) -> DomainError {
    let text = error.to_string();
    if !text.is_empty()
        && text.len() <= 128
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return DomainError::code(text);
    }
    // Only known historical machine-code prefixes are bridged.
    for code in [
        "journal_error",
        "invalid_zip",
        "zip_member",
        "file_changed",
        "recovery_conflict",
        "baseline_conflict",
        "insufficient_space",
        "archive_source_missing",
    ] {
        if let Some(detail) = text.strip_prefix(&format!("{code}:")) {
            return DomainError::new(code, detail);
        }
    }
    DomainError::new("io_error", text)
}

fn task_error(error: orchestrator::ZipTaskError) -> DomainError {
    match error.detail {
        Some(detail) => DomainError::new(error.code, detail),
        None => DomainError::code(error.code),
    }
}

fn cancellation(_deps: &SignedInstallDeps<'_>, _control: &TaskControl) -> DomainError {
    DomainError::code("cancelled")
}

fn normalize_cancel(
    error: DomainError,
    deps: &SignedInstallDeps<'_>,
    control: &TaskControl,
) -> DomainError {
    if matches!(
        error.code.as_str(),
        "cancelled" | "task_cancelled" | "download_cancelled"
    ) {
        cancellation(deps, control)
    } else {
        error
    }
}

fn boundary(deps: &SignedInstallDeps<'_>, control: &TaskControl) -> Result<(), DomainError> {
    if (deps.cancel_reason)().is_some() || deps.cancel.load(Ordering::Acquire) {
        return Err(cancellation(deps, control));
    }
    control
        .check()
        .map_err(io_error)
        .map_err(|e| normalize_cancel(e, deps, control))?;
    operation::check()
        .map_err(io_error)
        .map_err(|e| normalize_cancel(e, deps, control))
}

pub(crate) fn install_signed(
    inputs: SignedInstallInputs<'_>,
    deps: SignedInstallDeps<'_>,
) -> Result<super::InstallExecution, DomainError> {
    super::init_atomic(deps.paths).map_err(io_error)?;
    let _claim =
        super::requests::RequestClaim::acquire(deps.paths, inputs.request_id).map_err(io_error)?;
    let control = TaskControl::new(
        deps.paths,
        inputs.request_id,
        &inputs.game.instance_id,
        inputs.release_id,
    )
    .map_err(io_error)?;
    control.run_with_cause(deps.cancel, deps.cancel_reason, || {
        install_signed_inner(&inputs, &deps, &control)
    })
}

fn ready_release<'a>(
    verified: &'a VerifiedCatalog,
    inputs: &SignedInstallInputs<'_>,
    platform: Platform,
    now: u64,
) -> Result<&'a crate::catalog::PatchRelease, DomainError> {
    verified
        .ensure_active(inputs.release_id, now)
        .map_err(io_error)?;
    let release = verified
        .catalog()
        .releases
        .iter()
        .find(|r| r.release_id == inputs.release_id && r.appid == inputs.game.appid)
        .ok_or_else(|| DomainError::code("release_not_matched"))?;
    if !release.platforms.contains(&platform) {
        return Err(DomainError::code("platform_not_verified"));
    }
    if inputs.game.availability != "installed" {
        return Err(DomainError::code("game_not_ready"));
    }
    if !inputs
        .game
        .buildid
        .as_ref()
        .is_some_and(|b| release.verified_buildids.contains(b))
    {
        return Err(DomainError::code("build_not_verified"));
    }
    if release.support_level != SupportLevel::ManagedZip {
        return Err(DomainError::code("source_requires_guidance"));
    }
    Ok(release)
}

fn expectation(
    release: &crate::catalog::PatchRelease,
    game: &GameInstall,
    cancel: &AtomicBool,
) -> facts::FactsExpectation {
    facts::FactsExpectation {
        appid: release.appid,
        language: facts::current_language(game, cancel).unwrap_or_else(|| release.language.clone()),
        verified_buildids: release.verified_buildids.clone(),
        require_language: true,
        requires_dlc: Vec::new(),
        excludes_dlc: Vec::new(),
        baselines: Vec::new(),
    }
}

fn require_facts(facts: &facts::GameFacts) -> Result<(), DomainError> {
    if facts.is_valid() {
        return Ok(());
    }
    Err(match facts.failures().first() {
        Some(failure) => DomainError::new(failure.code.clone(), failure.detail.clone()),
        None => DomainError::code("game_not_ready"),
    })
}

fn direct_error(error: io::Error) -> DomainError {
    if error.to_string() == "cancelled" {
        return DomainError::code("cancelled");
    }
    DomainError::new("zip_extract_failed", error.to_string())
}

fn install_signed_inner(
    inputs: &SignedInstallInputs<'_>,
    deps: &SignedInstallDeps<'_>,
    control: &TaskControl,
) -> Result<super::InstallExecution, DomainError> {
    let binding = super::request_binding(
        &inputs.game.instance_id,
        None,
        Some(inputs.release_id),
        inputs.operation,
        None,
    );
    if let Some(transaction) =
        super::begin_request(deps.paths, inputs.request_id, &binding).map_err(task_error)?
    {
        return Ok(super::InstallExecution::replay(transaction));
    }
    boundary(deps, control)?;
    // The catalog state of this request is read once; there is no deferred
    // apply-time re-check and no cross-request trust cache.
    let catalog = revision::verify_catalog(deps.provider, deps.revision_store, inputs.now)?;
    deps.revision_store.remember_verified_revision(&catalog)?;
    let release = ready_release(&catalog, inputs, deps.platform, inputs.now)?;
    let identity = install::recipe_identity(release).map_err(io_error)?;
    if super::content_conflict(deps.paths, inputs.request_id, &identity).map_err(task_error)? {
        return Err(DomainError::code("request_id_binding_conflict"));
    }
    let recipe = release
        .recipe
        .as_ref()
        .ok_or_else(|| DomainError::code("recipe_missing"))?;
    let mappings = recipe.files.clone();
    boundary(deps, control)?;
    // A user-owned local ZIP stays where it is. One handle serves the declared
    // identity check and the single extraction pass.
    let local = match inputs.archive {
        Some(path) => {
            let file = File::open(path).map_err(io_error)?;
            operation::archive(file.metadata().map_err(io_error)?.len());
            let (actual, file) = facts::digest_open_file(file, deps.cancel).map_err(io_error)?;
            if actual.sha256 != recipe.artifact_sha256 {
                return Err(DomainError::code("artifact_hash_mismatch"));
            }
            if release
                .artifact
                .as_ref()
                .is_some_and(|artifact| artifact.size != actual.size)
            {
                return Err(DomainError::code("artifact_size_mismatch"));
            }
            Some(file)
        }
        None => None,
    };
    boundary(deps, control)?;

    let mut zip = match local {
        Some(file) => zip_recipe::from_file(file).map_err(io_error)?,
        None => {
            let archive = fetch_signed(
                release
                    .artifact
                    .as_ref()
                    .ok_or_else(|| DomainError::code("artifact_requires_manual_download"))?,
                deps,
                control,
            )?;
            boundary(deps, control)?;
            zip_recipe::open(&archive).map_err(io_error)?
        }
    };
    let members =
        zip_recipe::plan_mapped(&mut zip, &inputs.game.install_path, &mappings, deps.cancel)
            .map_err(io_error)?;
    let expected = expectation(release, inputs.game, deps.cancel);
    let game_facts = facts::game_facts(inputs.game, &expected, deps.cancel);
    require_facts(&game_facts)?;
    let files: Vec<(String, u64)> = members
        .iter()
        .map(|member| (member.destination.clone(), member.size))
        .collect();
    let sizes = space::zip_target_sizes(&inputs.game.install_path, &members).map_err(io_error)?;
    space::ZipSpaceBudget::for_direct_overwrite(&sizes)
        .and_then(|budget| budget.inspect(deps.paths.root(), &inputs.game.install_path))
        .map_err(io_error)?;
    let total = members.iter().try_fold(0u64, |sum, member| {
        sum.checked_add(member.size)
            .ok_or_else(|| DomainError::code("zip_size_overflow"))
    })?;
    boundary(deps, control)?;
    let plan = install::begin_direct_at(
        deps.paths.root(),
        inputs.game,
        inputs.release_id,
        None,
        None,
        inputs.operation,
        &files,
        Some(&identity),
    )
    .map_err(io_error)?;
    let mut wrote = false;
    let outcome = (|| -> Result<Transaction, DomainError> {
        control
            .attach_plan(deps.paths, &plan.plan_id)
            .map_err(io_error)?;
        super::record_plan(
            deps.paths,
            inputs.request_id,
            &binding,
            &plan.plan_id,
            &identity,
        )
        .map_err(task_error)?;
        boundary(deps, control)?;
        operation::stage("extracting", Some(total), Some(files.len()));
        let installed =
            zip_direct::extract(&mut zip, &members, &plan.targets, deps.cancel, &mut wrote)
                .map_err(direct_error)?;
        boundary(deps, control)?;
        install::commit_direct_at(deps.paths.root(), &plan.plan_id, &installed).map_err(io_error)
    })();
    let transaction = match outcome {
        Ok(transaction) => transaction,
        Err(error) => {
            let error = normalize_cancel(error, deps, control);
            let cancelled = error.code == "cancelled";
            let code = if cancelled {
                if wrote {
                    "cancelled_incomplete".to_owned()
                } else {
                    "task_cancelled".to_owned()
                }
            } else if wrote {
                format!("install_incomplete: {error}")
            } else {
                error.to_string()
            };
            let _ = control.revoke_for_failure();
            let failed = install::fail_direct_at(deps.paths.root(), &plan.plan_id, &code)
                .map_err(io_error)?;
            let _ =
                super::record_terminal(deps.paths, inputs.request_id, &binding, &identity, &failed);
            return Ok(super::InstallExecution::fresh(failed));
        }
    };
    operation::stage("cleanup", None, None);
    // The journal is the durable authority. A late ledger write failure must
    // not turn a committed transaction into a synthetic RPC failure.
    let _ = super::record_terminal(
        deps.paths,
        inputs.request_id,
        &binding,
        &identity,
        &transaction,
    );
    Ok(super::InstallExecution::fresh(transaction))
}

// This signed-only adapter preserves the mature download URL policy: unlike
// dynamic ZIP discovery, a signed artifact need not have a .zip URL suffix.
fn approved_url(value: &str, artifact: &Artifact) -> Result<reqwest::Url, DomainError> {
    let mut candidate = artifact.clone();
    candidate.url = value.to_string();
    crate::download::validate_artifact(&candidate).map_err(io_error)?;
    let url = reqwest::Url::parse(value).map_err(|_| DomainError::code("invalid_download_url"))?;
    let original = reqwest::Url::parse(&artifact.url)
        .map_err(|_| DomainError::code("invalid_download_url"))?;
    if original.host_str() != url.host_str()
        && !artifact
            .allowed_redirect_hosts
            .iter()
            .any(|h| Some(h.as_str()) == url.host_str())
    {
        return Err(DomainError::code("redirect_host_not_approved"));
    }
    Ok(url)
}

fn no_link(path: &Path) -> Result<(), DomainError> {
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        let metadata = fs::symlink_metadata(ancestor).map_err(io_error)?;
        #[cfg(windows)]
        let linked = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let linked = metadata.file_type().is_symlink();
        if linked {
            return Err(DomainError::code("linked_download_cache"));
        }
    }
    Ok(())
}

fn copy_signed(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    artifact: &Artifact,
    deps: &SignedInstallDeps<'_>,
    control: &TaskControl,
) -> Result<(), DomainError> {
    let mut total = 0u64;
    let mut hash = Sha256::new();
    let mut magic = Vec::with_capacity(4);
    let mut buffer = [0u8; 65536];
    loop {
        boundary(deps, control)?;
        let count = reader.read(&mut buffer).map_err(io_error)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or_else(|| DomainError::code("download_size_overflow"))?;
        if total > artifact.size {
            return Err(DomainError::code("download_size_mismatch"));
        }
        let take = count.min(4 - magic.len());
        magic.extend_from_slice(&buffer[..take]);
        hash.update(&buffer[..count]);
        writer.write_all(&buffer[..count]).map_err(io_error)?;
        operation::advance(count as u64);
    }
    if total != artifact.size {
        return Err(DomainError::code("download_size_mismatch"));
    }
    if magic.as_slice() != b"PK\x03\x04" && magic.as_slice() != b"PK\x05\x06" {
        return Err(DomainError::code("download_not_zip"));
    }
    if format!("{:x}", hash.finalize()) != artifact.sha256 {
        return Err(DomainError::code("download_hash_mismatch"));
    }
    Ok(())
}

fn fetch_signed(
    artifact: &Artifact,
    deps: &SignedInstallDeps<'_>,
    control: &TaskControl,
) -> Result<PathBuf, DomainError> {
    crate::download::validate_artifact(artifact).map_err(io_error)?;
    operation::archive(artifact.size);
    operation::stage("connecting", None, None);
    let cache = deps.paths.downloads_dir();
    fs::create_dir_all(&cache).map_err(io_error)?;
    no_link(&cache)?;
    let cache = fs::canonicalize(cache).map_err(io_error)?;
    let partial = cache.join(format!("{}.partial", uuid::Uuid::new_v4()));
    let result =
        (|| {
            let mut url = approved_url(&artifact.url, artifact)?;
            let mut response = None;
            for _ in 0..=5 {
                boundary(deps, control)?;
                let mut attempt = deps.transport.get(url.as_str(), None, None);
                if attempt.is_err() {
                    boundary(deps, control)?;
                    attempt = deps.transport.get(url.as_str(), None, None);
                }
                let current = attempt
                    .map_err(|e| DomainError::new("download_network_error", e.to_string()))?;
                if (300..400).contains(&current.status) {
                    let next = url
                        .join(
                            current
                                .location
                                .as_deref()
                                .ok_or_else(|| DomainError::code("redirect_location_missing"))?,
                        )
                        .map_err(|_| DomainError::code("invalid_redirect_url"))?;
                    url = approved_url(next.as_str(), artifact)?;
                    continue;
                }
                if !(200..300).contains(&current.status) {
                    return Err(DomainError::code("download_http_error"));
                }
                if approved_url(&current.final_url, artifact)?.as_str() != url.as_str() {
                    return Err(DomainError::code("download_response_url_changed"));
                }
                if current.content_length.is_some_and(|n| n != artifact.size) {
                    return Err(DomainError::code("download_size_mismatch"));
                }
                if current.content_type.as_deref().is_some_and(|mime| {
                    mime.contains("text/html") || mime.contains("application/json")
                }) {
                    return Err(DomainError::code("download_not_zip"));
                }
                response = Some(current);
                break;
            }
            let mut response = response.ok_or_else(|| DomainError::code("too_many_redirects"))?;
            let required = artifact
                .size
                .checked_add(8 * 1024 * 1024)
                .ok_or_else(|| DomainError::code("download_size_overflow"))?;
            let available = fs2::available_space(&cache).map_err(io_error)?;
            if available < required {
                return Err(DomainError::new(
                    "insufficient_space",
                    format!("state:required={required}:available={available}"),
                ));
            }
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&partial)
                .map_err(io_error)?;
            operation::stage("downloading", Some(artifact.size), None);
            copy_signed(response.body.as_mut(), &mut file, artifact, deps, control)?;
            file.sync_all().map_err(io_error)?;
            drop(file);
            let destination = cache.join(format!("{}.zip", artifact.sha256));
            if destination.exists() {
                no_link(&destination)?;
                let mut existing = File::open(&destination).map_err(io_error)?;
                copy_signed(&mut existing, &mut io::sink(), artifact, deps, control)?;
                fs::remove_file(&partial).map_err(io_error)?;
            } else {
                // Atomic no-replace publication, matching dynamic acquisition.
                fs::hard_link(&partial, &destination).map_err(io_error)?;
                fs::remove_file(&partial).map_err(io_error)?;
            }
            Ok(destination)
        })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}
