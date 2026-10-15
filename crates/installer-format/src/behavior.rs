//! Static, fail-closed lowering of bounded Lua syntax into installation predicates.
//! Only reviewed read-only APIs and inert UI operations are modeled. Lua is never executed.
use crate::{audit::safe_target, lua, require, Error, Result};
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

pub const VERSION: &str = "sf9-behavior-v2";
const ROOT: &str = "X:\\__scanned_game__";
const MAX_STEPS: usize = 200_000;
const MAX_BRANCHES: usize = 1024;
const MAX_VALUE_SIZE: usize = 1024 * 1024;
const MAX_IR_NODES: usize = 8192;
const MAX_ANALYSIS_ALLOCATION: usize = 64 * 1024 * 1024;
fn error(code: &str) -> Error {
    Error(code.into())
}
fn array(value: &Value) -> Result<&Vec<Value>> {
    value
        .as_array()
        .ok_or_else(|| error("exe_lua_syntax_invalid"))
}
fn string(value: &Value) -> Result<&str> {
    value
        .as_str()
        .ok_or_else(|| error("exe_lua_syntax_invalid"))
}

type TableValue = Rc<RefCell<Rc<Vec<(Val, Val)>>>>;

#[derive(Clone, Debug)]
enum Val {
    Nil,
    Bool(bool),
    Int(i128),
    Float(f64),
    Str(Rc<str>),
    Table(TableValue),
    Function(Rc<Value>),
    Namespace(String),
    Api(String),
    Symbol(Rc<(String, Vec<Val>, usize, usize)>),
}
impl Val {
    fn literal(value: &Value) -> Result<Self> {
        Ok(match value {
            Value::Null => Self::Nil,
            Value::Bool(b) => Self::Bool(*b),
            Value::String(s) => Self::Str(s.as_str().into()),
            Value::Number(n) if n.is_i64() => Self::Int(n.as_i64().unwrap() as i128),
            Value::Number(n) if n.is_u64() => Self::Int(n.as_u64().unwrap() as i128),
            Value::Number(n) => Self::Float(
                n.as_f64()
                    .ok_or_else(|| error("exe_lua_number_unsupported"))?,
            ),
            _ => return Err(error("exe_lua_expression_unsupported")),
        })
    }
    fn symbol(kind: &str, values: Vec<Self>) -> Self {
        // Track the expanded size of the shared DAG without walking it. This
        // also bounds structural equality work before the final IR encoding.
        let nodes = values.iter().fold(1usize, |n, v| {
            n.saturating_add(match v {
                Self::Symbol(s) => s.2,
                _ => 1,
            })
        });
        let depth = 1 + values
            .iter()
            .map(|v| match v {
                Self::Symbol(s) => s.3,
                _ => 0,
            })
            .max()
            .unwrap_or(0);
        Self::Symbol(Rc::new((kind.into(), values, nodes, depth)))
    }
    fn check_symbol_bound(&self) -> Result<()> {
        if let Self::Symbol(s) = self {
            require(s.2 <= MAX_IR_NODES && s.3 <= 96, "exe_condition_ir_limit")?;
        }
        if let Self::Table(table) = self {
            Self::table_dimensions(table, &mut HashMap::new(), 0)?;
        }
        Ok(())
    }
    fn table_dimensions(
        table: &TableValue,
        memo: &mut HashMap<usize, (usize, usize)>,
        parent_depth: usize,
    ) -> Result<(usize, usize)> {
        require(parent_depth < 96, "exe_lua_table_limit")?;
        let key = Rc::as_ptr(table) as usize;
        if let Some(dimensions) = memo.get(&key) {
            return Ok(*dimensions);
        }
        let fields = table.borrow().clone();
        let mut nodes = 1usize;
        let mut depth = 1usize;
        for (key, value) in fields.iter() {
            for value in [key, value] {
                let (child_nodes, child_depth) = match value {
                    Self::Table(child) => Self::table_dimensions(child, memo, parent_depth + 1)?,
                    Self::Symbol(symbol) => (symbol.2, symbol.3),
                    _ => (1, 0),
                };
                nodes = nodes.saturating_add(child_nodes);
                depth = depth.max(child_depth + 1);
                require(nodes <= MAX_IR_NODES && depth <= 96, "exe_lua_table_limit")?;
            }
        }
        memo.insert(key, (nodes, depth));
        Ok((nodes, depth))
    }
    fn symbolic(&self) -> bool {
        matches!(self, Self::Symbol(_))
    }
    fn truth(&self) -> Self {
        if self.symbolic() {
            self.clone()
        } else {
            Self::Bool(!matches!(self, Self::Nil | Self::Bool(false)))
        }
    }
    fn is_true(&self) -> bool {
        matches!(self, Self::Bool(true))
    }
    fn is_false(&self) -> bool {
        matches!(self, Self::Bool(false))
    }
    fn kind(&self) -> u8 {
        match self {
            Self::Nil => 0,
            Self::Bool(_) => 1,
            Self::Int(_) => 2,
            Self::Float(_) => 3,
            Self::Str(_) => 4,
            Self::Table(_) => 5,
            Self::Function(_) => 6,
            Self::Namespace(_) => 7,
            Self::Api(_) => 8,
            Self::Symbol(_) => 9,
        }
    }
    fn eq(&self, other: &Self) -> bool {
        self.eq_depth(other, 0)
    }
    fn eq_depth(&self, other: &Self, depth: usize) -> bool {
        if depth > 96 {
            return false;
        }
        match (self, other) {
            (Self::Nil, Self::Nil) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a == b,
            (Self::Int(a), Self::Float(b)) | (Self::Float(b), Self::Int(a)) => {
                int_float_cmp(*a, *b).is_some_and(|c| c.is_eq())
            }
            (Self::Bool(a), Self::Int(b)) | (Self::Int(b), Self::Bool(a)) => i128::from(*a) == *b,
            (Self::Bool(a), Self::Float(b)) | (Self::Float(b), Self::Bool(a)) => {
                i32::from(*a) as f64 == *b
            }
            (Self::Str(a), Self::Str(b)) => a == b,
            (Self::Namespace(a), Self::Namespace(b)) | (Self::Api(a), Self::Api(b)) => a == b,
            (Self::Function(a), Self::Function(b)) => Rc::ptr_eq(a, b) || a == b,
            (Self::Symbol(a), Self::Symbol(b)) => {
                Rc::ptr_eq(a, b)
                    || (a.0 == b.0
                        && a.1.len() == b.1.len()
                        && a.1.iter().zip(&b.1).all(|(x, y)| x.eq_depth(y, depth + 1)))
            }
            (Self::Table(a), Self::Table(b)) => {
                if Rc::ptr_eq(a, b) {
                    return true;
                }
                let a = a.borrow();
                let b = b.borrow();
                a.len() == b.len()
                    && a.iter().all(|(k, v)| {
                        b.iter()
                            .any(|(j, w)| k.eq_depth(j, depth + 1) && v.eq_depth(w, depth + 1))
                    })
            }
            _ => false,
        }
    }
    fn table(fields: Vec<(Val, Val)>) -> Self {
        Self::Table(Rc::new(RefCell::new(Rc::new(fields))))
    }
    fn text(&self) -> Option<&str> {
        if let Self::Str(s) = self {
            Some(s)
        } else {
            None
        }
    }
    fn to_json(&self, remaining: &mut usize, depth: usize) -> Result<Value> {
        require(*remaining > 0 && depth <= 96, "exe_condition_ir_limit")?;
        *remaining -= 1;
        Ok(match self {
            Self::Nil => Value::Null,
            Self::Bool(v) => json!(v),
            Self::Str(v) => json!(v.as_ref()),
            Self::Int(v) if *v >= 0 && *v <= u64::MAX as i128 => json!(*v as u64),
            Self::Int(v) if *v >= i64::MIN as i128 && *v <= i64::MAX as i128 => json!(*v as i64),
            Self::Symbol(symbol) => {
                let mut result = vec![json!("symbol"), json!(symbol.0)];
                for value in &symbol.1 {
                    result.push(value.to_json(remaining, depth + 1)?);
                }
                json!(result)
            }
            _ => return Err(error("exe_condition_ir_invalid")),
        })
    }
}
fn boolean(op: &str, left: Val, right: Option<Val>) -> Val {
    if op == "not" {
        return if let Val::Bool(value) = left {
            Val::Bool(!value)
        } else {
            Val::symbol("not", vec![left])
        };
    }
    let right = right.unwrap_or(Val::Nil);
    if op == "and" {
        if left.is_false() || right.is_false() {
            return Val::Bool(false);
        }
        if left.is_true() {
            return right;
        }
        if right.is_true() {
            return left;
        }
    }
    if op == "or" {
        if left.is_true() || right.is_true() {
            return Val::Bool(true);
        }
        if left.is_false() {
            return right;
        }
        if right.is_false() {
            return left;
        }
    }
    if left.eq(&right) {
        return left;
    }
    Val::symbol(op, vec![left, right])
}
#[derive(Clone, Debug)]
struct Env {
    values: HashMap<String, Val>,
    locals: HashSet<String>,
    writes: HashSet<String>,
    allocation: Rc<Cell<usize>>,
}
impl Env {
    fn scope_copy(&self) -> Result<Self> {
        let bytes = self
            .values
            .len()
            .saturating_mul(std::mem::size_of::<Val>() + 96)
            .saturating_add(self.values.keys().map(String::len).sum::<usize>())
            .saturating_add((self.locals.len() + self.writes.len()).saturating_mul(96));
        charge_allocation(&self.allocation, bytes)?;
        let mut copied = self.clone();
        for value in copied.values.values_mut() {
            if let Val::Table(table) = value {
                // Each top-level table gets independent mutation identity, but
                // immutable contents remain shared until a reviewed UI update.
                let fields = table.borrow().clone();
                *value = Val::Table(Rc::new(RefCell::new(fields)));
            }
        }
        Ok(copied)
    }
    fn get(&self, name: &str) -> Val {
        self.values.get(name).cloned().unwrap_or(Val::Nil)
    }
}
#[derive(Clone, Debug)]
struct State {
    env: Env,
    path: Val,
    ended: bool,
    result: Val,
    global_origin: bool,
}
impl State {
    fn new(env: Env, path: Val) -> Self {
        Self {
            env,
            path,
            ended: false,
            result: Val::Nil,
            global_origin: false,
        }
    }
}
struct Compiler {
    steps: usize,
    depth: usize,
    value_depth: usize,
    next_paths: Vec<Val>,
    mode: String,
    event_name: String,
    lifecycle: String,
    protected: HashSet<String>,
    packages: HashMap<String, Val>,
    baseline: Env,
}
impl Compiler {
    fn new(total: i128) -> Self {
        let mut values = HashMap::new();
        for name in ["_NeedsReboot", "_SilentInstall", "_DoingUninstall"] {
            values.insert(name.into(), Val::Bool(false));
        }
        values.insert("_SpaceRequired".into(), Val::Int(total));
        values.insert("_SpaceAvailable".into(), Val::Int(1i128 << 63));
        values.insert("strInstallFolderPath".into(), Val::Str(ROOT.into()));
        values.insert(
            "e_Details".into(),
            Val::table(vec![(Val::Str("Text".into()), Val::Str(ROOT.into()))]),
        );
        values.insert("e_Stage".into(), Val::Int(2));
        for (key, value) in [
            ("e_CtrlID", "control"),
            ("e_MsgID", "message"),
            ("e_StagePct", "stage-percent"),
            ("e_CurrentItemText", "item-text"),
            ("e_TotalPct", "total-percent"),
        ] {
            values.insert(key.into(), Val::symbol("ui", vec![Val::Str(value.into())]));
        }
        Self {
            steps: 0,
            depth: 0,
            value_depth: 0,
            next_paths: Vec::new(),
            mode: "project".into(),
            event_name: "Global Functions".into(),
            lifecycle: "global".into(),
            protected: HashSet::new(),
            packages: HashMap::new(),
            baseline: Env {
                values,
                locals: HashSet::new(),
                writes: HashSet::new(),
                allocation: Rc::new(Cell::new(0)),
            },
        }
    }
    fn remember_global_origin(&mut self, state: &State) {
        if state.global_origin {
            self.baseline = state.env.clone();
        }
    }
    fn allocate(&self, bytes: usize) -> Result<()> {
        charge_allocation(&self.baseline.allocation, bytes)
    }
    fn function_value(&self, node: &Value) -> Result<Val> {
        let mut pending = vec![node];
        let mut bytes = 0usize;
        while let Some(value) = pending.pop() {
            bytes = bytes.saturating_add(std::mem::size_of::<Value>());
            match value {
                Value::String(s) => bytes = bytes.saturating_add(s.len()),
                Value::Array(values) => pending.extend(values),
                _ => {}
            }
            require(bytes <= MAX_ANALYSIS_ALLOCATION, "exe_lua_allocation_limit")?;
        }
        self.allocate(bytes)?;
        Ok(Val::Function(Rc::new(node.clone())))
    }
    fn tick(&mut self) -> Result<()> {
        self.steps += 1;
        require(self.steps <= MAX_STEPS, "exe_lua_analysis_limit")
    }
    fn value(&mut self, node: &Value, env: &mut Env, path: &Val) -> Result<Val> {
        self.value_depth += 1;
        require(self.value_depth <= 192, "exe_lua_analysis_depth_limit")?;
        let result = self.value_inner(node, env, path);
        self.value_depth -= 1;
        let value = result?;
        value.check_symbol_bound()?;
        Ok(value)
    }
    fn value_inner(&mut self, node: &Value, env: &mut Env, path: &Val) -> Result<Val> {
        self.tick()?;
        let kind = string(&node[0])?;
        match kind {
            "literal" => {
                if let Some(text) = node[1].as_str() {
                    self.allocate(text.len())?;
                }
                Val::literal(&node[1])
            }
            "name" => {
                let name = string(&node[1])?;
                if let Some(value) = env.values.get(name) {
                    return Ok(value.clone());
                }
                if matches!(
                    name,
                    "File"
                        | "Registry"
                        | "SessionVar"
                        | "Screen"
                        | "SetupData"
                        | "Dialog"
                        | "String"
                        | "string"
                        | "Drive"
                        | "Application"
                        | "System"
                ) || name.starts_with("Dlg")
                {
                    return Ok(Val::Namespace(name.into()));
                }
                Err(error(&format!("exe_lua_unknown_value:{name}")))
            }
            "function" => self.function_value(node),
            "index" => {
                let parent = self.value(&node[1], env, path)?;
                let key = self.value(&node[2], env, path)?;
                if let (Val::Namespace(name), Val::Str(key)) = (&parent, &key) {
                    return Ok(Val::Api(format!("{name}.{key}")));
                }
                if let Val::Table(table) = parent {
                    if let Some((_, value)) = table.borrow().iter().find(|(k, _)| k.eq(&key)) {
                        return Ok(value.clone());
                    }
                }
                Err(error("exe_lua_index_unsupported"))
            }
            "table" => {
                let mut fields: Vec<(Val, Val)> = Vec::new();
                for field in array(&node[1])? {
                    let key = self.value(&field[0], env, path)?;
                    let value = self.value(&field[1], env, path)?;
                    require(
                        !matches!(key, Val::Table(_) | Val::Nil),
                        "exe_lua_table_key_unsupported",
                    )?;
                    require(fields.len() < MAX_IR_NODES, "exe_lua_table_limit")?;
                    self.allocate(std::mem::size_of::<(Val, Val)>())?;
                    if let Some((_, old)) = fields.iter_mut().find(|(k, _)| k.eq(&key)) {
                        *old = value;
                    } else {
                        fields.push((key, value));
                    }
                }
                Ok(Val::table(fields))
            }
            "unary" => {
                let value = self.value(&node[2], env, path)?;
                let op = string(&node[1])?;
                if op == "not" {
                    return Ok(boolean("not", value.truth(), None));
                }
                require(!value.symbolic(), "exe_lua_arithmetic_unsupported")?;
                match (op, value) {
                    ("-", Val::Int(v)) => Ok(Val::Int(-v)),
                    ("-", Val::Float(v)) => Ok(Val::Float(-v)),
                    ("-", Val::Bool(v)) => Ok(Val::Int(-i128::from(v))),
                    ("#", Val::Str(s)) => Ok(Val::Int(s.chars().count() as i128)),
                    ("#", Val::Table(t)) => Ok(Val::Int(t.borrow().len() as i128)),
                    _ => Err(error("exe_lua_arithmetic_unsupported")),
                }
            }
            "binary" => {
                let op = string(&node[1])?;
                let left = self.value(&node[2], env, path)?;
                if op == "and" && left.truth().is_false() {
                    return Ok(left);
                }
                if op == "or" && left.truth().is_true() {
                    return Ok(left);
                }
                let right = self.value(&node[3], env, path)?;
                if matches!(op, "and" | "or") {
                    if !left.symbolic() {
                        return Ok(right);
                    }
                    require(
                        matches!(right, Val::Bool(_)) || right.symbolic(),
                        "exe_lua_operand_result_unsupported",
                    )?;
                    return Ok(boolean(op, left.truth(), Some(right.truth())));
                }
                if left.symbolic() || right.symbolic() {
                    if matches!(op, "==" | "~=") {
                        for (candidate, literal) in [(&left, &right), (&right, &left)] {
                            if let (Val::Symbol(symbol), Val::Bool(literal)) = (candidate, literal)
                            {
                                if symbol.0 != "crc32" {
                                    return Ok(if *literal == (op == "==") {
                                        candidate.clone()
                                    } else {
                                        boolean("not", candidate.clone(), None)
                                    });
                                }
                            }
                        }
                    }
                    require(
                        matches!(op, "==" | "~=" | "<" | ">" | "<=" | ">="),
                        "exe_lua_arithmetic_unsupported",
                    )?;
                    return Ok(Val::symbol(op, vec![left, right]));
                }
                if op == ".." {
                    fn concat(v: Val) -> Result<String> {
                        match v {
                            Val::Str(s) => Ok(s.to_string()),
                            Val::Int(n) => Ok(n.to_string()),
                            Val::Bool(b) => Ok(if b { "True" } else { "False" }.into()),
                            _ => Err(error("exe_lua_concat_unsupported")),
                        }
                    }
                    let left = concat(left)?;
                    let right = concat(right)?;
                    require(
                        left.len()
                            .checked_add(right.len())
                            .is_some_and(|n| n <= MAX_VALUE_SIZE),
                        "exe_lua_value_size_limit",
                    )?;
                    self.allocate(left.len() + right.len())?;
                    return Ok(Val::Str((left + &right).into()));
                }
                if matches!(op, "==" | "~=") {
                    let equal = left.kind() == right.kind() && left.eq(&right);
                    return Ok(Val::Bool(if op == "==" { equal } else { !equal }));
                }
                if matches!(op, "<" | ">" | "<=" | ">=") {
                    let ordering = match (&left, &right) {
                        (Val::Str(a), Val::Str(b)) => Some(a.cmp(b)),
                        _ => numeric_cmp(&left, &right),
                    }
                    .ok_or_else(|| error("exe_lua_comparison_unsupported"))?;
                    return Ok(Val::Bool(match op {
                        "<" => ordering.is_lt(),
                        ">" => ordering.is_gt(),
                        "<=" => !ordering.is_gt(),
                        _ => !ordering.is_lt(),
                    }));
                }
                require(
                    matches!(left, Val::Int(_) | Val::Float(_))
                        && matches!(right, Val::Int(_) | Val::Float(_)),
                    "exe_lua_arithmetic_unsupported",
                )?;
                if let (Val::Int(a), Val::Int(b)) = (&left, &right) {
                    let result = match op {
                        "+" => a.checked_add(*b),
                        "-" => a.checked_sub(*b),
                        "*" => a.checked_mul(*b),
                        "%" if *b != 0 => Some(
                            a.rem_euclid(*b)
                                .checked_add(if *b < 0 && a.rem_euclid(*b) != 0 {
                                    *b
                                } else {
                                    0
                                })
                                .ok_or_else(|| error("exe_lua_arithmetic_unsupported"))?,
                        ),
                        _ => None,
                    };
                    if let Some(value) = result {
                        require(
                            value >= i64::MIN as i128 && value <= u64::MAX as i128,
                            "exe_lua_number_unsupported",
                        )?;
                        return Ok(Val::Int(value));
                    }
                    if op != "/" {
                        return Err(error("exe_lua_arithmetic_unsupported"));
                    }
                }
                let a = number(&left).unwrap();
                let b = number(&right).unwrap();
                let value = match op {
                    "+" => a + b,
                    "-" => a - b,
                    "*" => a * b,
                    "/" if b != 0.0 => a / b,
                    "%" if b != 0.0 => a - b * (a / b).floor(),
                    _ => return Err(error("exe_lua_arithmetic_unsupported")),
                };
                require(value.is_finite(), "exe_lua_arithmetic_unsupported")?;
                Ok(Val::Float(value))
            }
            "call" => {
                let target = self.value(&node[1], env, path)?;
                let mut args = Vec::new();
                for arg in array(&node[2])? {
                    args.push(self.value(arg, env, path)?);
                }
                if let Val::Api(name) = target {
                    return self.api(&name, args, env, path);
                }
                let Val::Function(target) = target else {
                    return Err(error("exe_lua_dynamic_call_unsupported"));
                };
                let parameters = array(&target[1])?;
                require(
                    args.len() == parameters.len(),
                    "exe_lua_argument_unsupported",
                )?;
                self.depth += 1;
                require(self.depth <= 32, "exe_lua_call_depth_limit")?;
                let mut child = env.scope_copy()?;
                child.locals.clear();
                child.writes.clear();
                for (parameter, arg) in parameters.iter().zip(args) {
                    let name = string(parameter)?.to_owned();
                    child.locals.insert(name.clone());
                    child.values.insert(name, arg);
                }
                let states =
                    self.block(array(&target[2])?, vec![State::new(child, path.clone())])?;
                self.depth -= 1;
                require(!states.is_empty(), "exe_lua_function_result_unsupported")?;
                let writes: HashSet<String> = states
                    .iter()
                    .flat_map(|s| s.env.writes.iter().cloned())
                    .collect();
                for key in writes {
                    let first = states[0].env.get(&key);
                    require(
                        states.iter().all(|s| s.env.get(&key).eq(&first)),
                        "exe_lua_global_mutation_unsupported",
                    )?;
                    env.values.insert(key.clone(), first);
                    if self.depth > 0 && !env.locals.contains(&key) {
                        env.writes.insert(key);
                    }
                }
                let first = states[0].result.clone();
                require(
                    states.iter().all(|s| s.result.eq(&first)),
                    "exe_lua_function_result_unsupported",
                )?;
                Ok(first)
            }
            _ => Err(error("exe_lua_expression_unsupported")),
        }
    }
    fn api(&mut self, name: &str, args: Vec<Val>, env: &mut Env, path: &Val) -> Result<Val> {
        match name {
            "File.DoesExist" | "File.GetCRC" => {
                require(
                    args.len() == 1 && args[0].text().is_some(),
                    "exe_file_condition_unsupported",
                )?;
                let value = args[0].text().unwrap();
                let relative = if let Some(v) = value.strip_prefix("%AppFolder%\\") {
                    v
                } else if let Some(v) = value.strip_prefix(&format!("{ROOT}\\")) {
                    v
                } else if self.mode == "locator" && name == "File.DoesExist" {
                    value.rsplit('\\').next().unwrap()
                } else {
                    return Err(error("exe_condition_path_unsupported"));
                };
                Ok(Val::symbol(
                    if name == "File.DoesExist" {
                        "exists"
                    } else {
                        "crc32"
                    },
                    vec![Val::Str({
                        let relative = relative.replace('/', "\\");
                        let (parent, leaf) = relative.rsplit_once('\\').unwrap_or(("", &relative));
                        let directory = if parent.is_empty() {
                            "%AppFolder%".to_owned()
                        } else {
                            format!("%AppFolder%\\{parent}")
                        };
                        safe_target(&directory, leaf)?.into()
                    })],
                ))
            }
            "Registry.GetValue" => {
                require(
                    self.mode == "locator" && matches!(args.len(), 3 | 4),
                    "exe_registry_read_unsupported",
                )?;
                Ok(Val::Str(ROOT.into()))
            }
            "SessionVar.Set" => {
                require(
                    args.len() == 2
                        && matches!(
                            args[0].text(),
                            Some("%AppFolder%" | "%SpaceRequired%" | "%SpaceAvailable%")
                        ),
                    "exe_variable_write_unsupported",
                )?;
                if args[0].text() == Some("%AppFolder%") {
                    require(
                        self.mode == "locator" && args[1].text().is_some(),
                        "exe_target_change_unsupported",
                    )?;
                }
                Ok(Val::Nil)
            }
            "SessionVar.Expand" => {
                let arg = args
                    .first()
                    .ok_or_else(|| error("exe_lua_argument_unsupported"))?;
                if let Val::Str(s) = arg {
                    let expanded = s.replace("%AppFolder%", ROOT);
                    require(expanded.len() <= MAX_VALUE_SIZE, "exe_lua_value_size_limit")?;
                    self.allocate(expanded.len())?;
                    Ok(Val::Str(expanded.into()))
                } else {
                    Ok(arg.clone())
                }
            }
            "Screen.Next" => {
                require(
                    self.event_name == "On Next",
                    "exe_screen_transition_unsupported",
                )?;
                require(self.next_paths.len() < MAX_STEPS, "exe_lua_analysis_limit")?;
                self.next_paths.push(path.clone());
                Ok(Val::Nil)
            }
            "SetupData.GetPackageProperties" => {
                let name = args
                    .first()
                    .and_then(Val::text)
                    .ok_or_else(|| error("exe_package_name_missing"))?;
                let install = self
                    .packages
                    .get(name)
                    .cloned()
                    .ok_or_else(|| error("exe_package_not_found"))?;
                Ok(Val::table(vec![(Val::Str("Install".into()), install)]))
            }
            "SetupData.SetPackageProperties" => {
                // Only phases on the default forward installation route change
                // the selection. Back/help/cancel/post/uninstall are not executed.
                let choosing = (self.lifecycle == "global")
                    || (self.lifecycle == "before"
                        && matches!(self.event_name.as_str(), "On Preload" | "On Next"))
                    || (self.lifecycle == "project"
                        && matches!(self.event_name.as_str(), "On Startup" | "On Pre Install"))
                    || (self.lifecycle == "while" && self.event_name == "On Preload");
                if !choosing {
                    return Ok(Val::Nil);
                }
                let name = args
                    .first()
                    .and_then(Val::text)
                    .ok_or_else(|| error("exe_package_name_missing"))?
                    .to_owned();
                let old = self
                    .packages
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| error("exe_package_not_found"))?;
                let table = match args.get(1) {
                    Some(Val::Table(t)) => t.borrow().clone(),
                    _ => return Err(error("exe_package_properties_invalid")),
                };
                if let Some((_, value)) =
                    table.iter().find(|(key, _)| key.text() == Some("Install"))
                {
                    let chosen = boolean(
                        "or",
                        boolean("and", path.clone(), Some(value.truth())),
                        Some(boolean(
                            "and",
                            boolean("not", path.clone(), None),
                            Some(old),
                        )),
                    );
                    chosen.check_symbol_bound()?;
                    self.packages.insert(name, chosen);
                }
                Ok(Val::Nil)
            }
            "Screen.Back" => Ok(Val::Nil),
            "Application.Exit" => {
                require(
                    self.mode == "cancel"
                        && args.len() == 1
                        && args[0].eq(&env.get("EXIT_REASON_USER_ABORTED")),
                    "exe_application_exit_unsupported",
                )?;
                Ok(Val::Nil)
            }
            "Screen.GetLocalizedString"
            | "SetupData.GetLocalizedString"
            | "String.GetFormattedSize" => Ok(Val::Str(String::new().into())),
            "SetupData.CalculateRequiredSpace" => Ok(env.get("_SpaceRequired")),
            "Drive.GetFreeSpace" => Ok(Val::Int(1i128 << 43)),
            "Dialog.Message" => Ok(env.values.get("IDNO").cloned().unwrap_or(Val::Int(7))),
            "Dialog.FolderBrowse" => Ok(Val::Str(ROOT.into())),
            "String.AbbreviateFilePath" => args
                .first()
                .cloned()
                .ok_or_else(|| error("exe_lua_argument_unsupported")),
            "String.Length" => {
                require(
                    args.len() == 1 && args[0].text().is_some(),
                    "exe_lua_dynamic_string_unsupported",
                )?;
                Ok(Val::Int(args[0].text().unwrap().chars().count() as i128))
            }
            "string.find" => {
                require(
                    matches!(args.len(), 2 | 3)
                        && args[0].text() == Some(ROOT)
                        && args[1].text().is_some(),
                    "exe_string_pattern_unsupported",
                )?;
                let pattern = args[1].text().unwrap();
                if pattern == r#"^\\[^/%*%?"<>|:\][^/%*%?"<>|:]+$"# {
                    Ok(Val::Nil)
                } else if pattern == r#"^%a:\[^/%*%?"<>|:]*$"# {
                    Ok(Val::Int(1))
                } else if matches!(pattern, r#"\\"# | r#"[/%*%?"<>|]"# | r#"[/%*%?"<>|:]"#) {
                    Ok(Val::Nil)
                } else {
                    Err(error("exe_string_pattern_unsupported"))
                }
            }
            "DlgStaticText.GetProperties"
            | "DlgEditField.GetProperties"
            | "DlgRadioButton.GetProperties" => Ok(Val::table(vec![
                (
                    Val::Str("Text".into()),
                    Val::Str(if name.contains("EditField") { ROOT } else { "" }.into()),
                ),
                (Val::Str("Checked".into()), Val::Bool(true)),
            ])),
            "DlgStaticText.SetProperties"
            | "DlgButton.SetProperties"
            | "DlgEditField.SetProperties"
            | "DlgProgressBar.SetPos" => Ok(Val::Nil),
            n if n.starts_with("Dlg")
                && (n.contains(".Set") || n.ends_with(".Show") || n.ends_with(".Hide")) =>
            {
                Ok(Val::Nil)
            }
            _ => Err(error(&format!("exe_lua_call_unsupported:{name}"))),
        }
    }
    fn block(&mut self, statements: &[Value], mut states: Vec<State>) -> Result<Vec<State>> {
        for statement in statements {
            let mut output = Vec::new();
            for mut state in states {
                self.tick()?;
                if state.ended {
                    output.push(state);
                    continue;
                }
                let kind = string(&statement[0])?;
                match kind {
                    "define" => {
                        require(self.depth == 0, "exe_lua_nested_function_unsupported")?;
                        let name = string(&statement[1])?;
                        require(
                            !state.env.values.contains_key(name),
                            "exe_lua_function_redefinition_unsupported",
                        )?;
                        state
                            .env
                            .values
                            .insert(name.into(), self.function_value(&statement[2])?);
                    }
                    "assign" => {
                        let mut values = Vec::new();
                        for node in array(&statement[2])? {
                            values.push(self.value(node, &mut state.env, &state.path)?);
                        }
                        for (index, target) in array(&statement[1])?.iter().enumerate() {
                            let value = values.get(index).cloned().unwrap_or(Val::Nil);
                            match string(&target[0])? {
                                "name" => {
                                    let name = string(&target[1])?;
                                    require(
                                        !matches!(
                                            name,
                                            "File"
                                                | "Registry"
                                                | "Screen"
                                                | "SessionVar"
                                                | "SetupData"
                                                | "System"
                                        ),
                                        "exe_lua_api_redefinition_unsupported",
                                    )?;
                                    if self.protected.contains(name) {
                                        if name == "_SpaceAvailable" {
                                            let required = self.baseline.get("_SpaceRequired");
                                            require(
                                                matches!((&value,&required),(Val::Int(a),Val::Int(b)) if a>=b),
                                                "exe_lua_runtime_mutation_unsupported",
                                            )?;
                                        } else {
                                            require(
                                                value.eq(&self.baseline.get(name)),
                                                "exe_lua_runtime_mutation_unsupported",
                                            )?;
                                        }
                                    }
                                    state.env.values.insert(name.into(), value);
                                    if self.depth > 0 {
                                        if statement[3] == true {
                                            state.env.locals.insert(name.into());
                                        } else if !state.env.locals.contains(name) {
                                            state.env.writes.insert(name.into());
                                        }
                                    }
                                }
                                "index" => {
                                    let parent =
                                        self.value(&target[1], &mut state.env, &state.path)?;
                                    let key =
                                        self.value(&target[2], &mut state.env, &state.path)?;
                                    require(
                                        !matches!(
                                            value,
                                            Val::Table(_)
                                                | Val::Function(_)
                                                | Val::Namespace(_)
                                                | Val::Api(_)
                                        ) && matches!(parent, Val::Table(_))
                                            && matches!(
                                                key.text(),
                                                Some("Text" | "Checked" | "Enabled" | "Install")
                                            ),
                                        "exe_lua_assignment_unsupported",
                                    )?;
                                    if let Val::Table(table) = parent {
                                        let mut fields = table.borrow_mut();
                                        if Rc::strong_count(&fields) > 1 {
                                            self.allocate(
                                                fields.len().saturating_mul(std::mem::size_of::<(
                                                    Val,
                                                    Val,
                                                )>(
                                                )),
                                            )?;
                                        }
                                        let fields = Rc::make_mut(&mut *fields);
                                        if let Some((_, old)) =
                                            fields.iter_mut().find(|(k, _)| k.eq(&key))
                                        {
                                            *old = value;
                                        } else {
                                            fields.push((key, value));
                                        }
                                    }
                                }
                                _ => return Err(error("exe_lua_assignment_unsupported")),
                            }
                        }
                    }
                    "call" => {
                        self.value(&statement[1], &mut state.env, &state.path)?;
                    }
                    "return" => {
                        let values = array(&statement[1])?;
                        require(values.len() <= 1, "exe_lua_multiple_return_unsupported")?;
                        state.result = if let Some(value) = values.first() {
                            self.value(value, &mut state.env, &state.path)?
                        } else {
                            Val::Nil
                        };
                        state.ended = true;
                    }
                    "if" => {
                        let mut pending = vec![state];
                        for arm in array(&statement[1])? {
                            let mut remaining = Vec::new();
                            for mut scope in pending {
                                let value =
                                    self.value(&arm[0], &mut scope.env, &scope.path)?.truth();
                                self.remember_global_origin(&scope);
                                if !value.is_false() {
                                    let path =
                                        boolean("and", scope.path.clone(), Some(value.clone()));
                                    path.check_symbol_bound()?;
                                    output.extend(self.block(
                                        array(&arm[1])?,
                                        vec![State::new(scope.env.scope_copy()?, path)],
                                    )?);
                                    require(output.len() <= MAX_BRANCHES, "exe_lua_branch_limit")?;
                                }
                                if !value.is_true() {
                                    let path = boolean(
                                        "and",
                                        scope.path,
                                        Some(boolean("not", value, None)),
                                    );
                                    path.check_symbol_bound()?;
                                    remaining.push(State::new(scope.env.scope_copy()?, path));
                                }
                            }
                            pending = remaining;
                        }
                        output.extend(self.block(array(&statement[2])?, pending)?);
                        require(output.len() <= MAX_BRANCHES, "exe_lua_branch_limit")?;
                        continue;
                    }
                    "do" => {
                        self.remember_global_origin(&state);
                        output.extend(self.block(
                            array(&statement[1])?,
                            vec![State::new(state.env.scope_copy()?, state.path)],
                        )?);
                        require(output.len() <= MAX_BRANCHES, "exe_lua_branch_limit")?;
                        continue;
                    }
                    _ => return Err(error(&format!("exe_lua_control_flow_unsupported:{kind}"))),
                }
                output.push(state);
                require(output.len() <= MAX_BRANCHES, "exe_lua_branch_limit")?;
            }
            states = output;
            require(states.len() <= MAX_BRANCHES, "exe_lua_branch_limit")?;
        }
        Ok(states)
    }
}
fn int_float_cmp(integer: i128, float: f64) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    if float.is_nan() {
        return None;
    }
    if float >= i128::MAX as f64 {
        return Some(Ordering::Less);
    }
    if float < i128::MIN as f64 {
        return Some(Ordering::Greater);
    }
    let floor = float.floor();
    match integer.cmp(&(floor as i128)) {
        Ordering::Equal if float != floor => Some(Ordering::Less),
        ordering => Some(ordering),
    }
}
fn numeric_cmp(left: &Val, right: &Val) -> Option<std::cmp::Ordering> {
    let left = if let Val::Bool(b) = left {
        Val::Int(i128::from(*b))
    } else {
        left.clone()
    };
    let right = if let Val::Bool(b) = right {
        Val::Int(i128::from(*b))
    } else {
        right.clone()
    };
    match (&left, &right) {
        (Val::Int(a), Val::Int(b)) => Some(a.cmp(b)),
        (Val::Int(a), Val::Float(b)) => int_float_cmp(*a, *b),
        (Val::Float(a), Val::Int(b)) => int_float_cmp(*b, *a).map(std::cmp::Ordering::reverse),
        (Val::Float(a), Val::Float(b)) => a.partial_cmp(b),
        _ => None,
    }
}
fn charge_allocation(allocation: &Cell<usize>, bytes: usize) -> Result<()> {
    let total = allocation
        .get()
        .checked_add(bytes)
        .ok_or_else(|| error("exe_lua_allocation_limit"))?;
    require(total <= MAX_ANALYSIS_ALLOCATION, "exe_lua_allocation_limit")?;
    allocation.set(total);
    Ok(())
}
fn number(value: &Val) -> Option<f64> {
    match value {
        Val::Int(v) => Some(*v as f64),
        Val::Float(v) => Some(*v),
        Val::Bool(v) => Some(i32::from(*v) as f64),
        _ => None,
    }
}

