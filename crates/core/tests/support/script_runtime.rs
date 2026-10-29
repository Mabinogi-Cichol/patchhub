//! P0 experiment only. Not registered in the production adapter dispatcher.
//! Keep the borrowed HTTP context scoped; DOM handles own their document.

use std::{
    cell::RefCell,
    collections::HashSet,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use mlua::{
    chunk::ChunkMode, Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt, MultiValue, StdLib,
    Table, UserData, UserDataMethods, Value, VmState,
};
use patchhub_core::{
    operation,
    sources::{
        http::{FetchResponse, RequestContext},
        model::SourceRule,
    },
};
use scraper::{ElementRef, Html, Selector};
use serde::de::DeserializeOwned;

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

pub fn environment(lua: &Lua, cancel: &Arc<AtomicBool>) -> mlua::Result<Table> {
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

pub fn vm() -> mlua::Result<Lua> {
    Lua::new_with(
        StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8,
        LuaOptions::default(),
    )
}

pub fn syntax_check(code: &[u8]) -> mlua::Result<()> {
    vm()?
        .load(code)
        .set_mode(ChunkMode::Text)
        .into_function()
        .map(|_| ())
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
pub fn call(
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
pub struct Invocation<T> {
    pub value: T,
    pub lua_bytes: usize,
}

pub fn invoke<T: DeserializeOwned>(
    code: &str,
    entry: &str,
    rule: &SourceRule,
    input: &serde_json::Value,
    ctx: &mut RequestContext,
    cancel: Arc<AtomicBool>,
    progress: &mut Vec<String>,
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
                    progress.push(message);
                    Ok(())
                })?,
            )?;
            let module = lua
                .load(code)
                .set_name(format!("@source/{}.lua", rule.id))
                .set_mode(ChunkMode::Text)
                .set_environment(environment(&lua, &cancel)?)
                .into_function()?;
            let exports = call(module, (), &cancel)?;
            let Value::Table(exports) = exports else {
                return Err(mlua::Error::runtime("script module must return a table"));
            };
            let function: Function = exports.get(entry)?;
            let output = call(
                function,
                (host, lua.to_value(rule)?, lua.to_value(input)?),
                &cancel,
            )?;
            let value = from_value(&lua, output)?;
            operation::check().map_err(mlua::Error::external)?;
            Ok(Invocation {
                value,
                lua_bytes: lua.used_memory(),
            })
        })
    })
}
