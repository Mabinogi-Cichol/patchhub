//! Bounded, read-only Steam instance facts and reviewed preconditions
//! (design §4.1/§5.2; port of `exe_facts.py::read_file`/`game_facts`).
//!
//! Everything here runs **outside** the sandboxed worker and never writes to the
//! game tree. Each observed item carries its own evidence and, when a check
//! fails, a [`FactFailure`] with a stable code. The resulting [`GameFacts`] is a
//! deterministic snapshot used both to evaluate conditions and to re-verify an
//! [`super::authorization::Authorization`] before apply.
//!
//! File identity is checked before/after every read, as in the Python reference:
//! the descriptor is compared to itself and to a fresh path stat, with the
//! platform-specific metadata difference documented in [`digest_file`].

use super::zip_recipe::no_links;
use super::{invalid, Result};
use crate::keyvalues::{self, Value};
use crate::steam::GameInstall;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Maximum Steam app-manifest size (`exe_facts.py::MAX_MANIFEST`).
pub const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
/// Read chunk size (`exe_facts.py` reads 1 MiB at a time).
const CHUNK_BYTES: usize = 1024 * 1024;

/// Steam UI language key to the value persisted in `appmanifest_*.acf`
/// (`exe_facts.py::LANGUAGES`).
const LANGUAGES: &[(&str, &str)] = &[
    ("en", "english"),
    ("zh-CN", "schinese"),
    ("zh-TW", "tchinese"),
    ("ja", "japanese"),
    ("ko", "koreana"),
    ("de", "german"),
    ("fr", "french"),
    ("es", "spanish"),
    ("ru", "russian"),
];

/// A single item's failure: stable code plus bounded human-readable detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FactFailure {
    pub code: String,
    pub detail: String,
}

impl FactFailure {
    pub fn new(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: detail.into(),
        }
    }
}

/// Evidence when available, failure reason when the check did not pass.
///
/// A successful read has `evidence: Some(_)` and `failure: None`; a failed read
/// may still expose partial evidence with a `Some(_)` failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fact<T> {
    pub evidence: Option<T>,
    pub failure: Option<FactFailure>,
}

impl<T> Fact<T> {
    pub fn evidence(value: T) -> Self {
        Self {
            evidence: Some(value),
            failure: None,
        }
    }

    pub fn failed(failure: FactFailure) -> Self {
        Self {
            evidence: None,
            failure: Some(failure),
        }
    }

    pub fn failed_code(code: &str, detail: impl Into<String>) -> Self {
        Self::failed(FactFailure::new(code, detail))
    }

    pub fn is_ok(&self) -> bool {
        self.failure.is_none()
    }
}

/// `sha256` + `crc32` + size of one regular file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileDigest {
    pub sha256: String,
    pub crc32: u32,
    pub size: u64,
}

/// Instance identity and resolved paths (always known before any read).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstanceFacts {
    pub instance_id: String,
    pub appid: u32,
    pub library_path: PathBuf,
    pub install_path: PathBuf,
}

/// Manifest identity and the raw scalar fields used for re-verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManifestFacts {
    pub sha256: String,
    pub installdir: String,
    pub stateflags: Option<String>,
    pub buildid: Option<String>,
}

/// Build identity and whether it is in the reviewed build set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BuildFacts {
    pub buildid: Option<String>,
    pub verified: bool,
}

/// `UserConfig` / `MountedConfig` language state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LanguageFacts {
    pub user: Option<String>,
    pub mounted: Option<String>,
    pub selected: Option<String>,
}

/// Installed depot/DLC evidence (installation evidence, never ownership proof).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DlcFacts {
    pub installed: Vec<u32>,
}

/// One reviewed baseline file and its observed state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BaselineFact {
    pub path: String,
    pub expected_sha256: String,
    pub expected_crc32: Option<u32>,
    pub observed: Option<FileDigest>,
    pub failure: Option<FactFailure>,
}

/// Structured, deterministic snapshot of the current instance (design §5.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GameFacts {
    pub instance: InstanceFacts,
    pub manifest: Fact<ManifestFacts>,
    pub build: Fact<BuildFacts>,
    pub language: Fact<LanguageFacts>,
    pub dlc: Fact<DlcFacts>,
    pub baselines: Vec<BaselineFact>,
}

impl GameFacts {
    /// All failures in snapshot order, for diagnostics and authorization gating.
    pub fn failures(&self) -> Vec<&FactFailure> {
        let mut failures = Vec::new();
        for fact in [
            self.manifest.failure.as_ref(),
            self.build.failure.as_ref(),
            self.language.failure.as_ref(),
            self.dlc.failure.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            failures.push(fact);
        }
        for baseline in &self.baselines {
            if let Some(failure) = &baseline.failure {
                failures.push(failure);
            }
        }
        failures
    }

    /// Whether every observed item passed its reviewed expectation.
    pub fn is_valid(&self) -> bool {
        self.failures().is_empty()
    }

    fn all_failed(instance: InstanceFacts, failure: FactFailure) -> Self {
        Self {
            instance,
            manifest: Fact::failed(failure.clone()),
            build: Fact::failed(failure.clone()),
            language: Fact::failed(failure.clone()),
            dlc: Fact::failed(failure),
            baselines: Vec::new(),
        }
    }
}

