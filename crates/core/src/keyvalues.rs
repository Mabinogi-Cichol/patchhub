//! A bounded parser for the KeyValues subset used by Steam library metadata.
use std::collections::{BTreeMap, HashSet};

const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Value {
    Text(String),
    Object(BTreeMap<String, Value>),
}

impl Value {
    pub(crate) fn object(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Self::Object(value) => Some(value),
            _ => None,
        }
    }

    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParseError(pub(crate) String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

struct Parser<'a> {
    source: &'a [u8],
    pos: usize,
}

pub(crate) fn parse(source: &[u8]) -> Result<BTreeMap<String, Value>, ParseError> {
    if source.len() > MAX_BYTES {
        return Err(ParseError("KeyValues file exceeds 4 MiB".into()));
    }
    let source = source.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(source);
    std::str::from_utf8(source).map_err(|_| ParseError("KeyValues file is not UTF-8".into()))?;
    let mut parser = Parser { source, pos: 0 };
    let result = parser.object(false, 0)?;
    parser.skip_space()?;
    if parser.pos != source.len() {
        return Err(parser.error("unexpected trailing input"));
    }
    Ok(result)
}

impl Parser<'_> {
    fn error(&self, message: &str) -> ParseError {
        ParseError(format!("{message} at byte {}", self.pos))
    }

    fn skip_space(&mut self) -> Result<(), ParseError> {
        loop {
            while self
                .source
                .get(self.pos)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.pos += 1;
            }
            if self.source.get(self.pos..self.pos + 2) == Some(b"//") {
                self.pos += 2;
                while self.pos < self.source.len() && self.source[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else if self.source.get(self.pos..self.pos + 2) == Some(b"/*") {
                self.pos += 2;
                let remainder = &self.source[self.pos..];
                let end = remainder
                    .windows(2)
                    .position(|pair| pair == b"*/")
                    .ok_or_else(|| self.error("unterminated comment"))?;
                self.pos += end + 2;
            } else {
                return Ok(());
            }
        }
    }

    fn token(&mut self) -> Result<String, ParseError> {
        self.skip_space()?;
        if self.pos >= self.source.len() {
            return Err(self.error("expected token"));
        }
        if self.source[self.pos] == b'"' {
            self.pos += 1;
            let mut result = Vec::new();
            while let Some(&byte) = self.source.get(self.pos) {
                self.pos += 1;
                match byte {
                    b'"' => {
                        return String::from_utf8(result)
                            .map_err(|_| self.error("invalid UTF-8 escape"))
                    }
                    b'\\' => {
                        let escaped = *self
                            .source
                            .get(self.pos)
                            .ok_or_else(|| self.error("unterminated escape"))?;
                        self.pos += 1;
                        match escaped {
                            b'"' | b'\\' => result.push(escaped),
                            b'n' => result.push(b'\n'),
                            b't' => result.push(b'\t'),
                            b'r' => result.push(b'\r'),
                            // Steam paths often contain literal backslashes in older files.
                            other => {
                                result.push(b'\\');
                                result.push(other);
                            }
                        }
                    }
                    other => result.push(other),
                }
            }
            Err(self.error("unterminated quoted string"))
        } else {
            let start = self.pos;
            while let Some(&byte) = self.source.get(self.pos) {
                if byte.is_ascii_whitespace()
                    || matches!(byte, b'{' | b'}' | b'"')
                    || self.source.get(self.pos..self.pos + 2) == Some(b"//")
                    || self.source.get(self.pos..self.pos + 2) == Some(b"/*")
                {
                    break;
                }
                self.pos += 1;
            }
            if start == self.pos {
                return Err(self.error("expected token"));
            }
            Ok(std::str::from_utf8(&self.source[start..self.pos])
                .unwrap()
                .to_owned())
        }
    }

    fn object(
        &mut self,
        nested: bool,
        depth: usize,
    ) -> Result<BTreeMap<String, Value>, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.error("KeyValues nesting exceeds 32 levels"));
        }
        let mut entries = BTreeMap::new();
        let mut keys = HashSet::new();
        loop {
            self.skip_space()?;
            match self.source.get(self.pos) {
                None if nested => return Err(self.error("unclosed object")),
                None => return Ok(entries),
                Some(b'}') if nested => {
                    self.pos += 1;
                    return Ok(entries);
                }
                Some(b'}') => return Err(self.error("unexpected closing brace")),
                Some(b'{') => return Err(self.error("expected key")),
                _ => {}
            }
            let key = self.token()?;
            self.skip_space()?;
            let value = if self.source.get(self.pos) == Some(&b'{') {
                self.pos += 1;
                Value::Object(self.object(true, depth + 1)?)
            } else {
                Value::Text(self.token()?)
            };
            if !keys.insert(key.to_ascii_lowercase()) {
                return Err(self.error(&format!("duplicate key {key:?}")));
            }
            entries.insert(key, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_comments_nested_objects_and_escaped_strings() {
        let parsed = parse(
            br#"// comment
            root { /* inside */ child "a\"b\\c" }"#,
        )
        .unwrap();
        let child = parsed
            .get("root")
            .unwrap()
            .object()
            .unwrap()
            .get("child")
            .unwrap();
        assert_eq!(child.text(), Some("a\"b\\c"));
    }

    #[test]
    fn rejects_ambiguous_and_unbounded_input() {
        assert!(parse(br#"root { Name 1 name 2 }"#).is_err());
        assert!(parse(&vec![b'a'; MAX_BYTES + 1]).is_err());
        let nested = format!(
            "{}x 1{}",
            "a { ".repeat(MAX_DEPTH + 1),
            " }".repeat(MAX_DEPTH + 1)
        );
        assert!(parse(nested.as_bytes()).is_err());
    }
}
