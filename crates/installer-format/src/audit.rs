//! Bounded, read-only Setup Factory container framing and payload audit.
//! File table layout derived from CybercentreCanada/sfextract
//! 21f5c5595a70e9284fff83476c0e307d5b92f573 (licenses/sfextract-MIT.txt).
use crate::{require, Error, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use caseless::Caseless;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    io::{Cursor, Read, Write},
    time::{Duration, Instant},
};
use unicode_normalization::UnicodeNormalization;

pub const VERSION: &str = "sf9-audit-v3";
pub const SIGNATURE: [u8; 16] = [
    0xe0, 0xe0, 0xe1, 0xe1, 0xe2, 0xe2, 0xe3, 0xe3, 0xe4, 0xe4, 0xe5, 0xe5, 0xe6, 0xe6, 0xe7, 0xe7,
];
pub const MAX_EXE: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_FILE: u64 = MAX_EXE;
pub const MAX_TOTAL: u64 = MAX_EXE;
pub const MAX_METADATA: usize = 16 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 8192;
pub const MAX_DICTIONARY: u64 = 128 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;
const MAX_MANIFEST_DEPTH: usize = 96;

pub struct Reader<'a> {
    data: &'a [u8],
    pub pos: usize,
    end: usize,
}
impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            end: data.len(),
        }
    }
    pub fn at(data: &'a [u8], start: usize, end: usize) -> Result<Self> {
        require(start <= end && end <= data.len(), "invalid_range")?;
        Ok(Self {
            data,
            pos: start,
            end,
        })
    }
    pub fn read(&mut self, size: usize) -> Result<&'a [u8]> {
        require(
            size <= self.end.saturating_sub(self.pos),
            "truncated_record",
        )?;
        let start = self.pos;
        self.pos += size;
        Ok(&self.data[start..self.pos])
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.read(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.read(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.read(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.read(8)?.try_into().unwrap()))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.read(8)?.try_into().unwrap()))
    }
    pub fn string(&mut self) -> Result<String> {
        let mut length = self.u8()? as usize;
        let mut wide = false;
        if length == 0xff {
            length = self.u16()? as usize;
            if length == 0xfffe {
                wide = true;
                length = self.u8()? as usize;
                if length == 0xff {
                    length = self.u16()? as usize;
                }
            }
            if length == 0xffff {
                length = self.u32()? as usize;
                require(length != u32::MAX as usize, "unsupported_string_length")?;
            }
        }
        let bytes = length
            .checked_mul(if wide { 2 } else { 1 })
            .ok_or_else(|| Error("string_size_limit".into()))?;
        require(bytes <= MAX_METADATA, "string_size_limit")?;
        let data = self.read(bytes)?;
        let value = if wide {
            let units: Vec<u16> = data
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            String::from_utf16(&units).map_err(|_| Error("invalid_utf8".into()))?
        } else {
            std::str::from_utf8(data)
                .map_err(|_| Error("invalid_utf8".into()))?
                .to_owned()
        };
        let value = value.trim_end_matches('\0').to_owned();
        require(!value.contains('\0'), "embedded_nul")?;
        Ok(value)
    }
}
fn sha(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
pub fn evidence(data: &[u8], offset: usize) -> Value {
    json!({"offset":offset,"size":data.len(),"sha256":sha(data)})
}
fn opaque(r: &mut Reader<'_>, n: usize) -> Result<Value> {
    let offset = r.pos;
    let bytes = r.read(n)?;
    let mut value = evidence(bytes, offset);
    value["base64"] = json!(STANDARD.encode(bytes));
    Ok(value)
}
fn at_u32(data: &[u8], at: usize) -> Result<u32> {
    Reader::at(data, at, data.len())?.u32()
}

pub fn safe_target(directory: &str, basename: &str) -> Result<String> {
    require(
        directory == "%AppFolder%" || directory.starts_with("%AppFolder%\\"),
        "unsupported_target_variable",
    )?;
    let relative = directory[11..].trim_start_matches('\\');
    let name = if relative.is_empty() {
        basename.to_owned()
    } else {
        format!("{relative}\\{basename}")
    };
    require(
        !name.is_empty()
            && name.chars().count() <= 1024
            && !name.contains(['/', ':', '%'])
            && name.chars().all(|c| c as u32 >= 32),
        "unsafe_target_path",
    )?;
    for part in name.split('\\') {
        require(
            !["", ".", ".."].contains(&part)
                && !part.ends_with(['.', ' '])
                && !part.contains(['<', '>', '"', '|', '?', '*']),
            "unsafe_target_path",
        )?;
        let device = part.split('.').next().unwrap().to_uppercase();
        let numbered = (device.starts_with("COM") || device.starts_with("LPT"))
            && device.chars().count() == 4
            && device
                .chars()
                .nth(3)
                .is_some_and(|c| "0123456789¹²³".contains(c));
        require(
            !["CON", "PRN", "AUX", "NUL"].contains(&device.as_str()) && !numbered,
            "unsafe_target_path",
        )?;
        require(
            !part
                .chars()
                .default_case_fold()
                .collect::<String>()
                .starts_with(".steam-patch-"),
            "reserved_target_path",
        )?;
    }
    require(!basename.contains('\\'), "unsafe_basename")?;
    Ok(name.replace('\\', "/"))
}
fn target_key(s: &str) -> String {
    s.nfc().default_case_fold().collect()
}
pub fn validate_targets(records: &[Value]) -> Result<()> {
    let mut names = HashMap::new();
    let mut prefixes = HashMap::new();
    for record in records {
        let name = record["destination"]
            .as_str()
            .ok_or_else(|| Error("unsafe_target_path".into()))?;
        let key = target_key(name);
        require(
            !names.contains_key(&key),
            "duplicate_or_unicode_case_collision",
        )?;
        names.insert(key, name);
        let parts: Vec<_> = name.split('/').collect();
        for i in 1..=parts.len() {
            let prefix = parts[..i].join("/");
            let key = target_key(&prefix);
            if let Some(previous) = prefixes.get(&key) {
                require(previous == &prefix, "duplicate_or_unicode_case_collision")?;
            } else {
                prefixes.insert(key, prefix);
            }
        }
    }
    for key in names.keys() {
        let parts: Vec<_> = key.split('/').collect();
        for i in 1..parts.len() {
            require(
                !names.contains_key(&parts[..i].join("/")),
                "file_directory_collision",
            )?;
        }
    }
    Ok(())
}
pub fn file_table(metadata: &[u8], version: &str) -> Result<(Vec<Value>, Value)> {
    require(
        ["9.5", "9.6", "9.7"].contains(&version),
        "unsupported_installer_version",
    )?;
    let marker = b"CSetupFileData";
    let found: Vec<_> = metadata
        .windows(marker.len())
        .enumerate()
        .filter(|(_, b)| *b == marker)
        .map(|(i, _)| i)
        .collect();
    require(found.len() == 1 && found[0] >= 8, "ambiguous_file_table")?;
    let start = found[0] - 8;
    let mut r = Reader::at(metadata, start, metadata.len())?;
    let count = r.u16()? as usize;
    require(count > 0 && count <= MAX_ENTRIES, "file_count_limit")?;
    require(
        r.read(4)? == [0xff, 0xff, 1, 0] && r.u16()? == 14,
        "unsupported_file_table",
    )?;
    require(r.read(14)? == marker, "unsupported_file_table")?;
    let header = r.read(5)?;
    let mut records = Vec::with_capacity(count);
    let mut total = 0u64;
    for index in 0..count {
        let record_start = r.pos;
        let source = r.string()?;
        let basename = r.string()?;
        let source_directory = r.string()?;
        let suffix = r.string()?;
        let runtime_folder = r.string()?;
        let description = r.string()?;
        let mut unknown = Vec::new();
        unknown.push(opaque(&mut r, 2)?);
        let unpacked = r.i64()?;
        require(
            unpacked >= 0 && unpacked as u64 <= MAX_FILE,
            "file_size_limit",
        )?;
        total += unpacked as u64;
        require(total <= MAX_TOTAL, "expanded_size_limit")?;
        let attributes = r.u8()?;
        unknown.push(opaque(&mut r, 4)?);
        let creation = r.i64()?;
        unknown.push(opaque(&mut r, 16)?);
        let modified = r.i64()?;
        unknown.push(opaque(&mut r, 25)?);
        let directory = r.string()?;
        unknown.push(opaque(&mut r, 11)?);
        let shortcuts = (0..5).map(|_| r.string()).collect::<Result<Vec<_>>>()?;
        unknown.push(opaque(&mut r, 1)?);
        let icon = r.string()?;
        unknown.push(opaque(&mut r, 8)?);
        let font = r.string()?;
        unknown.push(opaque(&mut r, if version == "9.7" { 4 } else { 3 })?);
        let compressed = r.u8()?;
        let original = r.u8()?;
        let forced = r.u8()?;
        require(compressed <= 1, "invalid_file_flags")?;
        unknown.push(opaque(&mut r, 10)?);
        let size = r.u16()? as usize * 2;
        unknown.push(opaque(&mut r, size)?);
        let condition = r.string()?;
        unknown.push(opaque(&mut r, 2)?);
        let install_type = r.string()?;
        let extra = r.string()?;
        let packages_count = r.u16()? as usize;
        require(packages_count <= 256, "package_count_limit")?;
        let packages = (0..packages_count)
            .map(|_| r.string())
            .collect::<Result<Vec<_>>>()?;
        let notes = r.string()?;
        let packed = r.i64()?;
        let crc = r.u32()?;
        require(
            packed >= 0 && packed as u64 <= MAX_EXE,
            "invalid_packed_size",
        )?;
        let record_end = r.pos;
        let framing = if index + 1 < count {
            opaque(&mut r, 8)?
        } else {
            Value::Null
        };
        records.push(json!({"index":index,"destination":safe_target(&directory,&basename)?,"source":source,"source_directory":source_directory,"suffix":suffix,"runtime_folder":runtime_folder,"description":description,"unpacked_size":unpacked,"packed_size":packed,"crc32":crc,"compressed":compressed!=0,"attributes":attributes,"use_original_attributes":original,"forced_attributes":forced,"creation_time":creation,"modified_time":modified,"shortcuts":shortcuts,"icon":icon,"font":font,"condition":condition,"install_type":install_type,"extra":extra,"packages":packages,"notes":notes,"serialized_record":evidence(&metadata[record_start..record_end],record_start),"unparsed_inter_record_framing":framing,"unreviewed_fields":unknown}));
    }
    validate_targets(&records)?;
    Ok((
        records,
        json!({"start":start,"end":r.pos,"end_basis":"last_known_crc_end_not_serialized_object_end","header_base64":STANDARD.encode(header),"unparsed_regions":[evidence(&metadata[..start],0),evidence(&metadata[r.pos..],r.pos)]}),
    ))
}

#[derive(Clone, Copy)]
struct Section {
    rva: u64,
    raw_offset: usize,
    raw_size: usize,
}
fn resource_resolve(sections: &[Section], rva: u32, size: u32) -> Result<usize> {
    let rva = rva as u64;
    let size = size as u64;
    let matches: Vec<_> = sections
        .iter()
        .filter(|s| s.rva <= rva && rva + size <= s.rva + s.raw_size as u64)
        .map(|s| s.raw_offset + (rva - s.rva) as usize)
        .collect();
    require(matches.len() == 1, "invalid_resource_rva")?;
    Ok(matches[0])
}
fn resource_directory(
    data: &[u8],
    resource: usize,
    size: usize,
    relative: u32,
) -> Result<Vec<(u32, u32)>> {
    let mut r = Reader::at(data, resource + relative as usize, resource + size)?;
    r.read(12)?;
    let count = r.u16()? as usize + r.u16()? as usize;
    require(count <= 256, "resource_entry_limit")?;
    (0..count).map(|_| Ok((r.u32()?, r.u32()?))).collect()
}
fn xml_char(c: char) -> bool {
    matches!(c as u32, 9 | 10 | 13 | 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff)
}
fn xml_name(name: &[u8]) -> bool {
    fn start(c: char) -> bool {
        matches!(c as u32, 0x3a | 0x41..=0x5a | 0x5f | 0x61..=0x7a | 0xc0..=0xd6 | 0xd8..=0xf6 | 0xf8..=0x2ff | 0x370..=0x37d | 0x37f..=0x1fff | 0x200c..=0x200d | 0x2070..=0x218f | 0x2c00..=0x2fef | 0x3001..=0xd7ff | 0xf900..=0xfdcf | 0xfdf0..=0xfffd | 0x10000..=0xeffff)
    }
    let Ok(name) = std::str::from_utf8(name) else {
        return false;
    };
    if name
        .split(':')
        .any(|part| !part.chars().next().is_some_and(start))
        || name.matches(':').count() > 1
    {
        return false;
    }
    let mut chars = name.chars();
    chars.next().is_some_and(start) && chars.all(|c| start(c) || matches!(c as u32, 0x2d | 0x2e | 0x30..=0x39 | 0xb7 | 0x300..=0x36f | 0x203f..=0x2040))
}
fn normalized_namespace(raw: &[u8]) -> Result<Cow<'_, str>> {
    let text = std::str::from_utf8(raw).map_err(|_| Error("invalid_manifest_xml".into()))?;
    // Namespace values are XML attributes: literal whitespace normalizes before
    // entity expansion, so numeric whitespace references remain distinct.
    if text.contains(['\r', '\n', '\t']) {
        let normalized = text.replace("\r\n", "\n").replace(['\r', '\n', '\t'], " ");
        Ok(Cow::Owned(
            quick_xml::escape::unescape(&normalized)
                .map_err(|_| Error("invalid_manifest_xml".into()))?
                .into_owned(),
        ))
    } else {
        quick_xml::escape::unescape(text).map_err(|_| Error("invalid_manifest_xml".into()))
    }
}
fn manifest_version(manifest: &[u8]) -> Result<String> {
    use quick_xml::events::Event;
    let text = std::str::from_utf8(manifest).map_err(|_| Error("invalid_manifest_xml".into()))?;
    require(text.chars().all(xml_char), "invalid_manifest_xml")?;
    let upper = text.to_uppercase();
    require(
        !upper.contains("<!DOCTYPE") && !upper.contains("<!ENTITY"),
        "unsafe_manifest_xml",
    )?;
    let mut reader = quick_xml::NsReader::from_str(text);
    reader.config_mut().expand_empty_elements = true;
    reader.config_mut().check_comments = true;
    let mut depth = 0usize;
    let mut roots = 0;
    let mut versions = Vec::new();
    let mut descriptions = Vec::new();
    let mut description: Option<String> = None;
    let mut description_text = true;
    loop {
        let (namespace, event) = reader
            .read_resolved_event()
            .map_err(|_| Error("invalid_manifest_xml".into()))?;
        require(
            !matches!(namespace, quick_xml::name::ResolveResult::Unknown(_)),
            "invalid_manifest_xml",
        )?;
        match event {
            Event::Start(e) => {
                // A conservative framing limit bounds the namespace stack even
                // when a hostile manifest repeats very small nested elements.
                require(depth < MAX_MANIFEST_DEPTH, "manifest_depth_limit")?;
                require(xml_name(e.name().as_ref()), "invalid_manifest_xml")?;
                let mut expanded_attributes = HashSet::new();
                let mut uri_ids = HashMap::new();
                let mut uri_pointers = HashMap::new();
                for attr in e.attributes() {
                    let attr = attr.map_err(|_| Error("invalid_manifest_xml".into()))?;
                    require(xml_name(attr.key.as_ref()), "invalid_manifest_xml")?;
                    let (namespace, local) = reader.resolve_attribute(attr.key);
                    let uri_id = match namespace {
                        quick_xml::name::ResolveResult::Bound(uri) => {
                            let raw = uri.into_inner();
                            let pointer = (raw.as_ptr() as usize, raw.len());
                            if let Some(id) = uri_pointers.get(&pointer) {
                                *id
                            } else {
                                let normalized = normalized_namespace(raw)?;
                                let next_id = uri_ids.len() + 1;
                                let id = *uri_ids.entry(normalized).or_insert(next_id);
                                uri_pointers.insert(pointer, id);
                                id
                            }
                        }
                        quick_xml::name::ResolveResult::Unbound => 0usize,
                        quick_xml::name::ResolveResult::Unknown(_) => {
                            return Err(Error("invalid_manifest_xml".into()))
                        }
                    };
                    // XML forbids duplicate expanded names even when their raw
                    // prefixes differ but resolve to the same namespace URI.
                    // Store IDs and borrowed local names, never one URI clone per
                    // attribute. Pointer memoization hashes each binding once.
                    require(
                        expanded_attributes.insert((uri_id, local.into_inner())),
                        "invalid_manifest_xml",
                    )?;
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(|_| Error("invalid_manifest_xml".into()))?;
                    require(value.chars().all(xml_char), "invalid_manifest_xml")?;
                }
                if depth == 0 {
                    roots += 1;
                    require(roots == 1, "invalid_manifest_xml")?;
                }
                if depth == 1 {
                    if e.local_name().as_ref() == b"assemblyIdentity" {
                        let mut version = String::new();
                        for attr in e.attributes() {
                            let attr = attr.map_err(|_| Error("invalid_manifest_xml".into()))?;
                            if attr.key.as_ref() == b"version" {
                                version = attr
                                    .decode_and_unescape_value(reader.decoder())
                                    .map_err(|_| Error("invalid_manifest_xml".into()))?
                                    .into_owned();
                            }
                        }
                        versions.push(version);
                    }
                    if e.local_name().as_ref() == b"description" {
                        description = Some(String::new());
                        description_text = true;
                    }
                } else if depth >= 2 && description.is_some() {
                    description_text = false;
                }
                depth += 1;
            }
            Event::End(_) => {
                require(depth > 0, "invalid_manifest_xml")?;
                if depth == 2 {
                    if let Some(d) = description.take() {
                        descriptions.push(d);
                    }
                }
                depth -= 1;
            }
            Event::Text(e) => {
                let text = e
                    .xml_content()
                    .map_err(|_| Error("invalid_manifest_xml".into()))?;
                if depth == 0 {
                    require(text.trim().is_empty(), "invalid_manifest_xml")?;
                }
                if depth == 2 && description_text {
                    if let Some(d) = description.as_mut() {
                        d.push_str(&text);
                    }
                }
            }
            Event::CData(e) => {
                let text = e
                    .xml_content()
                    .map_err(|_| Error("invalid_manifest_xml".into()))?;
                require(
                    depth > 0 && text.chars().all(xml_char),
                    "invalid_manifest_xml",
                )?;
                if depth == 2 && description_text {
                    if let Some(d) = description.as_mut() {
                        d.push_str(&text);
                    }
                }
            }
            Event::GeneralRef(e) => {
                let name = e
                    .decode()
                    .map_err(|_| Error("invalid_manifest_xml".into()))?;
                let escaped = format!("&{name};");
                let text = quick_xml::escape::unescape(&escaped)
                    .map_err(|_| Error("invalid_manifest_xml".into()))?;
                require(
                    depth > 0 && text.chars().all(xml_char),
                    "invalid_manifest_xml",
                )?;
                if depth == 2 && description_text {
                    if let Some(d) = description.as_mut() {
                        d.push_str(&text);
                    }
                }
            }
            Event::DocType(_) => return Err(Error("unsafe_manifest_xml".into())),
            Event::Eof => {
                require(depth == 0 && roots == 1, "invalid_manifest_xml")?;
                break;
            }
            _ => {}
        }
    }
    let version = if versions.len() == 1 {
        versions.remove(0)
    } else {
        String::new()
    };
    let parts: Vec<_> = version.split('.').collect();
    require(
        parts.len() == 4
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            && parts[0] == "9"
            && ["5", "6", "7"].contains(&parts[1]),
        "unsupported_installer_version",
    )?;
    require(
        descriptions
            .iter()
            .any(|d| ["Setup Factory 9 Run-time", "Setup Factory Run-time"].contains(&d.as_str())),
        "unsupported_installer_brand",
    )?;
    Ok(version)
}

