//! Script import validation. Compilation never executes module top-level code.
use super::{
    invalid,
    model::{ScriptDescriptor, ScriptSourceDocument, SourceRule},
    Result,
};
use mlua::{chunk::ChunkMode, Lua, LuaOptions, StdLib};
use sha2::{Digest, Sha256};

fn digest(code: &str) -> String {
    format!("{:x}", Sha256::digest(code.as_bytes()))
}

pub const SCRIPT_API_VERSION: u32 = 1;

pub fn validate_code(source_id: &str, api_version: u32, code: &[u8]) -> Result<ScriptDescriptor> {
    if api_version != SCRIPT_API_VERSION {
        return Err(invalid("unsupported_script_api_version"));
    }
    let code = std::str::from_utf8(code).map_err(|_| invalid("invalid_script_source_utf8"))?;
    let compile = || -> mlua::Result<()> {
        let lua = Lua::new_with(StdLib::NONE, LuaOptions::default())?;
        lua.load(code)
            .set_name(format!("@source/{source_id}.lua"))
            .set_mode(ChunkMode::Text)
            .into_function()?;
        Ok(())
    };
    compile().map_err(|error| {
        invalid(format!(
            "script_source_syntax_error: source={source_id}: {error}"
        ))
    })?;
    Ok(ScriptDescriptor {
        api_version,
        sha256: digest(code),
    })
}

/// Reads only the captured rule/code pair. No compilation, execution or IO.
pub(crate) fn document(rule: &SourceRule, code: Option<&String>) -> Result<ScriptSourceDocument> {
    let descriptor = rule
        .script
        .as_ref()
        .filter(|_| rule.adapter == "lua")
        .ok_or_else(|| invalid("source_not_script"))?;
    if descriptor.api_version != SCRIPT_API_VERSION {
        return Err(invalid("unsupported_script_api_version"));
    }
    let code =
        code.ok_or_else(|| invalid(format!("script_source_digest_mismatch: source={}", rule.id)))?;
    if digest(code) != descriptor.sha256 {
        return Err(invalid(format!(
            "script_source_digest_mismatch: source={}",
            rule.id
        )));
    }
    Ok(ScriptSourceDocument {
        rule: rule.into(),
        api_version: descriptor.api_version,
        code: code.clone(),
        sha256: descriptor.sha256.clone(),
    })
}
