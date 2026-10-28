//! Pure admission of the fixed worker report. Never parses PE or executes Lua.
use super::{
    facts::{self, FileDigest, GameFacts},
    invalid, Result,
};
use crate::sources::canonical;
use base64::{engine::general_purpose::STANDARD, Engine};
use caseless::Caseless;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
use unicode_normalization::UnicodeNormalization;
const MAX_NODES: usize = 8192;
const MAX_DEPTH: usize = 96;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadEntry {
    pub path: String,
    pub size: u64,
    pub crc32: u32,
    pub payload_offset: u64,
    pub packed_size: u64,
    pub compressed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportBinding {
    pub exe_sha256: String,
    pub exe_size: u64,
    pub metadata_sha256: String,
    pub behavior_sha256: String,
    pub payload_sha256: String,
}
#[derive(Debug, Clone)]
pub struct PreparedReport {
    binding: ReportBinding,
    payload: Vec<PayloadEntry>,
    condition: Condition,
    condition_sha256: String,
    file_conditions: Vec<(String, Condition)>,
}
#[derive(Debug, Clone)]
enum Number {
    Literal(u32),
    Crc32(String),
}
#[derive(Debug, Clone)]
enum Condition {
    Bool(bool),
    Exists(String),
    Not(Box<Condition>),
    And(Box<Condition>, Box<Condition>),
    Or(Box<Condition>, Box<Condition>),
    Compare(String, Number, Number),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionFileFact {
    pub path: String,
    pub observed: Option<FileDigest>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionSnapshot {
    pub instance_id: String,
    pub behavior_sha256: String,
    pub files: Vec<ConditionFileFact>,
    pub evaluation_files: Vec<ConditionFileFact>,
    pub satisfied: bool,
}
impl ConditionSnapshot {
    pub fn sha256(&self) -> String {
        canonical::canonical_sha256(self)
    }
}
impl PreparedReport {
    /// Resolve optional members against the same read-only game facts used by
    /// the installation gate, before content identity is bound.
    pub fn select_payload(&mut self, game: &GameFacts, cancel: &AtomicBool) -> Result<()> {
        if self.file_conditions.is_empty() {
            return Ok(());
        }
        let files = self.observe_condition_files(game, cancel)?;
        let effective: BTreeMap<_, _> = files.into_iter().map(|f| (f.path, f.observed)).collect();
        let mut excluded = BTreeSet::new();
        for (path, condition) in &self.file_conditions {
            let selected = match condition.eval(&effective) {
                Ok(value) => value,
                Err(e) if e.to_string() == "exe_installer_condition_mismatch" => false,
                Err(e) => return Err(e),
            };
            if !selected {
                excluded.insert(path.clone());
            }
        }
        self.payload.retain(|p| !excluded.contains(&p.path));
        require(!self.payload.is_empty(), "exe_no_selected_files")?;
        self.binding.payload_sha256 = canonical::canonical_sha256(&self.payload);
        Ok(())
    }
    fn observe_condition_files(
        &self,
        game: &GameFacts,
        cancel: &AtomicBool,
    ) -> Result<Vec<ConditionFileFact>> {
        require(
            game.instance.install_path.is_absolute()
                && super::zip_recipe::no_links(&game.instance.install_path)
                && game.instance.install_path.is_dir(),
            "exe_game_identity_changed",
        )?;
        let mut paths = BTreeSet::new();
        self.condition.paths(&mut paths);
        for (_, condition) in &self.file_conditions {
            condition.paths(&mut paths);
        }
        let mut files = Vec::new();
        // Record all static references, including skipped boolean branches and absence.
        for path in paths {
            if cancel.load(Ordering::Relaxed) {
                return Err(invalid("download_cancelled"));
            }
            let target = game.instance.install_path.join(&path);
            require(super::zip_recipe::no_links(&target), "linked_target")?;
            let observed = match fs::symlink_metadata(&target) {
                Ok(m) if m.is_file() => Some(facts::digest_file(&target, cancel)?),
                Ok(_) => return Err(invalid("exe_condition_not_file")),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    require(
                        super::zip_recipe::no_links(&target) && !target.try_exists()?,
                        "exe_game_file_changed",
                    )?;
                    None
                }
                Err(e) => return Err(e),
            };
            files.push(ConditionFileFact { path, observed });
        }
        Ok(files)
    }
    pub fn binding(&self) -> &ReportBinding {
        &self.binding
    }
    pub fn payload(&self) -> &[PayloadEntry] {
        &self.payload
    }
    pub fn condition_sha256(&self) -> &str {
        &self.condition_sha256
    }
    pub fn payload_bytes(&self) -> Result<u64> {
        self.payload.iter().try_fold(0u64, |total, p| {
            total
                .checked_add(p.size)
                .ok_or_else(|| invalid("exe_manifest_invalid"))
        })
    }
    /// Content identity follows the Rust UTF-8 canonical contract, not worker report encoding.
    pub fn content_id(
        &self,
        appid: u32,
        source_id: &str,
        source_record_id: &str,
        language: &str,
        variant: &str,
    ) -> Result<String> {
        require(
            appid > 0
                && !source_id.is_empty()
                && !source_record_id.is_empty()
                && ["base", "dlc", "unknown", "demo"].contains(&variant),
            "exe_content_identity_invalid",
        )?;
        let identity = json!({"appid":appid,"source_id":source_id,"source_record_id":source_record_id,"language":language,"variant":variant,
   "exe_sha256":self.binding.exe_sha256,"metadata_sha256":self.binding.metadata_sha256,"payload":self.payload});
        Ok(format!(
            "exe-content-{}-{appid}",
            &canonical::canonical_sha256(&identity)[..24]
        ))
    }
    pub fn evaluate(&self, game: &GameFacts, cancel: &AtomicBool) -> Result<ConditionSnapshot> {
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("download_cancelled"));
        }
        // Unrelated fact-collection failures do not block wholesale (design D06).
        // The concrete preconditions below are the real gates.
        require(
            game.instance.install_path.is_absolute()
                && super::zip_recipe::no_links(&game.instance.install_path)
                && game.instance.install_path.is_dir(),
            "exe_game_identity_changed",
        )?;
        let files = self.observe_condition_files(game, cancel)?;
        let effective: BTreeMap<_, _> = files
            .iter()
            .map(|f| (f.path.clone(), f.observed.clone()))
            .collect();
        let selected_paths: BTreeSet<_> = self.payload.iter().map(|p| p.path.as_str()).collect();
        for (path, condition) in &self.file_conditions {
            let selected = match condition.eval(&effective) {
                Ok(value) => value,
                Err(e) if e.to_string() == "exe_installer_condition_mismatch" => false,
                Err(e) => return Err(e),
            };
            require(
                selected == selected_paths.contains(path.as_str()),
                "exe_selection_changed",
            )?;
        }
        Ok(ConditionSnapshot {
            instance_id: game.instance.instance_id.clone(),
            behavior_sha256: self.binding.behavior_sha256.clone(),
            satisfied: match self.condition.eval(&effective) {
                Ok(v) => v,
                Err(e) if e.to_string() == "exe_installer_condition_mismatch" => false,
                Err(e) => return Err(e),
            },
            evaluation_files: effective
                .into_iter()
                .map(|(path, observed)| ConditionFileFact { path, observed })
                .collect(),
            files,
        })
    }
}

