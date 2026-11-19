//! Legacy adapter names locate bundled text; every execution uses Lua.
use super::{
    invalid,
    model::{RulesConfig, ScriptSourceDocument, SourceRule},
    script::{self, SCRIPT_API_VERSION},
    validate, Result,
};
use std::sync::OnceLock;

include!(concat!(env!("OUT_DIR"), "/bundled_sources.rs"));

#[derive(Debug, Clone)]
pub struct BuiltinSource {
    pub rule: SourceRule,
    pub default_name: String,
}

fn parse_builtin_sources(rules_json: &str) -> Result<Vec<BuiltinSource>> {
    let config: RulesConfig =
        serde_json::from_str(rules_json).map_err(|_| invalid("invalid_sources_config"))?;
    if config.schema_version != 2 {
        return Err(invalid("invalid_sources_config"));
    }
    config
        .sources
        .into_iter()
        .map(|entry| {
            let source_code =
                code(&entry.adapter).ok_or_else(|| invalid("invalid_sources_config"))?;
            let parsed =
                script::parse_script(&entry.id, SCRIPT_API_VERSION, source_code.as_bytes())?;
            let (rule, default_name) = validate::build_rule_from_meta(
                entry.id,
                entry.adapter,
                entry.enabled,
                &parsed.meta,
                parsed.role,
                parsed.descriptor,
            )?;
            Ok(BuiltinSource { rule, default_name })
        })
        .collect()
}

pub fn load_rules(rules_json: &str) -> Result<Vec<BuiltinSource>> {
    static DEFAULT_PARSED: OnceLock<Vec<BuiltinSource>> = OnceLock::new();
    if std::ptr::eq(rules_json, DEFAULT_RULES_JSON) || rules_json == DEFAULT_RULES_JSON {
        return Ok(DEFAULT_PARSED
            .get_or_init(|| {
                parse_builtin_sources(DEFAULT_RULES_JSON)
                    .expect("bundled default rules must be valid")
            })
            .clone());
    }
    parse_builtin_sources(rules_json)
}

pub fn default_builtins() -> Vec<BuiltinSource> {
    load_rules(DEFAULT_RULES_JSON).expect("bundled default rules must be valid")
}

/// Bind the bundled revision without executing, writing, or networking.
/// User code uses adapter=lua and is never rebound to a bundled revision.
pub fn bind(rule: &mut SourceRule) {
    if let Some(descriptor) = descriptor(&rule.adapter) {
        rule.script = Some(descriptor);
    }
}

pub fn descriptor(adapter: &str) -> Option<super::model::ScriptDescriptor> {
    code(adapter).and_then(|source_code| {
        script::validate_code(adapter, SCRIPT_API_VERSION, source_code.as_bytes()).ok()
    })
}

/// Fresh installations can read a bundled script without creating source state.
pub fn document(source_id: &str) -> Result<ScriptSourceDocument> {
    let builtin = load_rules(DEFAULT_RULES_JSON)?
        .into_iter()
        .find(|entry| entry.rule.id == source_id)
        .ok_or_else(|| invalid("unknown_source"))?;
    script::document(
        &builtin.rule,
        &builtin.default_name,
        code(&builtin.rule.adapter),
    )
}