pub fn pe_overlay(data: &[u8]) -> Result<(usize, usize, Value, Value)> {
    require(data.len() >= 64 && &data[..2] == b"MZ", "not_pe")?;
    let pe = at_u32(data, 60)? as usize;
    let mut h = Reader::at(data, pe, data.len())?;
    require(h.read(4)? == b"PE\0\0", "not_pe")?;
    h.read(2)?;
    let count = h.u16()? as usize;
    h.read(12)?;
    let optional_size = h.u16()? as usize;
    h.read(2)?;
    require(
        (1..=96).contains(&count) && optional_size >= 96,
        "invalid_pe_header",
    )?;
    let optional_start = h.pos;
    let mut optional = Reader::at(data, optional_start, optional_start + optional_size)?;
    let magic = optional.u16()?;
    require(
        [0x10b, 0x20b].contains(&magic),
        "invalid_pe_optional_header",
    )?;
    let directory_offset = if magic == 0x10b { 96 } else { 112 };
    require(
        optional_size >= directory_offset + 40,
        "invalid_pe_directories",
    )?;
    let number_dirs = at_u32(data, optional_start + directory_offset - 4)?;
    require((5..=16).contains(&number_dirs), "invalid_pe_directories")?;
    let size_headers = at_u32(data, optional_start + 60)? as usize;
    let mut table = Reader::at(data, optional_start + optional_size, data.len())?;
    require(
        size_headers >= table.pos + count * 40 && size_headers <= data.len(),
        "invalid_pe_headers_size",
    )?;
    let mut sections = Vec::with_capacity(count);
    for _ in 0..count {
        let record = table.read(40)?;
        let mut r = Reader::at(record, 8, 40)?;
        r.u32()?;
        let rva = r.u32()? as u64;
        let raw_size = r.u32()? as usize;
        let raw_offset = r.u32()? as usize;
        require(
            raw_offset as u64 + raw_size as u64 <= data.len() as u64,
            "truncated_pe_section",
        )?;
        require(
            raw_size == 0 || raw_offset >= size_headers,
            "overlapping_pe_headers",
        )?;
        sections.push(Section {
            rva,
            raw_size,
            raw_offset,
        });
    }
    let mut intervals: Vec<_> = sections
        .iter()
        .filter(|s| s.raw_size > 0)
        .map(|s| (s.raw_offset, s.raw_offset + s.raw_size))
        .collect();
    intervals.sort();
    require(
        intervals.windows(2).all(|a| a[0].1 <= a[1].0),
        "overlapping_pe_sections",
    )?;
    let overlay = sections
        .iter()
        .map(|s| s.raw_offset + s.raw_size)
        .chain([size_headers])
        .max()
        .unwrap();
    let resource_rva = at_u32(data, optional_start + directory_offset + 16)?;
    let resource_size = at_u32(data, optional_start + directory_offset + 20)?;
    let cert_offset = at_u32(data, optional_start + directory_offset + 32)? as usize;
    let cert_size = at_u32(data, optional_start + directory_offset + 36)? as usize;
    let mut certificate = Value::Null;
    let mut overlay_end = data.len();
    if cert_offset != 0 || cert_size != 0 {
        require(
            cert_offset >= overlay
                && cert_offset.is_multiple_of(8)
                && (8..=MAX_METADATA).contains(&cert_size)
                && cert_offset as u64 + cert_size as u64 == data.len() as u64,
            "invalid_pe_certificate_range",
        )?;
        let mut cert = Reader::at(data, cert_offset, data.len())?;
        let length = cert.u32()? as usize;
        let revision = cert.u16()?;
        let kind = cert.u16()?;
        require(
            length >= 8
                && length <= cert_size
                && length.div_ceil(8) * 8 == cert_size
                && revision == 0x200
                && kind == 2,
            "unsupported_pe_certificate",
        )?;
        cert.read(length - 8)?;
        require(
            cert.read(cert_size - length)?.iter().all(|b| *b == 0),
            "invalid_certificate_padding",
        )?;
        certificate = evidence(&data[cert_offset..], cert_offset);
        certificate["signature_verified"] = json!(false);
        overlay_end = cert_offset;
    }
    require(
        resource_size > 0 && resource_size as usize <= MAX_METADATA,
        "invalid_resource_size",
    )?;
    let resource = resource_resolve(&sections, resource_rva, resource_size)?;
    let mut manifests = Vec::new();
    for (kind, node) in resource_directory(data, resource, resource_size as usize, 0)? {
        if kind != 24 {
            continue;
        }
        require(node & 0x80000000 != 0, "invalid_manifest_directory")?;
        for (_, language) in
            resource_directory(data, resource, resource_size as usize, node & 0x7fffffff)?
        {
            require(language & 0x80000000 != 0, "invalid_manifest_directory")?;
            for (_, leaf) in resource_directory(
                data,
                resource,
                resource_size as usize,
                language & 0x7fffffff,
            )? {
                require(leaf & 0x80000000 == 0, "invalid_manifest_leaf")?;
                let mut entry = Reader::at(
                    data,
                    resource + leaf as usize,
                    resource + resource_size as usize,
                )?;
                let rva = entry.u32()?;
                let size = entry.u32()?;
                require(
                    size > 0 && size as usize <= MAX_METADATA,
                    "manifest_size_limit",
                )?;
                let offset = resource_resolve(&sections, rva, size)?;
                manifests.push(&data[offset..offset + size as usize]);
                require(manifests.len() <= 1, "ambiguous_or_missing_manifest")?;
            }
        }
    }
    require(manifests.len() == 1, "ambiguous_or_missing_manifest")?;
    let manifest = manifests[0];
    let version = manifest_version(manifest)?;
    require(
        data.get(overlay..overlay + 16) == Some(SIGNATURE.as_slice()),
        "unsupported_overlay",
    )?;
    let mut info = evidence(manifest, 0);
    info["version"] = json!(version);
    Ok((overlay, overlay_end, info, certificate))
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0; 256];
    let mut i = 0;
    while i < 256 {
        let mut v = i as u32;
        let mut bit = 0;
        while bit < 8 {
            v = if v & 1 != 0 {
                0xedb88320 ^ (v >> 1)
            } else {
                v >> 1
            };
            bit += 1;
        }
        table[i] = v;
        i += 1;
    }
    table
}
const CRC_TABLE: [u32; 256] = crc_table();
fn crc_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for b in bytes {
        crc = CRC_TABLE[((crc ^ *b as u32) & 255) as usize] ^ (crc >> 8);
    }
    crc
}
struct Decoder {
    total: u64,
    deadline: Instant,
}
impl Decoder {
    fn new() -> Self {
        Self {
            total: 0,
            deadline: Instant::now() + Duration::from_secs(180),
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn decode(
        &mut self,
        data: &[u8],
        offset: usize,
        packed: u64,
        expected: Option<u64>,
        crc: Option<u32>,
        retain: bool,
        compressed: bool,
        mut sink: Option<&mut dyn Write>,
    ) -> Result<(Value, Vec<u8>)> {
        let limit = if retain {
            MAX_METADATA as u64
        } else {
            MAX_FILE
        };
        require(
            offset <= data.len() && packed <= data.len().saturating_sub(offset) as u64,
            "truncated_payload",
        )?;
        if let Some(expected) = expected {
            require(
                expected <= limit && self.total + expected <= MAX_TOTAL,
                "expanded_size_limit",
            )?;
        }
        let bytes = &data[offset..offset + packed as usize];
        let mut expected = expected;
        let mut input = Cursor::new(bytes);
        let mut lzma = None;
        if compressed {
            require(packed >= 10, "truncated_lzma2_header")?;
            let prop = bytes[0];
            require(prop <= 40, "unsupported_compression")?;
            let dict = if prop < 40 {
                (2u64 | (prop as u64 & 1)) << (prop as u32 / 2 + 11)
            } else {
                u32::MAX as u64
            };
            require(dict <= MAX_DICTIONARY, "dictionary_limit")?;
            let header = u64::from_le_bytes(bytes[1..9].try_into().unwrap());
            require(header <= limit, "expanded_size_limit")?;
            if let Some(size) = expected {
                require(header == size, "lzma2_size_mismatch")?;
            }
            expected = Some(header);
            require(self.total + header <= MAX_TOTAL, "expanded_size_limit")?;
            input.set_position(9);
            lzma = Some(
                lzma_rust2::Lzma2Reader::new_mem_limit(
                    input,
                    dict as u32,
                    (MAX_DICTIONARY / 1024 + 1024) as u32,
                    None,
                )
                .map_err(|_| Error("dictionary_limit".into()))?,
            );
            input = Cursor::new(&[]);
        }
        let mut digest = Sha256::new();
        let mut checksum = u32::MAX;
        let mut size = 0u64;
        let mut parts = Vec::new();
        let mut chunk = [0u8; CHUNK];
        loop {
            require(Instant::now() < self.deadline, "audit_timeout")?;
            // A one-byte probe beyond the advertised size bounds decoder output,
            // while still requiring an actual end-of-stream marker.
            let available = (limit - size + 1)
                .min(expected.map_or(u64::MAX, |n| n - size + 1))
                .min(CHUNK as u64) as usize;
            let count = if let Some(decoder) = lzma.as_mut() {
                decoder.read(&mut chunk[..available]).map_err(|e| {
                    Error(
                        if e.kind() == std::io::ErrorKind::UnexpectedEof {
                            "truncated_lzma2_stream"
                        } else {
                            "invalid_lzma2_stream"
                        }
                        .into(),
                    )
                })?
            } else {
                input.read(&mut chunk[..available])?
            };
            if count == 0 {
                break;
            }
            size += count as u64;
            self.total += count as u64;
            require(
                size <= limit && self.total <= MAX_TOTAL,
                "expanded_size_limit",
            )?;
            require(expected.is_none_or(|n| size <= n), "payload_size_mismatch")?;
            digest.update(&chunk[..count]);
            checksum = crc_update(checksum, &chunk[..count]);
            if retain {
                parts.extend_from_slice(&chunk[..count]);
            }
            if let Some(writer) = sink.as_mut() {
                writer.write_all(&chunk[..count])?;
            }
        }
        if let Some(decoder) = lzma {
            require(
                decoder.into_inner().position() == packed,
                "trailing_compressed_data",
            )?;
        }
        require(expected.is_none_or(|n| size == n), "payload_size_mismatch")?;
        let checksum = !checksum;
        require(
            crc.is_none_or(|crc| checksum == crc),
            "payload_crc_mismatch",
        )?;
        Ok((
            json!({"size":size,"crc32":checksum,"sha256":format!("{:x}",digest.finalize())}),
            parts,
        ))
    }
}
/// Stream a validated member to a caller-owned sink. The caller controls output
/// lifetime and discards partial bytes on any error; this module opens no files.
pub fn decode_payload(
    data: &[u8],
    offset: usize,
    packed: u64,
    unpacked: u64,
    crc: u32,
    compressed: bool,
    sink: Option<&mut dyn Write>,
) -> Result<Value> {
    Ok(Decoder::new()
        .decode(
            data,
            offset,
            packed,
            Some(unpacked),
            Some(crc),
            false,
            compressed,
            sink,
        )?
        .0)
}

/// Python json.dumps(ensure_ascii=True, sort_keys=True, separators=(",", ":"))
/// encoding used by the frozen worker behavior binding.
fn python_json(value: &Value) -> Result<String> {
    fn encode(value: &Value, output: &mut String) -> Result<()> {
        match value {
            Value::Null => output.push_str("null"),
            Value::Bool(b) => output.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) => {
                require(n.is_i64() || n.is_u64(), "invalid_canonical_value")?;
                output.push_str(&n.to_string());
            }
            Value::String(s) => {
                output.push('"');
                for c in s.chars() {
                    match c {
                        '"' => output.push_str("\\\""),
                        '\\' => output.push_str("\\\\"),
                        '\n' => output.push_str("\\n"),
                        '\r' => output.push_str("\\r"),
                        '\t' => output.push_str("\\t"),
                        '\u{8}' => output.push_str("\\b"),
                        '\u{c}' => output.push_str("\\f"),
                        c if (c as u32) < 32 || (c as u32) > 126 => {
                            for unit in c.encode_utf16(&mut [0; 2]) {
                                output.push_str(&format!("\\u{unit:04x}"));
                            }
                        }
                        c => output.push(c),
                    }
                }
                output.push('"');
            }
            Value::Array(a) => {
                output.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        output.push(',');
                    }
                    encode(v, output)?;
                }
                output.push(']');
            }
            Value::Object(o) => {
                output.push('{');
                let mut entries: Vec<_> = o.iter().collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        output.push(',');
                    }
                    encode(&Value::String((*k).clone()), output)?;
                    output.push(':');
                    encode(v, output)?;
                }
                output.push('}');
            }
        }
        Ok(())
    }
    let mut encoded = String::new();
    encode(value, &mut encoded)?;
    Ok(encoded)
}

