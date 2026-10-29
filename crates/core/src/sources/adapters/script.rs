//! Lua source execution. HTTP borrows the task context; DOM owns its document.

use std::{
    cell::RefCell,
    collections::HashSet,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use crate::sources::{
    invalid,
    model::{Diagnostic, PageFact, QueryContext, SourceIndex, SourceIndexEntry},
    validate, Result,
};
use crate::{
    operation,
    sources::{
        http::{FetchResponse, RequestContext},
        model::SourceRule,
    },
};
use mlua::{
    chunk::ChunkMode, Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt, MultiValue, StdLib,
    Table, UserData, UserDataMethods, Value, VmState,
};
use scraper::{ElementRef, Html, Selector};
use serde::de::DeserializeOwned;
use serde::Deserialize;

/// Host-bound progress; a script supplies only the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptProgress {
    pub source_id: String,
    pub entry: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptDiagnostic {
    code: String,
    #[serde(default)]
    detail: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptIndex {
    #[serde(default)]
    entries: Vec<SourceIndexEntry>,
    #[serde(default)]
    pending: Vec<String>,
    #[serde(default)]
    complete: bool,
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    modified: Option<String>,
    #[serde(default)]
    not_modified: bool,
    #[serde(default)]
    diagnostics: Vec<ScriptDiagnostic>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryPage {
    pub fact: PageFact,
    #[serde(default = "unknown_format")]
    pub reported_format: String,
    #[serde(default)]
    pub upstream_page: Option<String>,
}
fn unknown_format() -> String {
    "unknown".into()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityFact {
    pub appid: u32,
    pub page: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptDiscovery {
    pub pages: Vec<DiscoveryPage>,
    #[serde(default)]
    pub identities: Vec<IdentityFact>,
    pub complete: bool,
    #[serde(default)]
    diagnostics: Vec<ScriptDiagnostic>,
}

#[derive(Debug)]
pub struct DiscoveryFacts {
    pub pages: Vec<DiscoveryPage>,
    pub identities: Vec<IdentityFact>,
    pub complete: bool,
    pub diagnostics: Vec<Diagnostic>,
}

fn failure(
    rule: &SourceRule,
    entry: &str,
    code: &str,
    detail: impl std::fmt::Display,
) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        crate::projection::DomainError::new(
            code,
            format!("source={}, entry={entry}: {detail}", rule.id),
        ),
    )
}

#[allow(clippy::too_many_arguments)]
fn run<T: DeserializeOwned>(
    rule: &SourceRule,
    code: &str,
    entry: &str,
    inputs: &[serde_json::Value],
    ctx: &mut RequestContext,
    cancel: Arc<AtomicBool>,
    mut report: impl FnMut(ScriptProgress),
) -> Result<T> {
    if cancel.load(Ordering::Relaxed) {
        return Err(invalid("task_cancelled"));
    }
    crate::sources::script::verify(rule, Some(code)).map_err(|error| {
        let message = error.to_string();
        let code = if message.starts_with("script_source_digest_mismatch") {
            "script_source_digest_mismatch"
        } else if message == "unsupported_script_api_version" {
            "unsupported_script_api_version"
        } else {
            "script_source_contract_error"
        };
        failure(rule, entry, code, error)
    })?;
    let phase = std::cell::Cell::new("script_source_runtime_error");
    let result = execute(
        code,
        entry,
        rule,
        inputs,
        ctx,
        Arc::clone(&cancel),
        |message| {
            report(ScriptProgress {
                source_id: rule.id.clone(),
                entry: entry.into(),
                message: message.into(),
            });
        },
        &phase,
    );
    if cancel.load(Ordering::Relaxed) {
        return Err(invalid("task_cancelled"));
    }
    result
        .map(|value| value.value)
        .map_err(|error| failure(rule, entry, phase.get(), error))
}

fn diagnostics(rule: &SourceRule, values: Vec<ScriptDiagnostic>) -> Vec<Diagnostic> {
    values
        .into_iter()
        .map(|item| Diagnostic {
            source_id: Some(rule.id.clone()),
            code: item.code,
            detail: item.detail,
        })
        .collect()
}

fn contract<T>(rule: &SourceRule, entry: &str, result: Result<T>) -> Result<T> {
    result.map_err(|error| failure(rule, entry, "script_source_contract_error", error))
}

/// Execute an imported publisher's index entry, without publishing state.
pub fn collect_index(
    rule: &SourceRule,
    code: &str,
    previous: Option<&SourceIndex>,
    ctx: &mut RequestContext,
    cancel: Arc<AtomicBool>,
    report: impl FnMut(ScriptProgress),
) -> Result<super::IndexOutcome> {
    let previous_input = previous.map(|index| serde_json::json!({"entries":index.entries,
        "pending":index.pending,"complete":index.complete,"etag":index.etag,"modified":index.modified}))
        .unwrap_or(serde_json::Value::Null);
    let mut output: ScriptIndex = run(
        rule,
        code,
        "collect_index",
        &[previous_input],
        ctx,
        Arc::clone(&cancel),
        report,
    )?;
    contract(
        rule,
        "collect_index",
        (|| {
            if output.not_modified {
                let previous = previous
                    .filter(|index| index.complete)
                    .ok_or_else(|| invalid("source_cache_missing"))?;
                output.entries = previous.entries.clone();
                output.pending = previous.pending.clone();
                output.complete = true;
            }
            if output.complete && !output.pending.is_empty() {
                return Err(invalid("complete_index_has_pending_pages"));
            }
            for entry in &output.entries {
                if entry.title.trim().is_empty()
                    || !matches!(
                        entry.kind.as_str(),
                        "entry" | "product" | "article" | "category"
                    )
                    || entry.appid == Some(0)
                {
                    return Err(invalid("invalid_source_entry"));
                }
                validate::allowed_url(&entry.page, &rule.page_hosts, false)?;
                if entry.kind == "entry" {
                    let url = entry
                        .url
                        .as_deref()
                        .ok_or_else(|| invalid("invalid_source_entry"))?;
                    validate::allowed_url(url, &rule.download_hosts, true)?;
                }
            }
            for page in &output.pending {
                validate::allowed_url(page, &rule.page_hosts, false)?;
            }
            Ok(())
        })(),
    )?;
    if cancel.load(Ordering::Relaxed) {
        return Err(invalid("task_cancelled"));
    }
    Ok(super::IndexOutcome {
        entries: output.entries,
        pending: output.pending,
        complete: output.complete,
        etag: output.etag,
        modified: output.modified,
        not_modified: output.not_modified,
        checked_at: super::now_unix(),
        diagnostics: diagnostics(rule, output.diagnostics),
    })
}

fn validate_fact(rule: &SourceRule, fact: &mut PageFact) -> Result<()> {
    // Use an explicit worklist for category inline facts, without a page quota.
    let mut pending = vec![fact];
    while let Some(fact) = pending.pop() {
        if fact.title.trim().is_empty()
            || !matches!(fact.kind.as_str(), "product" | "article" | "category")
            || fact.steam_ids.contains(&0)
        {
            return Err(invalid("invalid_page_fact"));
        }
        validate::allowed_url(&fact.page, &rule.page_hosts, false)?;
        for mirror in &mut fact.mirrors {
            validate::navigation_url(&mirror.url)?;
            mirror.format = crate::sources::identity::format(&mirror.url);
        }
        for link in &fact.article_links {
            validate::allowed_url(&link.page, &rule.page_hosts, false)?;
        }
        if let Some(next) = &fact.next_page {
            validate::allowed_url(next, &rule.page_hosts, false)?;
        }
        pending.extend(fact.article_facts.iter_mut());
    }
    Ok(())
}

pub fn resolve_page(
    rule: &SourceRule,
    code: &str,
    page: &str,
    ctx: &mut RequestContext,
    cancel: Arc<AtomicBool>,
    report: impl FnMut(ScriptProgress),
) -> Result<PageFact> {
    validate::allowed_url(page, &rule.page_hosts, false)?;
    let mut fact: PageFact = run(
        rule,
        code,
        "resolve_page",
        &[serde_json::json!(page)],
        ctx,
        Arc::clone(&cancel),
        report,
    )?;
    contract(rule, "resolve_page", validate_fact(rule, &mut fact))?;
    if cancel.load(Ordering::Relaxed) {
        return Err(invalid("task_cancelled"));
    }
    Ok(fact)
}

pub fn discover(
    rule: &SourceRule,
    code: &str,
    query: &QueryContext,
    previous: &super::DiscoverPrevious,
    ctx: &mut RequestContext,
    cancel: Arc<AtomicBool>,
    report: impl FnMut(ScriptProgress),
) -> Result<DiscoveryFacts> {
    let query = serde_json::json!({"appid":query.appid,"name":query.name,"name_hints":query.name_hints,
        "terms":crate::sources::identity::query_terms(&query.name, &query.name_hints),"force":query.force});
    let previous = serde_json::json!({"known_candidates": previous.known_candidates});
    let mut output: ScriptDiscovery = run(
        rule,
        code,
        "discover",
        &[query, previous],
        ctx,
        Arc::clone(&cancel),
        report,
    )?;
    contract(
        rule,
        "discover",
        (|| {
            for page in &mut output.pages {
                validate_fact(rule, &mut page.fact)?;
                if !matches!(
                    page.reported_format.as_str(),
                    "unknown" | "zip" | "exe" | "web"
                ) {
                    return Err(invalid("invalid_reported_format"));
                }
                if let Some(upstream) = &page.upstream_page {
                    validate::navigation_url(upstream)?;
                }
            }
            for identity in &output.identities {
                if identity.appid == 0 {
                    return Err(invalid("invalid_source_appid"));
                }
                validate::allowed_url(&identity.page, &rule.page_hosts, false)?;
            }
            Ok(())
        })(),
    )?;
    if cancel.load(Ordering::Relaxed) {
        return Err(invalid("task_cancelled"));
    }
    Ok(DiscoveryFacts {
        pages: output.pages,
        identities: output.identities,
        complete: output.complete,
        diagnostics: diagnostics(rule, output.diagnostics),
    })
}

#[derive(Clone)]
struct Node {
    document: Rc<Html>,
    id: ego_tree::NodeId,
}

impl Node {
    fn element(&self) -> ElementRef<'_> {
        ElementRef::wrap(
            self.document
                .tree
                .get(self.id)
                .expect("owned document node"),
        )
        .expect("element handle")
    }
}

impl UserData for Node {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("select", |lua, node, css: String| {
            let selector = Selector::parse(&css)
                .map_err(|error| mlua::Error::runtime(format!("invalid CSS selector: {error}")))?;
            let nodes = node
                .element()
                .select(&selector)
                .map(|element| Node {
                    document: Rc::clone(&node.document),
                    id: element.id(),
                })
                .collect::<Vec<_>>();
            lua.create_sequence_from(nodes)
        });
        methods.add_method("text", |_, node, ()| {
            Ok(node
                .element()
                .text()
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "))
        });
        methods.add_method("attr", |_, node, name: String| {
            Ok(node.element().attr(&name).map(str::to_owned))
        });
        methods.add_method("tag", |_, node, ()| {
            Ok(node.element().value().name().to_string())
        });
        methods.add_method("parent", |_, node, ()| {
            Ok(node
                .element()
                .parent()
                .and_then(ElementRef::wrap)
                .map(|element| Node {
                    document: Rc::clone(&node.document),
                    id: element.id(),
                }))
        });
    }
}

fn environment(lua: &Lua, cancel: &Arc<AtomicBool>) -> mlua::Result<Table> {
    let globals = lua.globals();
    let environment = lua.create_table()?;
    for name in [
        "_VERSION", "assert", "error", "ipairs", "next", "pairs", "select", "tonumber", "tostring",
        "type", "string", "table", "math", "utf8",
    ] {
        environment.set(name, globals.get::<Value>(name)?)?;
    }
    // Bytecode is neither accepted nor an API output.
    environment
        .get::<Table>("string")?
        .set("dump", Value::Nil)?;
    // Raw protected-call primitives are private upvalues. Recheck cancellation
    // before and after them so catching a hook/host error cannot keep running,
    // including inside non-yieldable C callbacks (string.gsub/table.sort).
    for name in ["pcall", "xpcall"] {
        let protected: Function = globals.get(name)?;
        let flag = Arc::clone(cancel);
        environment.set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                if flag.load(Ordering::Relaxed) {
                    return Err(mlua::Error::runtime("task_cancelled"));
                }
                let result = protected.call::<MultiValue>(args);
                if flag.load(Ordering::Relaxed) {
                    return Err(mlua::Error::runtime("task_cancelled"));
                }
                result
            })?,
        )?;
    }
    environment.set("_G", environment.clone())?;
    Ok(environment)
}

