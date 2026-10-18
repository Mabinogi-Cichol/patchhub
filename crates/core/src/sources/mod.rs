//! Shared source discovery foundation.
//!
//! P1: [`model`], [`validate`], [`canonical`], the persistent [`store`], the
//! in-memory [`registry`] and the [`Sources`] facade, plus the frozen §0
//! constants, the [`Clock`] abstraction and the `invalid` error helper.
//!
//! The core crate must not depend on Tauri / Decky / Python / the front-end
//! appStore (design §1).

pub mod adapters;
pub mod canonical;
pub mod http;
pub mod identity;
pub mod index;
pub mod r#match;
pub mod matching;
pub mod model;
pub mod registry;
pub mod schedule;
pub mod snapshot;
pub mod store;
pub mod update;
pub mod validate;

pub use model::*;

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// ---- §0 frozen constants (P1 implements, P2/P3 consume) ------------------

pub const PARSER_VERSION: u32 = 2;
pub const QUERY_VERSION: u32 = 1;
/// Deterministic identity encoding version (P0 contract v0.5 §0/§1). Part of
/// `query_key` and persisted on discovery/index records so an encoding change
/// invalidates confirmed discovery and shared indexes.
pub const SOURCES_ENCODING_VERSION: u32 = 1;
pub const DEFAULT_RULES_SCHEMA: u32 = 2;
pub const STORE_SCHEMA_VERSION: u32 = 2;
/// On-disk schema of `index/<source_id>.json` (p1-contract §1, §5).
pub const SOURCE_INDEX_SCHEMA_VERSION: u32 = 1;
/// On-disk schema of `details/<source_id>/<key>.json` (p1-contract §1, §6).
pub const SOURCE_DETAIL_SCHEMA_VERSION: u32 = 1;
/// On-disk schema of `matches/<appid>.json` (p1-contract §1, §7.1).
pub const GAME_MATCH_SCHEMA_VERSION: u32 = 1;

pub const BUILTIN_ADAPTERS: &[&str] = &[
    "patchmoe",
    "table_html",
    "kagura_wordpress",
    "kagura_woocommerce",
    "hgamer_html",
];

/// discovery.py internal adapter set; not a valid rule `adapter` value on its own.
pub const DISCOVERY_ADAPTERS: &[&str] = &["kagura_wordpress", "kagura_woocommerce", "hgamer_html"];

/// Default rules resource, inlined at compile time (design §3).
///
/// Relative to `crates/core/src/sources/` this resolves to
/// `crates/core/resources/sources.json`.
pub const DEFAULT_RULES_JSON: &str = include_str!("../../resources/sources.json");

pub type Result<T> = std::io::Result<T>;

/// Uniform invalid-data error used by every P1 error code (design §2.6).
pub fn invalid(code: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, code.into())
}

/// Wall-clock source for TTL checks; `Instant`-based deadlines stay separate (R11).
pub trait Clock {
    fn now_unix(&self) -> u64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0)
    }
}

/// Test / fixture clock. Cloning is intentionally absent; wrap in `Arc` when shared.
pub struct FakeClock(AtomicU64);

impl FakeClock {
    pub fn new(now_unix: u64) -> Self {
        Self(AtomicU64::new(now_unix))
    }

    pub fn set(&self, now_unix: u64) {
        self.0.store(now_unix, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now_unix(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Facade tying the persistent store and the in-memory registry together
/// (design §1.1). The registry owns the store so every mutation shares one
/// revision authority.
pub struct Sources {
    registry: registry::SourceRegistry,
}

impl Sources {
    /// Opens using the compiled-in default rules.
    pub fn open(state_dir: &Path) -> Result<Self> {
        Self::open_with_rules(state_dir, DEFAULT_RULES_JSON)
    }

    /// Opens with an explicit rules document (`schema_version` must be 2).
    pub fn open_with_rules(state_dir: &Path, rules_json: &str) -> Result<Self> {
        let config: RulesConfig =
            serde_json::from_str(rules_json).map_err(|_| invalid("invalid_sources_config"))?;
        if config.schema_version != 2 {
            return Err(invalid("invalid_sources_config"));
        }
        let builtins: Vec<SourceRule> = config
            .sources
            .iter()
            .map(validate::validate_rule)
            .collect::<Result<Vec<_>>>()?;
        let store = store::SourceStore::open(state_dir)?;
        let registry = registry::SourceRegistry::load(store, builtins)?;
        Ok(Self { registry })
    }

    pub fn registry(&self) -> &registry::SourceRegistry {
        &self.registry
    }

    pub fn registry_mut(&mut self) -> &mut registry::SourceRegistry {
        &mut self.registry
    }

    pub fn store(&self) -> &store::SourceStore {
        self.registry.store()
    }

    pub fn settings_revision(&self) -> u64 {
        self.registry.settings_revision()
    }

    /// Read-only effective rule fingerprint (`sha256(canonical_bytes(rule))`),
    /// exposed so installer resolution and authorization share one definition
    /// of a rule's identity (p1-zip-orchestration §1.4).
    pub fn rule_fingerprint(rule: &SourceRule) -> String {
        rule_fingerprint(rule)
    }
}

/// `sha256(canonical_bytes(rule))`: the frozen rule identity digest.
pub fn rule_fingerprint(rule: &SourceRule) -> String {
    canonical::canonical_sha256(rule)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_schema_version_must_be_two() {
        let temp = tempfile::tempdir().unwrap();
        for version in [0_u32, 1, 3] {
            let json = format!(r#"{{"schema_version":{version},"sources":[]}}"#);
            let error = match Sources::open_with_rules(temp.path(), &json) {
                Ok(_) => panic!("schema_version {version} must be rejected"),
                Err(error) => error,
            };
            assert_eq!(
                error.to_string(),
                "invalid_sources_config",
                "schema_version {version} must be rejected"
            );
        }

        // Schema 2 is accepted (empty rules are structurally valid).
        if let Err(error) =
            Sources::open_with_rules(temp.path(), r#"{"schema_version":2,"sources":[]}"#)
        {
            panic!("schema 2 rules must be accepted: {error}");
        }
    }
}
