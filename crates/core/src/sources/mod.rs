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
pub mod model;
pub mod registry;
pub mod session;
pub mod snapshot;
pub mod store;
pub mod validate;

pub use model::*;

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ---- §0 frozen constants (P1 implements, P2/P3 consume) ------------------

pub const PARSER_VERSION: u32 = 2;
pub const QUERY_VERSION: u32 = 1;
/// Deterministic identity encoding version (P0 contract v0.5 §0/§1). Part of
/// `query_key` and persisted on discovery/index records so an encoding change
/// invalidates confirmed discovery and shared indexes.
pub const SOURCES_ENCODING_VERSION: u32 = 1;
pub const DEFAULT_RULES_SCHEMA: u32 = 2;
pub const STORE_SCHEMA_VERSION: u32 = 1;

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
/// revision authority. `fetcher` builds the HTTP context used by discovery and
/// refresh so tests can inject an offline fetcher.
pub type FetcherFactory = Box<dyn Fn() -> Result<Box<dyn http::Fetcher>>>;

pub struct Sources {
    registry: registry::SourceRegistry,
    fetcher: FetcherFactory,
}

impl Sources {
    /// Opens using the compiled-in default rules.
    pub fn open(state_dir: &Path) -> Result<Self> {
        Self::open_with_rules(state_dir, DEFAULT_RULES_JSON)
    }

    /// Opens with an explicit rules document (`schema_version` must be 2).
    pub fn open_with_rules(state_dir: &Path, rules_json: &str) -> Result<Self> {
        Self::open_with_rules_and_fetcher(state_dir, rules_json, default_fetcher())
    }

    /// Opens with an injected fetcher (offline / fixture use).
    pub fn open_with_rules_and_fetcher(
        state_dir: &Path,
        rules_json: &str,
        fetcher: FetcherFactory,
    ) -> Result<Self> {
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
        Ok(Self { registry, fetcher })
    }

    /// Wraps an already-built registry (e.g. one with a `FakeClock`).
    pub fn with_registry(registry: registry::SourceRegistry, fetcher: FetcherFactory) -> Self {
        Self { registry, fetcher }
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
        session::rule_fingerprint(rule)
    }

    fn new_context(&self) -> Result<http::RequestContext> {
        let fetcher = (self.fetcher)()?;
        Ok(http::RequestContext::with_settings(
            fetcher,
            Duration::from_secs_f64(1.0),
        ))
    }

    /// Discover one game; each call traverses the source's queued work to
    /// completion (design §9).
    pub fn discover_game(
        &mut self,
        game: &GameQuery,
        name_hints: &[String],
        preferred_language: Option<&str>,
        force: bool,
        cancel: &AtomicBool,
    ) -> Result<GameDiscovery> {
        let mut ctx = self.new_context()?;
        session::discover_game(
            &mut self.registry,
            &mut ctx,
            game,
            name_hints,
            preferred_language,
            force,
            cancel,
        )
    }

    /// Bounded batch discovery with per-game progress events.
    pub fn scan_sources(
        &mut self,
        games: &[GameQuery],
        force: bool,
        cancel: &AtomicBool,
        on_event: impl FnMut(&session::ScanEvent),
    ) -> Result<Vec<GameDiscovery>> {
        self.scan_sources_with_query(games, force, &[], None, cancel, on_event)
    }

    pub fn scan_sources_with_query(
        &mut self,
        games: &[GameQuery],
        force: bool,
        hints: &[String],
        language: Option<&str>,
        cancel: &AtomicBool,
        on_event: impl FnMut(&session::ScanEvent),
    ) -> Result<Vec<GameDiscovery>> {
        // Build contexts up front so the context factory never borrows `self`
        // alongside `&mut self.registry`. Extra contexts (duplicate AppIDs) are
        // cheap and unused.
        let mut contexts = Vec::with_capacity(games.len());
        for _ in games {
            contexts.push(self.new_context()?);
        }
        let mut contexts = contexts.into_iter();
        let make_ctx = move || contexts.next().ok_or_else(|| invalid("source_state_busy"));
        session::scan_sources_with_query(
            &mut self.registry,
            make_ctx,
            games,
            force,
            hints,
            language,
            cancel,
            on_event,
        )
    }

    /// Refresh enabled ZIP-index sources, preserving cache on failure.
    pub fn refresh_sources(
        &mut self,
        force: bool,
        cancel: &AtomicBool,
    ) -> Result<Vec<SourceStatus>> {
        let mut ctx = self.new_context()?;
        session::refresh_sources(&mut self.registry, &mut ctx, force, cancel)
    }
}

fn default_fetcher() -> FetcherFactory {
    Box::new(|| Ok(Box::new(http::ReqwestFetcher::new()?)))
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
