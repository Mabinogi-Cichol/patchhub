//! Dynamic publisher ZIP candidate selection and resolution
//! (p1-zip-orchestration §1/§2).
//!
//! `zip_selection_id` is a *candidate-level* identity: it proves which source
//! candidate the UI meant, never that a download is authorized. The value is a
//! deterministic function of an explicit `zip_candidate_binding` encoded with
//! the frozen source canonical JSON (`sources::canonical`), so it is stable
//! across processes and only changes when a meaningful candidate field changes.
//!
//! `resolve_zip` re-derives the candidate from the registry cache (the client's
//! candidate list is never trusted), re-checks source policy/subscription/cache
//! freshness, picks the single download URL and maps the candidate into the
//! [`FactsExpectation`] the installer will verify before planning.

use super::facts::FactsExpectation;
use crate::sources::{
    canonical,
    model::{SourceCandidate, SourceRule, Variant},
    registry::SourceRegistry,
    validate,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt;

/// Structured resolve failure carrying a stable host error `code`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveError {
    pub code: String,
    pub detail: Option<String>,
}

impl ResolveError {
    pub fn code(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: None,
        }
    }

    pub fn new(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: Some(detail.into()),
        }
    }
}

impl fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            Some(detail) => write!(formatter, "{}: {detail}", self.code),
            None => formatter.write_str(&self.code),
        }
    }
}

impl std::error::Error for ResolveError {}

pub type ResolveResult<T> = std::result::Result<T, ResolveError>;

/// Ordered, explicit candidate binding hashed into the selection id
/// (p1-zip-orchestration §1.1). `mirrors_zip` binds only ZIP mirrors; unrelated
/// EXE mirrors or labels must not invalidate a ZIP selection.
#[derive(Debug, Serialize)]
struct ZipCandidateBinding<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    parser_fingerprint: Option<&'a str>,
    source_id: &'a str,
    source_record_id: &'a str,
    game_appid: u32,
    variant: Variant,
    languages: &'a [String],
    version_identity: &'a str,
    source_page: &'a str,
    requires_dlc: &'a Option<Vec<u32>>,
    excludes_dlc: &'a Option<Vec<u32>>,
    mirrors_zip: Vec<&'a str>,
}

/// ZIP mirrors of a candidate in original order (labels dropped).
fn zip_mirrors(candidate: &SourceCandidate) -> Vec<&str> {
    candidate
        .mirrors
        .iter()
        .filter(|mirror| mirror.format == "zip")
        .map(|mirror| mirror.url.as_str())
        .collect()
}

