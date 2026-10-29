//! Deterministic source identity encoding (P0 contract v0.5 §1).
//!
//! Replaces the former Python-`json.dumps`-equivalent `python_json`: byte parity
//! with Python is explicitly **not** a requirement. Instead every identity hash
//! is derived from [`canonical_bytes`], a self-describing deterministic JSON
//! encoding versioned by `SOURCES_ENCODING_VERSION`.
//!
//! Encoding rules (contract §1.1):
//! - object keys sort by UTF-8 byte order (equal to Unicode codepoint order);
//! - separators carry no spaces (`,` / `:`);
//! - strings are raw UTF-8 with only the JSON-mandatory escapes
//!   (`"`, `\`, control characters); no `ensure_ascii` escaping;
//! - integers are decimal; floats are rejected with `invalid_canonical_value`.
//!
//! The encoder goes through a `serde_json::Value` intermediate: the field set
//! still comes from the explicit `Serialize` impl of `T` (contract §1.1), while
//! the runtime representation lets the identity helpers clone-and-strip the
//! volatile keys of `fingerprint`. `serde_json`'s default map backend is already
//! sorted, but the encoder sorts explicitly so key order never depends on the
//! backend.

use super::validate::split_url;
use super::{invalid, Result};
use sha2::{Digest, Sha256};

/// Deterministic JSON bytes for any `Serialize` value (contract §1.1).
///
/// Floats (including integers that overflow `i64`/`u64`) are rejected with
/// [`invalid`]`("invalid_canonical_value")`.
pub fn canonical_bytes<T: serde::Serialize + ?Sized>(value: &T) -> Result<Vec<u8>> {
    let value = serde_json::to_value(value).map_err(|_| invalid("invalid_canonical_value"))?;
    let mut out = Vec::new();
    write_value(&mut out, &value)?;
    Ok(out)
}

/// [`canonical_bytes`] rendered as a UTF-8 string.
pub fn canonical_string<T: serde::Serialize + ?Sized>(value: &T) -> Result<String> {
    let bytes = canonical_bytes(value)?;
    String::from_utf8(bytes).map_err(|_| invalid("invalid_canonical_value"))
}

/// `sha256(canonical_bytes(value))` as lowercase hex.
///
/// The input must not contain floats (guaranteed by the fixed identity
/// structures); float input is rejected with `invalid_canonical_value` at the
/// [`canonical_bytes`] layer, which this helper treats as unreachable.
pub fn canonical_sha256<T: serde::Serialize + ?Sized>(value: &T) -> String {
    sha256_hex(&canonical_bytes(value).expect("canonical identity input must not contain floats"))
}

fn write_value(out: &mut Vec<u8>, value: &serde_json::Value) -> Result<()> {
    match value {
        serde_json::Value::Null => out.extend_from_slice(b"null"),
        serde_json::Value::Bool(true) => out.extend_from_slice(b"true"),
        serde_json::Value::Bool(false) => out.extend_from_slice(b"false"),
        serde_json::Value::Number(number) => write_number(out, number)?,
        serde_json::Value::String(text) => write_string(out, text),
        serde_json::Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_value(out, item)?;
            }
            out.push(b']');
        }
        serde_json::Value::Object(map) => {
            // `String` ordering is UTF-8 byte order, which equals Unicode
            // codepoint order for scalar values.
            let mut entries: Vec<(&String, &serde_json::Value)> = map.iter().collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            out.push(b'{');
            for (index, (key, item)) in entries.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_string(out, key);
                out.push(b':');
                write_value(out, item)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

fn write_number(out: &mut Vec<u8>, number: &serde_json::Number) -> Result<()> {
    if let Some(value) = number.as_u64() {
        out.extend_from_slice(value.to_string().as_bytes());
        Ok(())
    } else if let Some(value) = number.as_i64() {
        out.extend_from_slice(value.to_string().as_bytes());
        Ok(())
    } else {
        // `serde_json` only stores a float (or an integer that overflows) here.
        Err(invalid("invalid_canonical_value"))
    }
}

fn write_string(out: &mut Vec<u8>, text: &str) {
    out.push(b'"');
    for character in text.chars() {
        match character {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\u{08}' => out.extend_from_slice(b"\\b"),
            '\u{0c}' => out.extend_from_slice(b"\\f"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            c if (c as u32) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes());
            }
            c => {
                let mut buffer = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
            }
        }
    }
    out.push(b'"');
}

