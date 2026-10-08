//! Pure admission of the fixed worker report. Never parses PE or executes Lua.
use super::{
    facts::{self, FileDigest, GameFacts},
    invalid, Result,
};
use crate::{install::Transaction, sources::canonical};
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
pub const MAX_REPORT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_PAYLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_NODES: usize = 8192;
const MAX_DEPTH: usize = 96;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadEntry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
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
/// Construct only from core-loaded successful authorization and committed journal.
#[derive(Debug, Clone)]
pub struct ReinstallEvidence {
    instance_id: String,
    content_id: String,
    behavior_sha256: String,
    binding: ReportBinding,
    owned: BTreeMap<String, String>,
    original: BTreeMap<String, Option<FileDigest>>,
}
impl ReinstallEvidence {
    pub fn from_committed(
        transaction: &Transaction,
        authorization: &super::authorization::Authorization,
        original: &ConditionSnapshot,
    ) -> Result<Self> {
        super::authorization::validate_stored(authorization)?;
        let content_id = authorization
            .content
            .content_id
            .as_ref()
            .ok_or_else(|| invalid("exe_reinstall_evidence_invalid"))?;
        require(
            transaction.state == "committed"
                && transaction.error.is_none()
                && transaction.transaction_id == transaction.plan_id
                && transaction.plan_id == authorization.plan_id
                && transaction.instance_id == authorization.instance_id
                && transaction.release_id == *content_id
                && authorization.release_id == *content_id
                && original.instance_id == transaction.instance_id
                && original.satisfied
                && !authorization.revoked
                && authorization.conditions_sha256 == original.sha256(),
            "exe_reinstall_evidence_invalid",
        )?;
        let mut owned = BTreeMap::new();
        for mapping in &authorization.content.mappings {
            relative_path(&mapping.destination)?;
            hash(&mapping.installed_sha256)?;
            require(
                owned
                    .insert(
                        mapping.destination.clone(),
                        mapping.installed_sha256.clone(),
                    )
                    .is_none(),
                "exe_reinstall_evidence_invalid",
            )?;
        }
        let mut previous = BTreeMap::new();
        for fact in &original.evaluation_files {
            relative_path(&fact.path)?;
            if let Some(d) = &fact.observed {
                hash(&d.sha256)?;
            }
            require(
                previous
                    .insert(fact.path.clone(), fact.observed.clone())
                    .is_none(),
                "exe_reinstall_evidence_invalid",
            )?;
        }
        let c = &authorization.content;
        let binding = ReportBinding {
            exe_sha256: c
                .exe_sha256
                .clone()
                .ok_or_else(|| invalid("exe_reinstall_evidence_invalid"))?,
            exe_size: c
                .exe_size
                .ok_or_else(|| invalid("exe_reinstall_evidence_invalid"))?,
            metadata_sha256: c
                .metadata_sha256
                .clone()
                .ok_or_else(|| invalid("exe_reinstall_evidence_invalid"))?,
            behavior_sha256: original.behavior_sha256.clone(),
            payload_sha256: c
                .payload_sha256
                .clone()
                .ok_or_else(|| invalid("exe_reinstall_evidence_invalid"))?,
        };
        Ok(Self {
            instance_id: transaction.instance_id.clone(),
            content_id: content_id.clone(),
            behavior_sha256: original.behavior_sha256.clone(),
            binding,
            owned,
            original: previous,
        })
    }
}
impl PreparedReport {
    pub fn binding(&self) -> &ReportBinding {
        &self.binding
    }
    pub fn payload(&self) -> &[PayloadEntry] {
        &self.payload
    }
    pub fn condition_sha256(&self) -> &str {
        &self.condition_sha256
    }
    pub fn payload_bytes(&self) -> u64 {
        self.payload.iter().map(|p| p.size).sum()
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
                && !language.is_empty()
                && ["base", "dlc"].contains(&variant),
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
        self.evaluate_with_reinstall(game, cancel, None, None)
    }
    pub fn evaluate_with_reinstall(
        &self,
        game: &GameFacts,
        cancel: &AtomicBool,
        content_id: Option<&str>,
        evidence: Option<&ReinstallEvidence>,
    ) -> Result<ConditionSnapshot> {
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("download_cancelled"));
        }
        if let Some(f) = game.failures().first() {
            return Err(invalid(f.code.clone()));
        }
        require(
            game.instance.install_path.is_absolute()
                && super::zip_recipe::no_links(&game.instance.install_path)
                && game.instance.install_path.is_dir(),
            "exe_game_identity_changed",
        )?;
        if let Some(e) = evidence {
            require(
                e.instance_id == game.instance.instance_id
                    && content_id == Some(e.content_id.as_str())
                    && e.behavior_sha256 == self.binding.behavior_sha256
                    && e.binding == self.binding
                    && e.owned.len() == self.payload.len()
                    && self
                        .payload
                        .iter()
                        .all(|p| e.owned.get(&p.path) == Some(&p.sha256)),
                "exe_reinstall_evidence_invalid",
            )?;
        }
        let mut paths = BTreeSet::new();
        self.condition.paths(&mut paths);
        let mut files = Vec::new();
        // Record all static references, including skipped boolean branches and absence.
        for path in paths {
            if cancel.load(Ordering::Relaxed) {
                return Err(invalid("download_cancelled"));
            }
            let target = game.instance.install_path.join(&path);
            require(super::zip_recipe::no_links(&target), "linked_target")?;
            let observed = match fs::symlink_metadata(&target) {
                Ok(m) if m.is_file() => Some(facts::digest_file(
                    &target,
                    facts::MAX_BASELINE_BYTES,
                    cancel,
                )?),
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
        let actual: BTreeMap<_, _> = files
            .iter()
            .map(|f| (f.path.clone(), f.observed.clone()))
            .collect();
        let mut effective = actual.clone();
        if let Some(e) = evidence {
            for (path, sha) in &e.owned {
                // External edits never obtain baseline exemptions.
                if actual
                    .get(path)
                    .and_then(Option::as_ref)
                    .is_some_and(|d| &d.sha256 == sha)
                {
                    if let Some(o) = e.original.get(path) {
                        effective.insert(path.clone(), o.clone());
                    }
                }
            }
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
                    .starts_with(".steam-patch-"),
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
    require(
        exe_size > 0 && exe_size <= MAX_PAYLOAD_BYTES,
        "exe_size_limit",
    )?;
    bounded_json(response, 0, &mut 1_000_000usize)?;
    require(
        serde_json::to_vec(response)
            .map_err(|_| invalid("exe_worker_invalid_response"))?
            .len()
            < MAX_REPORT_BYTES,
        "exe_report_size_limit",
    )?;
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
        report["metadata"]["offset"] == 0
            && uint(&report["metadata"], "size")? > 0
            && uint(&report["metadata"], "size")? <= MAX_REPORT_BYTES as u64,
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
            "terms",
            "coverage",
        ],
    )?;
    require(
        behavior["schema_version"] == 1
            && behavior["policy"] == "sf9-behavior-v1"
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
        report["script_inventory"]["decoder"] == "sf9-graph-v1"
            && uint(&report["script_inventory"]["coverage"], "bytes")?
                == uint(&report["metadata"], "size")?,
        "exe_report_coverage_incomplete",
    )?;
    for k in ["events", "includes", "steps"] {
        require(
            uint(&behavior["coverage"], k)? <= 200_000,
            "exe_report_coverage_incomplete",
        )?;
    }
    require(
        behavior["terms"]
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_string)),
        "exe_worker_invalid_response",
    )?;
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
        !rows.is_empty() && rows.len() <= MAX_NODES,
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
        require(
            size <= MAX_PAYLOAD_BYTES
                && uint(&row["payload"], "size")? == size
                && uint(&row["payload"], "crc32")? == uint(row, "crc32")?
                && uint(row, "crc32")? <= u32::MAX as u64,
            "exe_manifest_invalid",
        )?;
        let sha256 = text(&row["payload"], "sha256")?;
        hash(sha256)?;
        exact(&row["payload"], &["size", "crc32", "sha256"])?;
        require(
            uint(row, "payload_offset")?
                .checked_add(uint(row, "packed_size")?)
                .is_some_and(|end| end <= exe_size),
            "exe_manifest_invalid",
        )?;
        total = total
            .checked_add(size)
            .ok_or_else(|| invalid("exe_manifest_invalid"))?;
        require(
            total <= MAX_PAYLOAD_BYTES
                && paths.insert(path.nfc().default_case_fold().collect::<String>()),
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
            sha256: sha256.into(),
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
        condition,
        condition_sha256: canonical::canonical_sha256(&behavior["condition"]),
    })
}
fn bounded_json(v: &Value, d: usize, b: &mut usize) -> Result<()> {
    require(d <= 128 && *b > 0, "exe_report_size_limit")?;
    *b -= 1;
    match v {
        Value::Array(a) => {
            for v in a {
                bounded_json(v, d + 1, b)?;
            }
        }
        Value::Object(o) => {
            for v in o.values() {
                bounded_json(v, d + 1, b)?;
            }
        }
        Value::Number(n) => require(n.is_i64() || n.is_u64(), "exe_worker_invalid_response")?,
        _ => {}
    }
    Ok(())
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
    bounded_json(v, 0, &mut 1_000_000usize)?;
    let mut bytes = String::new();
    encode(v, &mut bytes)?;
    Ok(format!("{:x}", Sha256::digest(bytes.as_bytes())))
}