/// Reviewed expectation for one instance (`exe_facts.py::profile` subset).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FactsExpectation {
    pub appid: u32,
    pub language: String,
    pub verified_buildids: Vec<String>,
    /// Whether the manifest language is a gate. Publisher sources keep the
    /// gate; index sources record the selected language as evidence only.
    #[serde(default = "default_true")]
    pub require_language: bool,
    #[serde(default)]
    pub requires_dlc: Vec<u32>,
    #[serde(default)]
    pub excludes_dlc: Vec<u32>,
    #[serde(default)]
    pub baselines: Vec<BaselineExpectation>,
}

fn default_true() -> bool {
    true
}

/// One baseline to hash and compare against the reviewed value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BaselineExpectation {
    pub path: String,
    pub sha256: String,
    #[serde(default)]
    pub crc32: Option<u32>,
}

/// Streams a regular unlinked file to EOF with a fixed buffer and returns
/// `sha256`/`crc32`/size, checking identity before and after the read
/// (`exe_facts.py::read_file`). Digest-only reads have no artificial size cap;
/// the file's own length and identity checks bound the work.
pub fn digest_file(path: &Path, cancel: &AtomicBool) -> Result<FileDigest> {
    read_bounded(path, None, cancel, false).map(|(digest, _)| digest)
}

/// Like [`digest_file`] but also returns the bytes (`contents=True`). `limit` is
/// a real caller memory bound for the returned buffer.
pub fn read_file(path: &Path, limit: u64, cancel: &AtomicBool) -> Result<(FileDigest, Vec<u8>)> {
    read_bounded(path, Some(limit), cancel, true)
        .map(|(digest, bytes)| (digest, bytes.unwrap_or_default()))
}

/// Read a trusted, core-/worker-produced report/condition file in full with no
/// arbitrary fixed byte cap; memory is bounded by the file's own checked length
/// (design D08: a report byte ceiling with no independent resource basis is
/// deleted, not enlarged). Identity, digest and cancellation checks are kept.
pub fn read_file_full(path: &Path, cancel: &AtomicBool) -> Result<(FileDigest, Vec<u8>)> {
    read_bounded(path, None, cancel, true)
        .map(|(digest, bytes)| (digest, bytes.unwrap_or_default()))
}

fn read_bounded(
    path: &Path,
    limit: Option<u64>,
    cancel: &AtomicBool,
    contents: bool,
) -> Result<(FileDigest, Option<Vec<u8>>)> {
    if !no_links(path) {
        return Err(invalid("exe_linked_game_file"));
    }
    let mut file = open_nofollow(path)?;
    let before = file.metadata()?;
    #[cfg(windows)]
    let identity_before = windows_handle_identity(&file)?;
    if !before.is_file() || limit.is_some_and(|limit| before.len() > limit) {
        return Err(invalid("exe_game_file_limit"));
    }

    let mut hasher = Sha256::new();
    let mut crc: u32 = !0;
    let mut size: u64 = 0;
    let mut pieces = contents.then(Vec::new);
    let mut buffer = vec![0u8; CHUNK_BYTES];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("download_cancelled"));
        }
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| invalid("exe_game_file_limit"))?;
        if limit.is_some_and(|limit| size > limit) {
            return Err(invalid("exe_game_file_limit"));
        }
        hasher.update(&buffer[..count]);
        crc = crc32_update(crc, &buffer[..count]);
        if let Some(pieces) = pieces.as_mut() {
            pieces.extend_from_slice(&buffer[..count]);
        }
    }

    let after = file.metadata()?;
    // Re-stat through the path after the read. `no_links` is re-checked because
    // the final component could have been replaced mid-read.
    if !no_links(path) {
        return Err(invalid("exe_game_file_changed"));
    }
    #[cfg(not(windows))]
    let current = fs::metadata(path)?;

    #[cfg(not(windows))]
    let (identity_before, identity_after, identity_current) = (
        file_identity(&before),
        file_identity(&after),
        file_identity(&current),
    );
    #[cfg(windows)]
    let (identity_after, identity_current) = (
        windows_handle_identity(&file)?,
        windows_handle_identity(&open_nofollow(path)?)?,
    );
    let changed_before = changed_time(&before);
    let changed_after = changed_time(&after);
    #[cfg(unix)]
    let changed_matches = changed_after == changed_time(&current);
    // Windows `fstat` legacy ctime differs from `path.stat` creation time, so
    // the cross-handle change check is platform-specific (exe_facts.py comment).
    #[cfg(windows)]
    let changed_matches = true;
    if identity_before != identity_after
        || changed_before != changed_after
        || identity_after != identity_current
        || !changed_matches
    {
        return Err(invalid("exe_game_file_changed"));
    }

    Ok((
        FileDigest {
            sha256: format!("{:x}", hasher.finalize()),
            crc32: !crc,
            size,
        },
        pieces,
    ))
}

