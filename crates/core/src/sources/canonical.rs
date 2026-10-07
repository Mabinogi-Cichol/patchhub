//! Cross-language byte-exact identity: Python-`json.dumps`-equivalent
//! serialization plus the `candidate_id` / `fingerprint` / `query_key` hashes
//! (design §5.1).
//!
//! `serde_json::to_string` is intentionally NOT used: Python emits `", "` / `": "`
//! separators, codepoint key ordering and an `ensure_ascii` escaping mode.

use super::validate::split_url;
use sha2::{Digest, Sha256};

/// Reproduces Python `json.dumps(value, separators=(", ", ": "), sort_keys=sort_keys, ensure_ascii=ensure_ascii)`
/// for the value shapes used by the identity golden (null / bool / integer /
/// string / array / object). Floats are out of contract (§5.1); if one appears it
/// falls back to serde's rendering.
///
/// `sort_keys = false` iterates `serde_json::Map` in its natural order. Under
/// serde_json's default backend that is codepoint order, not Python insertion
/// order; the only `sort_keys = false` contract use is `candidate_id`, whose
/// `identity` is an ordered array and therefore unaffected.
pub fn python_json(value: &serde_json::Value, sort_keys: bool, ensure_ascii: bool) -> String {
    let mut out = String::new();
    write_value(&mut out, value, sort_keys, ensure_ascii);
    out
}

fn write_value(out: &mut String, value: &serde_json::Value, sort_keys: bool, ensure_ascii: bool) {
    match value {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(true) => out.push_str("true"),
        serde_json::Value::Bool(false) => out.push_str("false"),
        serde_json::Value::Number(number) => out.push_str(&number_to_string(number)),
        serde_json::Value::String(text) => write_string(out, text, ensure_ascii),
        serde_json::Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_value(out, item, sort_keys, ensure_ascii);
            }
            out.push(']');
        }
        serde_json::Value::Object(map) => {
            out.push('{');
            let mut first = true;
            if sort_keys {
                // `String` ordering is UTF-8 byte order, which equals Unicode
                // codepoint order for scalar values; Python sorts by codepoint.
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                for key in keys {
                    write_entry(out, key, &map[key], &mut first, sort_keys, ensure_ascii);
                }
            } else {
                for (key, item) in map {
                    write_entry(out, key, item, &mut first, sort_keys, ensure_ascii);
                }
            }
            out.push('}');
        }
    }
}

fn write_entry(
    out: &mut String,
    key: &str,
    value: &serde_json::Value,
    first: &mut bool,
    sort_keys: bool,
    ensure_ascii: bool,
) {
    if !*first {
        out.push_str(", ");
    }
    *first = false;
    write_string(out, key, ensure_ascii);
    out.push_str(": ");
    write_value(out, value, sort_keys, ensure_ascii);
}

fn number_to_string(number: &serde_json::Number) -> String {
    if let Some(value) = number.as_u64() {
        value.to_string()
    } else if let Some(value) = number.as_i64() {
        value.to_string()
    } else {
        number.to_string()
    }
}

