//! Shared Steam patch domain services: projection, actions, trusted installation,
//! transaction recovery, cache policy and source discovery. Writing entry points
//! validate host identity, trusted catalog state and paths before mutating game files.
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
extern crate self as patchhub_core;
#[cfg(test)]
mod install_tests;
