//! Read-only foundation for Steam patch management.
//! No download or game-file mutation is exposed in this milestone.
mod appinfo;
pub mod catalog;
pub mod download;
pub mod install;
pub mod keyvalues;
pub mod operation;
pub mod sources;
pub mod steam;
mod steam_metadata;
pub mod trust;
