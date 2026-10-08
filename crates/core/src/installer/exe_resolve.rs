//! EXE selections bind authoritative source candidates, never download permission.
use super::{
    facts::FactsExpectation,
    resolve::{ResolveError, ResolveResult},
};
use crate::sources::{
    canonical,
    model::{CacheStatus, SourceCandidate, SourceRole},
    registry::SourceRegistry,
    validate,
};
use serde_json::json;
pub type ResolvedExeCandidate = super::resolve::ResolvedZipCandidate;
pub fn exe_selection_id(candidate: &SourceCandidate) -> Option<String> {
    let appid = candidate.game_appid?;
    if !candidate.mirrors.iter().any(|m| m.format == "exe") {
        return None;
    }
    let binding = json!({"source_id":candidate.source_id,"source_record_id":candidate.source_record_id,
        "game_appid":appid,"languages":candidate.languages,"variant":candidate.variant,
        "version_identity":candidate.version_identity,"source_page":candidate.source_page,
        "requires_dlc":candidate.requires_dlc,"excludes_dlc":candidate.excludes_dlc,"mirrors":candidate.mirrors});
    Some(format!(
        "exe-selection-{}-{appid}",
        &canonical::canonical_sha256(&binding)[..24]
    ))
}
pub fn parse_exe_selection_id(id: &str) -> Option<u32> {
    let rest = id.strip_prefix("exe-selection-")?;
    super::resolve::parse_zip_selection_id(&format!("zip-selection-{rest}"))
}
pub fn expectation(selection: &ResolvedExeCandidate) -> FactsExpectation {
    FactsExpectation {
        appid: selection.appid,
        language: selection.language.clone(),
        verified_buildids: Vec::new(),
        requires_dlc: selection.candidate.requires_dlc.clone().unwrap_or_default(),
        excludes_dlc: selection.candidate.excludes_dlc.clone().unwrap_or_default(),
        baselines: Vec::new(),
    }
}
pub fn resolve_exe(
    registry: &SourceRegistry,
    game: &crate::sources::GameQuery,
    id: &str,
) -> ResolveResult<ResolvedExeCandidate> {
    let appid =
        parse_exe_selection_id(id).ok_or_else(|| ResolveError::code("invalid_selection_id"))?;
    if appid != game.appid {
        return Err(ResolveError::code("selection_not_found"));
    }
    let candidate = registry
        .cached_discovery(game)
        .candidates
        .into_iter()
        .find(|c| exe_selection_id(c).as_deref() == Some(id))
        .ok_or_else(|| ResolveError::code("selection_not_found"))?;
    let rule = registry
        .rules()
        .iter()
        .find(|r| r.id == candidate.source_id)
        .cloned()
        .ok_or_else(|| ResolveError::code("exe_source_disabled"))?;
    if !matches!(rule.id.as_str(), "kagura-cn" | "kagura-en")
        || rule.role != SourceRole::Publisher
        || !rule.enabled
        || !registry.game_subscribed(appid, &rule.id)
        || registry.source_origin(&rule.id) != Some("builtin")
    {
        return Err(ResolveError::code("exe_source_disabled"));
    }
    if candidate.cache_status == CacheStatus::Stale {
        return Err(ResolveError::code("selection_stale"));
    }
    if !matches!(
        candidate.match_basis,
        crate::sources::model::MatchBasis::Appid
            | crate::sources::model::MatchBasis::VerifiedAlias
            | crate::sources::model::MatchBasis::OfficialTitle
    ) {
        return Err(ResolveError::code("exe_candidate_ambiguous"));
    }
    let mut urls = Vec::new();
    for m in candidate.mirrors.iter().filter(|m| m.format == "exe") {
        let url = validate::allowed_url(&m.url, &rule.download_hosts, false)
            .map_err(|_| ResolveError::code("invalid_download_url"))?;
        if !url::Url::parse(&url)
            .map_err(|_| ResolveError::code("invalid_download_url"))?
            .path()
            .to_ascii_lowercase()
            .ends_with(".exe")
        {
            return Err(ResolveError::code("invalid_download_url"));
        }
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    if urls.len() != 1 {
        return Err(ResolveError::code("exe_download_ambiguous"));
    }
    let language = candidate
        .languages
        .iter()
        .find(|s| s.eq_ignore_ascii_case("zh-CN"))
        .or(candidate.languages.first())
        .cloned()
        .or(rule.language.clone())
        .unwrap_or_default();
    Ok(ResolvedExeCandidate {
        selection_id: id.into(),
        release_id: String::new(),
        appid,
        source_id: rule.id.clone(),
        source_record_id: candidate.source_record_id.clone(),
        candidate_sha256: candidate.fingerprint.clone(),
        rule_fingerprint: crate::sources::Sources::rule_fingerprint(&rule),
        settings_revision: registry.settings_revision(),
        candidate,
        rule,
        download_url: urls.remove(0),
        language,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_binds_all_mirrors_and_conditions_but_not_observation() {
        let mut c = super::super::orchestrator::tests::candidate();
        c.mirrors[0].format = "exe".into();
        c.mirrors[0].url = "https://kaguraserver.com/a.exe".into();
        let id = exe_selection_id(&c).unwrap();
        assert_eq!(parse_exe_selection_id(&id), c.game_appid);
        let mut changed = c.clone();
        changed.mirrors[0].label = Some("new label".into());
        assert_ne!(exe_selection_id(&changed).unwrap(), id);
        let mut changed = c.clone();
        changed.requires_dlc = Some(vec![99]);
        assert_ne!(exe_selection_id(&changed).unwrap(), id);
        let mut changed = c.clone();
        changed.languages = vec!["en".into()];
        assert_ne!(exe_selection_id(&changed).unwrap(), id);
        c.observed_at = "new date".into();
        c.fingerprint = "a".repeat(64);
        assert_eq!(exe_selection_id(&c).unwrap(), id);
    }
    #[test]
    fn id_parser_is_strict() {
        assert_eq!(
            parse_exe_selection_id("exe-selection-0123456789abcdef01234567-42"),
            Some(42)
        );
        for id in [
            "exe-selection-0123456789abcdef01234567-0",
            "zip-selection-0123456789abcdef01234567-42",
            "exe-selection-0123456789ABCDEF01234567-42",
        ] {
            assert!(parse_exe_selection_id(id).is_none());
        }
    }
}