fn file_policy(row: &Value, installer: &str) -> Result<()> {
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
            "payload",
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
        row["runtime_folder"] == "Archive"
            && row["condition"] == ""
            && row["install_type"] == "All"
            && row["extra"] == "None"
            && row["packages"] == json!([]),
        "exe_file_selection_unsupported",
    )?;
    require(
        row["shortcuts"]
            .as_array()
            .is_some_and(|a| a.len() == 5 && a.iter().all(|v| v == ""))
            && row["icon"] == ""
            && row["font"] == "",
        "exe_file_registration_unsupported",
    )?;
    require(
        matches!(row["attributes"].as_u64(), Some(0 | 32))
            && row["use_original_attributes"] == 1
            && row["forced_attributes"] == 0
            && row["compressed"].is_boolean(),
        "exe_file_attributes_unsupported",
    )?;
    let path = text(row, "destination")?
        .nfc()
        .default_case_fold()
        .collect::<String>();
    require(
        !path
            .split('/')
            .any(|p| ["save", "saves", "savedata", "userdata", "user_data"].contains(&p))
            && !path.ends_with(".sav")
            && !path.ends_with(".save"),
        "exe_user_data_target_unsupported",
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
    let stamp = [10, 0, 0, 128];
    let mut mask = vec![0, 128];
    mask.extend(std::iter::repeat_n(
        255,
        if installer.ends_with("9.5") { 30 } else { 36 },
    ));
    let mut flag = vec![2];
    flag.extend([0; 10]);
    require(
        decoded[0] == [1, 0]
            && decoded[1] == stamp
            && decoded[2].len() == 16
            && decoded[2][..4] == stamp
            && decoded[2][12..] == stamp
            && decoded[3] == vec![0; 25]
            && decoded[4] == flag
            && decoded[5] == [0]
            && decoded[6] == [0; 8]
            && decoded[7] == vec![0; if installer.ends_with("9.7") { 4 } else { 3 }]
            && decoded[8] == [0, 0, 0, 0, 232, 3, 0, 0, 0, 0]
            && decoded[9] == mask
            && decoded[10] == [1, 0],
        "exe_file_actions_or_options_unsupported",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::facts::{
        BuildFacts, DlcFacts, Fact, InstanceFacts, LanguageFacts, ManifestFacts,
    };
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
                buildid: "1".into(),
                verified: false,
            }),
            language: Fact::evidence(LanguageFacts {
                user: Some("schinese".into()),
                mounted: Some("schinese".into()),
                selected: Some("schinese".into()),
            }),
            dlc: Fact::evidence(DlcFacts { installed: vec![] }),
            baselines: vec![],
        }
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
        r["report"]["files"][0]["extra"] = json!("Delete");
        assert!(admit(&r).is_err());
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

