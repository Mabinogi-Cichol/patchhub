//! Immutable, fully materialized source state for projection and resolution.
use super::{
    invalid, model::*, registry::SourceRegistry, store::SourceStore, validate, Clock, Result,
    DEFAULT_RULES_JSON,
};
use std::path::{Path, PathBuf};
fn linked_metadata(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
struct SnapshotClock(u64);
impl Clock for SnapshotClock {
    fn now_unix(&self) -> u64 {
        self.0
    }
}
/// Read-only source state captured for supplied games at one fixed time.
pub struct SourceSnapshot {
    state_root: PathBuf,
    now_unix: u64,
    registry: SourceRegistry,
}
impl SourceSnapshot {
    /// Missing source state returns `Ok(None)` without creating any directory.
    pub fn load(state_dir: &Path, games: &[GameQuery], now_unix: u64) -> Result<Option<Self>> {
        let absolute_state = std::path::absolute(state_dir)?;
        let state_dir = absolute_state.as_path();
        let paths = super::store::StorePaths::new(state_dir);
        let root = paths.root();
        let mut absent = false;
        for component in root.ancestors() {
            match std::fs::symlink_metadata(component) {
                Ok(metadata) if linked_metadata(&metadata) => {
                    return Err(invalid("linked_source_state"))
                }
                Ok(metadata) if !metadata.is_dir() && component != root => {
                    return Err(invalid("linked_source_state"))
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => absent = true,
                Err(error) => return Err(error),
            }
        }
        if absent {
            return Ok(None);
        }
        let store = SourceStore::open_readonly(state_dir)?;
        let config: RulesConfig = serde_json::from_str(DEFAULT_RULES_JSON)
            .map_err(|_| invalid("invalid_sources_config"))?;
        if config.schema_version != 2 {
            return Err(invalid("invalid_sources_config"));
        }
        let builtins = config
            .sources
            .iter()
            .map(validate::validate_rule)
            .collect::<Result<Vec<_>>>()?;
        let mut registry =
            SourceRegistry::load_with_clock(store, builtins, Box::new(SnapshotClock(now_unix)))?;
        for rule in registry.rules() {
            if let Some(status) = registry.store().load_status(&rule.id)? {
                if status.code.is_empty() {
                    return Err(invalid("invalid_source_status"));
                }
            }
        }
        registry.preload_snapshot_games(games)?;
        Ok(Some(Self {
            state_root: state_dir.to_path_buf(),
            now_unix,
            registry,
        }))
    }
    pub fn settings_revision(&self) -> u64 {
        self.registry.settings_revision()
    }
    pub fn now_unix(&self) -> u64 {
        self.now_unix
    }
    pub fn diagnostics(&self) -> &[Diagnostic] {
        self.registry.diagnostics()
    }
    pub fn state_root(&self) -> &Path {
        &self.state_root
    }
    /// Crate-only access keeps public callers away from the registry's store/mutators.
    pub(crate) fn registry(&self) -> &SourceRegistry {
        &self.registry
    }
}
