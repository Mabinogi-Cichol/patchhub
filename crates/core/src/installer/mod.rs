//! P1 installer state schema `installer/rust-v1/` (p0-contracts §3).
//!
//! This module freezes the on-disk layout and provides the atomic, idempotent
//! bootstrap ([`store::init_atomic`]) plus the request-idempotency ledger
//! ([`requests`]). It intentionally does **not** wire into the protocol layer:
//! production callers still use the legacy `install::Store` root until a later
//! P1 task switches them over.

pub mod acquire;
pub mod requests;
pub mod store;
pub mod zip_recipe;

pub use acquire::{
    acquire, acquire_with, validate_download, AcquiredArchive, Acquisition, DownloadRequest,
    HttpResponse, Transport,
};
pub use requests::{RequestBinding, RequestLedger, RequestRecord};
pub use store::{init_atomic, instance_hash, InstallerPaths};
pub use zip_recipe::build_recipe;

/// `installer/rust-v1/schema.json` version (p0-contracts §0).
pub const STORE_SCHEMA_VERSION: u32 = 1;

pub type Result<T> = std::io::Result<T>;

/// Uniform invalid-data error matching the `sources` module convention.
pub fn invalid(code: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, code.into())
}
