//! Read-only foundation for Steam patch management.
//! No download or game-file mutation is exposed in this milestone.
pub mod actions;
mod appinfo;
pub mod catalog;
pub mod detection;
pub mod download;
pub mod install;
pub mod installer;
pub mod keyvalues;
pub mod operation;
pub mod projection;
pub mod sources;
pub mod steam;
mod steam_metadata;
pub mod trust;

pub mod zip_metadata;

#[cfg(test)]
extern crate self as steam_patch_core;
#[cfg(test)]
mod install_tests;