/// Reads and parses the current instance facts (`exe_facts.py::game_facts`).
///
/// Returns a structured snapshot even when checks fail; callers gate on
/// [`GameFacts::is_valid`] instead of relying on an early error.
pub fn game_facts(
    game: &GameInstall,
    expectation: &FactsExpectation,
    cancel: &AtomicBool,
) -> GameFacts {
    game_facts_inner(game, expectation, cancel, false)
}
/// Current generic EXE facts; the scanned build is drift evidence, not publisher verification.
pub fn game_facts_unreviewed(
    game: &GameInstall,
    expectation: &FactsExpectation,
    cancel: &AtomicBool,
) -> GameFacts {
    game_facts_inner(game, expectation, cancel, true)
}
fn game_facts_inner(
    game: &GameInstall,
    expectation: &FactsExpectation,
    cancel: &AtomicBool,
    unreviewed: bool,
) -> GameFacts {
    let instance = InstanceFacts {
        instance_id: game.instance_id.clone(),
        appid: game.appid,
        library_path: game.library_path.clone(),
        install_path: game.install_path.clone(),
    };

    let expected_id = format!(
        "{}|{}|{}",
        game.appid,
        game.library_path.display(),
        game.install_path.display()
    );
    let identity_ok = game.appid == expectation.appid
        && game.library_path.is_absolute()
        && game.install_path.is_absolute()
        && no_links(&game.library_path)
        && no_links(&game.install_path)
        && game.library_path.is_dir()
        && game.install_path.is_dir()
        && game.instance_id == expected_id;
    if !identity_ok {
        return GameFacts::all_failed(
            instance,
            FactFailure::new(
                "exe_game_identity_changed",
                "instance identity or root mismatch",
            ),
        );
    }

    let manifest_path = game
        .library_path
        .join("steamapps")
        .join(format!("appmanifest_{}.acf", game.appid));
    let parsed = match read_manifest(&manifest_path, cancel) {
        Ok(parsed) => parsed,
        Err(failure) => {
            return GameFacts::all_failed(instance, failure);
        }
    };

    let manifest = manifest_fact(game, &parsed);
    let build = if unreviewed {
        let observed = field_text(&parsed.state, "buildid").map(str::to_owned);
        if observed.as_ref().is_some_and(|v| !valid_buildid(v)) || observed != game.buildid {
            Fact::failed_code("exe_game_identity_changed", "current build changed")
        } else {
            Fact::evidence(BuildFacts {
                buildid: observed,
                verified: false,
            })
        }
    } else {
        build_fact(game, &parsed.state, expectation)
    };
    let language = language_fact(&parsed.state, expectation);
    let dlc = if unreviewed
        && ["installeddepots", "installeddlc", "installdlc"]
            .iter()
            .all(|key| field(&parsed.state, key).is_none())
    {
        if expectation.requires_dlc.is_empty() && expectation.excludes_dlc.is_empty() {
            // Unknown is not proof that no DLC is installed, nor a blocker
            // when this installation has no DLC dependency.
            Fact {
                evidence: None,
                failure: None,
            }
        } else {
            Fact::failed_code("exe_dlc_unknown", "manifest has no installation evidence")
        }
    } else {
        dlc_fact(&parsed.state, expectation)
    };
    let baselines = baseline_facts(&game.install_path, expectation, cancel);

    GameFacts {
        instance,
        manifest,
        build,
        language,
        dlc,
        baselines,
    }
}

/// Best-effort current selected Steam UI language for a game, read from
/// `UserConfig.language` then `MountedConfig.language` (the same fields
/// [`game_facts`] compares). Used to make a source that carries no language
/// self-consistent instead of wrongly blocking with `exe_language_mismatch`.
pub fn current_language(game: &GameInstall, cancel: &AtomicBool) -> Option<String> {
    let manifest_path = game
        .library_path
        .join("steamapps")
        .join(format!("appmanifest_{}.acf", game.appid));
    let parsed = read_manifest(&manifest_path, cancel).ok()?;
    let state = &parsed.state;
    nested(state, "userconfig")
        .ok()
        .flatten()
        .and_then(|object| field_text(object, "language"))
        .map(str::to_owned)
        .or_else(|| {
            nested(state, "mountedconfig")
                .ok()
                .flatten()
                .and_then(|object| field_text(object, "language"))
                .map(str::to_owned)
        })
}

struct ParsedManifest {
    digest: FileDigest,
    state: BTreeMap<String, Value>,
}

fn read_manifest(
    path: &Path,
    cancel: &AtomicBool,
) -> std::result::Result<ParsedManifest, FactFailure> {
    let (digest, bytes) = read_file(path, MAX_MANIFEST_BYTES, cancel).map_err(|error| {
        FactFailure::new(
            read_failure_code(&error, "exe_manifest_unavailable"),
            error.to_string(),
        )
    })?;
    let document = keyvalues::parse(&bytes)
        .map_err(|error| FactFailure::new("exe_manifest_invalid", error.to_string()))?;
    if document.len() != 1 {
        return Err(FactFailure::new(
            "exe_manifest_invalid",
            "unexpected top-level manifest keys",
        ));
    }
    let state = field(&document, "appstate")
        .and_then(Value::object)
        .cloned()
        .ok_or_else(|| FactFailure::new("exe_manifest_invalid", "missing AppState"))?;
    Ok(ParsedManifest { digest, state })
}

fn manifest_fact(game: &GameInstall, parsed: &ParsedManifest) -> Fact<ManifestFacts> {
    let state = &parsed.state;
    let installdir = field_text(state, "installdir").map(str::to_owned);
    let stateflags = field_text(state, "stateflags").map(str::to_owned);
    let buildid = field_text(state, "buildid").map(str::to_owned);
    let evidence = ManifestFacts {
        sha256: parsed.digest.sha256.clone(),
        installdir: installdir.clone().unwrap_or_default(),
        stateflags: stateflags.clone(),
        buildid: buildid.clone(),
    };

    let mut failure = None;
    match parse_number(field_text(state, "appid")) {
        Some(appid) if appid == game.appid as u64 => {}
        _ => {
            failure = Some(FactFailure::new(
                "exe_game_identity_changed",
                "manifest appid mismatch",
            ));
        }
    }
    match &installdir {
        Some(directory) if valid_install_dir(directory) => {
            let expected = game
                .library_path
                .join("steamapps")
                .join("common")
                .join(directory);
            if expected != game.install_path {
                failure = Some(FactFailure::new(
                    "exe_game_identity_changed",
                    "installdir does not resolve to the game root",
                ));
            }
        }
        _ => {
            failure = Some(FactFailure::new(
                "exe_manifest_invalid",
                "invalid installdir",
            ));
        }
    }
    let downloading = game
        .library_path
        .join("steamapps")
        .join("downloading")
        .join(game.appid.to_string());
    if stateflags.as_deref() != Some("4") || downloading.exists() {
        failure = Some(FactFailure::new(
            "game_not_ready",
            "stateflags is not 4 or the app is downloading",
        ));
    }
    Fact {
        evidence: Some(evidence),
        failure,
    }
}