/// Check the fields used by member selection; publisher preferences are data.
pub fn file_policy(files: &[Value], _version: &str) -> Result<()> {
    for row in files {
        require(
            row["destination"].is_string()
                && row["condition"].is_string()
                && row["packages"].is_array(),
            "exe_file_schema_unsupported",
        )?;
    }
    Ok(())
}
fn validate(node: &Val, budget: &mut usize, depth: usize) -> Result<()> {
    require(*budget > 0 && depth <= 96, "exe_condition_ir_limit")?;
    *budget -= 1;
    if matches!(node, Val::Bool(_)) {
        return Ok(());
    }
    let Val::Symbol(symbol) = node else {
        return Err(error("exe_condition_ir_invalid"));
    };
    match symbol.0.as_str() {
        "and" | "or" => {
            require(symbol.1.len() == 2, "exe_condition_ir_invalid")?;
            validate(&symbol.1[0], budget, depth + 1)?;
            validate(&symbol.1[1], budget, depth + 1)?;
        }
        "not" => {
            require(symbol.1.len() == 1, "exe_condition_ir_invalid")?;
            validate(&symbol.1[0], budget, depth + 1)?;
        }
        "exists" => require(
            symbol.1.len() == 1 && symbol.1[0].text().is_some(),
            "exe_condition_ir_invalid",
        )?,
        "==" | "~=" => {
            require(symbol.1.len()==2 && symbol.1.iter().any(|v|matches!(v,Val::Symbol(s) if s.0=="crc32" && s.1.len()==1 && s.1[0].text().is_some()))
                && symbol.1.iter().any(|v|matches!(v,Val::Int(n) if (0..=0xffff_ffff).contains(n))),"exe_condition_ir_unsupported")?;
        }
        _ => return Err(error("exe_condition_ir_unsupported")),
    }
    Ok(())
}