/// `canonical_url(u) = urlunsplit(scheme, netloc.lower(), path, "", "")`
/// (design §5.1): drop query/fragment, lowercase host, preserve the port as-is
/// (current behavior; folding port 443 would require an encoding version bump),
/// and keep the path raw (percent-encoding normalization is not part of the
/// frozen contract).
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

/// `"candidate-" + sha256(canonical_bytes(identity))[:24]` (contract §1.2).
///
/// `identity` is the ordered array
/// `[source_id, canonical_url(source_page), variant, languages, version_identity]`.
///
/// The input must not contain floats (guaranteed by the fixed identity shape);
/// float input is rejected with `invalid_canonical_value` at [`canonical_bytes`].
pub fn candidate_id(identity: &serde_json::Value) -> String {
    let digest =
        sha256_hex(&canonical_bytes(identity).expect("candidate identity must not contain floats"));
    format!("candidate-{}", &digest[..24])
}

/// `sha256(canonical_bytes(candidate minus fingerprint/observed_at/cache_status))`
/// (contract §1.2). The `serde_json::Value` intermediate is the frozen choice for
/// dropping the three volatile keys.
///
/// The input must not contain floats (guaranteed by the candidate structure);
/// float input is rejected with `invalid_canonical_value` at [`canonical_bytes`].
pub fn fingerprint(candidate: &serde_json::Value) -> String {
    let mut value = candidate.clone();
    if let serde_json::Value::Object(map) = &mut value {
        map.remove("fingerprint");
        map.remove("observed_at");
        map.remove("cache_status");
    }
    sha256_hex(&canonical_bytes(&value).expect("candidate fingerprint must not contain floats"))
}

