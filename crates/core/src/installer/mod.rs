//! Shared installer services and persistent `installer/rust-v1/` state schema.
//!
//! The CLI calls [`service::install`] for signed ZIP, publisher ZIP and publisher
//! EXE requests. The service validates the host and delegates to the shared
//! request ledger and direct-install orchestrators. Recovery and
//! discard also enter through [`service`]; [`cache::CacheService`] owns cache
//! deletion and coordinates with writers. All routes retain the same state root.

pub mod acquire;
pub mod backup;
pub mod cache;
mod cache_lock;
mod direct;
pub mod exe_orchestrator;
pub mod exe_report;
pub mod exe_resolve;
pub mod facts;
pub mod manual_selection;
pub mod orchestrator;
pub mod outcome;
pub mod requests;
pub mod resolve;
pub mod service;
mod signed;
pub mod space;
pub mod store;
pub mod task;
mod zip_direct;
pub mod zip_recipe;

pub use acquire::{
    acquire, acquire_with, https_transport, validate_download, AcquiredArchive, Acquisition,
    DownloadRequest, HttpResponse, Transport,
};
pub use facts::{
    digest_file, game_facts, read_file, BaselineExpectation, BaselineFact, BuildFacts, DlcFacts,
    Fact, FactFailure, FactsExpectation, FileDigest, GameFacts, InstanceFacts, LanguageFacts,
    ManifestFacts, MAX_MANIFEST_BYTES,
};
pub use orchestrator::{
    begin_request, content_conflict, record_plan, record_terminal, request_binding, ZipTaskError,
};
pub(crate) use orchestrator::{ZipInstallDeps, ZipInstallInputs};
pub use requests::{RequestBinding, RequestLedger, RequestRecord};
pub use resolve::{
    parse_zip_selection_id, resolve_zip, source_released, zip_selection_id, ResolveError,
    ResolvedZipCandidate,
};
pub use store::{init_atomic, instance_hash, InstallerPaths};

/// `installer/rust-v1/schema.json` version (p0-contracts §0).
pub const STORE_SCHEMA_VERSION: u32 = 1;

pub type Result<T> = std::io::Result<T>;

/// Uniform invalid-data error matching the `sources` module convention.
pub fn invalid(code: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, code.into())
}

pub use patchhub_installer_worker::production::WorkerRuntime;

/// The trusted Linux host must launch the whole core under its chosen non-root user.
pub fn host_identity_blocker() -> Option<&'static str> {
    #[cfg(target_os = "linux")]
    if unsafe { libc::geteuid() } == 0 {
        return Some("installer_host_requires_non_root");
    }
    None
}

/// Execution metadata is transient: a committed replay never repeats cache cleanup.
pub(crate) struct InstallExecution {
    pub transaction: crate::install::Transaction,
    pub replayed: bool,
}
impl InstallExecution {
    fn fresh(transaction: crate::install::Transaction) -> Self {
        Self {
            transaction,
            replayed: false,
        }
    }
    fn replay(transaction: crate::install::Transaction) -> Self {
        Self {
            transaction,
            replayed: true,
        }
    }
}