fn write_string(out: &mut String, text: &str, ensure_ascii: bool) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c if ensure_ascii && (c as u32) > 0x7f => {
                let mut buffer = [0u16; 2];
                for &unit in c.encode_utf16(&mut buffer).iter() {
                    out.push_str(&format!("\\u{:04x}", unit));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `canonical_url(u) = urlunsplit(scheme, netloc.lower(), path, "", "")`
/// (design §5.1).
pub fn canonical_url(url: &str) -> String {
    let parsed = split_url(url);
    urlunsplit(
        &parsed.scheme,
        &parsed.netloc.to_lowercase(),
        &parsed.path,
        "",
        "",
    )
}

fn urlunsplit(scheme: &str, netloc: &str, path: &str, query: &str, fragment: &str) -> String {
    let mut url = path.to_string();
    if !netloc.is_empty()
        || (!scheme.is_empty() && scheme_uses_netloc(scheme) && !url.starts_with("//"))
    {
        if !url.is_empty() && !url.starts_with('/') {
            url.insert(0, '/');
        }
        url = format!("//{netloc}{url}");
    }
    if !scheme.is_empty() {
        url = format!("{scheme}:{url}");
    }
    if !query.is_empty() {
        url.push('?');
        url.push_str(query);
    }
    if !fragment.is_empty() {
        url.push('#');
        url.push_str(fragment);
    }
    url
}

/// `urllib.parse.uses_netloc`, restricted to the schemes this subsystem handles.
fn scheme_uses_netloc(scheme: &str) -> bool {
    matches!(
        scheme,
        "" | "http"
            | "https"
            | "ftp"
            | "gopher"
            | "nntp"
            | "telnet"
            | "imap"
            | "wais"
            | "file"
            | "mms"
            | "shttp"
            | "snews"
            | "prospero"
            | "rtsp"
            | "rtspu"
            | "rsync"
            | "svn"
            | "svn+ssh"
            | "sftp"
            | "nfs"
            | "git"
            | "git+ssh"
            | "ws"
            | "wss"
            | "itms-services"
    )
}

/// `"candidate-" + sha256(python_json(identity, false, false))[:24]`.
pub fn candidate_id(identity: &serde_json::Value) -> String {
    let digest = sha256_hex(python_json(identity, false, false).as_bytes());
    format!("candidate-{}", &digest[..24])
}

/// `sha256(python_json(candidate minus fingerprint/observed_at/cache_status, true, false))`.
pub fn fingerprint(candidate: &serde_json::Value) -> String {
    let mut value = candidate.clone();
    if let serde_json::Value::Object(map) = &mut value {
        map.remove("fingerprint");
        map.remove("observed_at");
        map.remove("cache_status");
    }
    sha256_hex(python_json(&value, true, false).as_bytes())
}

/// `sha256(python_json(query, true, false))`; `query` must carry the eight
/// §5.1 keys (the fixture `input` object does).
pub fn query_key(query: &serde_json::Value) -> String {
    sha256_hex(python_json(query, true, false).as_bytes())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn fixture() -> Value {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sources/identity-golden.json"
        ))
        .expect("identity golden fixture must be readable");
        serde_json::from_str(&raw).expect("identity golden fixture must be valid JSON")
    }

    #[test]
    fn canonical_matches_python_golden() {
        let fixture = fixture();

        for case in fixture["canonical_urls"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            assert_eq!(canonical_url(input), case["expected"].as_str().unwrap());
        }

        for case in fixture["candidate_id"].as_array().unwrap() {
            let payload = case["payload"].as_str().unwrap();
            let value: Value = serde_json::from_str(payload).unwrap();
            assert_eq!(python_json(&value, false, false), payload);
            assert_eq!(candidate_id(&value), case["expected"].as_str().unwrap());
        }

        for case in fixture["query_key"].as_array().unwrap() {
            let payload = case["payload"].as_str().unwrap();
            let value: Value = serde_json::from_str(payload).unwrap();
            assert_eq!(python_json(&value, true, false), payload);
            assert_eq!(query_key(&value), case["expected"].as_str().unwrap());
        }

        for case in fixture["fingerprint"].as_array().unwrap() {
            let payload = case["payload"].as_str().unwrap();
            let value: Value = serde_json::from_str(payload).unwrap();
            assert_eq!(python_json(&value, true, false), payload);
            assert_eq!(fingerprint(&value), case["expected"].as_str().unwrap());
        }
    }

    #[test]
    fn python_json_default_separators_sorted_keys_and_ascii_escaping() {
        let value: Value = serde_json::json!({"b": 1, "a": [true, null, "x"]});
        assert_eq!(
            python_json(&value, true, false),
            "{\"a\": [true, null, \"x\"], \"b\": 1}"
        );

        let array: Value = serde_json::json!([1, "a", true, null]);
        assert_eq!(python_json(&array, false, false), "[1, \"a\", true, null]");

        let unicode: Value = serde_json::json!({"é": "→"});
        assert_eq!(python_json(&unicode, true, false), "{\"é\": \"→\"}");
        assert_eq!(
            python_json(&unicode, true, true),
            "{\"\\u00e9\": \"\\u2192\"}"
        );

        let control: Value = serde_json::Value::String("\"\u{1}\n".into());
        assert_eq!(python_json(&control, false, false), "\"\\\"\\u0001\\n\"");
    }

    #[test]
    fn fingerprint_drops_volatile_keys() {
        let base: Value = serde_json::json!({"a": 1, "fingerprint": "x", "observed_at": "y", "cache_status": "fresh"});
        let expected: Value = serde_json::json!({"a": 1});
        assert_eq!(fingerprint(&base), fingerprint(&expected));
        assert_eq!(fingerprint(&expected), sha256_hex(b"{\"a\": 1}"));
    }
}
