//! EXE selections bind authoritative source candidates, never download permission.
use super::{
    facts::FactsExpectation,
    resolve::{select_download_url, MirrorKind, ResolveError, ResolveResult},
};
use crate::sources::{
    canonical,
    model::{CacheStatus, SourceCandidate, SourceRole, Variant},
    registry::SourceRegistry,
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
        // EXE publisher sources keep the language gate (legacy behaviour).
        require_language: true,
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
    // demo/unknown variants can never yield a supported EXE content identity;
    // reject them at candidate admission so the CLI surfaces a clean blocker
    // instead of failing mid-plan with `exe_content_identity_invalid`.
    if !matches!(candidate.variant, Variant::Base | Variant::Dlc) {
        return Err(ResolveError::code("exe_candidate_variant_unsupported"));
    }
    let download_url = select_download_url(&candidate, &rule, MirrorKind::Exe)?;
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
        download_url,
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
    #[test]
    fn demo_and_unknown_variants_are_blocked_at_resolution() {
        use crate::sources::model::{ArtifactFormat, DiscoveryRecord};
        use crate::sources::registry::DiscoveryOutcome;
        use crate::sources::Sources;
        for variant in [Variant::Demo, Variant::Unknown] {
            let temp = tempfile::tempdir().unwrap();
            let mut candidate = super::super::orchestrator::tests::candidate();
            candidate.variant = variant;
            candidate.mirrors[0].format = "exe".into();
            candidate.mirrors[0].url = "https://kaguraserver.com/a.exe".into();
            candidate.format_reported = ArtifactFormat::Exe;
            candidate.format_observed = ArtifactFormat::Exe;
            // Projection identity is still generated for demo/unknown candidates;
            // only resolution rejects them, so the CLI can emit a clean blocker.
            let selection = exe_selection_id(&candidate).expect("projection id");
            let mut sources = Sources::open(temp.path()).unwrap();
            let rule = sources
                .registry()
                .rules()
                .iter()
                .find(|rule| rule.id == "kagura-cn")
                .unwrap()
                .clone();
            sources
                .registry_mut()
                .commit(
                    &rule,
                    candidate.game_appid.unwrap(),
                    DiscoveryOutcome {
                        status: "complete".into(),
                        candidates: vec![candidate],
                        ..Default::default()
                    },
                    &DiscoveryRecord::default(),
                    "test",
                )
                .unwrap();
            assert_eq!(
                resolve_exe(
                    sources.registry(),
                    &super::super::orchestrator::tests::query(),
                    &selection,
                )
                .unwrap_err()
                .code,
                "exe_candidate_variant_unsupported",
                "{variant:?}",
            );
        }
    }

    #[test]
    fn resolve_exe_selects_first_of_homogeneous_mirrors() {
        use crate::sources::model::{ArtifactFormat, DiscoveryRecord};
        use crate::sources::registry::DiscoveryOutcome;
        use crate::sources::Sources;
        let temp = tempfile::tempdir().unwrap();
        let mut candidate = super::super::orchestrator::tests::candidate();
        candidate.mirrors[0].format = "exe".into();
        candidate.mirrors[0].url = "https://kaguraserver.com/a.exe".into();
        let mut second = candidate.mirrors[0].clone();
        second.url = "https://kaguragamespatch.com/a.exe".into();
        candidate.mirrors.push(second);
        candidate.format_reported = ArtifactFormat::Exe;
        candidate.format_observed = ArtifactFormat::Exe;
        let selection = exe_selection_id(&candidate).unwrap();
        let mut sources = Sources::open(temp.path()).unwrap();
        let rule = sources
            .registry()
            .rules()
            .iter()
            .find(|rule| rule.id == "kagura-cn")
            .unwrap()
            .clone();
        sources
            .registry_mut()
            .commit(
                &rule,
                candidate.game_appid.unwrap(),
                DiscoveryOutcome {
                    status: "complete".into(),
                    candidates: vec![candidate],
                    ..Default::default()
                },
                &DiscoveryRecord::default(),
                "test",
            )
            .unwrap();
        let resolved = resolve_exe(
            sources.registry(),
            &super::super::orchestrator::tests::query(),
            &selection,
        )
        .unwrap();
        assert_eq!(resolved.download_url, "https://kaguraserver.com/a.exe");
        // EXE publisher candidates keep the language gate.
        assert!(expectation(&resolved).require_language);
    }

    #[test]
    fn resolve_exe_blocks_heterogeneous_mirrors() {
        use crate::sources::model::{ArtifactFormat, DiscoveryRecord};
        use crate::sources::registry::DiscoveryOutcome;
        use crate::sources::Sources;
        let temp = tempfile::tempdir().unwrap();
        let mut candidate = super::super::orchestrator::tests::candidate();
        candidate.mirrors[0].format = "exe".into();
        candidate.mirrors[0].url = "https://kaguraserver.com/a.exe".into();
        let mut second = candidate.mirrors[0].clone();
        second.url = "https://kaguragamespatch.com/b.exe".into();
        candidate.mirrors.push(second);
        candidate.format_reported = ArtifactFormat::Exe;
        candidate.format_observed = ArtifactFormat::Exe;
        let selection = exe_selection_id(&candidate).unwrap();
        let mut sources = Sources::open(temp.path()).unwrap();
        let rule = sources
            .registry()
            .rules()
            .iter()
            .find(|rule| rule.id == "kagura-cn")
            .unwrap()
            .clone();
        sources
            .registry_mut()
            .commit(
                &rule,
                candidate.game_appid.unwrap(),
                DiscoveryOutcome {
                    status: "complete".into(),
                    candidates: vec![candidate],
                    ..Default::default()
                },
                &DiscoveryRecord::default(),
                "test",
            )
            .unwrap();
        assert_eq!(
            resolve_exe(
                sources.registry(),
                &super::super::orchestrator::tests::query(),
                &selection,
            )
            .unwrap_err()
            .code,
            "exe_download_ambiguous"
        );
    }
}
