//! Bounded SF9 CArchive decoding and diagnostic-only serialized inventory.
//!
//! Complete decoding follows the sequential class map. Configuration values are data, not admission rules.
//! The scanning inventory is evidence only and never authorizes installation.
use crate::{
    audit::{evidence, file_table, Reader},
    require, Error, Result,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

const MAX_METADATA: usize = 16 * 1024 * 1024;
const SCREENS: &[&str] = &[
    "CSUFScreenStaticText",
    "CSUFScreenLicenseAgreement",
    "CSUFScreenInstallFolder",
    "CSUFScreenProgressBars",
];
const CONTROLS: &[(&str, usize)] = &[
    ("CSUFCtrlButton", 8),
    ("CSUFCtrlStaticText", 8),
    ("CSUFCtrlRadioButton", 10),
    ("CSUFCtrlScrollingText", 15),
    ("CSUFCtrlEditField", 17),
    ("CSUFCtrlProgressBar", 11),
];
fn python_space(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{001c}'..='\u{001f}')
}
fn trim_parameter(s: &str) -> &str {
    s.trim_matches(python_space)
}
fn identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
fn parameter(s: &str) -> bool {
    let parts: Vec<_> = s.split(python_space).filter(|v| !v.is_empty()).collect();
    parts.len() == 2 && identifier(parts[0]) && identifier(parts[1])
}
fn map_evidence(map: &mut Map<String, Value>, data: &[u8], offset: usize) {
    if let Value::Object(fields) = evidence(data, offset) {
        map.extend(fields);
    }
}

// Python's gb18030 codec uses the older serialization mapping, whereas
// encoding_rs follows current web mappings. Keep the legacy byte values exact.
fn legacy_display(raw: &[u8]) -> Result<String> {
    let decoded = encoding_rs::GB18030
        .decode_without_bom_handling_and_without_replacement(raw)
        .ok_or_else(|| Error("exe_display_encoding_unsupported".into()))?;
    let mut chars = decoded.chars();
    let mut result = String::with_capacity(decoded.len());
    let mut position = 0;
    while position < raw.len() {
        let first = raw[position];
        require(first != 0x80, "exe_display_encoding_unsupported")?;
        let (size, replacement) = if first < 0x80 {
            (1, None)
        } else if raw[position + 1].is_ascii_digit() {
            (
                4,
                if raw[position..position + 4] == [0x81, 0x35, 0xf4, 0x37] {
                    Some(0x1e3f)
                } else {
                    None
                },
            )
        } else {
            let code = u16::from_be_bytes([first, raw[position + 1]]);
            let replacement = match code {
                0xa6d9 => Some(0xe78d),
                0xa6da => Some(0xe78e),
                0xa6db => Some(0xe78f),
                0xa6dc => Some(0xe790),
                0xa6dd => Some(0xe791),
                0xa6de => Some(0xe792),
                0xa6df => Some(0xe793),
                0xa6ec => Some(0xe794),
                0xa6ed => Some(0xe795),
                0xa6f3 => Some(0xe796),
                0xfe59 => Some(0xe81e),
                0xfe61 => Some(0xe826),
                0xfe66 => Some(0xe82b),
                0xfe67 => Some(0xe82c),
                0xfe6d => Some(0xe832),
                0xfe7e => Some(0xe843),
                0xfe90 => Some(0xe854),
                0xfea0 => Some(0xe864),
                0xa3a0 => Some(0xe5e5),
                0xa8bc => Some(0xe7c7),
                _ => None,
            };
            (2, replacement)
        };
        let character = chars
            .next()
            .ok_or_else(|| Error("exe_display_encoding_unsupported".into()))?;
        result.push(replacement.and_then(char::from_u32).unwrap_or(character));
        position += size;
    }
    require(chars.next().is_none(), "exe_display_encoding_unsupported")?;
    Ok(result)
}
struct Archive<'a> {
    data: &'a [u8],
    r: Reader<'a>,
    version: &'a str,
    classes: BTreeMap<usize, String>,
    next_id: usize,
    objects: Vec<Value>,
    events: Vec<Value>,
    includes: Vec<Value>,
    variables: Map<String, Value>,
    packages: Vec<String>,
    terms: Vec<String>,
    files: Vec<Value>,
    file_index: usize,
    context: String,
    lifecycle: &'static str,
}
impl<'a> Archive<'a> {
    fn new(data: &'a [u8], version: &'a str) -> Result<Self> {
        require(
            matches!(version, "9.5" | "9.6" | "9.7"),
            "unsupported_installer_version",
        )?;
        let (files, _) = file_table(data, version)?;
        Ok(Self {
            data,
            r: Reader::new(data),
            version,
            classes: BTreeMap::new(),
            next_id: 1,
            objects: vec![],
            events: vec![],
            includes: vec![],
            variables: Map::new(),
            packages: vec![],
            terms: vec![],
            files,
            file_index: 0,
            context: "project".into(),
            lifecycle: "project",
        })
    }
    fn nums(&mut self, count: usize) -> Result<Vec<u32>> {
        (0..count).map(|_| self.r.u32()).collect()
    }
    fn expected(&mut self, values: &[u32], code: &str) -> Result<()> {
        require(self.nums(values.len())? == values, code)
    }
    fn byte(&mut self) -> Result<()> {
        self.r.u8().map(|_| ())
    }
    fn string(&mut self) -> Result<String> {
        self.r.string()
    }
    fn display(&mut self) -> Result<String> {
        let mut length = self.r.u8()? as usize;
        let mut wide = false;
        if length == 255 {
            length = self.r.u16()? as usize;
            if length == 65534 {
                wide = true;
                length = self.r.u8()? as usize;
                if length == 255 {
                    length = self.r.u16()? as usize;
                }
            }
            if length == 65535 {
                length = self.r.u32()? as usize;
            }
        }
        require(
            length <= MAX_METADATA / if wide { 2 } else { 1 },
            "string_size_limit",
        )?;
        let raw = self.r.read(length * if wide { 2 } else { 1 })?;
        let text = if wide {
            let words: Vec<u16> = raw
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect();
            String::from_utf16(&words)
                .map_err(|_| Error("exe_display_encoding_unsupported".into()))?
        } else {
            legacy_display(raw)?
        };
        let text = text.trim_end_matches('\0').to_owned();
        require(!text.contains('\0'), "embedded_nul")?;
        Ok(text)
    }
    fn array(&mut self, allowed: &[&str], limit: usize) -> Result<Vec<Value>> {
        let count = self.r.u16()? as usize;
        require(count <= limit, "exe_object_count_limit")?;
        (0..count).map(|_| self.object(allowed)).collect()
    }
    fn empty_array(&mut self) -> Result<()> {
        require(self.r.u16()? == 0, "exe_external_objects_unsupported")
    }
    fn object(&mut self, allowed: &[&str]) -> Result<Value> {
        let start = self.r.pos;
        let tag = self.r.u16()?;
        let name = if tag == 0xffff {
            let schema = self.r.u16()?;
            let length = self.r.u16()? as usize;
            require(length > 0 && length <= 128, "exe_class_name_invalid")?;
            let raw = self.r.read(length)?;
            require(raw.is_ascii(), "exe_class_name_invalid")?;
            let name = std::str::from_utf8(raw)
                .map_err(|_| Error("exe_class_name_invalid".into()))?
                .to_owned();
            require(
                schema == if name == "CImageInfo" { 0 } else { 1 },
                "exe_class_schema_unsupported",
            )?;
            self.classes.insert(self.next_id, name.clone());
            self.next_id += 1;
            name
        } else {
            require(
                tag & 0x8000 != 0 && tag != 0x8000,
                "exe_object_reference_unsupported",
            )?;
            self.classes
                .get(&((tag & 0x7fff) as usize))
                .cloned()
                .ok_or_else(|| Error("exe_object_class_unsupported".into()))?
        };
        require(
            allowed.contains(&name.as_str()),
            "exe_object_class_unsupported",
        )?;
        self.next_id += 1;
        require(self.next_id < 0x7fff, "exe_object_count_limit")?;
        let mut result = Map::new();
        result.insert("class".into(), json!(name));
        result.insert("offset".into(), json!(start));
        match name.as_str() {
            "CGlobalIncludeLuaFile" => {
                self.expected(&[1], "exe_include_schema_unsupported")?;
                let kind = self.r.u32()?;
                result.insert("name".into(), json!(self.string()?));
                result.insert("script".into(), json!(self.string()?));
                result.insert("source_kind".into(), json!(kind));
                let count = self.r.u16()? as usize;
                require(count <= 64, "exe_include_selectors_unsupported")?;
                let selectors: Vec<String> =
                    (0..count).map(|_| self.string()).collect::<Result<_>>()?;
                result.insert("selectors".into(), json!(selectors));
            }
            "CSetupFileData" => {
                self.expected(&[1], "exe_file_schema_unsupported")?;
                self.byte()?;
                let row = self
                    .files
                    .get(self.file_index)
                    .ok_or_else(|| Error("exe_file_count_mismatch".into()))?;
                self.file_index += 1;
                let offset = row["serialized_record"]["offset"]
                    .as_u64()
                    .ok_or_else(|| Error("exe_file_framing_invalid".into()))?;
                let size = row["serialized_record"]["size"]
                    .as_u64()
                    .ok_or_else(|| Error("exe_file_framing_invalid".into()))?;
                require(self.r.pos as u64 == offset, "exe_file_framing_invalid")?;
                self.r.read(
                    usize::try_from(size).map_err(|_| Error("exe_file_framing_invalid".into()))?,
                )?;
                let index = row["index"].clone();
                result.insert("tail_flag".into(), json!(self.r.u8()?));
                result.insert("file_index".into(), index);
            }
            n if SCREENS.contains(&n) => {
                if n == "CSUFScreenProgressBars" {
                    self.expected(&[1, 1], "exe_screen_schema_unsupported")?;
                }
                self.expected(&[1], "exe_screen_schema_unsupported")?;
                result.insert("kind".into(), json!(self.r.u32()?));
                let screen_name = self.string()?;
                result.insert("name".into(), json!(screen_name));
                self.nums(1)?;
                self.display()?;
                self.nums(3)?;
                let old = std::mem::replace(&mut self.context, format!("{n}:{screen_name}"));
                let controls: Vec<&str> = CONTROLS.iter().map(|v| v.0).collect();
                result.insert("controls".into(), json!(self.array(&controls, 128)?));
                let events = self.array(&["CIREvent"], 32)?;
                result.insert("events".into(), json!(events));
                let languages = self.array(&["CSUFLanguageFile"], 16)?;
                self.appearance()?;
                match n {
                    "CSUFScreenProgressBars" => {
                        self.expected(&[1], "exe_screen_schema_unsupported")?
                    }
                    "CSUFScreenLicenseAgreement" => {
                        self.nums(2)?;
                        for language in &languages {
                            for message in language["messages"].as_array().into_iter().flatten() {
                                if message["name"] == "IDS_CTRL_SCROLLTEXT_BODY" {
                                    self.terms.push(
                                        message["text"].as_str().unwrap_or_default().to_owned(),
                                    );
                                }
                            }
                        }
                    }
                    "CSUFScreenInstallFolder" => {
                        self.expected(&[1], "exe_screen_schema_unsupported")?;
                        self.string()?;
                    }
                    _ => {}
                }
                result.insert("languages".into(), json!(languages));
                self.context = old;
            }
            n if CONTROLS.iter().any(|v| v.0 == n) => {
                let count = CONTROLS.iter().find(|v| v.0 == n).unwrap().1;
                let fields = self.nums(count)?;
                require(fields[0] == 1, "exe_control_schema_unsupported")?;
                result.insert("fields".into(), json!(fields));
                match n {
                    "CSUFCtrlScrollingText" => self.byte()?,
                    "CSUFCtrlEditField" => {
                        let variable = self.string()?;

                        result.insert("variable".into(), json!(variable));
                    }
                    _ => {}
                }
            }
            "CIREvent" => {
                self.expected(&[1], "exe_event_schema_unsupported")?;
                let parameters = self.string()?;
                require(
                    parameters.chars().count() <= 8192
                        && parameters
                            .split(',')
                            .filter(|v| !v.is_empty())
                            .all(|v| parameter(trim_parameter(v))),
                    "exe_event_parameters_unsupported",
                )?;
                result.insert("parameters".into(), json!(parameters));
                let event_name = self.string()?;
                result.insert("name".into(), json!(event_name));
                result.insert("script".into(), json!(self.string()?));
                result.insert("context".into(), json!(self.context));
                result.insert("lifecycle".into(), json!(self.lifecycle));
            }
            "CSUFLanguageFile" => {
                self.expected(&[1], "exe_language_schema_unsupported")?;
                self.nums(1)?;
                result.insert("name".into(), json!(self.string()?));
                self.byte()?;
                self.nums(4)?;
                let count = self.r.u16()? as usize;
                require(count <= 128, "exe_language_count_limit")?;
                result.insert("order".into(), json!(self.r.read(count)?));
                result.insert(
                    "messages".into(),
                    json!(self.array(&["CSUFMessageID"], 2048)?),
                );
            }
            "CSUFMessageID" => {
                self.expected(&[1], "exe_message_schema_unsupported")?;
                result.insert("name".into(), json!(self.string()?));
                result.insert("text".into(), json!(self.display()?));
            }
            "CSessionVar" => {
                self.expected(&[1], "exe_variable_schema_unsupported")?;
                let variable = self.string()?;
                let value = self.display()?;
                let _flag = self.r.u32()?;
                require(
                    !self.variables.contains_key(&variable),
                    "exe_variable_invalid",
                )?;
                self.variables.insert(variable, json!(value));
            }
            "CSUFPackage" => {
                self.expected(&[1], "exe_package_schema_unsupported")?;
                let package = self.string()?;
                result.insert("name".into(), json!(package));
                let options = self.nums(3)?;
                result.insert("install".into(), json!(options[0] != 0));
                result.insert("enabled".into(), json!(options[1] != 0));
                result.insert("additional_space".into(), json!(options[2]));
                result.insert("condition".into(), json!(self.string()?));
                self.array(&["CSUFLanguageFile"], 16)?;
                if options[0] == 0 {
                    self.packages.push(package);
                }
            }
            "CPasswordData" => {
                self.display()?;
                let count = self.r.u16()? as usize;
                require(count <= 64, "exe_password_list_limit")?;
                let components: Vec<String> =
                    (0..count).map(|_| self.string()).collect::<Result<_>>()?;
                let _ = components;
                require(self.r.u16()? == 0, "exe_password_unsupported")?;
            }
            "CImageInfo" => {
                self.display()?;
                self.display()?;
                self.nums(1)?;
            }
            _ => return Err(Error("exe_object_class_unsupported".into())),
        }
        let size = self.r.pos - start;
        result.insert("size".into(), json!(size));
        let result = Value::Object(result);
        if name == "CIREvent" {
            self.events.push(result.clone());
        }
        if name == "CGlobalIncludeLuaFile" {
            self.includes.push(result.clone());
        }
        self.objects.push(if name == "CSUFPackage" {
            result.clone()
        } else {
            json!({"class": name, "offset": start, "size": size})
        });
        Ok(result)
    }
    fn appearance(&mut self) -> Result<()> {
        self.expected(&[1], "exe_appearance_schema_unsupported")?;
        self.nums(16)?;
        for _ in 0..5 {
            self.nums(5)?;
            self.display()?;
        }
        self.expected(&[1], "exe_appearance_schema_unsupported")?;
        self.display()?;
        self.display()?;
        self.byte()?;
        self.nums(if self.version == "9.7" { 26 } else { 25 })?;
        Ok(())
    }
    fn labels(&mut self) -> Result<()> {
        for _ in 0..3 {
            self.display()?;
            self.nums(4)?;
            self.display()?;
            self.nums(4)?;
            self.byte()?;
        }
        Ok(())
    }
    fn uninstall(&mut self) -> Result<()> {
        self.lifecycle = "uninstall";
        self.expected(&[1], "exe_uninstall_schema_unsupported")?;
        self.nums(1)?;
        for _ in 0..3 {
            self.display()?;
        }
        self.nums(5)?;
        self.byte()?;
        self.nums(5)?;
        self.byte()?;
        self.labels()?;
        self.nums(4)?;
        self.display()?;
        self.display()?;
        self.nums(2)?;
        self.byte()?;
        for _ in 0..5 {
            self.display()?;
        }
        self.nums(1)?;
        self.byte()?;
        self.display()?;
        self.nums(2)?;
        self.empty_array()?;
        self.display()?;
        self.display()?;
        self.nums(2)?;
        self.empty_array()?;
        self.display()?;
        self.empty_array()?;
        let old = std::mem::replace(&mut self.context, "uninstall".into());
        self.array(SCREENS, 32)?;
        self.nums(2)?;
        self.array(SCREENS, 32)?;
        self.array(SCREENS, 32)?;
        self.array(&["CIREvent"], 32)?;
        self.context = old;
        Ok(())
    }
    fn timestamp(&mut self) -> Result<()> {
        require(
            self.r.u32()? == 0x8000000a,
            "exe_timestamp_schema_unsupported",
        )?;
        self.r.read(8)?;
        Ok(())
    }
    fn project(&mut self) -> Result<()> {
        self.lifecycle = "project";
        self.expected(&[1], "exe_project_schema_unsupported")?;
        self.byte()?;
        self.display()?;
        self.r.read(6)?;
        self.nums(2)?;
        self.byte()?;
        self.nums(4)?;
        self.r.read(2)?;
        self.display()?;
        self.r.read(2)?;
        self.labels()?;
        self.nums(6)?;
        self.timestamp()?;
        self.nums(7)?;
        self.display()?;
        self.nums(1)?;
        self.display()?;
        self.display()?;
        self.byte()?;
        self.byte()?;
        self.nums(1)?;
        let count = self.r.u16()? as usize;
        self.r.read(count * 2)?;
        self.nums(6)?;
        for _ in 0..11 {
            self.display()?;
        }
        self.nums(2)?;
        self.empty_array()?;
        self.context = "project".into();
        self.array(&["CIREvent"], 32)?;
        self.array(&["CImageInfo"], 16)?;
        self.empty_array()?;
        self.array(&["CSUFLanguageFile"], 16)?;
        self.timestamp()
    }
    fn decode(mut self) -> Result<Value> {
        self.array(&["CGlobalIncludeLuaFile"], 64)?;
        self.array(&["CSetupFileData"], u16::MAX as usize)?;
        require(
            self.file_index == self.files.len(),
            "exe_file_count_mismatch",
        )?;
        self.empty_array()?;
        self.lifecycle = "before";
        self.array(SCREENS, 32)?;
        self.nums(2)?;
        self.lifecycle = "while";
        self.array(SCREENS, 32)?;
        self.lifecycle = "after";
        self.array(SCREENS, 32)?;
        self.array(&["CSessionVar"], 256)?;
        self.array(&["CSUFPackage"], 64)?;
        self.array(&["CPasswordData"], 64)?;
        self.uninstall()?;
        self.project()?;
        require(self.r.pos == self.data.len(), "exe_metadata_trailing_data")?;
        Ok(
            json!({"decoder": "sf9-graph-v2", "coverage": {"complete": true, "bytes": self.r.pos},
            "metadata": evidence(self.data, 0), "events": self.events, "includes": self.includes,
            "variables": self.variables, "disabled_packages": self.packages, "terms": self.terms,
            "objects": self.objects, "installable": false}),
        )
    }
}