fn vm() -> mlua::Result<Lua> {
    Lua::new_with(
        StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8,
        LuaOptions::default(),
    )
}

// mlua's default conversion drops string keys on non-empty sequence tables;
// detect_mixed_tables also encodes {} as []. Validate the actual JSON/DTO
// boundary once, then use normal serde conversion to preserve empty objects.
fn from_value<T: DeserializeOwned>(lua: &Lua, value: Value) -> mlua::Result<T> {
    let mut active = HashSet::new();
    let mut pending = vec![(value.clone(), false)];
    while let Some((value, leaving)) = pending.pop() {
        if matches!(value, Value::Number(number) if !number.is_finite()) {
            return Err(mlua::Error::runtime("non-finite script number"));
        }
        let Value::Table(table) = value else { continue };
        if leaving {
            active.remove(&table.to_pointer());
            continue;
        }
        if !active.insert(table.to_pointer()) {
            return Err(mlua::Error::runtime("recursive script table"));
        }
        pending.push((Value::Table(table.clone()), true));
        let (mut strings, mut integers, mut largest) = (0, 0, 0);
        for pair in table.pairs::<Value, Value>() {
            let (key, value) = pair?;
            match key {
                Value::String(_) => strings += 1,
                Value::Integer(i) if i > 0 => {
                    integers += 1;
                    largest = largest.max(i);
                }
                _ => {
                    return Err(mlua::Error::runtime(
                        "script table keys must be strings or array indices",
                    ))
                }
            }
            pending.push((value, false));
        }
        if integers > 0 && strings > 0 {
            return Err(mlua::Error::runtime("mixed script table"));
        }
        if largest != integers {
            return Err(mlua::Error::runtime("sparse script array"));
        }
        if strings > 0 && table.metatable().as_ref() == Some(&lua.array_metatable()) {
            return Err(mlua::Error::runtime("string keys on script array"));
        }
    }
    lua.from_value(value)
}

