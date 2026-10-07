//! Bounded, independent parser for Steam's V28/V29 appinfo cache.
//! Only requested payloads are decoded; malformed containers fail atomically.

use std::collections::{HashMap, HashSet};
use std::io::{BufReader, Read, Seek, SeekFrom};

const V28: u32 = 0x0756_4428;
const V29: u32 = 0x0756_4429;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedName {
    pub name: String,
    pub language: Option<String>,
}

#[derive(Clone, Copy)]
struct Limits {
    file: u64,
    payload: u64,
    keys: usize,
    key_bytes: usize,
    string: usize,
    depth: usize,
    nodes: usize,
}

const LIMITS: Limits = Limits {
    file: 256 * 1024 * 1024,
    payload: 16 * 1024 * 1024,
    keys: 65_536,
    key_bytes: 8 * 1024 * 1024,
    string: 1024 * 1024,
    depth: 64,
    nodes: 262_144,
};

pub(crate) fn parse_names<R: Read + Seek>(
    reader: &mut R,
    targets: &HashSet<u32>,
    language: Option<&str>,
) -> Result<HashMap<u32, ResolvedName>, String> {
    parse_with_limits(reader, targets, language, LIMITS)
}

fn io_error(error: std::io::Error) -> String {
    format!("appinfo I/O: {error}")
}

fn read_u32(reader: &mut impl Read) -> Result<u32, String> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes).map_err(io_error)?;
    Ok(u32::from_le_bytes(bytes))
}

fn parse_with_limits<R: Read + Seek>(
    reader: &mut R,
    targets: &HashSet<u32>,
    language: Option<&str>,
    limits: Limits,
) -> Result<HashMap<u32, ResolvedName>, String> {
    let length = reader.seek(SeekFrom::End(0)).map_err(io_error)?;
    if length > limits.file || length < 12 {
        return Err("appinfo file size out of bounds".into());
    }
    reader.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let version = read_u32(reader)?;
    let _universe = read_u32(reader)?;
    let (start, entries_end, keys) = match version {
        V28 => (8, length - 4, None),
        V29 => {
            let mut bytes = [0; 8];
            reader.read_exact(&mut bytes).map_err(io_error)?;
            // Offsets are signed on disk; negative values must never become seeks.
            let offset = i64::from_le_bytes(bytes);
            let offset = u64::try_from(offset)
                .map_err(|_| "negative appinfo key table offset".to_owned())?;
            if offset < 20 || offset > length.saturating_sub(4) {
                return Err("appinfo key table offset out of bounds".into());
            }
            let table_bytes = length - offset;
            if table_bytes - 4 > limits.key_bytes as u64 {
                return Err("appinfo key table exceeds byte limit".into());
            }
            reader.seek(SeekFrom::Start(offset)).map_err(io_error)?;
            let count = read_u32(reader)?;
            let count =
                usize::try_from(count).map_err(|_| "appinfo key count out of bounds".to_owned())?;
            if count > limits.keys || count as u64 > table_bytes - 4 {
                return Err("appinfo key count out of bounds".into());
            }
            let mut buffered = BufReader::new(reader.by_ref().take(table_bytes - 4));
            let mut keys = Vec::with_capacity(count);
            let mut total = 0usize;
            for _ in 0..count {
                let mut value = Vec::new();
                loop {
                    let mut byte = [0];
                    buffered.read_exact(&mut byte).map_err(io_error)?;
                    total = total.checked_add(1).ok_or("appinfo key byte overflow")?;
                    if total > limits.key_bytes {
                        return Err("appinfo key table exceeds byte limit".into());
                    }
                    if byte[0] == 0 {
                        break;
                    }
                    if value.len() >= limits.string {
                        return Err("appinfo key exceeds string limit".into());
                    }
                    value.push(byte[0]);
                }
                keys.push(String::from_utf8(value).map_err(|_| "invalid UTF-8 key")?);
            }
            if total as u64 != table_bytes - 4 {
                return Err("trailing appinfo key table bytes".into());
            }
            (16, offset - 4, Some(keys))
        }
        _ => return Err("unsupported appinfo version".into()),
    };

    let mut names = HashMap::new();
    let mut offset = start;
    loop {
        if offset > entries_end {
            return Err("missing appinfo terminator".into());
        }
        reader.seek(SeekFrom::Start(offset)).map_err(io_error)?;
        let appid = read_u32(reader)?;
        if appid == 0 {
            if offset != entries_end {
                return Err("early appinfo terminator".into());
            }
            break;
        }
        if entries_end - offset < 68 {
            return Err("truncated appinfo entry header".into());
        }
        let size = u64::from(read_u32(reader)?);
        let payload_length = size.checked_sub(60).ok_or("invalid appinfo entry size")?;
        let next = offset
            .checked_add(8)
            .and_then(|offset| offset.checked_add(size))
            .ok_or("appinfo entry offset overflow")?;
        if next > entries_end {
            return Err("appinfo entry crosses terminator".into());
        }
        if targets.contains(&appid) {
            // Duplicate entries must not retain an earlier partial/stale result.
            names.remove(&appid);
            if payload_length <= limits.payload {
                reader
                    .seek(SeekFrom::Start(
                        offset.checked_add(68).ok_or("offset overflow")?,
                    ))
                    .map_err(io_error)?;
                let allocation = usize::try_from(payload_length)
                    .map_err(|_| "appinfo payload allocation overflow".to_owned())?;
                let mut payload = vec![0; allocation];
                reader.read_exact(&mut payload).map_err(io_error)?;
                if let Ok(Some(name)) = decode_name(&payload, keys.as_deref(), language, limits) {
                    names.insert(appid, name);
                }
            }
        }
        offset = next;
    }
    // Detect in-place truncation/growth while using the same handle.
    if reader.seek(SeekFrom::End(0)).map_err(io_error)? != length {
        return Err("appinfo length changed while reading".into());
    }
    Ok(names)
}