fn build_fact(
    game: &GameInstall,
    state: &BTreeMap<String, Value>,
    expectation: &FactsExpectation,
) -> Fact<BuildFacts> {
    let buildid = field_text(state, "buildid").map(str::to_owned);
    match buildid {
        Some(build)
            if valid_buildid(&build)
                && game.buildid.as_deref() == Some(build.as_str())
                && expectation
                    .verified_buildids
                    .iter()
                    .any(|item| item == &build) =>
        {
            Fact::evidence(BuildFacts {
                buildid: Some(build),
                verified: true,
            })
        }
        Some(build) => Fact {
            evidence: Some(BuildFacts {
                buildid: Some(build),
                verified: false,
            }),
            failure: Some(FactFailure::new("build_not_verified", "buildid mismatch")),
        },
        None => Fact::failed_code("build_not_verified", "missing buildid"),
    }
}

fn language_fact(
    state: &BTreeMap<String, Value>,
    expectation: &FactsExpectation,
) -> Fact<LanguageFacts> {
    let user = match nested(state, "userconfig") {
        Ok(value) => value,
        Err(failure) => return Fact::failed(failure),
    };
    let mounted = match nested(state, "mountedconfig") {
        Ok(value) => value,
        Err(failure) => return Fact::failed(failure),
    };
    let user_language = user
        .and_then(|object| field_text(object, "language"))
        .map(str::to_owned);
    let mounted_language = mounted
        .and_then(|object| field_text(object, "language"))
        .map(str::to_owned);
    let selected = user_language.clone().or_else(|| mounted_language.clone());
    let expected = expected_language(&expectation.language);

    let mut failure = None;
    let gate = expectation.require_language;
    if gate && selected.as_deref() != Some(expected) {
        failure = Some(FactFailure::new(
            "exe_language_mismatch",
            format!("expected language {expected}"),
        ));
    } else if gate
        && user_language.is_some()
        && mounted_language.is_some()
        && user_language != mounted_language
    {
        failure = Some(FactFailure::new(
            "exe_language_mismatch",
            "UserConfig and MountedConfig disagree",
        ));
    }
    // `evidence` records the observed/selected language regardless of whether it
    // is a gate, so the language stays part of the content identity.
    Fact {
        evidence: Some(LanguageFacts {
            user: user_language,
            mounted: mounted_language,
            selected,
        }),
        failure,
    }
}

fn dlc_fact(state: &BTreeMap<String, Value>, expectation: &FactsExpectation) -> Fact<DlcFacts> {
    match compute_dlc(state) {
        Ok(installed) => {
            let mut failure = None;
            if installed
                .iter()
                .any(|appid| expectation.excludes_dlc.contains(appid))
            {
                failure = Some(FactFailure::new(
                    "exe_dlc_excluded",
                    "an excluded DLC is installed",
                ));
            } else if expectation
                .requires_dlc
                .iter()
                .any(|appid| !installed.contains(appid))
            {
                failure = Some(FactFailure::new(
                    "exe_dlc_required",
                    "a required DLC is missing",
                ));
            }
            Fact {
                evidence: Some(DlcFacts {
                    installed: installed.iter().copied().collect(),
                }),
                failure,
            }
        }
        Err(failure) => Fact::failed(failure),
    }
}

fn compute_dlc(state: &BTreeMap<String, Value>) -> std::result::Result<BTreeSet<u32>, FactFailure> {
    let mut dlc = BTreeSet::new();
    if let Some(depots) = nested(state, "installeddepots")? {
        for (depot, detail) in depots {
            if parse_number(Some(depot)).is_none() {
                return Err(FactFailure::new(
                    "exe_manifest_invalid",
                    "non-numeric depot",
                ));
            }
            let detail = detail.object().ok_or_else(|| {
                FactFailure::new("exe_manifest_invalid", "depot is not an object")
            })?;
            if let Some(appid) = field_text(detail, "dlcappid") {
                match parse_number(Some(appid)) {
                    Some(appid) => {
                        dlc.insert(appid as u32);
                    }
                    None => {
                        return Err(FactFailure::new("exe_manifest_invalid", "invalid dlcappid"));
                    }
                }
            }
        }
    }

    let installed_dlc = field_text(state, "installeddlc")
        .or_else(|| field_text(state, "installdlc"))
        .unwrap_or("");
    if !installed_dlc.is_empty() {
        for value in installed_dlc.split(',') {
            match parse_number(Some(value.trim())) {
                Some(appid) => {
                    dlc.insert(appid as u32);
                }
                None => {
                    return Err(FactFailure::new(
                        "exe_manifest_invalid",
                        "invalid installed DLC",
                    ));
                }
            }
        }
    }
    Ok(dlc)
}