/// `sha256(canonical_bytes(query))`; `query` carries the matching-input keys plus
/// `encoding_version` (contract §1.2).
///
/// The input must not contain floats (guaranteed by the fixed query shape);
/// float input is rejected with `invalid_canonical_value` at [`canonical_bytes`].
pub fn query_key(query: &serde_json::Value) -> String {
    sha256_hex(&canonical_bytes(query).expect("query key must not contain floats"))
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
            "/tests/fixtures/sources/encoding-golden.json"
        ))
        .expect("encoding golden fixture must be readable");
        serde_json::from_str(&raw).expect("encoding golden fixture must be valid JSON")
    }

    #[test]
    fn canonical_url_matches_frozen_semantics() {
        let fixture = fixture();
        for case in fixture["canonical_urls"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            assert_eq!(canonical_url(input), case["expected"].as_str().unwrap());
        }
        // Dropped query/fragment, lowercased host, path kept as-is.
        assert_eq!(
            canonical_url("https://Example.COM/Game/?q=1#frag"),
            "https://example.com/Game/"
        );
    }

    #[test]
    fn canonical_bytes_is_deterministic_and_orders_keys() {
        // Insertion order must not change the bytes: keys sort by UTF-8 bytes.
        let first: Value =
            serde_json::from_str(r#"{"b":1,"a":2,"emoji":"😀","中文":"值"}"#).unwrap();
        let second: Value =
            serde_json::from_str(r#"{"中文":"值","emoji":"😀","a":2,"b":1}"#).unwrap();
        assert_eq!(
            canonical_bytes(&first).unwrap(),
            canonical_bytes(&second).unwrap()
        );
        // Raw UTF-8, no `ensure_ascii`, no separator spaces.
        assert_eq!(
            String::from_utf8(canonical_bytes(&first).unwrap()).unwrap(),
            "{\"a\":2,\"b\":1,\"emoji\":\"😀\",\"中文\":\"值\"}"
        );
    }

    #[test]
    fn canonical_bytes_escapes_only_json_mandatory_characters() {
        let control: Value = serde_json::Value::String("\"\u{1}\n/".into());
        assert_eq!(
            String::from_utf8(canonical_bytes(&control).unwrap()).unwrap(),
            "\"\\\"\\u0001\\n/\""
        );
        let cjk: Value = serde_json::json!({"é": "→"});
        assert_eq!(
            String::from_utf8(canonical_bytes(&cjk).unwrap()).unwrap(),
            "{\"é\":\"→\"}"
        );
    }

    #[test]
    fn canonical_bytes_rejects_floats() {
        let error = canonical_bytes(&serde_json::json!(1.5)).unwrap_err();
        assert_eq!(error.to_string(), "invalid_canonical_value");
        let nested = canonical_bytes(&serde_json::json!({"a": [1, 2.5]})).unwrap_err();
        assert_eq!(nested.to_string(), "invalid_canonical_value");
        // Integers are accepted and rendered in decimal.
        assert_eq!(
            String::from_utf8(canonical_bytes(&serde_json::json!({"a": [-3, 42]})).unwrap())
                .unwrap(),
            "{\"a\":[-3,42]}"
        );
    }

    #[test]
    fn canonical_bytes_rejects_integer_overflow() {
        // `serde_json` stores integers beyond `u64`/`i64` as floats, which the
        // encoder must reject rather than round.
        let over_u64: Value = serde_json::from_str("18446744073709551616").unwrap();
        assert_eq!(
            canonical_bytes(&over_u64).unwrap_err().to_string(),
            "invalid_canonical_value"
        );
        let under_i64: Value = serde_json::from_str("-9223372036854775809").unwrap();
        assert_eq!(
            canonical_bytes(&under_i64).unwrap_err().to_string(),
            "invalid_canonical_value"
        );
        // The boundaries themselves stay integers.
        assert_eq!(
            String::from_utf8(canonical_bytes(&serde_json::json!(u64::MAX)).unwrap()).unwrap(),
            u64::MAX.to_string()
        );
        assert_eq!(
            String::from_utf8(canonical_bytes(&serde_json::json!(i64::MIN)).unwrap()).unwrap(),
            i64::MIN.to_string()
        );
    }

    #[test]
    fn canonical_bytes_does_not_normalize_unicode() {
        // NFC "é" (U+00E9) and NFD "e" + U+0301 are distinct codepoint sequences:
        // the encoder never applies Unicode normalization, so each form is stable
        // and they must not collide.
        let nfc = Value::String("\u{00e9}".to_string());
        let nfd = Value::String("e\u{0301}".to_string());
        assert_eq!(
            String::from_utf8(canonical_bytes(&nfc).unwrap()).unwrap(),
            "\"\u{00e9}\""
        );
        assert_eq!(
            String::from_utf8(canonical_bytes(&nfd).unwrap()).unwrap(),
            "\"e\u{0301}\""
        );
        assert_ne!(
            canonical_bytes(&nfc).unwrap(),
            canonical_bytes(&nfd).unwrap()
        );
        // Object keys follow the same rule.
        let nfc_key = canonical_bytes(&serde_json::json!({"\u{00e9}": 1})).unwrap();
        let nfd_key = canonical_bytes(&serde_json::json!({"e\u{0301}": 1})).unwrap();
        assert_ne!(nfc_key, nfd_key);
    }

    #[test]
    fn golden_matches_new_rust_encoding() {
        let fixture = fixture();
        assert_eq!(
            fixture["encoding_version"].as_u64(),
            Some(crate::sources::SOURCES_ENCODING_VERSION as u64)
        );

        for case in fixture["canonical"].as_array().unwrap() {
            let bytes = canonical_bytes(&case["input"]).unwrap();
            assert_eq!(
                String::from_utf8(bytes).unwrap(),
                case["expected"].as_str().unwrap()
            );
        }

        for case in fixture["candidate_id"].as_array().unwrap() {
            assert_eq!(
                candidate_id(&case["identity"]),
                case["expected"].as_str().unwrap()
            );
        }

        for case in fixture["query_key"].as_array().unwrap() {
            assert_eq!(
                query_key(&case["input"]),
                case["expected"].as_str().unwrap()
            );
        }

        for case in fixture["fingerprint"].as_array().unwrap() {
            assert_eq!(
                fingerprint(&case["input"]),
                case["expected"].as_str().unwrap()
            );
        }

        for case in fixture["rule_fingerprint"].as_array().unwrap() {
            assert_eq!(
                canonical_sha256(&case["input"]),
                case["expected"].as_str().unwrap()
            );
        }
    }

    #[test]
    fn fingerprint_drops_volatile_keys() {
        let base: Value = serde_json::json!({"a": 1, "fingerprint": "x", "observed_at": "y", "cache_status": "fresh"});
        let expected: Value = serde_json::json!({"a": 1});
        assert_eq!(fingerprint(&base), fingerprint(&expected));
        assert_eq!(fingerprint(&expected), sha256_hex(b"{\"a\":1}"));
    }

    #[test]
    fn candidate_id_is_stable_across_field_reordering() {
        let first: Value = serde_json::json!([
            "kagura-cn",
            "https://kagurafan.com/game/",
            "base",
            ["zh-CN"],
            "1"
        ]);
        let second: Value = serde_json::from_str(
            r#"["kagura-cn","https://kagurafan.com/game/","base",["zh-CN"],"1"]"#,
        )
        .unwrap();
        assert_eq!(candidate_id(&first), candidate_id(&second));
    }

    /// Regenerates `crates/core/tests/fixtures/sources/encoding-golden.json`.
    ///
    /// Run manually with `cargo test -p patchhub-core --lib -- --ignored
    /// regenerate_encoding_golden`. This is the Rust replacement for the former
    /// `gen_identity_golden.py` (now deleted along with the Python byte golden).
    #[test]
    #[ignore = "writes crates/core/tests/fixtures/sources/encoding-golden.json"]
    fn regenerate_encoding_golden() {
        // Built through the real `Serialize` impl so re-serializing the
        // deserialized fixture reproduces the exact same bytes.
        let policy_rule = serde_json::to_value(crate::sources::model::SourceRule {
            script: None,
            id: "patchmoe".to_string(),
            name: "PatchMoe".to_string(),
            adapter: "patchmoe".to_string(),
            role: crate::sources::model::SourceRole::Publisher,
            discovery_only: false,
            priority: 100,
            enabled: true,
            pages: vec!["https://www.patch.moe/".to_string()],
            page_hosts: vec!["www.patch.moe".to_string()],
            download_hosts: vec!["download.patch.moe".to_string()],
            root_layout: Some(crate::sources::model::RootLayout::GameRoot),
            columns: None,
            table_id: None,
            language: None,
            url: None,
        })
        .unwrap();

        let canonical_cases = [
            serde_json::json!({"emoji": "😀", "a": 1, "中文": "值", "b": [true, null, "x"]}),
            serde_json::json!({"z": [], "a": {}}),
        ];
        let canonical: Vec<Value> = canonical_cases
            .iter()
            .map(|input| {
                serde_json::json!({
                    "input": input,
                    "expected": String::from_utf8(canonical_bytes(input).unwrap()).unwrap(),
                })
            })
            .collect();

        let candidate_identities = [
            serde_json::json!([
                "official",
                "https://publisher.example/game/",
                "base",
                ["en"],
                "page-unknown"
            ]),
            serde_json::json!([
                "kagura-cn",
                "https://kagurafan.com/game/",
                "dlc",
                ["zh-CN"],
                "v1.2.3:patch_v1.2.3.zip"
            ]),
            serde_json::json!([
                "hgamer",
                "https://hgamer.tw/zh-TW/patches/my-life-as-a-cult-leader-patch",
                "base",
                ["zh-CN"],
                "4"
            ]),
        ];
        let candidate_id_cases: Vec<Value> = candidate_identities
            .iter()
            .map(|identity| {
                serde_json::json!({
                    "identity": identity,
                    "expected": candidate_id(identity),
                })
            })
            .collect();

        let query_input = serde_json::json!({
            "appid": 42,
            "name": "A Game",
            "name_hints": [],
            "legacy_match": false,
            "source_context": ["official"],
            "parser_version": 2,
            "query_version": 1,
            "encoding_version": crate::sources::SOURCES_ENCODING_VERSION,
        });
        let query_key_cases = [serde_json::json!({
            "input": query_input,
            "expected": query_key(&query_input),
        })];

        let fingerprint_input = serde_json::json!({
            "aliases": ["A Game"],
            "candidate_id": "candidate-49d4abc4342bbe0aa2491fcb",
            "capability": "discover_only",
            "excludes_dlc": null,
            "fingerprint": "volatile",
            "format_observed": "unknown",
            "format_reported": "zip",
            "game_appid": 42,
            "game_title": "A Game",
            "identity_evidence": [{"appid": 42, "kind": "steam_appid", "page": "https://hgamer.tw/zh-CN/patches/game"}],
            "languages": [],
            "match_basis": "appid",
            "mirrors": [],
            "observed_at": "2026-01-01T00:00:00+00:00",
            "cache_status": "fresh",
            "parent_appid": null,
            "patch_kind": "adult_content",
            "patch_title": "A Game",
            "patch_updated_at": null,
            "platforms": ["steam"],
            "provenance": [{"page": "https://hgamer.tw/zh-CN/patches/game", "source_id": "hgamer"}],
            "published_at": null,
            "requires_dlc": null,
            "source_id": "hgamer",
            "source_page": "https://hgamer.tw/zh-CN/patches/game",
            "source_record_id": "https://hgamer.tw/zh-CN/patches/game",
            "upstream_page": null,
            "variant": "base",
            "version_evidence": ["1"],
            "version_identity": "1",
            "version_raw": "1",
            "warnings": [],
        });
        let fingerprint_cases = [serde_json::json!({
            "input": fingerprint_input,
            "expected": fingerprint(&fingerprint_input),
        })];

        let rule_cases = [serde_json::json!({
            "input": policy_rule,
            "expected": canonical_sha256(&policy_rule),
        })];

        let document = serde_json::json!({
            "schema_version": 1,
            "encoding_version": crate::sources::SOURCES_ENCODING_VERSION,
            "note": "Golden values produced by the frozen Rust deterministic encoding (P0 contract v0.5 §1). Byte/ID parity with the former Python implementation is explicitly NOT required. URL normalization keeps the path raw and the port as-is (port 443 is NOT folded); percent-encoding normalization remains 待调研 (p0-contracts §1.1).",
            "canonical_urls": [
                {"input": "https://publisher.example/game/", "expected": canonical_url("https://publisher.example/game/")},
                {"input": "https://Example.COM/Game/?q=1#frag", "expected": canonical_url("https://Example.COM/Game/?q=1#frag")},
                {"input": "https://example.com:443/x", "expected": canonical_url("https://example.com:443/x")},
                {"input": "https://kagurafan.com/game/", "expected": canonical_url("https://kagurafan.com/game/")},
            ],
            "canonical": canonical,
            "candidate_id": candidate_id_cases,
            "query_key": query_key_cases,
            "fingerprint": fingerprint_cases,
            "rule_fingerprint": rule_cases,
        });

        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sources/encoding-golden.json"
        );
        let mut text = serde_json::to_string_pretty(&document).unwrap();
        text.push('\n');
        std::fs::write(path, text).unwrap();
    }
}