#[derive(Debug)]
enum Value<'a> {
    Object(Vec<(&'a str, Value<'a>)>),
    Text(&'a str),
    Scalar,
}

impl<'a> Value<'a> {
    fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(children) => children
                .iter()
                .rev()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value.trim()).filter(|value| !value.is_empty()),
            _ => None,
        }
    }
}

struct Decoder<'a> {
    source: &'a [u8],
    keys: Option<&'a [String]>,
    pos: usize,
    nodes: usize,
    limits: Limits,
}

impl<'a> Decoder<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(length).ok_or("VDF offset overflow")?;
        let value = self
            .source
            .get(self.pos..end)
            .ok_or("truncated binary VDF")?;
        self.pos = end;
        Ok(value)
    }

    fn cstring(&mut self) -> Result<&'a str, String> {
        let remaining = self.source.get(self.pos..).ok_or("invalid VDF offset")?;
        let search_length = remaining.len().min(self.limits.string.saturating_add(1));
        let length = remaining[..search_length]
            .iter()
            .position(|byte| *byte == 0)
            .ok_or("unterminated or oversized VDF string")?;
        let value = self.take(length)?;
        self.take(1)?;
        std::str::from_utf8(value).map_err(|_| "invalid UTF-8 VDF string".into())
    }

    fn key(&mut self) -> Result<&'a str, String> {
        if let Some(keys) = self.keys {
            let index = i32::from_le_bytes(self.take(4)?.try_into().unwrap());
            let index = usize::try_from(index).map_err(|_| "negative VDF key index")?;
            keys.get(index)
                .map(String::as_str)
                .ok_or_else(|| "VDF key index out of bounds".into())
        } else {
            self.cstring()
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value<'a>, String> {
        if depth > self.limits.depth {
            return Err("binary VDF nesting limit exceeded".into());
        }
        let mut children = Vec::new();
        loop {
            let tag = self.take(1)?[0];
            if tag == 8 {
                return Ok(Value::Object(children));
            }
            self.nodes = self.nodes.checked_add(1).ok_or("VDF node overflow")?;
            if self.nodes > self.limits.nodes {
                return Err("binary VDF node limit exceeded".into());
            }
            let key = self.key()?;
            let value = match tag {
                0 => self.object(depth.checked_add(1).ok_or("VDF depth overflow")?)?,
                1 => Value::Text(self.cstring()?),
                2 | 3 | 4 | 6 => {
                    self.take(4)?;
                    Value::Scalar
                }
                // No verified appinfo wide-string layout: fail the whole entry.
                5 => return Err("unsupported binary VDF wide string".into()),
                7 | 10 => {
                    self.take(8)?;
                    Value::Scalar
                }
                _ => return Err("unknown binary VDF tag".into()),
            };
            children.push((key, value));
        }
    }
}