fn baseline_facts(
    root: &Path,
    expectation: &FactsExpectation,
    cancel: &AtomicBool,
) -> Vec<BaselineFact> {
    expectation
        .baselines
        .iter()
        .map(|baseline| {
            let mut fact = BaselineFact {
                path: baseline.path.clone(),
                expected_sha256: baseline.sha256.clone(),
                expected_crc32: baseline.crc32,
                observed: None,
                failure: None,
            };
            if !valid_relative_path(&baseline.path) {
                fact.failure = Some(FactFailure::new(
                    "exe_baseline_invalid",
                    "unsafe baseline path",
                ));
                return fact;
            }
            let target = root.join(baseline.path.split('/').collect::<PathBuf>());
            match digest_file(&target, cancel) {
                Ok(digest) => {
                    if digest.sha256 != baseline.sha256
                        || baseline.crc32.is_some_and(|crc| digest.crc32 != crc)
                    {
                        fact.failure = Some(FactFailure::new(
                            "exe_baseline_mismatch",
                            "baseline content differs from the reviewed value",
                        ));
                    }
                    fact.observed = Some(digest);
                }
                Err(error) => {
                    fact.failure = Some(FactFailure::new(
                        read_failure_code(&error, "exe_baseline_mismatch"),
                        error.to_string(),
                    ));
                }
            }
            fact
        })
        .collect()
}

/// Maps an [`invalid`] error to its stable code, falling back for OS errors.
fn read_failure_code(error: &std::io::Error, fallback: &str) -> String {
    if error.kind() == std::io::ErrorKind::InvalidData {
        error.to_string()
    } else {
        fallback.to_string()
    }
}

fn expected_language(key: &str) -> &str {
    LANGUAGES
        .iter()
        .find(|(language, _)| *language == key)
        .map(|(_, value)| *value)
        .unwrap_or(key)
}

/// `exe_facts.py::_number`: `[1-9][0-9]{0,9}` only (no leading zero, ≤10 digits).
fn parse_number(value: Option<&str>) -> Option<u64> {
    let value = value?;
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 10
        || bytes[0] == b'0'
        || !bytes.iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    value.parse().ok()
}

fn valid_buildid(value: &str) -> bool {
    !value.is_empty() && value.len() <= 20 && value.bytes().all(|byte| byte.is_ascii_digit())
}

/// `steam::valid_install_dir` equivalent (kept local to avoid widening `steam`).
fn valid_install_dir(name: &str) -> bool {
    !name.trim().is_empty()
        && name != "."
        && name != ".."
        && !name.ends_with('.')
        && !name.ends_with(' ')
        && !name.chars().any(|ch| {
            ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        && Path::new(name)
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

/// `exe_facts.py` baseline path rule: `'/'`-separated, no empty/`.`/`..`, no
/// `\\`, `:` or NUL.
fn valid_relative_path(path: &str) -> bool {
    !path.contains(['\\', ':', '\0'])
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn field<'a>(object: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a Value> {
    object
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
        .map(|(_, value)| value)
}

fn field_text<'a>(object: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a str> {
    field(object, key).and_then(Value::text)
}

fn nested<'a>(
    object: &'a BTreeMap<String, Value>,
    key: &str,
) -> std::result::Result<Option<&'a BTreeMap<String, Value>>, FactFailure> {
    match field(object, key) {
        None => Ok(None),
        Some(value) => value.object().map(Some).ok_or_else(|| {
            FactFailure::new("exe_manifest_invalid", format!("{key} is not an object"))
        }),
    }
}

// ---- file identity --------------------------------------------------------

/// Comparable identity fields (`st_dev`/`st_ino`/size/mtime on unix; volume
/// serial/file index/size/last-write on Windows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    volume: u64,
    index: u64,
    size: u64,
    modified_secs: i64,
    modified_nanos: i64,
}

#[cfg(unix)]
fn file_identity(metadata: &fs::Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;
    FileIdentity {
        volume: metadata.dev(),
        index: metadata.ino(),
        size: metadata.len(),
        modified_secs: metadata.mtime(),
        modified_nanos: metadata.mtime_nsec(),
    }
}

#[cfg(all(windows, test))]
fn file_identity(metadata: &fs::Metadata) -> FileIdentity {
    use std::os::windows::fs::MetadataExt;
    // `volume_serial_number`/`file_index` are still unstable (`windows_by_handle`).
    // Size + last-write time through the same-platform metadata calls is the
    // stable identity available here; creation time is compared only between the
    // two descriptor snapshots via `changed_time`.
    FileIdentity {
        volume: 0,
        index: 0,
        size: metadata.len(),
        modified_secs: metadata.last_write_time() as i64,
        modified_nanos: 0,
    }
}

#[cfg(not(any(unix, windows)))]
fn file_identity(metadata: &fs::Metadata) -> FileIdentity {
    FileIdentity {
        volume: 0,
        index: 0,
        size: metadata.len(),
        modified_secs: 0,
        modified_nanos: 0,
    }
}

/// Change timestamp used only for the descriptor-vs-descriptor comparison and
/// (on unix) descriptor-vs-path comparison.
#[cfg(unix)]
fn changed_time(metadata: &fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    metadata.ctime()
}

#[cfg(windows)]
fn changed_time(metadata: &fs::Metadata) -> i64 {
    use std::os::windows::fs::MetadataExt;
    metadata.creation_time() as i64
}

#[cfg(not(any(unix, windows)))]
fn changed_time(_metadata: &fs::Metadata) -> i64 {
    0
}