/// `zip-selection-<hex24>-<appid>`, or `None` when the candidate has no AppID
/// or no ZIP mirror (p1-zip-orchestration §1.1).
pub fn zip_selection_id(candidate: &SourceCandidate) -> Option<String> {
    let appid = candidate.game_appid?;
    let mirrors_zip = zip_mirrors(candidate);
    if mirrors_zip.is_empty() {
        return None;
    }
    let binding = ZipCandidateBinding {
        parser_fingerprint: candidate.parser_fingerprint.as_deref(),
        source_id: &candidate.source_id,
        source_record_id: &candidate.source_record_id,
        game_appid: appid,
        variant: candidate.variant,
        languages: &candidate.languages,
        version_identity: &candidate.version_identity,
        source_page: &candidate.source_page,
        requires_dlc: &candidate.requires_dlc,
        excludes_dlc: &candidate.excludes_dlc,
        mirrors_zip,
    };
    let bytes = canonical::canonical_bytes(&binding).ok()?;
    let digest = format!("{:x}", Sha256::digest(bytes));
    Some(format!("zip-selection-{}-{appid}", &digest[..24]))
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

/// Parses `^zip-selection-[0-9a-f]{24}-[1-9][0-9]*$` to the AppID.
pub fn parse_zip_selection_id(selection_id: &str) -> Option<u32> {
    let rest = selection_id.strip_prefix("zip-selection-")?;
    let (digest, appid) = rest.rsplit_once('-')?;
    if digest.len() != 24 || !digest.bytes().all(is_lower_hex) {
        return None;
    }
    if appid.is_empty() || appid.starts_with('0') || !appid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    appid.parse().ok()
}

/// Resolved, source-level candidate ready for the orchestrator
/// (p1-zip-orchestration §2.1).
#[derive(Debug, Clone)]
pub struct ResolvedZipCandidate {
    pub selection_id: String,
    /// Source release id: `patchmoe-<appid>` or `source-<rule-id>-<appid>`.
    pub release_id: String,
    pub appid: u32,
    pub source_id: String,
    pub source_record_id: String,
    pub candidate: SourceCandidate,
    pub rule: SourceRule,
    /// The single distinct, validated ZIP URL.
    pub download_url: String,
    pub language: String,
    /// `candidate.fingerprint` (`sha256` of the candidate minus volatile keys).
    pub candidate_sha256: String,
    pub rule_fingerprint: String,
    pub settings_revision: u64,
}

impl ResolvedZipCandidate {
    pub fn is_manual(&self) -> bool {
        let automatic = if self.selection_id.starts_with("exe-") {
            super::exe_resolve::exe_selection_id(&self.candidate)
        } else {
            zip_selection_id(&self.candidate)
        };
        automatic.as_deref() != Some(self.selection_id.as_str())
    }

    /// Facts expectation for this candidate (`verified_buildids` = the live
    /// build, p1-zip-orchestration §2.1 step 8). The orchestrator supplies the
    /// current `buildid` because the pure source layer has none.
    pub fn expectation(&self, _buildid: Option<&str>) -> FactsExpectation {
        FactsExpectation {
            appid: self.appid,
            verified_buildids: Vec::new(),
            requires_dlc: self.candidate.requires_dlc.clone().unwrap_or_default(),
            excludes_dlc: self.candidate.excludes_dlc.clone().unwrap_or_default(),
            baselines: Vec::new(),
        }
    }

    /// Deterministic digest of the candidate conditions plus the live build
    /// (`conditions_sha256`, p1-zip-orchestration §7.3). The full condition
    /// evaluator is not yet implemented; this is the caller-provided placeholder.
    pub fn conditions_sha256(&self, buildid: Option<&str>) -> String {
        let conditions = serde_json::json!({
            "requires_dlc": self.candidate.requires_dlc.clone().unwrap_or_default(),
            "excludes_dlc": self.candidate.excludes_dlc.clone().unwrap_or_default(),
            "verified_buildids": buildid.map(str::to_string).into_iter().collect::<Vec<_>>(),
        });
        canonical::canonical_sha256(&conditions)
    }
}

pub fn source_release_id(rule: &SourceRule, appid: u32) -> String {
    source_release_id_from_source(&rule.id, appid)
}

pub fn source_release_id_from_source(source_id: &str, appid: u32) -> String {
    if source_id == "patchmoe" {
        format!("patchmoe-{appid}")
    } else {
        format!("source-{source_id}-{appid}")
    }
}

/// Shared source readiness facts used by ZIP projection and resolution.
pub fn source_admission_blockers(
    registry: &SourceRegistry,
    candidate: &SourceCandidate,
    appid: u32,
) -> Vec<String> {
    let Some(rule) = registry
        .rules()
        .iter()
        .find(|rule| rule.id == candidate.source_id)
    else {
        return vec!["source_not_enabled".to_string()];
    };
    let mut blockers = Vec::new();
    if !rule.enabled {
        blockers.push("source_not_enabled".to_string());
    }
    if !registry.game_subscribed(appid, &rule.id) {
        blockers.push("source_not_subscribed".to_string());
    }
    if candidate.cache_status == crate::sources::model::CacheStatus::Stale {
        blockers.push("selection_stale".to_string());
    }
    blockers
}

pub fn source_admission_blocker(
    registry: &SourceRegistry,
    candidate: &SourceCandidate,
    appid: u32,
) -> Option<String> {
    source_admission_blockers(registry, candidate, appid)
        .into_iter()
        .next()
}

fn choose_language(candidate: &SourceCandidate, rule: &SourceRule) -> String {
    if let Some(chinese) = candidate
        .languages
        .iter()
        .find(|language| language.eq_ignore_ascii_case("zh-CN"))
    {
        return chinese.clone();
    }
    candidate
        .languages
        .first()
        .cloned()
        .or_else(|| rule.language.clone())
        .unwrap_or_default()
}

fn normalized_url(url: &str) -> Option<String> {
    let split = validate::split_url(url);
    let host = split.hostname()?;
    let port = split.port().ok().flatten();
    let mut out = format!("{}://{host}", split.scheme.to_lowercase());
    if let Some(port) = port {
        out.push_str(&format!(":{port}"));
    }
    out.push_str(&split.path);
    if !split.query.is_empty() {
        out.push('?');
        out.push_str(&split.query);
    }
    Some(out)
}

/// Artifact-format profile for the multi-mirror selector (P4 ruling). ZIP and
/// EXE share the selection algorithm but differ in validation and error codes.
#[derive(Clone, Copy)]
pub(crate) enum MirrorKind {
    Zip,
    Exe,
}

impl MirrorKind {
    fn format(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Exe => "exe",
        }
    }

    /// `*_download_unavailable` for the empty-format branch. EXE returns
    /// `None`: a selection id implies at least one EXE mirror, so the branch is
    /// unreachable there and falls through to the existing `invalid_download_url`
    /// instead of introducing a new host code (P4 ruling: keep the code set
    /// stable).
    fn unavailable_code(self) -> Option<&'static str> {
        match self {
            Self::Zip => Some("zip_download_unavailable"),
            Self::Exe => None,
        }
    }

    fn ambiguous_code(self) -> &'static str {
        match self {
            Self::Zip => "zip_download_ambiguous",
            Self::Exe => "exe_download_ambiguous",
        }
    }

    /// Validates one raw mirror under the profile `acquire` enforces: both
    /// kinds use `allowed_url` for the host/HTTPS/port contract and differ only
    /// in whether a fragment is rejected (see `acquire.rs` `ArchiveKind::url`).
    /// Returns `None` when illegal.
    fn validate(self, url: &str, hosts: &[String]) -> Option<String> {
        match self {
            Self::Zip => validate::allowed_url(url, hosts, true).ok(),
            Self::Exe => validate::allowed_url(url, hosts, false).ok(),
        }
    }
}

