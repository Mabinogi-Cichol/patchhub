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
pub mod store;
pub mod validate;

pub use model::*;

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ---- §0 frozen constants (P1 implements, P2/P3 consume) ------------------

pub const MAX_HTML: usize = 4 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 4096;
pub const MAX_SOURCES: usize = 32;
pub const MAX_DISCOVERY_INPUT: usize = 32 * 1024;
pub const MAX_DISCOVERY_OUTPUT: u64 = 512 * 1024;
pub const MAX_OVERRIDES_BYTES: u64 = 256 * 1024;
pub const MAX_STATUS_BYTES: u64 = 4096;
pub const MAX_CACHE_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_CANDIDATES: usize = 128;
pub const MAX_MIRRORS_PERSIST: usize = 32;
pub const MAX_MIRRORS_WORKER: usize = 64;
pub const MAX_EVIDENCE: usize = 64;

pub const PARSER_VERSION: u32 = 2;
pub const QUERY_VERSION: u32 = 1;
/// Deterministic identity encoding version (P0 contract v0.5 §0/§1). Part of
/// `query_key` and persisted on discovery/index records so an encoding change
/// invalidates confirmed discovery and shared indexes.
pub const SOURCES_ENCODING_VERSION: u32 = 1;
pub const DEFAULT_RULES_SCHEMA: u32 = 2;
pub const STORE_SCHEMA_VERSION: u32 = 1;

pub const SOURCE_CACHE_TTL: u64 = 24 * 3600;
pub const DISCOVERY_SUCCESS_TTL: u64 = 6 * 3600;
pub const DISCOVERY_FALLBACK_TTL: u64 = 24 * 3600;
pub const DISCOVERY_INDEX_TTL: u64 = 24 * 3600;
pub const OBSERVED_TTL: u64 = 6 * 3600;

pub const REFRESH_WORKER_TIMEOUT: Duration = Duration::from_secs(180);
pub const DISCOVERY_CONTEXT_DEADLINE: Duration = Duration::from_secs(29);
pub const DISCOVERY_CONTEXT_BUDGET: u32 = 24;
pub const OFFICIAL_SUBDEADLINE: Duration = Duration::from_secs(6);
/// Additive offset for `min(budget, requests + OFFICIAL_SUBBUDGET_ADDEND)`.
///
/// Python expresses `OFFICIAL_SUBBUDGET` as the expression `requests + 3`, which
/// cannot be a value constant; the additive part is frozen here and the
/// whole-value helper is [`official_subbudget`].
pub const OFFICIAL_SUBBUDGET_ADDEND: u32 = 3;
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
pub const DETAIL_REQUEST_LIMIT: u32 = 6;
pub const PAGINATION_SEEN_LIMIT: usize = 100;

pub const BUILTIN_ADAPTERS: &[&str] = &[
    "patchmoe",
    "table_html",
    "kagura_wordpress",
    "kagura_woocommerce",
    "hgamer_html",
];

/// discovery.py internal adapter set; not a valid rule `adapter` value on its own.
pub const DISCOVERY_ADAPTERS: &[&str] = &["kagura_wordpress", "kagura_woocommerce", "hgamer_html"];

/// `min(budget, requests + OFFICIAL_SUBBUDGET_ADDEND)` (design §0).
pub fn official_subbudget(budget: u32, requests: u32) -> u32 {
    budget.min(requests.saturating_add(OFFICIAL_SUBBUDGET_ADDEND))
}

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

    /// Opens with an explicit rules document (`schema_version` 1 or 2).
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
        if config.schema_version != 1 && config.schema_version != 2 {
            return Err(invalid("invalid_sources_config"));
        }
        let builtins: Vec<SourceRule> = config
            .sources
            .iter()
            .map(validate::validate_rule)
            .collect::<Result<Vec<_>>>()?;
        let store = store::SourceStore::open(state_dir, &builtins)?;
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
            DISCOVERY_CONTEXT_BUDGET,
            DISCOVERY_CONTEXT_DEADLINE,
            Duration::from_secs_f64(1.0),
        ))
    }

    /// Discover one game; the shared budget/deadline is per game (design §0).
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
        // Build contexts up front so the context factory never borrows `self`
        // alongside `&mut self.registry`. Extra contexts (duplicate AppIDs) are
        // cheap and unused.
        let mut contexts = Vec::with_capacity(games.len());
        for _ in games {
            contexts.push(self.new_context()?);
        }
        let mut contexts = contexts.into_iter();
        let make_ctx = move || contexts.next().ok_or_else(|| invalid("source_state_busy"));
        session::scan_sources(&mut self.registry, make_ctx, games, force, cancel, on_event)
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

    /// Capability negotiation payload.
    pub fn get_capabilities(&self) -> session::Capabilities {
        session::capabilities()
    }

    /// Host-only effective rule + origin + fingerprint for a source id.
    pub fn get_source_policy(&self, source_id: &str) -> Result<session::SourcePolicy> {
        session::source_policy(&self.registry, source_id)
    }

    /// Resolve a legacy release/record id for installation consumers.
    pub fn resolve_source_record(
        &self,
        instance_id: &str,
        record_id: &str,
    ) -> Result<session::ResolvedSource> {
        session::resolve_source_record(&self.registry, instance_id, record_id)
    }
}

fn default_fetcher() -> FetcherFactory {
    Box::new(|| Ok(Box::new(http::ReqwestFetcher::new()?)))
}