/// Lower a complete reviewed Setup Factory graph to JSON data only.
pub fn lower(graph: &Value, files: &[Value], version: &str) -> Result<Value> {
    require(
        graph["coverage"]["complete"] == true,
        "exe_serialized_object_graph_incomplete",
    )?;
    require(
        files.len() <= u16::MAX as usize,
        "exe_file_schema_unsupported",
    )?;
    file_policy(files, version)?;
    let mut total = 0i128;
    for file in files {
        let size = file["unpacked_size"]
            .as_u64()
            .ok_or_else(|| error("exe_file_schema_unsupported"))?;
        total = total
            .checked_add(size as i128)
            .ok_or_else(|| error("exe_file_schema_unsupported"))?;
    }
    let includes = graph["includes"]
        .as_array()
        .ok_or_else(|| error("exe_serialized_object_graph_incomplete"))?;
    let events = graph["events"]
        .as_array()
        .ok_or_else(|| error("exe_serialized_object_graph_incomplete"))?;
    require(
        events.len() + includes.len() <= MAX_STEPS,
        "exe_lua_analysis_limit",
    )?;
    let mut compiler = Compiler::new(total);
    for object in graph["objects"].as_array().into_iter().flatten() {
        if object["class"] == "CSUFPackage" {
            let name = object["name"]
                .as_str()
                .ok_or_else(|| error("exe_package_name_missing"))?;
            let install = object["install"]
                .as_bool()
                .ok_or_else(|| error("exe_package_properties_invalid"))?;
            compiler
                .packages
                .insert(name.to_owned(), Val::Bool(install));
        }
    }
    let mut global_nodes = Vec::new();
    let mut global_source_bytes = 0usize;
    for include in includes {
        let source = include["script"]
            .as_str()
            .ok_or_else(|| error("exe_lua_syntax_invalid"))?;
        global_source_bytes = global_source_bytes
            .checked_add(source.len())
            .ok_or_else(|| error("exe_lua_size_limit"))?;
        require(
            global_source_bytes <= 16 * 1024 * 1024,
            "exe_lua_size_limit",
        )?;
        global_nodes.extend(array(&lua::parse(source)?)?.clone());
        require(global_nodes.len() <= MAX_STEPS, "exe_lua_analysis_limit")?;
    }
    for event in events {
        if event["name"] == "Global Functions" {
            let source = event["script"]
                .as_str()
                .ok_or_else(|| error("exe_lua_syntax_invalid"))?;
            global_source_bytes = global_source_bytes
                .checked_add(source.len())
                .ok_or_else(|| error("exe_lua_size_limit"))?;
            require(
                global_source_bytes <= 16 * 1024 * 1024,
                "exe_lua_size_limit",
            )?;
            global_nodes.extend(array(&lua::parse(source)?)?.clone());
            require(global_nodes.len() <= MAX_STEPS, "exe_lua_analysis_limit")?;
        }
    }
    let states = compiler.block(
        &global_nodes,
        vec![State {
            global_origin: true,
            ..State::new(compiler.baseline.scope_copy()?, Val::Bool(true))
        }],
    )?;
    require(
        states.len() == 1 && !states[0].ended,
        "exe_lua_global_flow_unsupported",
    )?;
    // The original global scope is moved here when no if/do scope copy took
    // ownership of the active state. Scope-copy boundaries already captured
    // the original environment, avoiding a full clone after every assignment.
    let global_state = states.into_iter().next().unwrap();
    if global_state.global_origin {
        compiler.baseline = global_state.env;
    }
    compiler.protected = compiler.baseline.values.keys().cloned().collect();
    let mut gates = Vec::new();
    let lifecycle = |event: &Value| -> String {
        event["lifecycle"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if event["context"] == "uninstall" {
                    "uninstall".into()
                } else if event["context"] == "project" {
                    "project".into()
                } else {
                    "before".into()
                }
            })
    };
    let mut install_events: Vec<_> = events
        .iter()
        .enumerate()
        .filter(|(_, event)| lifecycle(event) != "uninstall" && event["name"] != "Global Functions")
        .collect();
    install_events.sort_by_key(|(index, event)| {
        let stage = lifecycle(event);
        let phase = match (stage.as_str(), event["name"].as_str()) {
            ("project", Some("On Startup")) => 0,
            ("before", _) => 1,
            ("project", Some("On Pre Install")) => 2,
            ("while", Some("On Preload")) => 3,
            ("while", _) => 4,
            ("project", Some("On Post Install")) => 5,
            ("after", _) => 6,
            _ => 7,
        };
        (phase, *index)
    });
    for (_, event) in install_events {
        compiler.lifecycle = lifecycle(event);
        let nodes = lua::parse(
            event["script"]
                .as_str()
                .ok_or_else(|| error("exe_lua_syntax_invalid"))?,
        )?;
        compiler.next_paths.clear();
        let name = event["name"]
            .as_str()
            .ok_or_else(|| error("exe_lua_syntax_invalid"))?;
        let context = event["context"]
            .as_str()
            .ok_or_else(|| error("exe_lua_syntax_invalid"))?;
        compiler.event_name = name.into();
        compiler.mode = if context.starts_with("CSUFScreenStaticText:") && name == "On Next" {
            "locator"
        } else if name == "On Cancel" {
            "cancel"
        } else {
            "event"
        }
        .into();
        let stages = if name == "On Progress" {
            vec![Val::Int(1), Val::Int(2), Val::Int(3), Val::Int(4)]
        } else {
            vec![compiler.baseline.get("e_Stage")]
        };
        for stage in stages {
            let mut scope = compiler.baseline.scope_copy()?;
            scope.values.insert("e_Stage".into(), stage);
            compiler.block(array(&nodes)?, vec![State::new(scope, Val::Bool(true))])?;
        }
        if name == "On Next" && compiler.lifecycle == "before" {
            let mut condition = Val::Bool(false);
            for path in &compiler.next_paths {
                condition = boolean("or", condition, Some(path.clone()));
                condition.check_symbol_bound()?;
            }
            require(!condition.is_false(), "exe_installation_never_advances")?;
            gates.push(condition);
        }
    }
    let mut condition = Val::Bool(true);
    for gate in gates {
        condition = boolean("and", condition, Some(gate));
        condition.check_symbol_bound()?;
    }
    let mut file_conditions = Vec::new();
    compiler.mode = "event".into();
    for row in files {
        let expression = row["condition"]
            .as_str()
            .ok_or_else(|| error("exe_file_condition_invalid"))?;
        let mut selected = if expression.trim().is_empty() {
            Val::Bool(true)
        } else {
            let nodes = lua::parse(&format!("return ({expression})"))?;
            let mut scope = compiler.baseline.scope_copy()?;
            compiler
                .value(&nodes[0][1][0], &mut scope, &Val::Bool(true))?
                .truth()
        };
        let packages = row["packages"]
            .as_array()
            .ok_or_else(|| error("exe_package_properties_invalid"))?;
        if !packages.is_empty() {
            let mut installed = Val::Bool(false);
            for package in packages {
                let name = package
                    .as_str()
                    .ok_or_else(|| error("exe_package_name_missing"))?;
                let value = compiler
                    .packages
                    .get(name)
                    .cloned()
                    .ok_or_else(|| error("exe_package_not_found"))?;
                installed = boolean("or", installed, Some(value));
            }
            selected = boolean("and", selected, Some(installed));
        }
        if !matches!(selected, Val::Bool(true)) {
            validate(&selected, &mut MAX_IR_NODES.clone(), 0)?;
            file_conditions.push(json!({"path":row["destination"],"condition":selected.to_json(&mut MAX_IR_NODES.clone(),0)?}));
        }
    }
    let mut validation_budget = MAX_IR_NODES;
    validate(&condition, &mut validation_budget, 0)?;
    let mut encoding_budget = MAX_IR_NODES;
    let condition = condition.to_json(&mut encoding_budget, 0)?;
    Ok(
        json!({"schema_version":1,"policy":VERSION,"condition":condition,
        "file_conditions":file_conditions,"copy_files":files.iter().map(|f|f["destination"].clone()).collect::<Vec<_>>(),"terms":graph["terms"],
        "coverage":{"complete":true,"events":events.len(),"includes":includes.len(),"steps":compiler.steps}}),
    )
}