fn decode_name(
    source: &[u8],
    keys: Option<&[String]>,
    language: Option<&str>,
    limits: Limits,
) -> Result<Option<ResolvedName>, String> {
    let mut decoder = Decoder {
        source,
        keys,
        pos: 0,
        nodes: 0,
        limits,
    };
    let root = decoder.object(0)?;
    if decoder.pos != source.len() {
        return Err("trailing binary VDF bytes".into());
    }
    let common = match root.get("appinfo").and_then(|value| value.get("common")) {
        Some(common) => common,
        None => return Ok(None),
    };
    if let Some(language) = language.filter(|value| !value.is_empty()) {
        let localized = common
            .get("name_localized")
            .and_then(|value| value.get(language))
            .and_then(Value::text)
            .or_else(|| {
                let value = common.get("localization")?.get(language)?;
                value.text().or_else(|| value.get("name")?.text())
            });
        if let Some(name) = localized {
            return Ok(Some(ResolvedName {
                name: name.into(),
                language: Some(language.into()),
            }));
        }
    }
    Ok(common
        .get("name")
        .and_then(Value::text)
        .map(|name| ResolvedName {
            name: name.into(),
            language: None,
        }))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const APPID: u32 = 2_303_800;
    const V29_SAMPLE: &[u8] =
        include_bytes!("../tests/fixtures/localized-game-names/v29-real-name-subtree.vdf");
    const V28_SAMPLE: &[u8] =
        include_bytes!("../tests/fixtures/localized-game-names/v28-synthetic-name-subtree.vdf");

    fn targets() -> HashSet<u32> {
        HashSet::from([APPID])
    }

    fn parse(source: &[u8], language: Option<&str>) -> Result<HashMap<u32, ResolvedName>, String> {
        parse_names(&mut Cursor::new(source), &targets(), language)
    }

    fn text(key: &str, value: &str) -> Vec<u8> {
        let mut bytes = vec![1];
        bytes.extend_from_slice(key.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
        bytes
    }

    fn object(key: &str, children: Vec<Vec<u8>>) -> Vec<u8> {
        let mut bytes = vec![0];
        bytes.extend_from_slice(key.as_bytes());
        bytes.push(0);
        bytes.extend(children.into_iter().flatten());
        bytes.push(8);
        bytes
    }

    fn payload(children: Vec<Vec<u8>>) -> Vec<u8> {
        let mut bytes = object("appinfo", vec![object("common", children)]);
        bytes.push(8);
        bytes
    }

    fn container(entries: Vec<(u32, Vec<u8>)>) -> Vec<u8> {
        let mut bytes = V28.to_le_bytes().to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        for (appid, payload) in entries {
            bytes.extend_from_slice(&appid.to_le_bytes());
            bytes.extend_from_slice(&(60 + payload.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&[0; 60]);
            bytes.extend_from_slice(&payload);
        }
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes
    }

    #[test]
    fn both_versions_resolve_real_names_and_preserve_language_contract() {
        for sample in [V29_SAMPLE, V28_SAMPLE] {
            for (language, expected) in [("schinese", "妃神会秘史"), ("japanese", "色々教団生活")]
            {
                assert_eq!(
                    parse(sample, Some(language)).unwrap()[&APPID],
                    ResolvedName {
                        name: expected.into(),
                        language: Some(language.into()),
                    }
                );
            }
            for language in [None, Some("english"), Some("unknown"), Some("")] {
                assert_eq!(
                    parse(sample, language).unwrap()[&APPID],
                    ResolvedName {
                        name: "My Life as a Cult Leader".into(),
                        language: None,
                    }
                );
            }
        }
    }

    #[test]
    fn compatibility_variants_and_whitespace_follow_priority() {
        let variants = vec![
            (text("schinese", " compatibility "), "compatibility"),
            (
                object("schinese", vec![text("name", " object name ")]),
                "object name",
            ),
        ];
        for (variant, expected) in variants {
            for primary in ["", "  ", " primary "] {
                let source = container(vec![(
                    APPID,
                    payload(vec![
                        text("name", " default "),
                        object("name_localized", vec![text("schinese", primary)]),
                        object("localization", vec![variant.clone()]),
                    ]),
                )]);
                let result = parse(&source, Some("schinese")).unwrap();
                assert_eq!(
                    result[&APPID].name,
                    if primary.trim().is_empty() {
                        expected
                    } else {
                        "primary"
                    }
                );
                assert_eq!(result[&APPID].language.as_deref(), Some("schinese"));
                assert_eq!(parse(&source, None).unwrap()[&APPID].name, "default");
            }
        }
        let source = container(vec![(
            APPID,
            payload(vec![
                text("name", " \t "),
                object("name_localized", vec![text("schinese", "\n")]),
                object(
                    "localization",
                    vec![object("schinese", vec![text("name", " ")])],
                ),
            ]),
        )]);
        assert!(parse(&source, Some("schinese")).unwrap().is_empty());
    }

    #[test]
    fn payload_must_fully_validate_before_any_name_is_returned() {
        let good = payload(vec![text("name", "good")]);
        let mut bad = good.clone();
        bad.push(255); // A good name followed by garbage must not survive.
        let source = container(vec![(APPID, bad), (2, good.clone())]);
        let names =
            parse_names(&mut Cursor::new(source), &HashSet::from([APPID, 2]), None).unwrap();
        assert!(!names.contains_key(&APPID));
        assert_eq!(names[&2].name, "good");
        for truncated in 0..good.len() {
            let source = container(vec![(APPID, good[..truncated].to_vec())]);
            assert!(parse(&source, None).unwrap().is_empty());
        }
    }

    #[test]
    fn scalar_types_are_consumed_and_truncation_is_rejected() {
        for (tag, size) in [(2, 4), (3, 4), (4, 4), (6, 4), (7, 8), (10, 8)] {
            let mut scalar = vec![tag, b'x', 0];
            scalar.extend(vec![0; size]);
            let source = container(vec![(
                APPID,
                payload(vec![text("name", "valid"), scalar.clone()]),
            )]);
            assert_eq!(parse(&source, None).unwrap()[&APPID].name, "valid");
            // Place the truncated value at the root tail, so no object-end bytes
            // can be mistaken for bytes belonging to the scalar.
            scalar.pop();
            let mut bad = text("name", "partial");
            bad.extend_from_slice(&scalar);
            assert!(decode_name(&bad, None, None, LIMITS).is_err());
        }
    }

    #[test]
    fn wide_strings_and_unknown_tags_fail_closed() {
        for tag in [5, 9, 11, 255] {
            let mut wide = vec![tag, b'x', 0];
            wide.extend_from_slice(&[0x60, 0x4f, 0, 0]);
            let source = container(vec![(APPID, payload(vec![text("name", "partial"), wide]))]);
            assert!(parse(&source, None).unwrap().is_empty());
        }
    }

    #[test]
    fn invalid_utf8_and_key_indexes_discard_only_the_target_entry() {
        let mut invalid_text = text("name", "x");
        *invalid_text.last_mut().unwrap() = 255;
        let source = container(vec![(APPID, payload(vec![invalid_text]))]);
        assert!(parse(&source, None).unwrap().is_empty());
        for index in [-1i32, 6, i32::MAX] {
            let mut source = V29_SAMPLE.to_vec();
            source[85..89].copy_from_slice(&index.to_le_bytes());
            assert!(parse(&source, Some("schinese")).unwrap().is_empty());
        }
        let mut source = V29_SAMPLE.to_vec();
        source[84] = 255;
        assert!(parse(&source, None).unwrap().is_empty());
    }

    #[test]
    fn duplicate_target_does_not_keep_a_previous_valid_result() {
        let source = container(vec![
            (APPID, payload(vec![text("name", "stale")])),
            (APPID, vec![255]),
        ]);
        assert!(parse(&source, None).unwrap().is_empty());
    }

    #[test]
    fn unsupported_versions_and_container_corruption_fail_atomically() {
        for version in [0u32, 0x0756_4427, 0x0756_4430] {
            let mut source = V28_SAMPLE.to_vec();
            source[..4].copy_from_slice(&version.to_le_bytes());
            assert!(parse(&source, None).is_err());
        }
        for size in [0u32, 59, u32::MAX] {
            let mut source = V29_SAMPLE.to_vec();
            source[20..24].copy_from_slice(&size.to_le_bytes());
            assert!(parse(&source, None).is_err());
        }
        let mut source = container(vec![(APPID, payload(vec![text("name", "valid")]))]);
        source.extend_from_slice(&[1, 2, 3, 4]); // Terminator no longer reaches EOF.
        assert!(parse(&source, None).is_err());
        for sample in [V28_SAMPLE, V29_SAMPLE] {
            for end in 0..sample.len() {
                let result = parse(&sample[..end], Some("schinese"));
                assert!(
                    result.is_err() || result.unwrap().is_empty(),
                    "prefix {end}"
                );
            }
        }
    }

    #[test]
    fn missing_early_and_nonzero_terminators_fail() {
        let mut source = V29_SAMPLE.to_vec();
        let key_offset = i64::from_le_bytes(source[8..16].try_into().unwrap()) as usize;
        source[key_offset - 4..key_offset].copy_from_slice(&1u32.to_le_bytes());
        assert!(parse(&source, None).is_err());
        let mut source = V28_SAMPLE.to_vec();
        source[8..12].fill(0);
        assert!(parse(&source, None).is_err());
        let empty = container(vec![]);
        assert!(parse(&empty, None).unwrap().is_empty());
    }

    #[test]
    fn key_table_offsets_counts_encoding_and_termination_are_validated() {
        for offset in [-1i64, i64::MIN, 0, 19, V29_SAMPLE.len() as i64, i64::MAX] {
            let mut source = V29_SAMPLE.to_vec();
            source[8..16].copy_from_slice(&offset.to_le_bytes());
            assert!(parse(&source, None).is_err());
        }
        let key_offset = i64::from_le_bytes(V29_SAMPLE[8..16].try_into().unwrap()) as usize;
        for count in [0u32, 7, 65_537, u32::MAX] {
            let mut source = V29_SAMPLE.to_vec();
            source[key_offset..key_offset + 4].copy_from_slice(&count.to_le_bytes());
            assert!(parse(&source, None).is_err());
        }
        let mut source = V29_SAMPLE.to_vec();
        source[key_offset + 4] = 255;
        assert!(parse(&source, None).is_err());
        let mut source = V29_SAMPLE.to_vec();
        *source.last_mut().unwrap() = b'x';
        assert!(parse(&source, None).is_err());
        let mut source = V29_SAMPLE.to_vec();
        source.push(0);
        assert!(parse(&source, None).is_err());
    }

    fn limited(limits: Limits) -> Result<HashMap<u32, ResolvedName>, String> {
        parse_with_limits(
            &mut Cursor::new(V29_SAMPLE),
            &targets(),
            Some("schinese"),
            limits,
        )
    }

    #[test]
    fn file_payload_key_count_and_key_byte_limits_accept_equal_reject_above() {
        let file = V29_SAMPLE.len() as u64;
        assert!(limited(Limits { file, ..LIMITS })
            .unwrap()
            .contains_key(&APPID));
        assert!(limited(Limits {
            file: file - 1,
            ..LIMITS
        })
        .is_err());
        let payload = u32::from_le_bytes(V29_SAMPLE[20..24].try_into().unwrap()) as u64 - 60;
        assert!(limited(Limits { payload, ..LIMITS })
            .unwrap()
            .contains_key(&APPID));
        assert!(limited(Limits {
            payload: payload - 1,
            ..LIMITS
        })
        .unwrap()
        .is_empty());
        assert!(limited(Limits { keys: 6, ..LIMITS })
            .unwrap()
            .contains_key(&APPID));
        assert!(limited(Limits { keys: 5, ..LIMITS }).is_err());
        let key_offset = i64::from_le_bytes(V29_SAMPLE[8..16].try_into().unwrap()) as usize;
        let key_bytes = V29_SAMPLE.len() - key_offset - 4;
        assert!(limited(Limits {
            key_bytes,
            ..LIMITS
        })
        .unwrap()
        .contains_key(&APPID));
        assert!(limited(Limits {
            key_bytes: key_bytes - 1,
            ..LIMITS
        })
        .is_err());
    }

    #[test]
    fn string_depth_and_node_limits_accept_equal_reject_above() {
        let string = "My Life as a Cult Leader".len();
        assert!(limited(Limits { string, ..LIMITS })
            .unwrap()
            .contains_key(&APPID));
        assert!(limited(Limits {
            string: string - 1,
            ..LIMITS
        })
        .unwrap()
        .is_empty());
        assert!(limited(Limits { depth: 3, ..LIMITS })
            .unwrap()
            .contains_key(&APPID));
        assert!(limited(Limits { depth: 2, ..LIMITS }).unwrap().is_empty());
        assert!(limited(Limits { nodes: 6, ..LIMITS })
            .unwrap()
            .contains_key(&APPID));
        assert!(limited(Limits { nodes: 5, ..LIMITS }).unwrap().is_empty());
        // Key strings fail the whole container, unlike payload value strings.
        assert!(limited(Limits {
            string: "name_localized".len() - 1,
            ..LIMITS
        })
        .is_err());
    }

    #[test]
    fn production_depth_and_node_limits_are_bounded() {
        let mut at_limit = object("x", vec![]);
        for _ in 1..LIMITS.depth {
            at_limit = object("x", vec![at_limit]);
        }
        at_limit.push(8);
        assert!(decode_name(&at_limit, None, None, LIMITS).is_ok());
        let mut over_limit = object("x", vec![at_limit[..at_limit.len() - 1].to_vec()]);
        over_limit.push(8);
        assert!(decode_name(&over_limit, None, None, LIMITS).is_err());
        let mut many = Vec::with_capacity(LIMITS.nodes * 4 + 1);
        for _ in 0..LIMITS.nodes {
            many.extend_from_slice(&[1, b'x', 0, 0]);
        }
        many.push(8);
        assert!(decode_name(&many, None, None, LIMITS).is_ok());
        many.pop();
        many.extend_from_slice(&[1, b'x', 0, 0, 8]);
        assert!(decode_name(&many, None, None, LIMITS).is_err());
    }

    #[test]
    fn production_string_limit_requires_a_terminator_within_the_limit() {
        let value = "x".repeat(LIMITS.string);
        let source = container(vec![(APPID, payload(vec![text("name", &value)]))]);
        assert_eq!(
            parse(&source, None).unwrap()[&APPID].name.len(),
            LIMITS.string
        );
        let too_large = format!("{value}x");
        let source = container(vec![(APPID, payload(vec![text("name", &too_large)]))]);
        assert!(parse(&source, None).unwrap().is_empty());
    }

    struct CountedReader {
        inner: Cursor<Vec<u8>>,
        read_bytes: usize,
    }

    impl Read for CountedReader {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            let count = self.inner.read(bytes)?;
            self.read_bytes += count;
            Ok(count)
        }
    }

    impl Seek for CountedReader {
        fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(from)
        }
    }

    #[test]
    fn unrequested_payloads_are_skipped_even_when_invalid_or_large() {
        let source = container(vec![
            (2, vec![255; 1024 * 1024]),
            (APPID, payload(vec![text("name", "requested")])),
        ]);
        let mut reader = CountedReader {
            inner: Cursor::new(source),
            read_bytes: 0,
        };
        assert_eq!(
            parse_names(&mut reader, &targets(), None).unwrap()[&APPID].name,
            "requested"
        );
        assert!(reader.read_bytes < 256, "read {} bytes", reader.read_bytes);
    }

    #[test]
    fn in_place_file_growth_invalidates_all_results() {
        struct GrowingReader {
            inner: Cursor<Vec<u8>>,
            ends: usize,
        }
        impl Read for GrowingReader {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                self.inner.read(bytes)
            }
        }
        impl Seek for GrowingReader {
            fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
                let result = self.inner.seek(from)?;
                if from == SeekFrom::End(0) {
                    self.ends += 1;
                    if self.ends > 1 {
                        return Ok(result + 1);
                    }
                }
                Ok(result)
            }
        }
        let mut reader = GrowingReader {
            inner: Cursor::new(V29_SAMPLE.to_vec()),
            ends: 0,
        };
        assert!(parse_names(&mut reader, &targets(), None).is_err());
    }
}