#[cfg(unix)]
pub(crate) fn open_nofollow(path: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

#[cfg(windows)]
pub(crate) fn open_nofollow(path: &Path) -> Result<File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(0x0020_0000)
        .share_mode(1)
        .open(path)?;
    if file.metadata()?.file_attributes() & 0x400 != 0 {
        return Err(invalid("exe_linked_game_file"));
    }
    Ok(file)
}
#[cfg(not(any(unix, windows)))]
pub(crate) fn open_nofollow(path: &Path) -> Result<File> {
    File::open(path)
}
#[cfg(windows)]
fn windows_handle_identity(file: &File) -> Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    struct Information {
        attributes: u32,
        times: [u32; 6],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut Information)
            -> i32;
    }
    let mut info: Information = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    if info.attributes & 0x400 != 0 {
        return Err(invalid("exe_linked_game_file"));
    }
    Ok(FileIdentity {
        volume: info.volume as u64,
        index: ((info.index_high as u64) << 32) | info.index_low as u64,
        size: ((info.size_high as u64) << 32) | info.size_low as u64,
        modified_secs: (((info.times[5] as u64) << 32) | info.times[4] as u64) as i64,
        modified_nanos: 0,
    })
}

// ---- CRC-32 (IEEE, matches Python `binascii.crc32` / zlib) ---------------

const CRC_TABLE: [u32; 256] = build_crc_table();

const fn build_crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[index] = crc;
        index += 1;
    }
    table
}

fn crc32_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for &byte in bytes {
        crc = (crc >> 8) ^ CRC_TABLE[((crc ^ byte as u32) & 0xff) as usize];
    }
    crc
}