fn json_value(lua: &Lua, value: &serde_json::Value) -> mlua::Result<Value> {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            serde_json::Value::Number(number) if number.is_u64() && number.as_i64().is_none() => {
                return Err(mlua::Error::runtime(
                    "JSON integer exceeds Lua signed 64-bit range; use a string ID",
                ));
            }
            serde_json::Value::Array(values) => pending.extend(values),
            serde_json::Value::Object(values) => pending.extend(values.values()),
            _ => {}
        }
    }
    lua.to_value(value)
}

fn response(lua: &Lua, response: FetchResponse) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("status", response.status)?;
    table.set("text", response.text())?;
    table.set("url", response.final_url)?;
    table.set("etag", response.etag)?;
    table.set("modified", response.last_modified)?;
    Ok(table)
}

/// The host owns cancellation; a caught Lua error cannot authorize publication.
fn call(
    function: Function,
    args: impl mlua::IntoLuaMulti,
    cancel: &AtomicBool,
) -> mlua::Result<Value> {
    operation::check().map_err(mlua::Error::external)?;
    let result = function.call::<Value>(args);
    if cancel.load(Ordering::Relaxed) {
        return Err(mlua::Error::external(std::io::Error::other(
            "task_cancelled",
        )));
    }
    operation::check().map_err(mlua::Error::external)?;
    result
}

