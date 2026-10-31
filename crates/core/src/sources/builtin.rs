//! Legacy adapter names locate bundled text; every execution uses Lua.
use super::{model::ScriptDescriptor, script::SCRIPT_API_VERSION, SourceRule};
use sha2::{Digest, Sha256};

pub fn code(adapter: &str) -> Option<&'static str> {
    match adapter {
        "patchmoe" => Some(concat!(
            include_str!("../../resources/source-scripts/common.lua"),
            "\n",
            include_str!("../../resources/source-scripts/patchmoe.lua")
        )),
        "table_html" | "table" => Some(concat!(
            include_str!("../../resources/source-scripts/common.lua"),
            "\n",
            include_str!("../../resources/source-scripts/table.lua")
        )),
        "kagura_wordpress" => Some(concat!(
            include_str!("../../resources/source-scripts/common.lua"),
            "\n",
            include_str!("../../resources/source-scripts/kagura.lua"),
            "\nreturn kagura(false)\n"
        )),
        "kagura_woocommerce" => Some(concat!(
            include_str!("../../resources/source-scripts/common.lua"),
            "\n",
            include_str!("../../resources/source-scripts/kagura.lua"),
            "\nreturn kagura(true)\n"
        )),
        "hgamer_html" => Some(concat!(
            include_str!("../../resources/source-scripts/common.lua"),
            "\n",
            include_str!("../../resources/source-scripts/hgamer.lua")
        )),
        _ => None,
    }
}

/// Bind the bundled revision without executing, writing, or networking.
/// User code uses adapter=lua and is never rebound to a bundled revision.
pub fn bind(rule: &mut SourceRule) {
    if let Some(descriptor) = descriptor(&rule.adapter) {
        rule.script = Some(descriptor);
    }
}
pub fn descriptor(adapter: &str) -> Option<ScriptDescriptor> {
    code(adapter).map(|code| ScriptDescriptor {
        api_version: SCRIPT_API_VERSION,
        sha256: format!("{:x}", Sha256::digest(code.as_bytes())),
    })
}

/// Fresh installations can read a bundled script without creating source state.
pub fn document(source_id: &str) -> super::Result<super::ScriptSourceDocument> {
    let config: super::RulesConfig = serde_json::from_str(super::DEFAULT_RULES_JSON)?;
    let raw = config
        .sources
        .iter()
        .find(|raw| raw.id == source_id)
        .ok_or_else(|| super::invalid("unknown_source"))?;
    let rule = super::validate::validate_rule(raw)?;
    super::script::document(&rule, code(&rule.adapter))
}