#[cfg(test)]
fn crc32(bytes: &[u8]) -> u32 {
    !crc32_update(!0, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steam::GameInstall;

    const APPID: u32 = 2_303_800;

    fn write_file(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, bytes).unwrap();
    }

    /// Builds a library tree and matching `GameInstall`.
    fn instance(temp: &Path, manifest: &str) -> (GameInstall, PathBuf) {
        let library = temp.join("library");
        let install = library.join("steamapps").join("common").join("TestGame");
        fs::create_dir_all(&install).unwrap();
        write_file(
            &library
                .join("steamapps")
                .join(format!("appmanifest_{APPID}.acf")),
            manifest.as_bytes(),
        );
        let game = GameInstall {
            instance_id: format!("{APPID}|{}|{}", library.display(), install.display()),
            appid: APPID,
            name: "Test Game".into(),
            localized_name: None,
            name_language: None,
            library_path: library,
            install_path: install.clone(),
            buildid: Some("12345".into()),
            availability: "installed".into(),
            kind: "unknown".into(),
        };
        (game, install)
    }

    fn manifest(language: &str, stateflags: &str, buildid: &str, extra: &str) -> String {
        format!(
            r#""AppState"
{{
    "appid"        "{APPID}"
    "name"         "Test Game"
    "installdir"   "TestGame"
    "StateFlags"   "{stateflags}"
    "buildid"      "{buildid}"
    "UserConfig"   {{ "language" "{language}" }}
    "MountedConfig" {{ "language" "{language}" }}
    "InstalledDepots"
    {{
        "2303801" {{ "dlcappid" "2303802" }}
    }}
    {extra}
}}"#
        )
    }

    fn expectation(baselines: Vec<BaselineExpectation>) -> FactsExpectation {
        FactsExpectation {
            appid: APPID,
            language: "zh-CN".into(),
            verified_buildids: vec!["12345".into()],
            require_language: true,
            requires_dlc: vec![2303802],
            excludes_dlc: vec![2303999],
            baselines,
        }
    }

    #[test]
    fn crc32_matches_reference_vector_and_incremental() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        let first = crc32_update(!0, b"1234");
        let second = crc32_update(first, b"56789");
        assert_eq!(!second, 0xcbf4_3926);
    }

    #[test]
    fn digest_file_reports_sha_crc_and_size() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.bin");
        write_file(&path, b"123456789");
        let digest = digest_file(&path, &AtomicBool::new(false)).unwrap();
        assert_eq!(digest.crc32, 0xcbf4_3926);
        assert_eq!(digest.size, 9);
        assert_eq!(digest.sha256, format!("{:x}", Sha256::digest(b"123456789")));
        assert_eq!(
            read_file(&path, 1024, &AtomicBool::new(false)).unwrap().1,
            b"123456789"
        );
    }

    #[test]
    fn digest_file_rejects_missing_and_directory_and_read_file_bounds_size() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing");
        assert!(digest_file(&missing, &AtomicBool::new(false)).is_err());

        let directory = temp.path().join("dir");
        fs::create_dir(&directory).unwrap();
        assert!(digest_file(&directory, &AtomicBool::new(false)).is_err());

        // Digest-only reads have no cap; the caller memory bound still guards
        // the buffered `read_file` variant.
        let big = temp.path().join("big");
        write_file(&big, &[0u8; 16]);
        assert_eq!(
            read_file(&big, 8, &AtomicBool::new(false))
                .unwrap_err()
                .to_string(),
            "exe_game_file_limit"
        );
        assert!(digest_file(&big, &AtomicBool::new(false)).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn digest_file_rejects_linked_path() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        write_file(&target, b"payload");
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(
            digest_file(&link, &AtomicBool::new(false))
                .unwrap_err()
                .to_string(),
            "exe_linked_game_file"
        );
    }

    #[test]
    fn identity_detects_size_change() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file");
        write_file(&path, b"a");
        let before = fs::metadata(&path).unwrap();
        write_file(&path, b"a much longer payload");
        let after = fs::metadata(&path).unwrap();
        assert_ne!(file_identity(&before), file_identity(&after));
    }

    #[test]
    fn game_facts_accepts_a_matching_instance() {
        let temp = tempfile::tempdir().unwrap();
        let (game, install) = instance(temp.path(), &manifest("schinese", "4", "12345", ""));
        write_file(&install.join("data.xp3"), b"baseline");
        let observed = digest_file(&install.join("data.xp3"), &AtomicBool::new(false)).unwrap();
        let facts = game_facts(
            &game,
            &expectation(vec![BaselineExpectation {
                path: "data.xp3".into(),
                sha256: observed.sha256.clone(),
                crc32: Some(observed.crc32),
            }]),
            &AtomicBool::new(false),
        );
        assert!(facts.is_valid(), "{:?}", facts.failures());
        assert_eq!(
            facts.manifest.evidence.as_ref().unwrap().installdir,
            "TestGame"
        );
        assert_eq!(
            facts.build.evidence.as_ref().unwrap().buildid.as_deref(),
            Some("12345")
        );
        assert_eq!(
            facts
                .language
                .evidence
                .as_ref()
                .unwrap()
                .selected
                .as_deref(),
            Some("schinese")
        );
        assert_eq!(
            facts.dlc.evidence.as_ref().unwrap().installed,
            vec![2303802]
        );
        assert_eq!(facts.manifest.evidence.as_ref().unwrap().sha256.len(), 64);
    }

    #[test]
    fn compute_dlc_accepts_more_than_the_former_depot_and_dlc_limits() {
        let mut depots = BTreeMap::new();
        for index in 0..4097u32 {
            let appid = 3_000_000 + index;
            depots.insert(
                (appid - 1).to_string(),
                Value::Object(BTreeMap::from([(
                    "dlcappid".to_string(),
                    Value::Text(appid.to_string()),
                )])),
            );
        }
        let installed_dlc = (0..129u32)
            .map(|index| (4_000_000 + index).to_string())
            .collect::<Vec<_>>()
            .join(",");
        let state = BTreeMap::from([
            ("installeddepots".to_string(), Value::Object(depots)),
            ("installeddlc".to_string(), Value::Text(installed_dlc)),
        ]);
        let installed = compute_dlc(&state).unwrap();
        assert_eq!(installed.len(), 4097 + 129);
        assert!(installed.contains(&3_000_000));
        assert!(installed.contains(&4_000_128));
    }

    #[test]
    fn game_facts_accepts_installed_dlc_beyond_the_former_limit() {
        let temp = tempfile::tempdir().unwrap();
        let installed_dlc = (0..129u32)
            .map(|index| (4_000_000 + index).to_string())
            .collect::<Vec<_>>()
            .join(",");
        let (game, _) = instance(
            temp.path(),
            &manifest(
                "schinese",
                "4",
                "12345",
                &format!(r#""installeddlc" "{installed_dlc}""#),
            ),
        );
        // This is exactly the gate `signed::require_facts` applies.
        let facts = game_facts(&game, &expectation(Vec::new()), &AtomicBool::new(false));
        assert!(facts.is_valid(), "{:?}", facts.failures());
        assert_eq!(facts.dlc.evidence.as_ref().unwrap().installed.len(), 130);
    }

    #[test]
    fn game_facts_surfaces_identity_build_language_dlc_and_baseline_failures() {
        let temp = tempfile::tempdir().unwrap();
        let (game, install) = instance(
            temp.path(),
            &manifest("english", "4", "99999", r#""installeddlc" "2303999""#),
        );
        write_file(&install.join("data.xp3"), b"different");
        let facts = game_facts(
            &game,
            &expectation(vec![BaselineExpectation {
                path: "data.xp3".into(),
                sha256: "0".repeat(64),
                crc32: None,
            }]),
            &AtomicBool::new(false),
        );
        let codes: Vec<&str> = facts.failures().iter().map(|f| f.code.as_str()).collect();
        assert!(codes.contains(&"build_not_verified"));
        assert!(codes.contains(&"exe_language_mismatch"));
        assert!(codes.contains(&"exe_dlc_excluded"));
        assert!(codes.contains(&"exe_baseline_mismatch"));
        assert!(!facts.is_valid());
    }

    #[test]
    fn facts_language_is_evidence_only_when_not_required() {
        let temp = tempfile::tempdir().unwrap();
        let (game, _) = instance(temp.path(), &manifest("english", "4", "12345", ""));
        let mut relaxed = expectation(Vec::new());
        relaxed.require_language = false;
        let facts = game_facts(&game, &relaxed, &AtomicBool::new(false));
        assert!(facts.is_valid(), "{:?}", facts.failures());
        assert!(facts.language.failure.is_none());
        let evidence = facts.language.evidence.as_ref().unwrap();
        assert_eq!(evidence.user.as_deref(), Some("english"));
        assert_eq!(evidence.mounted.as_deref(), Some("english"));
        assert_eq!(evidence.selected.as_deref(), Some("english"));
    }

    #[test]
    fn facts_language_mismatch_between_configs_is_suppressed_when_not_required() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = format!(
            r#""AppState"
{{
    "appid"        "{APPID}"
    "name"         "Test Game"
    "installdir"   "TestGame"
    "StateFlags"   "4"
    "buildid"      "12345"
    "UserConfig"   {{ "language" "schinese" }}
    "MountedConfig" {{ "language" "english" }}
    "InstalledDepots"
    {{
        "2303801" {{ "dlcappid" "2303802" }}
    }}
}}"#
        );
        let (game, _) = instance(temp.path(), &manifest);
        let mut relaxed = expectation(Vec::new());
        relaxed.require_language = false;
        let facts = game_facts(&game, &relaxed, &AtomicBool::new(false));
        assert!(facts.is_valid(), "{:?}", facts.failures());
        assert!(facts.language.failure.is_none());
        let evidence = facts.language.evidence.as_ref().unwrap();
        assert_eq!(evidence.user.as_deref(), Some("schinese"));
        assert_eq!(evidence.mounted.as_deref(), Some("english"));
        assert_eq!(evidence.selected.as_deref(), Some("schinese"));

        // The same manifest still fails under the publisher gate.
        let strict = expectation(Vec::new());
        let facts = game_facts(&game, &strict, &AtomicBool::new(false));
        assert_eq!(
            facts.language.failure.as_ref().unwrap().code,
            "exe_language_mismatch"
        );
    }

    #[test]
    fn game_facts_rejects_appid_mismatch_and_unsafe_baseline_path() {
        let temp = tempfile::tempdir().unwrap();
        let (game, _) = instance(temp.path(), &manifest("schinese", "4", "12345", ""));
        let mut wrong = expectation(Vec::new());
        wrong.appid = APPID + 1;
        let facts = game_facts(&game, &wrong, &AtomicBool::new(false));
        assert_eq!(
            facts.manifest.failure.as_ref().unwrap().code,
            "exe_game_identity_changed"
        );

        let facts = game_facts(
            &game,
            &expectation(vec![BaselineExpectation {
                path: "../escape".into(),
                sha256: "0".repeat(64),
                crc32: None,
            }]),
            &AtomicBool::new(false),
        );
        assert_eq!(
            facts.baselines[0].failure.as_ref().unwrap().code,
            "exe_baseline_invalid"
        );
    }

    #[test]
    fn game_facts_flags_downloading_stateflags() {
        let temp = tempfile::tempdir().unwrap();
        let (game, _) = instance(temp.path(), &manifest("schinese", "2", "12345", ""));
        let facts = game_facts(&game, &expectation(Vec::new()), &AtomicBool::new(false));
        assert_eq!(
            facts.manifest.failure.as_ref().unwrap().code,
            "game_not_ready"
        );
    }

    #[test]
    fn game_facts_rejects_invalid_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let (game, _) = instance(temp.path(), "broken { not closed");
        let facts = game_facts(&game, &expectation(Vec::new()), &AtomicBool::new(false));
        assert_eq!(
            facts.manifest.failure.as_ref().unwrap().code,
            "exe_manifest_invalid"
        );
    }
}

