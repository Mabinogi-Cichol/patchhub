//! Read-only foundation for Steam patch management.
//! No download or game-file mutation is exposed in this milestone.
pub mod catalog;
pub mod download;
pub mod install;
pub mod keyvalues;
pub mod operation;
pub mod steam;
pub mod trust;