impl Condition {
    fn paths(&self, p: &mut BTreeSet<String>) {
        match self {
            Self::Exists(s) => {
                p.insert(s.clone());
            }
            Self::Not(v) => v.paths(p),
            Self::And(a, b) | Self::Or(a, b) => {
                a.paths(p);
                b.paths(p);
            }
            Self::Compare(_, a, b) => {
                for n in [a, b] {
                    if let Number::Crc32(s) = n {
                        p.insert(s.clone());
                    }
                }
            }
            Self::Bool(_) => {}
        }
    }
    fn eval(&self, f: &BTreeMap<String, Option<FileDigest>>) -> Result<bool> {
        Ok(match self {
            Self::Bool(v) => *v,
            Self::Exists(p) => f.get(p).is_some_and(Option::is_some),
            Self::Not(v) => !v.eval(f)?,
            Self::And(a, b) => a.eval(f)? && b.eval(f)?,
            Self::Or(a, b) => a.eval(f)? || b.eval(f)?,
            Self::Compare(op, a, b) => {
                let val = |n: &Number| -> Result<Option<u32>> {
                    Ok(match n {
                        Number::Literal(v) => Some(*v),
                        Number::Crc32(p) => f
                            .get(p)
                            .ok_or_else(|| invalid("exe_condition_ir_invalid"))?
                            .as_ref()
                            .map(|d| d.crc32),
                    })
                };
                match (val(a)?, val(b)?) {
                    (Some(a), Some(b)) => match op.as_str() {
                        "==" => a == b,
                        "~=" => a != b,
                        "<" => a < b,
                        "<=" => a <= b,
                        ">" => a > b,
                        ">=" => a >= b,
                        _ => unreachable!(),
                    },
                    _ => return Err(invalid("exe_installer_condition_mismatch")),
                }
            }
        })
    }
}
fn require(ok: bool, code: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(invalid(code))
    }
}
fn hash(s: &str) -> Result<()> {
    require(
        s.len() == 64
            && s.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "exe_report_hash_invalid",
    )
}
pub(crate) fn relative_path(path: &str) -> Result<()> {
    require(
        !path.is_empty()
            && path.len() <= 1024
            && !path.contains(['\\', ':', '%'])
            && !path.chars().any(char::is_control),
        "exe_condition_path_unsupported",
    )?;
    for p in path.split('/') {
        let d = p.split('.').next().unwrap_or_default().to_ascii_uppercase();
        let numbered = d.chars().count() == 4
            && (d.starts_with("COM") || d.starts_with("LPT"))
            && d.chars()
                .last()
                .is_some_and(|c| "0123456789¹²³".contains(c));
        require(
            !p.is_empty()
                && ![".", ".."].contains(&p)
                && !p.ends_with(['.', ' '])
                && !p.chars().any(|c| "<>\"|?*".contains(c))
                && !["CON", "PRN", "AUX", "NUL"].contains(&d.as_str())
                && !numbered
                && !p
                    .chars()
                    .default_case_fold()
                    .collect::<String>()
                    .starts_with(".patchhub-"),
            "exe_condition_path_unsupported",
        )?;
    }
    Ok(())
}
fn exact(v: &Value, fields: &[&str]) -> Result<()> {
    let o = v
        .as_object()
        .ok_or_else(|| invalid("exe_worker_invalid_response"))?;
    require(
        o.len() == fields.len() && fields.iter().all(|k| o.contains_key(*k)),
        "exe_worker_invalid_response",
    )
}
/// Reject unknown keys inside a nested object whose key set is bounded but not
/// fixed (e.g. diagnostic rows that carry optional evidence fields).
fn keys_within(v: &Value, fields: &[&str]) -> Result<()> {
    let o = v
        .as_object()
        .ok_or_else(|| invalid("exe_worker_invalid_response"))?;
    require(
        o.keys().all(|k| fields.contains(&k.as_str())),
        "exe_worker_invalid_response",
    )
}
fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str()
        .ok_or_else(|| invalid("exe_worker_invalid_response"))
}
fn uint(v: &Value, k: &str) -> Result<u64> {
    v[k].as_u64()
        .ok_or_else(|| invalid("exe_worker_invalid_response"))
}
fn bump(b: &mut usize, d: usize) -> Result<()> {
    require(*b > 0 && d <= MAX_DEPTH, "exe_condition_ir_limit")?;
    *b -= 1;
    Ok(())
}
fn number(v: &Value, b: &mut usize, d: usize) -> Result<Number> {
    bump(b, d)?;
    if let Some(n) = v.as_u64().and_then(|n| u32::try_from(n).ok()) {
        return Ok(Number::Literal(n));
    }
    let a = v
        .as_array()
        .ok_or_else(|| invalid("exe_condition_ir_invalid"))?;
    require(
        a.len() == 3 && a[0] == "symbol" && a[1] == "crc32",
        "exe_condition_ir_invalid",
    )?;
    let p = a[2]
        .as_str()
        .ok_or_else(|| invalid("exe_condition_ir_invalid"))?;
    relative_path(p)?;
    bump(b, d + 1)?;
    Ok(Number::Crc32(p.into()))
}
fn condition(v: &Value, b: &mut usize, d: usize) -> Result<Condition> {
    bump(b, d)?;
    if let Some(v) = v.as_bool() {
        return Ok(Condition::Bool(v));
    }
    let a = v
        .as_array()
        .ok_or_else(|| invalid("exe_condition_ir_invalid"))?;
    require(a.len() >= 2 && a[0] == "symbol", "exe_condition_ir_invalid")?;
    let op = a[1]
        .as_str()
        .ok_or_else(|| invalid("exe_condition_ir_invalid"))?;
    match op {
        "exists" if a.len() == 3 => {
            let p = a[2]
                .as_str()
                .ok_or_else(|| invalid("exe_condition_ir_invalid"))?;
            relative_path(p)?;
            bump(b, d + 1)?;
            Ok(Condition::Exists(p.into()))
        }
        "not" if a.len() == 3 => Ok(Condition::Not(Box::new(condition(&a[2], b, d + 1)?))),
        "and" | "or" if a.len() == 4 => {
            let l = Box::new(condition(&a[2], b, d + 1)?);
            let r = Box::new(condition(&a[3], b, d + 1)?);
            Ok(if op == "and" {
                Condition::And(l, r)
            } else {
                Condition::Or(l, r)
            })
        }
        "==" | "~=" | "<" | "<=" | ">" | ">=" if a.len() == 4 => {
            let l = number(&a[2], b, d + 1)?;
            let r = number(&a[3], b, d + 1)?;
            require(
                matches!(l, Number::Crc32(_)) || matches!(r, Number::Crc32(_)),
                "exe_condition_ir_invalid",
            )?;
            Ok(Condition::Compare(op.into(), l, r))
        }
        _ => Err(invalid("exe_condition_ir_invalid")),
    }
}
/// Admit the current fixed-worker report; only game/source checks may remain pending.
pub fn validate_response(
    response: &Value,
    exe_sha256: &str,
    exe_size: u64,
) -> Result<PreparedReport> {
    hash(exe_sha256)?;
    require(exe_size > 0, "exe_size_limit")?;
    // No whole-response byte/node ceiling and no redundant recursive pre-walk:
    // the report grows with a legitimate installer's file table; structure and
    // values are validated by the typed field reads below, and content is bound
    // by the digest/hash checks (design D08 / AGENTS 精简设计).
    exact(
        response,
        &[
            "schema_version",
            "action",
            "exe_sha256",
            "report",
            "installable",
        ],
    )?;
    require(
        response["schema_version"] == 1
            && response["action"] == "analyze_behavior"
            && response["exe_sha256"] == exe_sha256
            && response["installable"] == false,
        "exe_worker_invalid_response",
    )?;
    let report = &response["report"];
    exact(
        report,
        &[
            "auditor",
            "behavior",
            "behavior_sha256",
            "blockers",
            "capability",
            "certificate",
            "certificate_alignment_bytes",
            "diagnostics",
            "exe",
            "expanded_total",
            "file_table",
            "files",
            "installable",
            "installer",
            "manifest",
            "metadata",
            "overlay_header_base64",
            "overlay_offset",
            "schema_version",
            "script_inventory",
            "semantics_reviewed",
        ],
    )?;
    require(
        report["schema_version"] == 1
            && report["auditor"] == "sf9-audit-v3"
            && report["capability"] == "behavior_supported"
            && report["installable"] == false
            && report["semantics_reviewed"] == false
            && [
                "Setup Factory 9.5",
                "Setup Factory 9.6",
                "Setup Factory 9.7",
            ]
            .contains(&text(report, "installer")?),
        "exe_worker_invalid_response",
    )?;
    exact(&report["exe"], &["offset", "size", "sha256"])?;
    require(
        report["exe"]["offset"] == 0
            && report["exe"]["size"] == exe_size
            && report["exe"]["sha256"] == exe_sha256,
        "exe_worker_invalid_response",
    )?;
    require(
        report["blockers"] == json!(["game_and_source_conditions_pending"]),
        "exe_report_blocked",
    )?;
    exact(&report["metadata"], &["offset", "size", "sha256"])?;
    let metadata_sha256 = text(&report["metadata"], "sha256")?;
    hash(metadata_sha256)?;
    require(
        report["metadata"]["offset"] == 0 && uint(&report["metadata"], "size")? > 0,
        "exe_worker_invalid_response",
    )?;
    let behavior = &report["behavior"];
    exact(
        behavior,
        &[
            "schema_version",
            "policy",
            "condition",
            "copy_files",
            "file_conditions",
            "coverage",
        ],
    )?;
    require(
        behavior["schema_version"] == 1
            && behavior["policy"] == "sf9-behavior-v3"
            && behavior["coverage"]["complete"] == true
            && report["script_inventory"]["coverage"]["complete"] == true,
        "exe_report_coverage_incomplete",
    )?;
    exact(
        &behavior["coverage"],
        &["complete", "events", "includes", "steps"],
    )?;
    exact(
        &report["script_inventory"]["coverage"],
        &["bytes", "complete"],
    )?;
    require(
        report["script_inventory"]["decoder"] == "sf9-graph-v3"
            && uint(&report["script_inventory"]["coverage"], "bytes")?
                == uint(&report["metadata"], "size")?,
        "exe_report_coverage_incomplete",
    )?;
    exact(
        &report["script_inventory"],
        &[
            "coverage",
            "decoder",
            "disabled_packages",
            "events",
            "includes",
            "installable",
            "metadata",
            "objects",
        ],
    )?;
    // A certificate is null when unsigned; when present it is exactly the
    // bounded evidence blob the fixed worker emits.
    if !report["certificate"].is_null() {
        exact(
            &report["certificate"],
            &["offset", "sha256", "signature_verified", "size"],
        )?;
    }
    // Diagnostic rows are bounded informational evidence, not a fixed shape:
    // every key must be one the fixed worker can emit, but optional evidence
    // fields may be absent per row. Unknown keys are still rejected fail-closed.
    for diagnostic in report["diagnostics"]
        .as_array()
        .ok_or_else(|| invalid("exe_worker_invalid_response"))?
    {
        keys_within(
            diagnostic,
            &[
                "crc32",
                "hashed_representation",
                "name",
                "offset",
                "packed_size",
                "sha256",
                "size",
                "unknown_header_base64",
            ],
        )?;
    }
    for k in ["events", "includes", "steps"] {
        require(
            uint(&behavior["coverage"], k)? <= 200_000,
            "exe_report_coverage_incomplete",
        )?;
    }
    let behavior_sha256 = text(report, "behavior_sha256")?;
    hash(behavior_sha256)?;
    require(
        worker_behavior_sha256(behavior)? == behavior_sha256,
        "exe_behavior_changed",
    )?;
    let mut budget = MAX_NODES;
    let condition = condition(&behavior["condition"], &mut budget, 0)?;
    let rows = report["files"]
        .as_array()
        .ok_or_else(|| invalid("exe_worker_invalid_response"))?;
    require(
        !rows.is_empty() && rows.len() <= u16::MAX as usize,
        "exe_manifest_invalid",
    )?;
    let mut payload = Vec::new();
    let mut total = 0u64;
    let mut paths = BTreeSet::new();
    let mut prefixes = BTreeMap::new();
    for row in rows {
        file_policy(row, text(report, "installer")?)?;
        let path = text(row, "destination")?;
        relative_path(path)?;
        let size = uint(row, "unpacked_size")?;
        let crc32 = uint(row, "crc32")?;
        require(crc32 <= u32::MAX as u64, "exe_manifest_invalid")?;
        let payload_offset = uint(row, "payload_offset")?;
        let packed_size = uint(row, "packed_size")?;
        require(
            payload_offset
                .checked_add(packed_size)
                .is_some_and(|end| end <= exe_size),
            "exe_manifest_invalid",
        )?;
        let compressed = row["compressed"]
            .as_bool()
            .ok_or_else(|| invalid("exe_manifest_invalid"))?;
        total = total
            .checked_add(size)
            .ok_or_else(|| invalid("exe_manifest_invalid"))?;
        require(
            paths.insert(path.nfc().default_case_fold().collect::<String>()),
            "exe_manifest_invalid",
        )?;
        let parts: Vec<_> = path.split('/').collect();
        for n in 1..=parts.len() {
            let prefix = parts[..n].join("/");
            let key = prefix.nfc().default_case_fold().collect::<String>();
            require(
                prefixes
                    .get(&key)
                    .is_none_or(|previous| previous == &prefix),
                "exe_manifest_invalid",
            )?;
            prefixes.insert(key, prefix);
        }
        payload.push(PayloadEntry {
            path: path.into(),
            size,
            crc32: crc32 as u32,
            payload_offset,
            packed_size,
            compressed,
        });
    }
    for p in &paths {
        for a in Path::new(p).ancestors().skip(1) {
            require(
                !paths.contains(&a.to_string_lossy().replace('\\', "/")),
                "exe_manifest_invalid",
            )?;
        }
    }
    let copy = behavior["copy_files"]
        .as_array()
        .ok_or_else(|| invalid("exe_manifest_invalid"))?;
    require(
        copy.len() == payload.len()
            && copy
                .iter()
                .zip(&payload)
                .all(|(v, p)| v.as_str() == Some(p.path.as_str())),
        "exe_manifest_invalid",
    )?;
    let payload_paths: BTreeSet<_> = payload.iter().map(|p| p.path.as_str()).collect();
    let mut file_conditions = Vec::new();
    let mut condition_paths = BTreeSet::new();
    for entry in behavior["file_conditions"]
        .as_array()
        .ok_or_else(|| invalid("exe_manifest_invalid"))?
    {
        exact(entry, &["path", "condition"])?;
        let path = text(entry, "path")?;
        require(
            paths.contains(&path.nfc().default_case_fold().collect::<String>())
                && payload_paths.contains(path)
                && condition_paths.insert(path.to_owned()),
            "exe_manifest_invalid",
        )?;
        file_conditions.push((
            path.to_owned(),
            self::condition(&entry["condition"], &mut MAX_NODES.clone(), 0)?,
        ));
    }
    let payload_sha256 = canonical::canonical_sha256(&payload);
    Ok(PreparedReport {
        binding: ReportBinding {
            exe_sha256: exe_sha256.into(),
            exe_size,
            metadata_sha256: metadata_sha256.into(),
            behavior_sha256: behavior_sha256.into(),
            payload_sha256,
        },
        payload,
        file_conditions,
        condition,
        condition_sha256: canonical::canonical_sha256(&behavior["condition"]),
    })
}
/// The frozen worker report binding has ASCII escaping, separate from content ID.
pub fn worker_behavior_sha256(v: &Value) -> Result<String> {
    fn encode(v: &Value, o: &mut String) -> Result<()> {
        match v {
            Value::String(s) => {
                o.push('"');
                for c in s.chars() {
                    match c {
                        '"' => o.push_str("\\\""),
                        '\\' => o.push_str("\\\\"),
                        '\n' => o.push_str("\\n"),
                        '\r' => o.push_str("\\r"),
                        '\t' => o.push_str("\\t"),
                        '\u{8}' => o.push_str("\\b"),
                        '\u{c}' => o.push_str("\\f"),
                        c if (c as u32) < 32 || (c as u32) > 126 => {
                            for u in c.encode_utf16(&mut [0; 2]) {
                                o.push_str(&format!("\\u{u:04x}"));
                            }
                        }
                        c => o.push(c),
                    }
                }
                o.push('"');
            }
            Value::Array(a) => {
                o.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    encode(v, o)?;
                }
                o.push(']');
            }
            Value::Object(a) => {
                o.push('{');
                let mut f: Vec<_> = a.iter().collect();
                f.sort_by(|a, b| a.0.cmp(b.0));
                for (i, (k, v)) in f.into_iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    encode(&Value::String(k.clone()), o)?;
                    o.push(':');
                    encode(v, o)?;
                }
                o.push('}');
            }
            Value::Number(n) => {
                require(n.is_i64() || n.is_u64(), "exe_worker_invalid_response")?;
                o.push_str(&n.to_string());
            }
            Value::Bool(v) => o.push_str(if *v { "true" } else { "false" }),
            Value::Null => o.push_str("null"),
        }
        Ok(())
    }
    let mut bytes = String::new();
    encode(v, &mut bytes)?;
    Ok(format!("{:x}", Sha256::digest(bytes.as_bytes())))
}