#[derive(Debug)]
struct Invocation<T> {
    pub value: T,
    #[cfg(test)]
    pub lua_bytes: usize,
}

#[allow(clippy::too_many_arguments)] // Explicit borrowed task context and script inputs.
fn execute<T: DeserializeOwned>(
    code: &str,
    entry: &str,
    rule: &SourceRule,
    inputs: &[serde_json::Value],
    ctx: &mut RequestContext,
    cancel: Arc<AtomicBool>,
    mut progress: impl FnMut(&str),
    phase: &std::cell::Cell<&'static str>,
) -> mlua::Result<Invocation<T>> {
    operation::with_cancel(Arc::clone(&cancel), || {
        let lua = vm()?;
        let flag = Arc::clone(&cancel);
        // Sampling cadence for cancellation, not an instruction quota.
        lua.set_global_hook(
            HookTriggers::new().every_nth_instruction(10_000),
            move |_, _| {
                if flag.load(Ordering::Relaxed) {
                    Err(mlua::Error::runtime("task_cancelled"))
                } else {
                    Ok(VmState::Continue)
                }
            },
        )?;
        let network = RefCell::new(ctx);
        lua.scope(|scope| {
            let host = lua.create_table()?;
            host.set(
                "get",
                scope.create_function(|lua, (url, validators): (String, Option<Table>)| {
                    let etag: Option<String> = validators
                        .as_ref()
                        .map(|v| v.get("etag"))
                        .transpose()?
                        .flatten();
                    let modified: Option<String> = validators
                        .as_ref()
                        .map(|v| v.get("modified"))
                        .transpose()?
                        .flatten();
                    let fetched = network
                        .borrow_mut()
                        .fetch_conditional(
                            &url,
                            &rule.page_hosts,
                            etag.as_deref(),
                            modified.as_deref(),
                        )
                        .map_err(mlua::Error::external)?;
                    response(lua, fetched)
                })?,
            )?;
            host.set(
                "post",
                scope.create_function(
                    |lua, (url, content_type, body): (String, String, String)| {
                        let fetched = network
                            .borrow_mut()
                            .fetch_post(
                                &url,
                                &rule.page_hosts,
                                Some(&content_type),
                                body.into_bytes(),
                            )
                            .map_err(mlua::Error::external)?;
                        response(lua, fetched)
                    },
                )?,
            )?;
            host.set(
                "form",
                scope.create_function(|lua, (url, fields): (String, Value)| {
                    let fields: Vec<(String, String)> = from_value(lua, fields)?;
                    let fetched = network
                        .borrow_mut()
                        .fetch_form(&url, &rule.page_hosts, &fields)
                        .map_err(mlua::Error::external)?;
                    response(lua, fetched)
                })?,
            )?;
            host.set(
                "html",
                lua.create_function(|_, html: String| {
                    let document = Rc::new(Html::parse_document(&html));
                    let id = document.root_element().id();
                    Ok(Node { document, id })
                })?,
            )?;
            host.set(
                "url_join",
                lua.create_function(|_, (base, reference): (String, String)| {
                    let base = url::Url::parse(&base).map_err(mlua::Error::external)?;
                    Ok(base
                        .join(&reference)
                        .map_err(mlua::Error::external)?
                        .to_string())
                })?,
            )?;
            host.set("null", lua.null())?;
            host.set(
                "array",
                lua.create_function(|lua, ()| {
                    let array = lua.create_table()?;
                    array.set_metatable(Some(lua.array_metatable()))?;
                    Ok(array)
                })?,
            )?;
            host.set(
                "json_decode",
                lua.create_function(|lua, text: String| {
                    let value: serde_json::Value =
                        serde_json::from_str(&text).map_err(mlua::Error::external)?;
                    json_value(lua, &value)
                })?,
            )?;
            host.set(
                "json_encode",
                lua.create_function(|lua, value: Value| {
                    let value: serde_json::Value = from_value(lua, value)?;
                    serde_json::to_string(&value).map_err(mlua::Error::external)
                })?,
            )?;
            host.set(
                "check_cancel",
                lua.create_function(|_, ()| operation::check().map_err(mlua::Error::external))?,
            )?;
            host.set(
                "progress",
                scope.create_function_mut(|_, message: String| {
                    operation::check().map_err(mlua::Error::external)?;
                    progress(&message);
                    Ok(())
                })?,
            )?;
            phase.set("script_source_syntax_error");
            let module = lua
                .load(code)
                .set_name(format!("@source/{}.lua", rule.id))
                .set_mode(ChunkMode::Text)
                .set_environment(environment(&lua, &cancel)?)
                .into_function()?;
            phase.set("script_source_runtime_error");
            let exports = call(module, (), &cancel)?;
            phase.set("script_source_contract_error");
            let Value::Table(exports) = exports else {
                return Err(mlua::Error::runtime("script module must return a table"));
            };
            let function: Function = exports.get(entry)?;
            let mut args = MultiValue::from_vec(vec![Value::Table(host), lua.to_value(rule)?]);
            for input in inputs {
                args.push_back(json_value(&lua, input)?);
            }
            phase.set("script_source_runtime_error");
            let output = call(function, args, &cancel)?;
            phase.set("script_source_contract_error");
            let value = from_value(&lua, output)?;
            operation::check().map_err(mlua::Error::external)?;
            Ok(Invocation {
                value,
                #[cfg(test)]
                lua_bytes: lua.used_memory(),
            })
        })
    })
}

#[cfg(test)]
#[path = "script/tests.rs"]
mod tests;