/// Decode the entire supported SF9 metadata graph. Unknown framing fails closed.
pub fn decode(metadata: &[u8], version: &str) -> Result<Value> {
    require(
        !metadata.is_empty() && metadata.len() <= MAX_METADATA,
        "exe_metadata_limit",
    )?;
    Archive::new(metadata, version)?.decode()
}

fn declarations(data: &[u8]) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    let mut position = 0;
    while position + 2 <= data.len() {
        let offset = position;
        if data[offset..offset + 2] != [255, 255] {
            position += 1;
            continue;
        }
        position += 2;
        if data[offset..offset + 2] != [255, 255] || offset + 6 > data.len() {
            continue;
        }
        let schema = u16::from_le_bytes([data[offset + 2], data[offset + 3]]);
        let length = u16::from_le_bytes([data[offset + 4], data[offset + 5]]) as usize;
        if !(1..=128).contains(&length) || offset + 6 + length > data.len() {
            continue;
        }
        let raw = &data[offset + 6..offset + 6 + length];
        if raw.len() < 2
            || raw[0] != b'C'
            || !raw[1].is_ascii_alphabetic()
            || !raw[2..].iter().all(u8::is_ascii_alphanumeric)
        {
            continue;
        }
        let mut row = Map::new();
        row.insert(
            "name".into(),
            json!(std::str::from_utf8(raw).map_err(|_| Error("exe_class_name_invalid".into()))?),
        );
        row.insert("schema".into(), json!(schema));
        map_evidence(&mut row, &data[offset..offset + 6 + length], offset);
        result.push(Value::Object(row));
        require(result.len() <= 256, "exe_class_count_limit")?;
    }
    Ok(result)
}
fn inventory_includes(data: &[u8]) -> Result<(Vec<Value>, usize)> {
    let mut r = Reader::new(data);
    let count = r.u16()? as usize;
    require(count <= 64, "exe_include_count_limit")?;
    let mut result = Vec::new();
    if count == 0 {
        return Ok((result, r.pos));
    }
    require(
        r.read(4)? == [255, 255, 1, 0],
        "exe_include_schema_unsupported",
    )?;
    let length = r.u16()? as usize;
    require(
        length == b"CGlobalIncludeLuaFile".len(),
        "exe_include_schema_unsupported",
    )?;
    require(
        r.read(length)? == b"CGlobalIncludeLuaFile",
        "exe_include_schema_unsupported",
    )?;
    for index in 0..count {
        let start = r.pos;
        if index != 0 {
            require(r.u16()? == 0x8001, "exe_include_reference_unsupported")?;
        }
        require(r.u32()? == 1, "exe_include_schema_unsupported")?;
        let source_kind = r.u32()?;
        require(source_kind <= 1, "exe_include_kind_unsupported")?;
        let name = r.string()?;
        let script_offset = r.pos;
        let script = r.string()?;
        let count = r.u16()? as usize;
        require(count <= 64, "exe_selector_count_limit")?;
        let selectors: Vec<String> = (0..count).map(|_| r.string()).collect::<Result<_>>()?;
        let mut row = json!({"name": name, "source_kind": source_kind, "script": script, "selectors": selectors, "script_offset": script_offset}).as_object().unwrap().clone();
        map_evidence(&mut row, &data[start..r.pos], start);
        result.push(Value::Object(row));
    }
    Ok((result, r.pos))
}
fn inventory_event(data: &[u8], r: &mut Reader<'_>) -> Result<Value> {
    let start = r.pos;
    require(r.u32()? == 1, "exe_event_schema_unsupported")?;
    let parameter_text = r.string()?;
    require(
        parameter_text.chars().count() <= 8192,
        "exe_event_parameter_limit",
    )?;
    let parameters: Vec<_> = if parameter_text.is_empty() {
        vec![]
    } else {
        parameter_text
            .split(',')
            .map(|v| trim_parameter(v).to_owned())
            .collect()
    };
    require(parameters.len() <= 16, "exe_event_parameter_limit")?;
    require(
        parameters.iter().all(|v| parameter(v)),
        "exe_event_parameters_unsupported",
    )?;
    let name = r.string()?;
    require(
        (name.starts_with("On ") || name == "Global Functions") && name.chars().count() <= 128,
        "exe_event_name_invalid",
    )?;
    let script_offset = r.pos;
    let script = r.string()?;
    let mut row = json!({"name": name, "parameters": parameters, "script": script, "script_offset": script_offset}).as_object().unwrap().clone();
    map_evidence(&mut row, &data[start..r.pos], start);
    Ok(Value::Object(row))
}

