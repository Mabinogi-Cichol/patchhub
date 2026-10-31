//! Current-parser bindings for synthetic unit-test records, never production.
use super::{model::*, registry::SourceRegistry, Result};

pub(crate) fn default_rule(id: &str) -> SourceRule {
    let config: RulesConfig = serde_json::from_str(super::DEFAULT_RULES_JSON).unwrap();
    super::validate::validate_rule(config.sources.iter().find(|r| r.id == id).unwrap()).unwrap()
}

pub(crate) fn save_match(registry: &SourceRegistry, mut record: GameMatchRecord) -> Result<()> {
    for (id, source) in &mut record.sources {
        let rule = registry.rules().iter().find(|r| r.id == *id).unwrap();
        let fingerprint = super::index::parser_fingerprint(rule);
        for candidate in &mut source.candidates {
            // Selection identities are deliberately supplied by each test.
            candidate.parser_fingerprint = fingerprint.clone();
        }
        if fingerprint.is_some() && registry.store().load_index(id)?.is_none() {
            registry.store().save_index(&SourceIndex {
                parser_fingerprint: fingerprint,
                schema_version: 1,
                source_id: id.clone(),
                revision: source.index_revision,
                complete: true,
                checked_at: source.checked_at,
                updated_at: source.checked_at,
                etag: None,
                modified: None,
                entries: vec![],
                pending: vec![],
            })?;
        }
    }
    registry.store().save_match(&record)
}