fn file_policy(row: &Value, _installer: &str) -> Result<()> {
    exact(
        row,
        &[
            "attributes",
            "compressed",
            "condition",
            "crc32",
            "creation_time",
            "description",
            "destination",
            "extra",
            "font",
            "forced_attributes",
            "icon",
            "index",
            "install_type",
            "modified_time",
            "notes",
            "packages",
            "packed_size",
            "payload_offset",
            "runtime_folder",
            "serialized_record",
            "shortcuts",
            "source",
            "source_directory",
            "suffix",
            "unpacked_size",
            "unparsed_inter_record_framing",
            "unreviewed_fields",
            "use_original_attributes",
        ],
    )?;
    require(
        row["compressed"].is_boolean(),
        "exe_file_schema_unsupported",
    )?;
    let fields = row["unreviewed_fields"]
        .as_array()
        .ok_or_else(|| invalid("exe_file_schema_unsupported"))?;
    require(fields.len() == 11, "exe_file_schema_unsupported")?;
    let mut decoded = Vec::new();
    for field in fields {
        exact(field, &["base64", "offset", "sha256", "size"])?;
        let bytes = STANDARD
            .decode(text(field, "base64")?)
            .map_err(|_| invalid("exe_file_schema_unsupported"))?;
        require(
            bytes.len() as u64 == uint(field, "size")?
                && format!("{:x}", Sha256::digest(&bytes)) == text(field, "sha256")?,
            "exe_file_schema_unsupported",
        )?;
        decoded.push(bytes);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::facts::{BuildFacts, DlcFacts, Fact, InstanceFacts, ManifestFacts};
    use super::*;
    fn golden() -> Value {
        serde_json::from_str(include_str!(
            "../../../installer-format/tests/fixtures/sf/worker-golden.json"
        ))
        .unwrap()
    }
    fn response(report: Value) -> Value {
        json!({"schema_version":1,"action":"analyze_behavior","exe_sha256":report["exe"]["sha256"],"report":report,"installable":false})
    }
    pub(super) fn initial() -> Value {
        response(golden()["cases"][0]["report"].clone())
    }
    pub(super) fn admit(r: &Value) -> Result<PreparedReport> {
        validate_response(
            r,
            r["report"]["exe"]["sha256"].as_str().unwrap(),
            r["report"]["exe"]["size"].as_u64().unwrap(),
        )
    }
    fn gate(v: Value) -> PreparedReport {
        let mut r = initial();
        r["report"]["behavior"]["condition"] = v;
        r["report"]["behavior_sha256"] =
            json!(worker_behavior_sha256(&r["report"]["behavior"]).unwrap());
        admit(&r).unwrap()
    }
    pub(super) fn facts(root: &Path) -> GameFacts {
        GameFacts {
            instance: InstanceFacts {
                instance_id: "trusted-instance".into(),
                appid: 42,
                library_path: root.to_owned(),
                install_path: root.to_owned(),
            },
            manifest: Fact::evidence(ManifestFacts {
                sha256: "a".repeat(64),
                installdir: "game".into(),
                stateflags: Some("4".into()),
                buildid: Some("1".into()),
            }),
            build: Fact::evidence(BuildFacts {
                buildid: Some("1".into()),
                verified: false,
            }),
            dlc: Fact::evidence(DlcFacts { installed: vec![] }),
            baselines: vec![],
        }
    }

    #[test]
    fn member_conditions_select_and_snapshot_actual_files() {
        let mut r = initial();
        let mut second = r["report"]["files"][0].clone();
        second["destination"] = json!("optional.bin");
        r["report"]["files"].as_array_mut().unwrap().push(second);
        let first_path = r["report"]["files"][0]["destination"].clone();
        r["report"]["behavior"]["copy_files"] = json!([first_path, "optional.bin"]);
        r["report"]["behavior"]["condition"] = json!(true);
        r["report"]["behavior"]["file_conditions"] = json!([
            {"path":"optional.bin","condition":["symbol","exists","dlc/marker.dat"]}
        ]);
        r["report"]["behavior_sha256"] =
            json!(worker_behavior_sha256(&r["report"]["behavior"]).unwrap());
        let root = tempfile::tempdir().unwrap();
        let game = facts(root.path());
        let cancel = AtomicBool::new(false);
        let mut absent = admit(&r).unwrap();
        absent.select_payload(&game, &cancel).unwrap();
        assert_eq!(absent.payload().len(), 1);
        let snapshot = absent.evaluate(&game, &cancel).unwrap();
        assert!(snapshot.satisfied);
        assert!(snapshot
            .files
            .iter()
            .any(|f| f.path == "dlc/marker.dat" && f.observed.is_none()));
        fs::create_dir(root.path().join("dlc")).unwrap();
        fs::write(root.path().join("dlc/marker.dat"), "present").unwrap();
        assert_eq!(
            absent.evaluate(&game, &cancel).unwrap_err().to_string(),
            "exe_selection_changed"
        );
        let mut present = admit(&r).unwrap();
        present.select_payload(&game, &cancel).unwrap();
        assert_eq!(present.payload().len(), 2);
        assert!(present.evaluate(&game, &cancel).unwrap().satisfied);
    }

    #[test]
    fn frozen_worker_reports_are_admitted() {
        for c in golden()["cases"].as_array().unwrap() {
            let r = response(c["report"].clone());
            let p = admit(&r).unwrap();
            assert_eq!(p.payload().len(), 1);
            assert_eq!(p.payload()[0].path, c["conversion"]["files"][0]["path"]);
        }
    }
    #[test]
    fn nested_unknown_keys_are_rejected() {
        let original = initial();
        let mut r = original.clone();
        r["report"]["script_inventory"]["injected"] = json!(1);
        assert!(admit(&r).is_err(), "script_inventory");
        let mut r = original.clone();
        r["report"]["certificate"] = json!({
            "offset": 0,
            "sha256": "a".repeat(64),
            "signature_verified": false,
            "size": 8,
            "injected": 1
        });
        assert!(admit(&r).is_err(), "certificate");
        let mut r = original.clone();
        r["report"]["diagnostics"][0]["injected"] = json!(1);
        assert!(admit(&r).is_err(), "diagnostics");
        // A bounded certificate object with only known keys is still admitted.
        let mut r = original.clone();
        r["report"]["certificate"] = json!({
            "offset": 0,
            "sha256": "a".repeat(64),
            "signature_verified": false,
            "size": 8
        });
        assert!(admit(&r).is_ok(), "valid certificate");
    }
    #[test]
    fn tampered_binding_and_unknown_actions_are_rejected() {
        let original = initial();
        for (field, value) in [
            ("installable", json!(true)),
            ("semantics_reviewed", json!(true)),
            ("capability", json!("discover_only")),
            ("blockers", json!([])),
            ("behavior_sha256", json!("a".repeat(64))),
        ] {
            let mut r = original.clone();
            r["report"][field] = value;
            assert!(admit(&r).is_err(), "{field}");
        }
        let mut r = original.clone();
        r["report"]["behavior"]["actions"] = json!(["execute"]);
        assert!(admit(&r).is_err());
        let mut r = original.clone();
        r["report"]["script_inventory"]["coverage"]["complete"] = json!(false);
        assert!(admit(&r).is_err());
        let mut r = original.clone();
        r["report"]["files"][0]["extra"] = json!("Custom build setting");
        assert!(admit(&r).is_ok());
        let mut r = original.clone();
        r["report"]["exe"]["size"] = json!(1);
        assert!(validate_response(
            &r,
            original["exe_sha256"].as_str().unwrap(),
            original["report"]["exe"]["size"].as_u64().unwrap()
        )
        .is_err());
        for node in [
            json!(["symbol", "or", true, ["symbol", "execute", "cmd"]]),
            json!(["symbol", "exists", "../escape"]),
            json!(["symbol", "==", true, 1]),
            json!(["symbol", "crc32", "game.dat"]),
            json!(["symbol", "locate", "registry"]),
        ] {
            let mut r = original.clone();
            r["report"]["behavior"]["condition"] = node;
            r["report"]["behavior_sha256"] =
                json!(worker_behavior_sha256(&r["report"]["behavior"]).unwrap());
            assert!(admit(&r).is_err());
        }
    }
    #[test]
    fn missing_files_and_short_circuit_branches_are_all_recorded() {
        let t = tempfile::tempdir().unwrap();
        let game = facts(t.path());
        let cancel = AtomicBool::new(false);
        let p = gate(json!([
            "symbol",
            "or",
            true,
            ["symbol", "exists", "missing.dat"]
        ]));
        let a = p.evaluate(&game, &cancel).unwrap();
        assert!(a.satisfied);
        assert_eq!(a.files.len(), 1);
        assert_eq!(a.files[0].observed, None);
        fs::write(t.path().join("missing.dat"), b"new").unwrap();
        let b = p.evaluate(&game, &cancel).unwrap();
        assert_ne!(a.sha256(), b.sha256());
        assert_eq!(b.sha256(), p.evaluate(&game, &cancel).unwrap().sha256());
        let p = gate(json!([
            "symbol",
            "not",
            ["symbol", "==", ["symbol", "crc32", "absent.dat"], 123]
        ]));
        assert!(!p.evaluate(&game, &cancel).unwrap().satisfied);
        let p = gate(json!(["symbol", "not", ["symbol", "exists", "absent.dat"]]));
        assert!(p.evaluate(&game, &cancel).unwrap().satisfied);
    }
    #[test]
    fn full_boolean_crc_and_order() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("baseline.dat"), b"123456789").unwrap();
        let game = facts(t.path());
        let cancel = AtomicBool::new(false);
        for op in ["==", "<=", ">="] {
            let p = gate(json!([
                "symbol",
                op,
                ["symbol", "crc32", "baseline.dat"],
                0xcbf43926u32
            ]));
            assert!(p.evaluate(&game, &cancel).unwrap().satisfied);
        }
        let p = gate(json!([
            "symbol",
            "and",
            ["symbol", "exists", "baseline.dat"],
            [
                "symbol",
                "not",
                [
                    "symbol",
                    "~=",
                    0xcbf43926u32,
                    ["symbol", "crc32", "baseline.dat"]
                ]
            ]
        ]));
        assert!(p.evaluate(&game, &cancel).unwrap().satisfied);
        fs::write(t.path().join("baseline.dat"), b"changed").unwrap();
        assert!(!p.evaluate(&game, &cancel).unwrap().satisfied);
    }
    #[test]
    fn node_depth_limits_are_checked_before_evaluation() {
        let mut node = json!(true);
        for _ in 0..98 {
            node = json!(["symbol", "not", node]);
        }
        let mut budget = MAX_NODES;
        assert!(condition(&node, &mut budget, 0).is_err());
        let mut budget = 1;
        assert!(condition(&json!(["symbol", "and", true, false]), &mut budget, 0).is_err());
    }
    #[test]
    fn unicode_worker_digest_and_rust_content_stability() {
        let r = initial();
        let p = admit(&r).unwrap();
        assert_eq!(
            p.content_id(42, "src", "page", "zh-CN", "base").unwrap(),
            p.content_id(42, "src", "page", "zh-CN", "base").unwrap()
        );
        assert_ne!(
            worker_behavior_sha256(&json!({"text":"中文😀"})).unwrap(),
            canonical::canonical_sha256(&json!({"text":"中文😀"}))
        );
    }
}
