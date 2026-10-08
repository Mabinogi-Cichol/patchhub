//! Bounded Lua syntax parsing into JSON-array AST data. This module never executes Lua.
use crate::{require, Error, Result};
use serde_json::{json, Value};

pub const VERSION: &str = "sf9-lua-v1";
const MAX_TOKENS: usize = 100_000;
const MAX_DEPTH: usize = 96;

#[derive(Clone, Debug)]
struct Token {
    kind: String,
    value: Value,
}
fn token(kind: &str, value: Value) -> Token {
    Token {
        kind: kind.into(),
        value,
    }
}
fn err(code: &str) -> Error {
    Error(code.into())
}
fn name_valid(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
fn long_open(source: &str, start: usize) -> Option<(usize, usize)> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'[') {
        return None;
    }
    let mut i = start + 1;
    while bytes.get(i) == Some(&b'=') {
        i += 1;
    }
    (bytes.get(i) == Some(&b'[')).then_some((i - start - 1, i + 1))
}
fn lex(source: &str) -> Result<Vec<Token>> {
    require(source.len() <= 1024 * 1024, "exe_lua_size_limit")?;
    let bytes = source.as_bytes();
    let mut output = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = source[i..]
            .chars()
            .next()
            .ok_or_else(|| err("exe_lua_token_unsupported"))?;
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        let comment = source[i..].starts_with("--");
        let start = if comment { i + 2 } else { i };
        if let Some((equals, content)) = long_open(source, start) {
            let closing = format!("]{}]", "=".repeat(equals));
            let end = source[content..]
                .find(&closing)
                .map(|n| content + n)
                .ok_or_else(|| err("exe_lua_unterminated_long_string"))?;
            if !comment {
                output.push(token("string", json!(&source[content..end])));
            }
            i = end + closing.len();
        } else if comment {
            i = source[start..]
                .find(['\r', '\n'])
                .map(|n| start + n)
                .unwrap_or(bytes.len());
        } else if c == '\'' || c == '"' {
            let quote = c;
            i += 1;
            let mut value = String::new();
            while i < bytes.len() && !source[i..].starts_with(quote) {
                let mut ch = source[i..].chars().next().unwrap();
                require(ch != '\r' && ch != '\n', "exe_lua_unterminated_string")?;
                if ch == '\\' {
                    i += 1;
                    require(i < bytes.len(), "exe_lua_unterminated_string")?;
                    ch = source[i..].chars().next().unwrap();
                    if ch.is_ascii_digit() {
                        let start = i;
                        while i < bytes.len() && i - start < 3 && bytes[i].is_ascii_digit() {
                            i += 1;
                        }
                        let number: u32 = source[start..i]
                            .parse()
                            .map_err(|_| err("exe_lua_escape_invalid"))?;
                        require(number <= 255, "exe_lua_escape_invalid")?;
                        value.push(char::from_u32(number).unwrap());
                        continue;
                    }
                    ch = match ch {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        'b' => '\u{8}',
                        'f' => '\u{c}',
                        'v' => '\u{b}',
                        'a' => '\u{7}',
                        c => c,
                    };
                    value.push(ch);
                    i += source[i..].chars().next().unwrap().len_utf8();
                } else {
                    value.push(ch);
                    i += ch.len_utf8();
                }
            }
            require(i < bytes.len(), "exe_lua_unterminated_string")?;
            output.push(token("string", json!(value)));
            i += 1;
        } else if c.is_ascii_digit() {
            let start = i;
            let is_hex = source[i..].starts_with("0x") || source[i..].starts_with("0X");
            if is_hex && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit) {
                i += 2;
                while bytes.get(i).is_some_and(u8::is_ascii_hexdigit) {
                    i += 1;
                }
                let value = u64::from_str_radix(&source[start + 2..i], 16)
                    .map_err(|_| err("exe_lua_number_unsupported"))?;
                output.push(token("number", json!(value)));
            } else {
                while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
                let mut float = false;
                if bytes.get(i) == Some(&b'.') && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
                    float = true;
                    i += 1;
                    while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                        i += 1;
                    }
                }
                if matches!(bytes.get(i), Some(b'e' | b'E')) {
                    let exponent_start = i;
                    let mut end = i + 1;
                    if matches!(bytes.get(end), Some(b'+' | b'-')) {
                        end += 1;
                    }
                    if bytes.get(end).is_some_and(u8::is_ascii_digit) {
                        float = true;
                        i = end + 1;
                        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                            i += 1;
                        }
                    } else {
                        i = exponent_start;
                    }
                }
                let value = if float {
                    let n: f64 = source[start..i]
                        .parse()
                        .map_err(|_| err("exe_lua_number_unsupported"))?;
                    require(n.is_finite(), "exe_lua_number_unsupported")?;
                    json!(n)
                } else {
                    json!(source[start..i]
                        .parse::<u64>()
                        .map_err(|_| err("exe_lua_number_unsupported"))?)
                };
                output.push(token("number", value));
            }
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            i += 1;
            while bytes
                .get(i)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
            {
                i += 1;
            }
            let value = &source[start..i];
            let kind = if matches!(
                value,
                "and"
                    | "break"
                    | "do"
                    | "else"
                    | "elseif"
                    | "end"
                    | "false"
                    | "for"
                    | "function"
                    | "if"
                    | "in"
                    | "local"
                    | "nil"
                    | "not"
                    | "or"
                    | "repeat"
                    | "return"
                    | "then"
                    | "true"
                    | "until"
                    | "while"
            ) {
                value
            } else {
                "identifier"
            };
            output.push(token(kind, json!(value)));
        } else {
            let symbol = ["...", "..", "==", "~=", "<=", ">="]
                .into_iter()
                .find(|s| source[i..].starts_with(s));
            if let Some(symbol) = symbol {
                output.push(token(symbol, json!(symbol)));
                i += symbol.len();
            } else {
                require(
                    "+*/%^#<>=(){}[],;:.-".contains(c),
                    "exe_lua_token_unsupported",
                )?;
                let symbol = c.to_string();
                output.push(token(&symbol, json!(symbol)));
                i += c.len_utf8();
            }
        }
        require(output.len() <= MAX_TOKENS, "exe_lua_token_limit")?;
    }
    output.push(token("eof", json!("eof")));
    Ok(output)
}
fn precedence(op: &str) -> Option<u8> {
    Some(match op {
        "or" => 1,
        "and" => 2,
        "<" | ">" | "<=" | ">=" | "~=" | "==" => 3,
        ".." => 4,
        "+" | "-" => 5,
        "*" | "/" | "%" => 6,
        "^" => 8,
        _ => return None,
    })
}
// Iterative postfix/binary parsing must also bound the resulting tree: token
// limits alone do not prevent a flat source from creating a deeply nested AST.
fn check_ast_depth(root: &Value) -> Result<()> {
    let mut pending = vec![(root, 0usize)];
    while let Some((value, parent_depth)) = pending.pop() {
        if let Value::Array(values) = value {
            let is_node = values.first().and_then(Value::as_str).is_some_and(|tag| {
                matches!(
                    tag,
                    "literal"
                        | "name"
                        | "vararg"
                        | "index"
                        | "table"
                        | "unary"
                        | "binary"
                        | "function"
                        | "assign"
                        | "define"
                        | "call"
                        | "if"
                        | "return"
                        | "break"
                        | "for"
                        | "while"
                        | "do"
                )
            });
            let depth = parent_depth + usize::from(is_node);
            require(depth <= MAX_DEPTH, "exe_lua_depth_limit")?;
            pending.extend(values.iter().map(|v| (v, depth)));
        }
    }
    Ok(())
}
struct Parser {
    tokens: Vec<Token>,
    i: usize,
    depth: usize,
}
impl Parser {
    fn peek(&self) -> &str {
        self.tokens
            .get(self.i)
            .map(|t| t.kind.as_str())
            .unwrap_or("eof")
    }
    fn take(&mut self, expected: Option<&str>) -> Result<Value> {
        let t = self
            .tokens
            .get(self.i)
            .ok_or_else(|| err("exe_lua_syntax_invalid"))?;
        require(
            expected.is_none_or(|e| t.kind == e),
            "exe_lua_syntax_invalid",
        )?;
        self.i += 1;
        Ok(t.value.clone())
    }
    fn accept(&mut self, kind: &str) -> bool {
        if self.peek() == kind {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn name(&mut self) -> Result<String> {
        let value = self.take(None)?;
        let name = value.as_str().ok_or_else(|| err("exe_lua_name_invalid"))?;
        require(name_valid(name), "exe_lua_name_invalid")?;
        Ok(name.into())
    }
    fn enter(&mut self) -> Result<()> {
        self.depth += 1;
        require(self.depth <= MAX_DEPTH, "exe_lua_depth_limit")
    }
    fn block(&mut self, stops: &[&str]) -> Result<Vec<Value>> {
        self.enter()?;
        let mut result = Vec::new();
        while !stops.contains(&self.peek()) {
            require(self.peek() != "eof", "exe_lua_syntax_invalid")?;
            if self.accept(";") {
                continue;
            }
            let statement = self.statement()?;
            check_ast_depth(&statement)?;
            result.push(statement);
            self.accept(";");
        }
        self.depth -= 1;
        Ok(result)
    }
    fn function(&mut self) -> Result<Value> {
        self.take(Some("("))?;
        let mut parameters = Vec::new();
        if self.peek() != ")" {
            loop {
                parameters.push(if self.peek() == "..." {
                    self.take(Some("..."))?
                } else {
                    json!(self.name()?)
                });
                if !self.accept(",") {
                    break;
                }
            }
        }
        self.take(Some(")"))?;
        let body = self.block(&["end"])?;
        self.take(Some("end"))?;
        Ok(json!(["function", parameters, body]))
    }
    fn statement(&mut self) -> Result<Value> {
        let local = self.accept("local");
        if self.accept("function") {
            let name = self.name()?;
            return Ok(json!(["define", name, self.function()?, local]));
        }
        if self.accept("if") {
            let mut arms = Vec::new();
            let condition = self.expr(0)?;
            self.take(Some("then"))?;
            arms.push(json!([condition, self.block(&["elseif", "else", "end"])?]));
            while self.accept("elseif") {
                let condition = self.expr(0)?;
                self.take(Some("then"))?;
                arms.push(json!([condition, self.block(&["elseif", "else", "end"])?]));
            }
            let other = if self.accept("else") {
                self.block(&["end"])?
            } else {
                Vec::new()
            };
            self.take(Some("end"))?;
            return Ok(json!(["if", arms, other]));
        }
        if self.accept("return") {
            let values = if ["end", "else", "elseif", "eof", ";"].contains(&self.peek()) {
                Vec::new()
            } else {
                self.exprs()?
            };
            return Ok(json!(["return", values]));
        }
        if self.accept("break") {
            return Ok(json!(["break"]));
        }
        if self.accept("for") {
            let mut names = vec![self.name()?];
            while self.accept(",") {
                names.push(self.name()?);
            }
            let numeric = self.accept("=");
            if !numeric {
                self.take(Some("in"))?;
            }
            let values = self.exprs()?;
            self.take(Some("do"))?;
            let body = self.block(&["end"])?;
            self.take(Some("end"))?;
            return Ok(json!(["for", names, values, body, numeric]));
        }
        if self.accept("while") {
            let test = self.expr(0)?;
            self.take(Some("do"))?;
            let body = self.block(&["end"])?;
            self.take(Some("end"))?;
            return Ok(json!(["while", test, body]));
        }
        if self.accept("do") {
            let body = self.block(&["end"])?;
            self.take(Some("end"))?;
            return Ok(json!(["do", body]));
        }
        let mut lhs = vec![self.expr(0)?];
        while self.accept(",") {
            lhs.push(self.expr(0)?);
        }
        if self.accept("=") {
            return Ok(json!(["assign", lhs, self.exprs()?, local]));
        }
        if local {
            require(lhs.iter().all(|n| n[0] == "name"), "exe_lua_syntax_invalid")?;
            return Ok(json!(["assign", lhs, [], true]));
        }
        require(
            lhs.len() == 1 && lhs[0][0] == "call",
            "exe_lua_statement_unsupported",
        )?;
        Ok(json!(["call", lhs.remove(0)]))
    }
    fn exprs(&mut self) -> Result<Vec<Value>> {
        let mut values = vec![self.expr(0)?];
        while self.accept(",") {
            values.push(self.expr(0)?);
        }
        Ok(values)
    }
    fn expr(&mut self, minimum: u8) -> Result<Value> {
        self.enter()?;
        let mut lhs = if ["not", "-", "#"].contains(&self.peek()) {
            let op = self.take(None)?;
            json!(["unary", op, self.expr(7)?])
        } else if ["string", "number"].contains(&self.peek()) {
            json!(["literal", self.take(None)?])
        } else if ["true", "false", "nil"].contains(&self.peek()) {
            let raw = self.take(None)?;
            json!([
                "literal",
                match raw.as_str().unwrap() {
                    "true" => json!(true),
                    "false" => json!(false),
                    _ => Value::Null,
                }
            ])
        } else if self.accept("(") {
            let value = self.expr(0)?;
            self.take(Some(")"))?;
            value
        } else if self.accept("function") {
            self.function()?
        } else if self.accept("{") {
            let mut fields = Vec::new();
            while self.peek() != "}" {
                let (key, value) = if self.accept("[") {
                    let key = self.expr(0)?;
                    self.take(Some("]"))?;
                    self.take(Some("="))?;
                    (key, self.expr(0)?)
                } else if self.tokens.get(self.i + 1).is_some_and(|t| t.kind == "=") {
                    let key = json!(["literal", self.name()?]);
                    self.take(Some("="))?;
                    (key, self.expr(0)?)
                } else {
                    (json!(["literal", fields.len() + 1]), self.expr(0)?)
                };
                fields.push(json!([key, value]));
                if !self.accept(",") && !self.accept(";") {
                    break;
                }
            }
            self.take(Some("}"))?;
            json!(["table", fields])
        } else if self.accept("...") {
            json!(["vararg"])
        } else {
            json!(["name", self.name()?])
        };
        check_ast_depth(&lhs)?;
        loop {
            if self.accept(".") {
                lhs = json!(["index", lhs, ["literal", self.name()?]]);
            } else if self.accept("[") {
                let key = self.expr(0)?;
                self.take(Some("]"))?;
                lhs = json!(["index", lhs, key]);
            } else if self.accept("(") {
                let args = if self.peek() == ")" {
                    Vec::new()
                } else {
                    self.exprs()?
                };
                self.take(Some(")"))?;
                lhs = json!(["call", lhs, args]);
            } else if let Some(priority) = precedence(self.peek()).filter(|p| *p >= minimum) {
                let op = self.take(None)?;
                let right_min = if op == ".." || op == "^" {
                    priority
                } else {
                    priority + 1
                };
                lhs = json!(["binary", op, lhs, self.expr(right_min)?]);
            } else {
                break;
            }
            check_ast_depth(&lhs)?;
        }
        self.depth -= 1;
        Ok(lhs)
    }
}

/// Parse bounded Lua 5.1 syntax as inert AST arrays; no runtime is involved.
pub fn parse(source: &str) -> Result<Value> {
    let mut parser = Parser {
        tokens: lex(source)?,
        i: 0,
        depth: 0,
    };
    let result = parser.block(&["eof"])?;
    parser.take(Some("eof"))?;
    Ok(json!(result))
}
