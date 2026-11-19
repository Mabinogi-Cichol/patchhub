//! Script import validation and single-file `meta` extraction.
use super::{
    adapters::script as runtime,
    invalid,
    model::{ScriptDescriptor, ScriptMeta, ScriptSourceDocument, SourceRole, SourceRule},
    Result,
};
use mlua::{chunk::ChunkMode, HookTriggers, Lua, LuaOptions, StdLib, Value, VmState};
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

const INIT_INSTRUCTION_LIMIT: u32 = 1_000_000;

fn digest(code: &str) -> String {
    format!("{:x}", Sha256::digest(code.as_bytes()))
}

pub const SCRIPT_API_VERSION: u32 = 1;
pub const AI_SOURCE_PROMPT: &str = include_str!("../../resources/ai-source-prompt.md");

#[derive(Debug, Clone)]
pub struct ParsedScript {
    pub meta: ScriptMeta,
    pub role: SourceRole,
    pub descriptor: ScriptDescriptor,
}

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

/// Compiles and evaluates the script module in a zero-network, zero-IO sandbox
/// to extract `meta` and verify exported entrypoints.
pub fn parse_script(source_label: &str, api_version: u32, code: &[u8]) -> Result<ParsedScript> {
    let descriptor = validate_code(source_label, api_version, code)?;
    let code_str = std::str::from_utf8(code).expect("validated utf-8");

    let lua = runtime::vm().map_err(|error| invalid(error.to_string()))?;
    let exceeded = Arc::new(AtomicBool::new(false));
    let hook_flag = Arc::clone(&exceeded);
    let remaining = std::cell::Cell::new(INIT_INSTRUCTION_LIMIT);
    lua.set_global_hook(
        HookTriggers::new().every_nth_instruction(10_000),
        move |_, _| {
            let left = remaining.get().saturating_sub(10_000);
            remaining.set(left);
            if left == 0 {
                hook_flag.store(true, Ordering::Relaxed);
                Err(mlua::Error::runtime("script_source_init_limit_exceeded"))
            } else {
                Ok(VmState::Continue)
            }
        },
    )
    .map_err(|error| invalid(error.to_string()))?;

    let env = runtime::environment(&lua, &exceeded).map_err(|error| invalid(error.to_string()))?;
    let module = lua
        .load(code_str)
        .set_name(format!("@source/{source_label}.lua"))
        .set_mode(ChunkMode::Text)
        .set_environment(env)
        .into_function()
        .map_err(|error| {
            invalid(format!(
                "script_source_syntax_error: source={source_label}: {error}"
            ))
        })?;

    let exports_val: Value = match module.call(()) {
        Ok(value) => value,
        Err(error) => {
            if exceeded.load(Ordering::Relaxed) {
                return Err(invalid("script_source_init_limit_exceeded"));
            }
            return Err(invalid(format!(
                "script_source_runtime_error: source={source_label}: {error}"
            )));
        }
    };
    if exceeded.load(Ordering::Relaxed) {
        return Err(invalid("script_source_init_limit_exceeded"));
    }

    let Value::Table(exports) = exports_val else {
        return Err(invalid(
            "script_source_contract_error: script module must return a table",
        ));
    };
    let meta_val: Value = exports
        .get("meta")
        .map_err(|error| invalid(format!("invalid_source_meta: {error}")))?;
    if !matches!(meta_val, Value::Table(_)) {
        return Err(invalid("invalid_source_meta: missing meta table"));
    }
    let meta: ScriptMeta = runtime::from_value(&lua, meta_val)
        .map_err(|error| invalid(format!("invalid_source_meta: {error}")))?;

    let check_fn = |name: &str| -> Result<bool> {
        let val: Value = exports
            .get(name)
            .map_err(|error| invalid(format!("script_source_contract_error: {error}")))?;
        match val {
            Value::Nil => Ok(false),
            Value::Function(_) => Ok(true),
            _ => Err(invalid(format!(
                "script_source_contract_error: {name} must be a function"
            ))),
        }
    };
    let has_collect = check_fn("collect_index")?;
    let has_discover = check_fn("discover")?;
    let _ = check_fn("resolve_detail")?;
    let _ = check_fn("match_appid")?;

    let role = match meta.role.as_deref() {
        Some("publisher") => {
            if !has_collect {
                return Err(invalid(
                    "script_source_contract_error: publisher must export collect_index",
                ));
            }
            SourceRole::Publisher
        }
        Some("fallback_indexer") => {
            if !has_discover {
                return Err(invalid(
                    "script_source_contract_error: fallback_indexer must export discover",
                ));
            }
            SourceRole::FallbackIndexer
        }
        Some(_) => return Err(invalid("invalid_source_rule")),
        None => match (has_collect, has_discover) {
            (true, false) => SourceRole::Publisher,
            (false, true) => SourceRole::FallbackIndexer,
            _ => {
                return Err(invalid(
                    "script_source_contract_error: export collect_index or discover, or specify meta.role",
                ))
            }
        },
    };

    Ok(ParsedScript {
        meta,
        role,
        descriptor,
    })
}

/// Reads only the captured rule/code pair. No compilation, execution or IO.
pub(crate) fn document(
    rule: &SourceRule,
    name: &str,
    code: Option<&str>,
) -> Result<ScriptSourceDocument> {
    let (descriptor, code) = verify(rule, code)?;
    let mut normalized = rule.clone();
    normalized.adapter = "lua".into();
    normalized.script = None;
    Ok(ScriptSourceDocument {
        source_id: rule.id.clone(),
        name: name.to_owned(),
        rule: normalized,
        api_version: descriptor.api_version,
        code: code.to_owned(),
        sha256: descriptor.sha256.clone(),
    })
}

pub(crate) fn verify<'rule, 'code>(
    rule: &'rule SourceRule,
    code: Option<&'code str>,
) -> Result<(&'rule ScriptDescriptor, &'code str)> {
    let descriptor = rule
        .script
        .as_ref()
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
    Ok((descriptor, code))
}

/// Compare a captured parser definition with the latest stored definition.
/// Enablement is checked separately by the task's existing policy binding.
pub(crate) fn rule_changed(rule: &SourceRule, settings: &super::model::SettingsFile) -> bool {
    let Some(current) = settings.added.iter().find(|current| current.id == rule.id) else {
        return false;
    };
    let mut current = current.clone();
    super::builtin::bind(&mut current);
    current.enabled = rule.enabled;
    super::rule_fingerprint(&current) != super::rule_fingerprint(rule)
}
