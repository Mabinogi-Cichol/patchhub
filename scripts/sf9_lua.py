"""Bounded Lua 5.1 syntax parser. Produces data only, never runs Lua."""
import re
from sf95_audit import require

VERSION = "sf9-lua-v1"
MAX_TOKENS = 100000
MAX_DEPTH = 96
PREC = {"or": 1, "and": 2, "<": 3, ">": 3, "<=": 3, ">=": 3,
        "~=": 3, "==": 3, "..": 4, "+": 5, "-": 5, "*": 6, "/": 6, "%": 6, "^": 8}


def tokens(source):
    require(isinstance(source, str) and len(source) <= 1024 * 1024, "exe_lua_size_limit")
    output = []
    i = 0
    while i < len(source):
        c = source[i]
        if c.isspace():
            i += 1
            continue
        comment = source.startswith("--", i)
        start = i + 2 if comment else i
        long = re.match(r"\[(=*)\[", source[start:])
        if long:
            closing = "]" + long[1] + "]"
            content = start + len(long[0])
            end = source.find(closing, content)
            require(end >= 0, "exe_lua_unterminated_long_string")
            if not comment:
                output.append(("string", source[content:end]))
            i = end + len(closing)
        elif comment:
            end = re.search(r"[\r\n]", source[start:])
            i = start + end.start() if end else len(source)
        elif c in "\"'":
            quote = c
            i += 1
            value = []
            while i < len(source) and source[i] != quote:
                c = source[i]
                require(c not in "\r\n", "exe_lua_unterminated_string")
                if c == "\\":
                    i += 1
                    require(i < len(source), "exe_lua_unterminated_string")
                    c = source[i]
                    escapes = {"n": "\n", "r": "\r", "t": "\t", "b": "\b", "f": "\f", "v": "\v", "a": "\a"}
                    if c.isdigit():
                        match = re.match(r"[0-9]{1,3}", source[i:])
                        require(int(match[0]) <= 255, "exe_lua_escape_invalid")
                        value.append(chr(int(match[0]))); i += len(match[0]); continue
                    value.append(escapes.get(c, c))
                else:
                    value.append(c)
                i += 1
            require(i < len(source), "exe_lua_unterminated_string")
            output.append(("string", "".join(value))); i += 1
        else:
            match = re.match(r"(?:0[xX][0-9a-fA-F]+|[0-9]+(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)", source[i:])
            if match:
                raw = match[0]
                value = int(raw, 16) if raw.lower().startswith("0x") else float(raw) if "." in raw or "e" in raw.lower() else int(raw)
                output.append(("number", value)); i += len(raw)
            else:
                match = re.match(r"[A-Za-z_][A-Za-z0-9_]*", source[i:])
                if match:
                    output.append((match[0] if match[0] in {"and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while"} else "identifier", match[0])); i += len(match[0])
                else:
                    match = re.match(r"\.\.\.|\.\.|==|~=|<=|>=|[+*/%^#<>=(){}\[\],;:.-]", source[i:])
                    require(match is not None, "exe_lua_token_unsupported")
                    output.append((match[0], match[0])); i += len(match[0])
        require(len(output) <= MAX_TOKENS, "exe_lua_token_limit")
    return output + [("eof", "eof")]