/// Final URL path segment, lowercased; the content-identity guard for the
/// multi-mirror selector's `*_download_ambiguous` code.
fn url_basename(url: &str) -> String {
    validate::split_url(url)
        .path
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

/// Selects the single deterministic download URL of `kind` from a candidate's
/// mirrors (P4 ruling). Legal mirrors are filtered in candidate order and
/// deduplicated by normalized URL; illegal mirrors are skipped rather than
/// failing the whole candidate. Decision order:
/// 1. no mirror of this format at all → `*_download_unavailable`;
/// 2. no legal mirror survives → `invalid_download_url`;
/// 3. legal mirrors span ≥ 2 distinct basenames (suspected cross-artifact) →
///    `*_download_ambiguous`;
/// 4. otherwise the first legal mirror, in original order.
pub(crate) fn select_download_url(
    candidate: &SourceCandidate,
    rule: &SourceRule,
    kind: MirrorKind,
) -> ResolveResult<String> {
    let total = candidate
        .mirrors
        .iter()
        .filter(|mirror| mirror.format == kind.format())
        .count();
    if total == 0 {
        if let Some(code) = kind.unavailable_code() {
            return Err(ResolveError::code(code));
        }
    }
    let mut legal: Vec<String> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    for mirror in candidate
        .mirrors
        .iter()
        .filter(|mirror| mirror.format == kind.format())
    {
        let Some(valid) = kind.validate(&mirror.url, &rule.download_hosts) else {
            continue;
        };
        let key = normalized_url(&valid).unwrap_or_else(|| valid.clone());
        if !keys.contains(&key) {
            keys.push(key);
            legal.push(valid);
        }
    }
    if legal.is_empty() {
        return Err(ResolveError::code("invalid_download_url"));
    }
    let mut basenames: Vec<String> = Vec::new();
    for url in &legal {
        let basename = url_basename(url);
        if !basenames.contains(&basename) {
            basenames.push(basename);
        }
    }
    if basenames.len() >= 2 {
        return Err(ResolveError::code(kind.ambiguous_code()));
    }
    legal
        .into_iter()
        .next()
        .ok_or_else(|| ResolveError::code("invalid_download_url"))
}

/// Whether a source-cache date evidence means "already released"
/// (p1-zip-orchestration §2.1 step 7). No date evidence is treated as released.
pub fn source_released(date: Option<&str>, now_unix: u64) -> bool {
    let Some(date) = date else {
        return true;
    };
    if date.eq_ignore_ascii_case("COMING SOON") {
        return false;
    }
    let parts: Vec<&str> = date.split('/').collect();
    if parts.len() != 3 {
        return true;
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        parts[0].parse::<i64>(),
        parts[1].parse::<i64>(),
        parts[2].parse::<i64>(),
    ) else {
        return true;
    };
    let (current_year, current_month, current_day) = civil_from_days((now_unix / 86_400) as i64);
    (year, month, day) <= (current_year, current_month, current_day)
}

/// Howard Hinnant's `civil_from_days` (proleptic Gregorian, days since epoch).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Resolves a dynamic ZIP selection against the registry cache. Does not touch
/// the network and does not trust the client's candidate list: the selection id
/// is recomputed for every cached candidate and must match exactly.
pub fn resolve_zip(
    registry: &SourceRegistry,
    game: &crate::sources::GameQuery,
    selection_id: &str,
) -> ResolveResult<ResolvedZipCandidate> {
    let appid = parse_zip_selection_id(selection_id)
        .ok_or_else(|| ResolveError::code("invalid_selection_id"))?;
    if appid != game.appid {
        return Err(ResolveError::code("selection_not_found"));
    }
    if let Ok(manual) = super::manual_selection::resolve(registry, game, selection_id, "zip") {
        return Ok(manual);
    }
    let candidate = registry
        .dynamic_zip_candidates(game)
        .into_iter()
        .find(|candidate| zip_selection_id(candidate).as_deref() == Some(selection_id))
        .ok_or_else(|| ResolveError::code("selection_not_found"))?;

    let rule = registry
        .rules()
        .iter()
        .find(|rule| rule.id == candidate.source_id)
        .cloned()
        .ok_or_else(|| ResolveError::code("source_not_enabled"))?;
    // Downloadability is not an automatic identity or variant confirmation.
    if rule.discovery_only
        && (!matches!(
            candidate.match_basis,
            crate::sources::model::MatchBasis::Appid
                | crate::sources::model::MatchBasis::VerifiedAlias
                | crate::sources::model::MatchBasis::OfficialTitle
        ) || matches!(candidate.variant, Variant::Unknown | Variant::Demo))
    {
        return Err(ResolveError::code("source_requires_guidance"));
    }
    // Discovery rules are the publisher-ZIP candidate source and may carry
    // `download_hosts`; only the enabled flag gates them (see progress note on
    // the §2.1 step-5 `!discovery_only` inconsistency).
    if let Some(code) = source_admission_blocker(registry, &candidate, appid) {
        return Err(ResolveError::code(code));
    }

    let download_url = select_download_url(&candidate, &rule, MirrorKind::Zip)?;

    // The released gate is intentionally absent: the frozen contract no longer
    // carries `source_not_released` for dynamic ZIP (fix-16 doc revision), and
    // dynamic candidates carry no reliable release date (`published_at` is
    // always `None`). Index projections additionally gate `COMING SOON` at
    // construction (`index_candidates`), so an unreleased entry never becomes a
    // candidate. `source_released` is retained for when a candidate-level date
    // evidence field exists.
    let release_id = source_release_id(&rule, appid);

    let language = choose_language(&candidate, &rule);
    Ok(ResolvedZipCandidate {
        selection_id: selection_id.to_string(),
        release_id,
        appid,
        source_id: rule.id.clone(),
        source_record_id: candidate.source_record_id.clone(),
        candidate_sha256: candidate.fingerprint.clone(),
        rule_fingerprint: crate::sources::Sources::rule_fingerprint(&rule),
        settings_revision: registry.settings_revision(),
        candidate,
        rule,
        download_url,
        language,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::model::{ArtifactFormat, Capability, MatchBasis, Mirror, SourceCandidate};

    fn mirror(url: &str, format: &str) -> Mirror {
        Mirror {
            url: url.to_string(),
            label: Some("label".to_string()),
            format: format.to_string(),
        }
    }

    fn candidate() -> SourceCandidate {
        SourceCandidate {
            parser_fingerprint: None,
            candidate_id: "candidate-000000000000000000000000".to_string(),
            source_id: "kagura-cn".to_string(),
            source_record_id: "https://kagura.example/p/1".to_string(),
            game_appid: Some(42),
            parent_appid: None,
            aliases: Vec::new(),
            game_title: "Game".to_string(),
            patch_title: "Game patch".to_string(),
            variant: Variant::Base,
            requires_dlc: None,
            excludes_dlc: None,
            platforms: vec!["steam".to_string()],
            patch_kind: "adult_content".to_string(),
            languages: vec!["zh-CN".to_string()],
            version_raw: Some("1".to_string()),
            version_evidence: Vec::new(),
            version_identity: "1".to_string(),
            published_at: None,
            patch_updated_at: None,
            source_page: "https://kagura.example/p/1".to_string(),
            upstream_page: None,
            mirrors: vec![mirror("https://dl.example/a.zip", "zip")],
            format_reported: ArtifactFormat::Zip,
            format_observed: ArtifactFormat::Zip,
            capability: Capability::DiscoverOnly,
            identity_evidence: Vec::new(),
            provenance: Vec::new(),
            match_basis: MatchBasis::Appid,
            observed_at: "2026-01-01T00:00:00+00:00".to_string(),
            cache_status: crate::sources::model::CacheStatus::Cached,
            warnings: Vec::new(),
            fingerprint: "f".repeat(64),
            related_index_pages: Vec::new(),
        }
    }

    #[test]
    fn selection_id_is_stable_and_field_sensitive() {
        let base = candidate();
        let id = zip_selection_id(&base).unwrap();
        assert_eq!(id, zip_selection_id(&base).unwrap());
        assert!(id.starts_with("zip-selection-"));
        assert!(id.ends_with("-42"));
        assert_eq!(parse_zip_selection_id(&id), Some(42));

        // Variant change.
        let mut variant = base.clone();
        variant.variant = Variant::Dlc;
        assert_ne!(zip_selection_id(&variant).unwrap(), id);

        // Language change.
        let mut language = base.clone();
        language.languages = vec!["en".to_string()];
        assert_ne!(zip_selection_id(&language).unwrap(), id);

        // version_identity change.
        let mut version = base.clone();
        version.version_identity = "2".to_string();
        assert_ne!(zip_selection_id(&version).unwrap(), id);

        // DLC condition change.
        let mut dlc = base.clone();
        dlc.requires_dlc = Some(vec![7]);
        assert_ne!(zip_selection_id(&dlc).unwrap(), id);

        // ZIP mirror change.
        let mut mirror_change = base.clone();
        mirror_change.mirrors = vec![mirror("https://dl.example/b.zip", "zip")];
        assert_ne!(zip_selection_id(&mirror_change).unwrap(), id);

        // Label-only change does not invalidate (labels are dropped).
        let mut labelled = base.clone();
        labelled.mirrors[0].label = Some("other".to_string());
        assert_eq!(zip_selection_id(&labelled).unwrap(), id);

        // A non-ZIP mirror does not count.
        let mut exe_only = base.clone();
        exe_only.mirrors = vec![mirror("https://dl.example/a.exe", "exe")];
        assert!(zip_selection_id(&exe_only).is_none());

        // No AppID → no selection.
        let mut no_appid = base.clone();
        no_appid.game_appid = None;
        assert!(zip_selection_id(&no_appid).is_none());
    }

    #[test]
    fn selection_id_prefix_is_isolated_from_exe_selection() {
        let id = zip_selection_id(&candidate()).unwrap();
        assert_ne!(
            crate::sources::canonical::canonical_sha256(&id),
            crate::sources::canonical::canonical_sha256(&format!(
                "exe-selection-{}-42",
                &id["zip-selection-".len().."zip-selection-".len() + 24]
            ))
        );
    }

    #[test]
    fn parse_rejects_hostile_selection_ids() {
        for bad in [
            "zip-selection-abc-42",
            "zip-selection-ABCDEF0123456789abcdef01-42",
            "zip-selection-0123456789abcdef01234567-0",
            "zip-selection-0123456789abcdef01234567-",
            "exe-selection-0123456789abcdef01234567-42",
        ] {
            assert!(parse_zip_selection_id(bad).is_none(), "{bad}");
        }
        assert_eq!(
            parse_zip_selection_id("zip-selection-0123456789abcdef01234567-2303800"),
            Some(2_303_800)
        );
    }

    #[test]
    fn released_gate_handles_dates_and_coming_soon() {
        // 2026-01-01T00:00:00Z.
        let now = 1_767_225_600;
        assert!(source_released(None, now));
        assert!(source_released(Some("2025/12/31"), now));
        assert!(source_released(Some("2026/1/1"), now));
        assert!(!source_released(Some("2026/1/2"), now));
        assert!(!source_released(Some("COMING SOON"), now));
    }

    #[test]
    fn resolve_reports_selection_not_found_for_unknown_but_well_formed_id() {
        let temp = tempfile::tempdir().unwrap();
        let sources = crate::sources::Sources::open(temp.path()).unwrap();
        let game = crate::sources::GameQuery {
            appid: 42,
            name: "Game".to_string(),
            localized_name: None,
            instance_id: None,
        };
        assert_eq!(
            resolve_zip(
                sources.registry(),
                &game,
                "zip-selection-0123456789abcdef01234567-42"
            )
            .unwrap_err()
            .code,
            "selection_not_found"
        );
        assert_eq!(
            resolve_zip(sources.registry(), &game, "nope")
                .unwrap_err()
                .code,
            "invalid_selection_id"
        );
        assert_eq!(
            resolve_zip(
                sources.registry(),
                &game,
                "zip-selection-0123456789abcdef01234567-43"
            )
            .unwrap_err()
            .code,
            "selection_not_found"
        );
    }

    fn seedable(version: &str) -> SourceCandidate {
        let mut value = candidate();
        value.source_page = "https://kagurafan.com/patches/game".to_string();
        value.source_record_id = value.source_page.clone();
        value.mirrors = vec![mirror("https://kaguraserver.com/a.zip", "zip")];
        value.version_identity = version.to_string();
        value
    }

    fn seed(state: &std::path::Path, candidate: SourceCandidate) {
        use crate::sources::model::{
            GameMatchRecord, GameMatchStatus, SourceMatchRecord, SourceMatchStatus,
        };
        let handle = crate::sources::Sources::open(state).unwrap();
        let source_id = candidate.source_id.clone();
        handle
            .store()
            .save_match(&GameMatchRecord {
                schema_version: 1,
                appid: 42,
                name: "Game".to_string(),
                localized_name: None,
                input_key: "key".to_string(),
                status: GameMatchStatus::PublisherMatch,
                sources: std::collections::BTreeMap::from([(
                    source_id,
                    SourceMatchRecord {
                        index_revision: 1,
                        status: SourceMatchStatus::Matched,
                        candidates: vec![candidate],
                        detail_pages: Vec::new(),
                        checked_at: 100,
                        diagnostics: Vec::new(),
                    },
                )]),
                fallback: None,
                updated_at: 100,
            })
            .unwrap();
    }

    fn seeded_registry(state: &std::path::Path, candidate: &SourceCandidate) -> String {
        let id = zip_selection_id(candidate).unwrap();
        seed(state, candidate.clone());
        id
    }

    fn game() -> crate::sources::GameQuery {
        crate::sources::GameQuery {
            appid: 42,
            name: "Game".to_string(),
            localized_name: None,
            instance_id: None,
        }
    }

    #[test]
    fn resolve_selects_first_legal_zip_mirror() {
        let temp = tempfile::tempdir().unwrap();
        let mut candidate = seedable("1");
        // Same-named mirrors on two whitelisted hosts (the real kagura-cn v4
        // shape): the deterministic choice is the first in original order.
        candidate.mirrors = vec![
            mirror("https://kaguraserver.com/a.zip", "zip"),
            mirror("https://kaguragamespatch.com/a.zip", "zip"),
        ];
        let selection_id = seeded_registry(temp.path(), &candidate);
        let sources = crate::sources::Sources::open(temp.path()).unwrap();
        let resolved = resolve_zip(sources.registry(), &game(), &selection_id).unwrap();
        assert_eq!(resolved.download_url, "https://kaguraserver.com/a.zip");
    }

    #[test]
    fn resolve_blocks_heterogeneous_zip_mirrors() {
        let temp = tempfile::tempdir().unwrap();
        let mut candidate = seedable("1");
        // Distinct basenames under the same candidate look like two different
        // artifacts, so the multi-mirror choice stays guarded.
        candidate.mirrors = vec![
            mirror("https://kaguraserver.com/a.zip", "zip"),
            mirror("https://kaguragamespatch.com/b.zip", "zip"),
        ];
        let selection_id = seeded_registry(temp.path(), &candidate);
        let sources = crate::sources::Sources::open(temp.path()).unwrap();
        assert_eq!(
            resolve_zip(sources.registry(), &game(), &selection_id)
                .unwrap_err()
                .code,
            "zip_download_ambiguous"
        );
    }

    #[test]
    fn resolve_rejects_a_mirror_outside_the_download_hosts() {
        let temp = tempfile::tempdir().unwrap();
        let mut candidate = seedable("1");
        candidate.mirrors = vec![mirror("https://evil.example.com/a.zip", "zip")];
        let selection_id = seeded_registry(temp.path(), &candidate);
        let sources = crate::sources::Sources::open(temp.path()).unwrap();
        // The only mirror is illegal, so nothing survives the filter.
        assert_eq!(
            resolve_zip(sources.registry(), &game(), &selection_id)
                .unwrap_err()
                .code,
            "invalid_download_url"
        );
    }

    #[test]
    fn resolve_skips_illegal_and_selects_the_legal_zip_mirror() {
        let temp = tempfile::tempdir().unwrap();
        let mut candidate = seedable("1");
        candidate.mirrors = vec![
            mirror("https://evil.example.com/a.zip", "zip"),
            mirror("https://kaguraserver.com/a.zip", "zip"),
        ];
        let selection_id = seeded_registry(temp.path(), &candidate);
        let sources = crate::sources::Sources::open(temp.path()).unwrap();
        let resolved = resolve_zip(sources.registry(), &game(), &selection_id).unwrap();
        assert_eq!(resolved.download_url, "https://kaguraserver.com/a.zip");
    }

    #[test]
    fn candidate_drift_is_reported_as_selection_not_found() {
        let temp = tempfile::tempdir().unwrap();
        let stale = seedable("1");
        let stale_id = seeded_registry(temp.path(), &stale);
        // A later discovery replaces the candidate; the recomputed digest no
        // longer matches the caller's selection id.
        seed(temp.path(), seedable("2"));
        let sources = crate::sources::Sources::open(temp.path()).unwrap();
        assert_eq!(
            resolve_zip(sources.registry(), &game(), &stale_id)
                .unwrap_err()
                .code,
            "selection_not_found"
        );
    }

    #[test]
    fn candidate_without_a_zip_mirror_has_no_selection_identity() {
        let temp = tempfile::tempdir().unwrap();
        let mut candidate = seedable("1");
        candidate.mirrors = vec![mirror("https://kaguraserver.com/a.rar", "unknown")];
        assert!(zip_selection_id(&candidate).is_none());
        // Seeding produces no projectable candidate, so any selection is absent.
        seed(temp.path(), candidate);
        let sources = crate::sources::Sources::open(temp.path()).unwrap();
        assert_eq!(
            resolve_zip(
                sources.registry(),
                &game(),
                "zip-selection-0123456789abcdef01234567-42"
            )
            .unwrap_err()
            .code,
            "selection_not_found"
        );
    }

    #[test]
    fn resolve_zip_projects_patchmoe_index_candidate() {
        let temp = tempfile::tempdir().unwrap();
        // Seed the patchmoe index snapshot, then reopen so the registry loads it.
        let handle = crate::sources::Sources::open(temp.path()).unwrap();
        handle
            .store()
            .save_index(&crate::sources::model::SourceIndex {
                parser_fingerprint: None,
                schema_version: 1,
                source_id: "patchmoe".to_string(),
                revision: 1,
                complete: true,
                checked_at: 1000,
                updated_at: 1000,
                etag: None,
                modified: None,
                entries: vec![crate::sources::model::SourceIndexEntry {
                    title: "Kudo's Kuppa".to_string(),
                    aliases: Vec::new(),
                    page: "https://www.patch.moe/Kudos-Kuppa".to_string(),
                    kind: "entry".to_string(),
                    appid: Some(42),
                    url: Some("https://download.patch.moe/Kudos_Kuppa_patch.zip".to_string()),
                    language: Some("zh-CN".to_string()),
                    date: Some("2026/6/23".to_string()),
                }],
                pending: Vec::new(),
            })
            .unwrap();
        drop(handle);

        let sources = crate::sources::Sources::open(temp.path()).unwrap();
        let candidate = sources
            .registry()
            .index_candidates(&game())
            .into_iter()
            .next()
            .unwrap();
        let selection_id = zip_selection_id(&candidate).unwrap();
        let resolved = resolve_zip(sources.registry(), &game(), &selection_id).unwrap();
        assert_eq!(
            resolved.download_url,
            "https://download.patch.moe/Kudos_Kuppa_patch.zip"
        );
        assert_eq!(resolved.release_id, "patchmoe-42");
        // `choose_language` prefers zh-CN.
        assert_eq!(resolved.language, "zh-CN");
    }

    #[test]
    fn persisted_match_remains_installable_without_ttl_reevaluation() {
        let temp = tempfile::tempdir().unwrap();
        let candidate = seedable("1");
        let selection_id = zip_selection_id(&candidate).unwrap();
        seed(temp.path(), candidate);
        // Reopening the read model later still resolves the persisted match:
        // nothing re-evaluates cache age or freshness.
        let sources = crate::sources::Sources::open(temp.path()).unwrap();
        assert!(resolve_zip(sources.registry(), &game(), &selection_id).is_ok());
    }
}