pub fn behavior_digest(value: &Value) -> Result<String> {
    Ok(sha(python_json(value)?.as_bytes()))
}
pub fn audit(data: &[u8], behavior: bool) -> Result<Value> {
    require(
        !data.is_empty() && data.len() as u64 <= MAX_EXE,
        "exe_size_limit",
    )?;
    let (overlay, overlay_end, manifest, certificate) = pe_overlay(data)?;
    let version = manifest["version"]
        .as_str()
        .unwrap()
        .split('.')
        .take(2)
        .collect::<Vec<_>>()
        .join(".");
    let mut r = Reader::at(data, overlay, overlay_end)?;
    require(r.read(16)? == SIGNATURE, "unsupported_overlay")?;
    let overlay_header = r.read(10)?;
    let mut decoder = Decoder::new();
    let mut diagnostics = Vec::new();
    for name in ["irsetup.exe", "lua5.1.dll"] {
        let size = r.i64()?;
        require(
            size > 0 && size as u64 <= MAX_FILE && decoder.total + size as u64 <= MAX_TOTAL,
            "runtime_size_limit",
        )?;
        let offset = r.pos;
        let mut info = decoder
            .decode(
                data,
                offset,
                size as u64,
                Some(size as u64),
                None,
                false,
                false,
                None,
            )?
            .0;
        r.read(size as usize)?;
        info["name"] = json!(name);
        info["offset"] = json!(offset);
        info["packed_size"] = json!(size);
        info["hashed_representation"] = json!("stored_blob");
        diagnostics.push(info);
    }
    require(r.u32()? == 3, "unsupported_metadata_records")?;
    let mut metadata = Vec::new();
    for name in ["irsetup.dat", "IRIMG1.BMP", "IRIMG2.BMP"] {
        let raw_name = r.read(264)?;
        require(
            raw_name[..name.len()] == *name.as_bytes()
                && raw_name[name.len()..].iter().all(|b| *b == 0),
            "unsupported_metadata_name",
        )?;
        let packed = r.i64()?;
        let crc = r.u32()?;
        let unknown = r.read(4)?;
        let offset = r.pos;
        require(
            packed >= 0 && packed as u64 <= r.end.saturating_sub(r.pos) as u64,
            "truncated_payload",
        )?;
        let (mut info, content) = decoder.decode(
            data,
            offset,
            packed as u64,
            None,
            Some(crc),
            name == "irsetup.dat",
            true,
            None,
        )?;
        r.read(packed as usize)?;
        info["name"] = json!(name);
        info["offset"] = json!(offset);
        info["packed_size"] = json!(packed);
        info["unknown_header_base64"] = json!(STANDARD.encode(unknown));
        diagnostics.push(info);
        if name == "irsetup.dat" {
            metadata = content;
        }
    }
    let inventory = if behavior {
        crate::graph::decode(&metadata, &version)?
    } else {
        crate::graph::inventory(&metadata)?
    };
    let (mut records, table) = file_table(&metadata, &version)?;
    let packed_total: u64 = records
        .iter()
        .map(|v| v["packed_size"].as_u64().unwrap())
        .sum();
    let payload_end = r.pos as u64 + packed_total;
    let padding = if certificate.is_null() {
        0
    } else {
        (8 - payload_end % 8) % 8
    };
    require(
        payload_end + padding == r.end as u64,
        "overlay_size_or_tail_mismatch",
    )?;
    let unpacked_total: u64 = records
        .iter()
        .map(|v| v["unpacked_size"].as_u64().unwrap())
        .sum();
    require(
        decoder.total + unpacked_total <= MAX_TOTAL,
        "expanded_size_limit",
    )?;
    for record in &mut records {
        let packed = record["packed_size"].as_u64().unwrap();
        record["payload_offset"] = json!(r.pos);
        let (info, _) = decoder.decode(
            data,
            r.pos,
            packed,
            Some(record["unpacked_size"].as_u64().unwrap()),
            Some(record["crc32"].as_u64().unwrap() as u32),
            false,
            record["compressed"].as_bool().unwrap(),
            None,
        )?;
        record["payload"] = info;
        r.read(packed as usize)?;
    }
    require(
        r.read(padding as usize)?.iter().all(|b| *b == 0) && r.pos == overlay_end,
        "trailing_overlay_data",
    )?;
    let mut report = json!({"schema_version":1,"auditor":VERSION,"exe":evidence(data,0),"installer":format!("Setup Factory {version}"),"overlay_offset":overlay,"overlay_header_base64":STANDARD.encode(overlay_header),"manifest":manifest,"certificate":certificate,"certificate_alignment_bytes":padding,"diagnostics":diagnostics,"metadata":evidence(&metadata,0),"file_table":table,"files":records,"expanded_total":decoder.total,"capability":"discover_only","installable":false,"semantics_reviewed":false,"blockers":["unparsed_installer_events","unreviewed_serialized_file_fields","game_language_build_dlc_not_verified","steamos_install_restore_not_verified"],"script_inventory":inventory});
    if behavior {
        let lowered = crate::behavior::lower(&report["script_inventory"], &records, &version)?;
        report["behavior_sha256"] = json!(behavior_digest(&lowered)?);
        report["behavior"] = lowered;
        report["capability"] = json!("behavior_supported");
        report["blockers"] = json!(["game_and_source_conditions_pending"]);
    }
    require(
        python_json(&report)?.len() < MAX_METADATA,
        "report_size_limit",
    )?;
    Ok(report)
}