class Parser:
    def __init__(self, source):
        self.t = tokens(source)
        self.i = 0
        self.depth = 0

    def peek(self): return self.t[self.i][0]
    def take(self, expected=None):
        token = self.t[self.i]
        require(expected is None or token[0] == expected, "exe_lua_syntax_invalid")
        self.i += 1
        return token[1]
    def accept(self, token):
        if self.peek() == token: self.take(); return True
        return False
    def name(self):
        value = self.take()
        require(isinstance(value, str) and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", value), "exe_lua_name_invalid")
        return value
    def block(self, stops=("eof",)):
        self.depth += 1
        require(self.depth <= MAX_DEPTH, "exe_lua_depth_limit")
        result = []
        while self.peek() not in stops:
            require(self.peek() != "eof", "exe_lua_syntax_invalid")
            if self.accept(";"): continue
            result.append(self.statement())
            self.accept(";")
        self.depth -= 1
        return result
    def function(self):
        self.take("(")
        parameters = []
        if self.peek() != ")":
            while True:
                parameters.append(self.take("...") if self.peek() == "..." else self.name())
                if not self.accept(","): break
        self.take(")")
        body = self.block(("end",)); self.take("end")
        return ("function", parameters, body)
    def statement(self):
        local = self.accept("local")
        if self.accept("function"):
            name = self.name()
            return ("define", name, self.function(), local)
        if self.accept("if"):
            arms = []
            condition = self.expr(); self.take("then")
            arms.append((condition, self.block(("elseif", "else", "end"))))
            while self.accept("elseif"):
                condition = self.expr(); self.take("then")
                arms.append((condition, self.block(("elseif", "else", "end"))))
            other = self.block(("end",)) if self.accept("else") else []
            self.take("end"); return ("if", arms, other)
        if self.accept("return"):
            values = [] if self.peek() in ("end", "else", "elseif", "eof", ";") else self.exprs()
            return ("return", values)
        if self.accept("break"): return ("break",)
        if self.accept("for"):
            names = [self.name()]
            while self.accept(","): names.append(self.name())
            numeric = self.accept("=")
            if not numeric: self.take("in")
            values = self.exprs(); self.take("do")
            body = self.block(("end",)); self.take("end")
            return ("for", names, values, body, numeric)
        if self.accept("while"):
            test = self.expr(); self.take("do")
            body = self.block(("end",)); self.take("end")
            return ("while", test, body)
        if self.accept("do"):
            body = self.block(("end",)); self.take("end")
            return ("do", body)
        lhs = [self.expr()]
        while self.accept(","): lhs.append(self.expr())
        if self.accept("="):
            return ("assign", lhs, self.exprs(), local)
        if local:
            require(all(n[0] == "name" for n in lhs), "exe_lua_syntax_invalid")
            return ("assign", lhs, [], True)
        require(len(lhs) == 1 and lhs[0][0] == "call", "exe_lua_statement_unsupported")
        return ("call", lhs[0])
    def exprs(self):
        result = [self.expr()]
        while self.accept(","): result.append(self.expr())
        return result
    def expr(self, minimum=0):
        self.depth += 1
        require(self.depth <= MAX_DEPTH, "exe_lua_depth_limit")
        if self.peek() in ("not", "-", "#"):
            lhs = ("unary", self.take(), self.expr(7))
        elif self.peek() in ("string", "number"):
            lhs = ("literal", self.take())
        elif self.peek() in ("true", "false", "nil"):
            lhs = ("literal", {"true": True, "false": False, "nil": None}[self.take()])
        elif self.accept("("):
            lhs = self.expr(); self.take(")")
        elif self.accept("function"):
            lhs = self.function()
        elif self.accept("{"):
            fields = []
            while self.peek() != "}":
                if self.accept("["):
                    key = self.expr(); self.take("]"); self.take("="); value = self.expr()
                elif self.t[self.i + 1][0] == "=":
                    key = ("literal", self.name()); self.take("="); value = self.expr()
                else:
                    key = ("literal", len(fields) + 1); value = self.expr()
                fields.append((key, value))
                if not self.accept(",") and not self.accept(";"): break
            self.take("}"); lhs = ("table", fields)
        elif self.accept("..."):
            lhs = ("vararg",)
        else:
            lhs = ("name", self.name())
        while True:
            if self.accept("."):
                lhs = ("index", lhs, ("literal", self.name()))
            elif self.accept("["):
                key = self.expr(); self.take("]"); lhs = ("index", lhs, key)
            elif self.accept("("):
                args = [] if self.peek() == ")" else self.exprs()
                self.take(")"); lhs = ("call", lhs, args)
            elif self.peek() in PREC and PREC[self.peek()] >= minimum:
                op = self.take(); priority = PREC[op]
                lhs = ("binary", op, lhs, self.expr(priority if op in ("..", "^") else priority + 1))
            else: break
        self.depth -= 1
        return lhs


def parse(source):
    parser = Parser(source)
    result = parser.block()
    parser.take("eof")
    return result