#[cfg(test)]
mod reinstall_tests {
    use super::*;
    use crate::{
        catalog::FileMapping,
        installer::authorization::{self, AuthorizeInput, ContentBinding, PolicyVersions},
    };
    fn authorization(
        p: &PreparedReport,
        g: &GameFacts,
        s: &ConditionSnapshot,
        content: &str,
    ) -> authorization::Authorization {
        let maps: Vec<_> = p
            .payload()
            .iter()
            .map(|p| FileMapping {
                source: p.path.clone(),
                destination: p.path.clone(),
                original_sha256: None,
                installed_sha256: p.sha256.clone(),
            })
            .collect();
        let c = ContentBinding::publisher_exe(
            content,
            &p.binding.exe_sha256,
            p.binding.exe_size,
            &p.binding.metadata_sha256,
            &p.binding.payload_sha256,
            &"a".repeat(64),
            123,
            Some("sf9-audit-v3"),
            &maps,
        );
        authorization::authorize(AuthorizeInput {
            release_id: content,
            selection_id: None,
            source_id: "source",
            source_record_id: Some("page"),
            candidate_sha256: &"a".repeat(64),
            rule_fingerprint: &"b".repeat(64),
            settings_revision: 1,
            conditions_sha256: &s.sha256(),
            content: c,
            policy: PolicyVersions::default(),
            plan_id: "11111111-1111-1111-1111-111111111111",
            task_id: None,
            request_id: None,
            facts: g.clone(),
            now: 1,
            ttl_seconds: 900,
        })
        .unwrap()
    }
    #[test]
    fn only_owned_matching_bytes_can_reuse_original_conditions() {
        let mut r = super::tests::initial();
        r["report"]["behavior"]["condition"] = json!([
            "symbol",
            "==",
            ["symbol", "crc32", "payload.bin"],
            0xcbf43926u32
        ]);
        r["report"]["behavior_sha256"] =
            json!(worker_behavior_sha256(&r["report"]["behavior"]).unwrap());
        let patched = b"patched payload";
        r["report"]["files"][0]["payload"]["sha256"] =
            json!(format!("{:x}", Sha256::digest(patched)));
        let p = super::tests::admit(&r).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let game = super::tests::facts(temp.path());
        let cancel = AtomicBool::new(false);
        fs::write(temp.path().join("payload.bin"), b"123456789").unwrap();
        let original = p.evaluate(&game, &cancel).unwrap();
        assert!(original.satisfied);
        let content = p.content_id(42, "source", "page", "zh-CN", "base").unwrap();
        let auth = authorization(&p, &game, &original, &content);
        let tx = Transaction {
            transaction_id: auth.plan_id.clone(),
            plan_id: auth.plan_id.clone(),
            instance_id: game.instance.instance_id.clone(),
            release_id: content.clone(),
            state: "committed".into(),
            error: None,
            created_at: 1,
            updated_at: 2,
            has_replacements: false,
        };
        let evidence = ReinstallEvidence::from_committed(&tx, &auth, &original).unwrap();
        fs::write(temp.path().join("payload.bin"), patched).unwrap();
        assert!(!p.evaluate(&game, &cancel).unwrap().satisfied);
        let second = p
            .evaluate_with_reinstall(&game, &cancel, Some(&content), Some(&evidence))
            .unwrap();
        assert!(second.satisfied);
        assert_ne!(second.files, second.evaluation_files);
        let second_auth = authorization(&p, &game, &second, &content);
        let second_evidence =
            ReinstallEvidence::from_committed(&tx, &second_auth, &second).unwrap();
        assert!(
            p.evaluate_with_reinstall(&game, &cancel, Some(&content), Some(&second_evidence))
                .unwrap()
                .satisfied
        );
        fs::write(temp.path().join("payload.bin"), b"external changed").unwrap();
        assert!(
            !p.evaluate_with_reinstall(&game, &cancel, Some(&content), Some(&evidence))
                .unwrap()
                .satisfied
        );
        let mut wrong = tx.clone();
        wrong.state = "rolled_back".into();
        assert!(ReinstallEvidence::from_committed(&wrong, &auth, &original).is_err());
        wrong = tx.clone();
        wrong.instance_id = "other".into();
        assert!(ReinstallEvidence::from_committed(&wrong, &auth, &original).is_err());
        let mut wrong = auth.clone();
        wrong.conditions_sha256 = "a".repeat(64);
        assert!(ReinstallEvidence::from_committed(&tx, &wrong, &original).is_err());
        assert!(p
            .evaluate_with_reinstall(&game, &cancel, Some("wrong"), Some(&evidence))
            .is_err());
    }
}