#[cfg(test)]
mod generic_current_tests {
    use super::*;
    fn game(temp: &Path, language: &str, dlc_evidence: &str) -> GameInstall {
        let library = temp.join("library");
        let install = library.join("steamapps/common/Game");
        fs::create_dir_all(&install).unwrap();
        let manifest = format!(
            r#""AppState" {{ "appid" "42" "installdir" "Game" "StateFlags" "4" "buildid" "123" "UserConfig" {{ {language} }} "MountedConfig" {{ {language} }} {dlc_evidence} }}"#
        );
        fs::write(library.join("steamapps/appmanifest_42.acf"), manifest).unwrap();
        GameInstall {
            instance_id: format!("42|{}|{}", library.display(), install.display()),
            appid: 42,
            name: "Game".into(),
            localized_name: None,
            name_language: None,
            library_path: library,
            install_path: install,
            buildid: Some("123".into()),
            availability: "installed".into(),
            kind: "unknown".into(),
        }
    }
    fn expectation() -> FactsExpectation {
        FactsExpectation {
            appid: 42,
            language: "zh-CN".into(),
            verified_buildids: vec![],
            require_language: true,
            requires_dlc: vec![],
            excludes_dlc: vec![],
            baselines: vec![],
        }
    }
    #[test]
    fn current_build_is_recorded_without_becoming_verified() {
        let t = tempfile::tempdir().unwrap();
        let g = game(
            t.path(),
            r#""language" "schinese""#,
            r#""InstalledDepots" {}"#,
        );
        let cancel = AtomicBool::new(false);
        let f = game_facts_unreviewed(&g, &expectation(), &cancel);
        assert!(f.is_valid(), "{:?}", f.failures());
        assert!(!f.build.evidence.unwrap().verified);
        assert!(!game_facts(&g, &expectation(), &cancel).is_valid());
        let mut changed = g;
        changed.buildid = Some("other".into());
        assert!(!game_facts_unreviewed(&changed, &expectation(), &cancel).is_valid());
    }
    #[test]
    fn unknown_language_or_dlc_never_satisfies_constraints() {
        let t = tempfile::tempdir().unwrap();
        let g = game(t.path(), "", " ");
        let mut expected = expectation();
        expected.requires_dlc = vec![99];
        let f = game_facts_unreviewed(&g, &expected, &AtomicBool::new(false));
        assert_eq!(f.language.failure.unwrap().code, "exe_language_mismatch");
        assert_eq!(f.dlc.failure.unwrap().code, "exe_dlc_unknown");
    }
    #[cfg(windows)]
    #[test]
    fn windows_identity_distinguishes_equal_size_files() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a");
        let b = t.path().join("b");
        fs::write(&a, b"123").unwrap();
        fs::write(&b, b"123").unwrap();
        let first = open_nofollow(&a).unwrap();
        let second = open_nofollow(&b).unwrap();
        assert_ne!(
            windows_handle_identity(&first).unwrap(),
            windows_handle_identity(&second).unwrap()
        );
    }
}
