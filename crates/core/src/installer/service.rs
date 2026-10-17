//! Validated public installation boundary. Recipes and game paths are never caller inputs.
use super::{
    outcome::{
        CancellationCause, CleanupErrorCode, CleanupFailure, CleanupResult, CleanupStatus,
        InstallerOutcome,
    },
    signed, InstallerPaths, Transport, WorkerRuntime,
};
use crate::{
    catalog::Platform,
    install::{self, InstallOperation},
    operation,
    projection::DomainError,
    sources, steam,
    trust::revision::{TrustRevisionStore, TrustedCatalogProvider},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

fn checked(value: &str, max: usize, code: &str) -> Result<String, DomainError> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(DomainError::code(code));
    }
    Ok(value.to_owned())
}
macro_rules! identity {
    ($name:ident,$max:expr,$code:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name(String);
        impl $name {
            pub fn new(value: &str) -> Result<Self, DomainError> {
                Ok(Self(checked(value, $max, $code)?))
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}
identity!(RequestId, 128, "invalid_request_id");
/// Steam identity includes both complete paths; never hash or truncate durable IDs.
pub const MAX_INSTANCE_ID_BYTES: usize = 16 * 1024;
identity!(InstanceId, MAX_INSTANCE_ID_BYTES, "invalid_instance_id");
identity!(ReleaseId, 256, "invalid_release_id");
identity!(PlanId, 256, "invalid_plan_id");
#[derive(Debug, Clone)]
pub struct ZipSelectionId(String);
impl ZipSelectionId {
    pub fn new(value: &str) -> Result<Self, DomainError> {
        checked(value, 256, "invalid_selection_id")?;
        super::resolve::parse_zip_selection_id(value)
            .ok_or_else(|| DomainError::code("invalid_selection_id"))?;
        Ok(Self(value.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Debug, Clone)]
pub struct ExeSelectionId(String);
impl ExeSelectionId {
    pub fn new(value: &str) -> Result<Self, DomainError> {
        checked(value, 256, "invalid_selection_id")?;
        if super::exe_resolve::parse_exe_selection_id(value).is_none() {
            return Err(DomainError::code("invalid_selection_id"));
        }
        Ok(Self(value.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Debug, Clone)]
pub struct LocalArchive(PathBuf);
impl LocalArchive {
    pub fn new(path: &Path) -> Result<Self, DomainError> {
        let path = std::path::absolute(path)?;
        // The signed entry checks this file's declared digest once before the
        // single extraction pass; the user keeps ownership of the file.
        let meta = std::fs::symlink_metadata(&path)?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(DomainError::code("invalid_local_archive"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(DomainError::code("invalid_local_archive"));
            }
        }
        install::validate_projection_state(&path, &[])?;
        Ok(Self(path))
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}
#[derive(Debug, Clone)]
pub enum InstallTarget {
    SignedRelease {
        release_id: ReleaseId,
        archive: Option<LocalArchive>,
    },
    PublisherZipSelection {
        selection_id: ZipSelectionId,
    },
    PublisherExeSelection {
        selection_id: ExeSelectionId,
    },
}
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum CleanupPolicy {
    #[default]
    Disabled,
    AfterCommit,
}
pub struct InstallRequest {
    pub request_id: RequestId,
    pub instance_id: InstanceId,
    pub operation: InstallOperation,
    pub target: InstallTarget,
    pub cleanup_policy: CleanupPolicy,
}

/// Only trusted Steam roots and a single state root are stored. Every write re-reads them.
#[derive(Debug)]
pub struct ValidatedHostContext {
    state: PathBuf,
    roots: Vec<PathBuf>,
    platform: Platform,
    paths: InstallerPaths,
}
impl ValidatedHostContext {
    pub fn new(state: &Path, roots: &[PathBuf], platform: Platform) -> Result<Self, DomainError> {
        if roots.len() > 32 {
            return Err(DomainError::code("too_many_roots"));
        }
        let actual = if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Steamos
        };
        if platform != actual {
            return Err(DomainError::code("platform_mismatch"));
        }
        let state = std::path::absolute(state)?;
        let roots = if roots.is_empty() {
            steam::discover_roots()
        } else {
            roots.to_vec()
        };
        let report = steam::scan(&roots);
        install::validate_projection_state(&state, &report.installs)?;
        let paths = InstallerPaths::new(&state);
        Ok(Self {
            state,
            roots,
            platform,
            paths,
        })
    }
    pub fn state_dir(&self) -> &Path {
        &self.state
    }
    pub fn installer_paths(&self) -> &InstallerPaths {
        &self.paths
    }
    pub fn platform(&self) -> Platform {
        self.platform
    }
    pub fn ensure_write_identity(&self) -> Result<(), DomainError> {
        if let Some(code) = super::host_identity_blocker() {
            Err(DomainError::code(code))
        } else {
            Ok(())
        }
    }
    pub(crate) fn refresh(&self) -> Result<steam::ScanReport, DomainError> {
        self.ensure_write_identity()?;
        self.refresh_ro()
    }
    pub(crate) fn refresh_ro(&self) -> Result<steam::ScanReport, DomainError> {
        let report = steam::scan(&self.roots);
        install::validate_projection_state(&self.state, &report.installs)?;
        install::validate_projection_state(self.paths.root(), &report.installs)?;
        Ok(report)
    }
}
pub trait Clock: Sync {
    fn now_seconds(&self) -> Result<u64, DomainError>;
}
pub struct SystemClock;
impl Clock for SystemClock {
    fn now_seconds(&self) -> Result<u64, DomainError> {
        crate::trust::now().map_err(Into::into)
    }
}
pub trait TaskLease: Sync {
    fn cancelled(&self) -> bool;
    fn cancellation_cause(&self) -> Option<CancellationCause>;
}
pub struct InstallerDeps<'a> {
    pub host: &'a ValidatedHostContext,
    pub catalog: &'a dyn TrustedCatalogProvider,
    pub revision_store: &'a dyn TrustRevisionStore,
    pub transport: &'a dyn Transport,
    pub worker: Option<&'a WorkerRuntime>,
    pub control: &'a dyn TaskLease,
    pub clock: &'a dyn Clock,
    pub emit: &'a mut dyn FnMut(operation::Progress),
}
fn task_error(e: super::ZipTaskError) -> DomainError {
    match e.detail {
        Some(detail) => DomainError::new(e.code, detail),
        None => DomainError::code(e.code),
    }
}
fn query(game: &steam::GameInstall) -> sources::GameQuery {
    sources::GameQuery {
        appid: game.appid,
        name: game.name.clone(),
        localized_name: game.localized_name.clone(),
        instance_id: Some(game.instance_id.clone()),
    }
}
pub fn install(
    request: InstallRequest,
    deps: InstallerDeps<'_>,
) -> Result<InstallerOutcome, DomainError> {
    let report = deps.host.refresh()?;
    let game = report
        .installs
        .iter()
        .find(|game| game.instance_id == request.instance_id.as_str())
        .ok_or_else(|| DomainError::code("instance_not_found"))?;
    if game.availability != "installed" {
        return Err(DomainError::code("game_not_ready"));
    }
    if deps.revision_store.state_dir() != Some(deps.host.state_dir()) {
        return Err(DomainError::code("trust_state_mismatch"));
    }
    let now = deps.clock.now_seconds()?;
    let reason = || {
        deps.control.cancellation_cause().or_else(|| {
            deps.control
                .cancelled()
                .then_some(CancellationCause::UserCancelled)
        })
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let done = AtomicBool::new(false);
    let paths = &deps.host.paths;
    let cache_guard = Arc::new(super::cache_lock::CacheLock::shared(deps.host)?);
    super::init_atomic(paths)?;
    (deps.emit)(operation::Progress {
        stage: "planning".into(),
        ..Default::default()
    });
    let execution = std::thread::scope(|scope| {
        let control = deps.control;
        let flag = cancel.clone();
        let done = &done;
        scope.spawn(move || {
            while !done.load(Ordering::Acquire) {
                if control.cancelled() {
                    flag.store(true, Ordering::Release);
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        struct Finish<'a>(&'a AtomicBool);
        impl Drop for Finish<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let _finish = Finish(done);
        let safety_guard = cache_guard.clone();
        operation::with_cancel(cancel.clone(), || {
            operation::with_safety(
                move || {
                    safety_guard
                        .validate()
                        .map_err(|error| std::io::Error::other(error.code))
                },
                || match &request.target {
                    InstallTarget::SignedRelease {
                        release_id,
                        archive,
                    } => signed::install_signed(
                        signed::SignedInstallInputs {
                            game,
                            release_id: release_id.as_str(),
                            archive: archive.as_ref().map(LocalArchive::path),
                            operation: request.operation,
                            request_id: request.request_id.as_str(),
                            now,
                        },
                        signed::SignedInstallDeps {
                            paths,
                            provider: deps.catalog,
                            revision_store: deps.revision_store,
                            transport: deps.transport,
                            cancel: &cancel,
                            cancel_reason: &reason,
                            platform: deps.host.platform,
                        },
                    ),
                    InstallTarget::PublisherZipSelection { selection_id } => {
                        let source = sources::Sources::open(deps.host.state_dir())?;
                        let selection = super::resolve::resolve_zip(
                            source.registry(),
                            &query(game),
                            selection_id.as_str(),
                        )
                        .map_err(|e| DomainError::code(e.code))?;
                        super::orchestrator::execute_zip(
                            super::ZipInstallInputs {
                                game,
                                selection: &selection,
                                operation: request.operation,
                                request_id: request.request_id.as_str(),
                            },
                            super::ZipInstallDeps {
                                paths,
                                transport: deps.transport,
                                cancel: &cancel,
                                cancel_reason: &reason,
                            },
                        )
                        .map_err(task_error)
                    }
                    InstallTarget::PublisherExeSelection { selection_id } => {
                        let source = sources::Sources::open(deps.host.state_dir())?;
                        let selection = super::exe_resolve::resolve_exe(
                            source.registry(),
                            &query(game),
                            selection_id.as_str(),
                        )
                        .map_err(|e| DomainError::code(e.code))?;
                        let runtime = deps
                            .worker
                            .ok_or_else(|| DomainError::code("exe_sandbox_unavailable"))?;
                        let probe = runtime.probe();
                        if !probe.available {
                            return Err(DomainError::new("exe_sandbox_unavailable", probe.reason));
                        }
                        super::exe_orchestrator::execute_exe(
                            super::exe_orchestrator::ExeInstallInputs {
                                game,
                                selection: &selection,
                                operation: request.operation,
                                request_id: request.request_id.as_str(),
                            },
                            super::exe_orchestrator::ExeInstallDeps {
                                paths,
                                transport: deps.transport,
                                runtime,
                                cancel: &cancel,
                                cancel_reason: &reason,
                            },
                        )
                        .map_err(task_error)
                    }
                },
            )
        })
    })
    .map_err(|error| {
        if matches!(
            error.code.as_str(),
            "cancelled" | "task_cancelled" | "download_cancelled"
        ) {
            DomainError::new("cancelled", error.code)
        } else {
            error
        }
    })?;
    let mut outcome = InstallerOutcome::from_transaction(&execution.transaction, None, None);
    // Never upgrade a shared lock. The route's task/request owners are gone here.
    drop(cache_guard);
    if request.cleanup_policy == CleanupPolicy::AfterCommit
        && !execution.replayed
        && execution.transaction.state == "committed"
    {
        operation::stage("cleanup", None, None);
        (deps.emit)(operation::Progress {
            stage: "cleanup".into(),
            ..Default::default()
        });
        outcome.cleanup = super::cache::CacheService::new(deps.host)
            .cleanup()
            .unwrap_or_else(|error| {
                let busy = error.code == "installer_busy";
                CleanupResult {
                    status: if busy {
                        CleanupStatus::Busy
                    } else {
                        CleanupStatus::Failed
                    },
                    performed: false,
                    removed_bytes: 0,
                    failures: vec![CleanupFailure {
                        code: if busy {
                            CleanupErrorCode::InstallerBusy
                        } else {
                            CleanupErrorCode::CleanupFailed
                        },
                        detail: Some(error.code),
                    }],
                }
            });
    }
    (deps.emit)(operation::Progress {
        stage: "finalizing".into(),
        ..Default::default()
    });
    Ok(outcome)
}
fn validate_saved_plan(
    report: &steam::ScanReport,
    plan: &install::InstallPlan,
) -> Result<(), DomainError> {
    let game = report
        .installs
        .iter()
        .find(|game| game.instance_id == plan.instance_id)
        .ok_or_else(|| DomainError::code("recovery_instance_not_found"))?;
    if plan.install.instance_id != game.instance_id
        || plan.install.appid != game.appid
        || plan.install.install_path != game.install_path
        || plan.install.library_path != game.library_path
    {
        return Err(DomainError::code("recovery_instance_mismatch"));
    }
    Ok(())
}
pub fn recover(host: &ValidatedHostContext) -> Result<Vec<install::Transaction>, DomainError> {
    host.refresh()?;
    // A missing/empty journal stays read-only: there is no lifecycle write to guard.
    if install::list_transactions_at(host.paths.root())?.is_empty() {
        return Ok(Vec::new());
    }
    let cache_guard = super::cache_lock::CacheLock::shared(host)?;
    install::recover_checked_at(host.paths.root(), |plan| {
        cache_guard
            .validate()
            .map_err(|error| std::io::Error::other(error.code))?;
        let report = host
            .refresh()
            .map_err(|error| std::io::Error::other(error.code))?;
        validate_saved_plan(&report, plan).map_err(|error| std::io::Error::other(error.code))
    })
    .map_err(Into::into)
}
pub fn discard_plan(
    host: &ValidatedHostContext,
    id: &PlanId,
) -> Result<install::Transaction, DomainError> {
    host.refresh()?;
    install::get_plan_ro(host.paths.root(), id.as_str())?;
    let cache_guard = super::cache_lock::CacheLock::shared(host)?;
    install::discard_plan_checked_at(host.paths.root(), id.as_str(), |plan| {
        cache_guard
            .validate()
            .map_err(|error| std::io::Error::other(error.code))?;
        let report = host
            .refresh()
            .map_err(|error| std::io::Error::other(error.code))?;
        validate_saved_plan(&report, plan).map_err(|error| std::io::Error::other(error.code))
    })
    .map_err(Into::into)
}