/// Static inventory only. Its scans deliberately cannot establish full coverage.
pub fn inventory(metadata: &[u8]) -> Result<Value> {
    require(
        !metadata.is_empty() && metadata.len() <= MAX_METADATA,
        "exe_metadata_limit",
    )?;
    let classes = declarations(metadata)?;
    let (includes, include_end) = inventory_includes(metadata)?;
    let event_classes: Vec<&Value> = classes.iter().filter(|v| v["name"] == "CIREvent").collect();
    let mut events = Vec::new();
    let mut diagnostics = Vec::new();
    if event_classes.len() == 1 {
        let declaration = event_classes[0];
        let offset = declaration["offset"].as_u64().unwrap() as usize;
        let size = declaration["size"].as_u64().unwrap() as usize;
        let mut r = Reader::at(metadata, offset + size, metadata.len())?;
        events.push(inventory_event(metadata, &mut r)?);
        if r.pos + 2 <= metadata.len() {
            let reference = &metadata[r.pos..r.pos + 2];
            if u16::from_le_bytes([reference[0], reference[1]]) & 0x8000 != 0 {
                let pattern = [reference[0], reference[1], 1, 0, 0, 0];
                let mut position = 0;
                while position + pattern.len() <= metadata.len() {
                    if metadata[position..position + pattern.len()] != pattern {
                        position += 1;
                        continue;
                    }
                    let mut candidate = Reader::at(metadata, position + 2, metadata.len())?;
                    match inventory_event(metadata, &mut candidate) {
                        Ok(current) => {
                            events.push(current);
                            require(events.len() <= 1024, "exe_event_count_limit")?;
                        }
                        Err(_) => {
                            diagnostics.push(json!({"code": "exe_event_reference_unresolved", "offset": position}));
                            require(diagnostics.len() <= 1024, "exe_event_count_limit")?;
                        }
                    }
                    position += pattern.len();
                }
            }
        }
    } else if !event_classes.is_empty() {
        diagnostics.push(json!({"code": "exe_event_class_ambiguous"}));
    } else {
        diagnostics.push(json!({"code": "exe_event_graph_unresolved"}));
    }
    Ok(
        json!({"schema_version": 1, "decoder": "sf9-metadata-v1", "metadata": evidence(metadata, 0),
        "classes": classes, "includes": includes, "include_end": include_end, "events": events,
        "diagnostics": diagnostics, "coverage": {"complete": false, "blockers": ["exe_serialized_object_graph_incomplete"]},
        "installable": false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_codec_keeps_python_legacy_mapping() {
        let pairs = [
            (0xa6d9u16, 0xe78d),
            (0xa6da, 0xe78e),
            (0xa6db, 0xe78f),
            (0xa6dc, 0xe790),
            (0xa6dd, 0xe791),
            (0xa6de, 0xe792),
            (0xa6df, 0xe793),
            (0xa6ec, 0xe794),
            (0xa6ed, 0xe795),
            (0xa6f3, 0xe796),
            (0xfe59, 0xe81e),
            (0xfe61, 0xe826),
            (0xfe66, 0xe82b),
            (0xfe67, 0xe82c),
            (0xfe6d, 0xe832),
            (0xfe7e, 0xe843),
            (0xfe90, 0xe854),
            (0xfea0, 0xe864),
            (0xa3a0, 0xe5e5),
            (0xa8bc, 0xe7c7),
        ];
        for (bytes, codepoint) in pairs {
            let expected = char::from_u32(codepoint).unwrap().to_string();
            assert_eq!(legacy_display(&bytes.to_be_bytes()).unwrap(), expected);
        }
        assert_eq!(
            legacy_display(&[0x81, 0x35, 0xf4, 0x37]).unwrap(),
            "\u{1e3f}"
        );
        assert_eq!(legacy_display(b"A\xd6\xd0\xce\xc4Z").unwrap(), "A中文Z");
        assert_eq!(legacy_display(&[0x94, 0x39, 0xda, 0x33]).unwrap(), "💩");
    }

    #[test]
    fn display_codec_rejects_malformed_bytes_without_panics() {
        for raw in [
            b"\x80".as_slice(),
            b"A\x80Z",
            b"\x81",
            b"\xff",
            b"\x81\x30",
            b"\x81\x30\x81",
            b"\x81\x30\x81\xff",
            b"\x81\x7f",
        ] {
            assert_eq!(
                legacy_display(raw).unwrap_err().0,
                "exe_display_encoding_unsupported"
            );
        }
        assert_eq!(legacy_display(b"").unwrap(), "");
    }
}
