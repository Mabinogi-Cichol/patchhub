//! Explicit user selections bind an unchanged cached candidate, download and target instance.
//! They do not rewrite publisher metadata or claim an automatic identity match.
use super::resolve::{
    select_download_url, source_admission_blocker, source_release_id, MirrorKind, ResolveError,
    ResolveResult, ResolvedZipCandidate,
};
use crate::sources::{canonical, model::SourceCandidate, registry::SourceRegistry, GameQuery};
use serde_json::json;

pub fn selection_id(
    candidate: &SourceCandidate,
    game: &GameQuery,
    mirror: &crate::sources::model::Mirror,
) -> Option<String> {
    if !matches!(mirror.format.as_str(), "zip" | "exe")
        || game.instance_id.is_none()
        || candidate
            .game_appid
            .is_some_and(|appid| appid != game.appid)
    {
        return None;
    }
    let binding = json!({"manual_selection":true,"instance_id":game.instance_id,"appid":game.appid,
        "candidate":canonical::fingerprint(&serde_json::to_value(candidate).ok()?),"candidate_id":candidate.candidate_id,"download":mirror.url,"format":mirror.format});
    Some(format!(
        "{}-selection-{}-{}",
        mirror.format,
        &canonical::canonical_sha256(&binding)[..24],
        game.appid
    ))
}

pub fn resolve(
    registry: &SourceRegistry,
    game: &GameQuery,
    id: &str,
    format: &str,
) -> ResolveResult<ResolvedZipCandidate> {
    for candidate in registry.dynamic_zip_candidates(game) {
        for mirror in &candidate.mirrors {
            if mirror.format != format
                || selection_id(&candidate, game, mirror).as_deref() != Some(id)
            {
                continue;
            }
            let rule = registry
                .rules()
                .iter()
                .find(|r| r.id == candidate.source_id)
                .ok_or_else(|| ResolveError::code("source_not_enabled"))?;
            if let Some(code) = source_admission_blocker(registry, &candidate, game.appid) {
                return Err(ResolveError::code(code));
            }
            let mut chosen = candidate.clone();
            chosen.mirrors = vec![mirror.clone()];
            let download_url = select_download_url(
                &chosen,
                rule,
                if format == "exe" {
                    MirrorKind::Exe
                } else {
                    MirrorKind::Zip
                },
            )?;
            let language = candidate
                .languages
                .iter()
                .find(|x| x.eq_ignore_ascii_case("zh-CN"))
                .or(candidate.languages.first())
                .cloned()
                .unwrap_or_default();
            return Ok(ResolvedZipCandidate {
                selection_id: id.into(),
                release_id: source_release_id(rule, game.appid),
                appid: game.appid,
                source_id: rule.id.clone(),
                source_record_id: candidate.source_record_id.clone(),
                candidate_sha256: candidate.fingerprint.clone(),
                rule_fingerprint: crate::sources::Sources::rule_fingerprint(rule),
                settings_revision: registry.settings_revision(),
                candidate: candidate.clone(),
                rule: rule.clone(),
                download_url,
                language,
            });
        }
    }
    Err(ResolveError::code("selection_not_found"))
}
