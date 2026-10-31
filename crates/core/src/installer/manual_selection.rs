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

/// Resolve an explicitly supplied URL against the actual selected source candidate.
/// No synthetic mirror is added to publisher metadata.
pub(crate) fn resolve_download(
    registry: &SourceRegistry,
    game: &GameQuery,
    input: &super::service::ManualDownloadInput,
) -> ResolveResult<ResolvedZipCandidate> {
    let candidate = registry
        .dynamic_zip_candidates(game)
        .into_iter()
        .find(|c| c.source_id == input.source_id && c.candidate_id == input.candidate_id)
        .ok_or_else(|| ResolveError::code("selection_not_found"))?;
    if candidate
        .game_appid
        .is_some_and(|appid| appid != game.appid)
    {
        return Err(ResolveError::code("selection_not_found"));
    }
    if let Some(code) = source_admission_blocker(registry, &candidate, game.appid) {
        return Err(ResolveError::code(code));
    }
    let rule = registry
        .rules()
        .iter()
        .find(|r| r.id == candidate.source_id)
        .ok_or_else(|| ResolveError::code("source_not_enabled"))?;
    let download_url = super::acquire::validate_manual_download(&input.url)
        .map_err(|_| ResolveError::code("invalid_download_url"))?;
    let binding = json!({"manual_download":true,"instance_id":game.instance_id,"appid":game.appid,
        "candidate":canonical::fingerprint(&serde_json::to_value(&candidate).map_err(|_| ResolveError::code("invalid_candidate"))?),
        "source_id":input.source_id,"candidate_id":input.candidate_id,"url":download_url});
    let selection_id = format!(
        "manual-download-{}-{}",
        &canonical::canonical_sha256(&binding)[..24],
        game.appid
    );
    let language = candidate
        .languages
        .iter()
        .find(|x| x.eq_ignore_ascii_case("zh-CN"))
        .or(candidate.languages.first())
        .cloned()
        .unwrap_or_default();
    Ok(ResolvedZipCandidate {
        selection_id,
        release_id: source_release_id(rule, game.appid),
        appid: game.appid,
        source_id: rule.id.clone(),
        source_record_id: candidate.source_record_id.clone(),
        candidate_sha256: candidate.fingerprint.clone(),
        rule_fingerprint: crate::sources::Sources::rule_fingerprint(rule),
        settings_revision: registry.settings_revision(),
        candidate,
        rule: rule.clone(),
        download_url,
        language,
    })
}

#[cfg(test)]
mod script_binding_tests {
    use super::*;
    use crate::sources::{
        self,
        http::{FakeFetcher, RequestContext},
        RawSourceRule, Sources,
    };
    use std::{
        sync::{atomic::AtomicBool, Arc},
        time::Duration,
    };

    #[test]
    fn explicit_url_binds_script_candidate_before_and_after_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let mut sources = Sources::open(temp.path()).unwrap();
        let raw: RawSourceRule=serde_json::from_value(json!({"id":"custom", "name":"Custom", "adapter":"lua", "enabled":true,
            "pages":["https://example.com/list"], "page_hosts":["example.com"], "download_hosts":["download.example.com"], "root_layout":"game_root"})).unwrap();
        let code="return {collect_index=function() return {entries={{title='Game',page='https://example.com/list',kind='entry',appid=42,url='https://download.example.com/a.zip'}},complete=true} end}";
        let game = GameQuery {
            appid: 42,
            name: "Game".into(),
            localized_name: None,
            instance_id: Some("test".into()),
        };
        let update = |sources: &Sources| {
            let rule = sources
                .registry()
                .rules()
                .iter()
                .find(|r| r.id == "custom")
                .unwrap();
            let mut ctx = RequestContext::with_settings(
                Box::new(Arc::new(FakeFetcher::new())),
                Duration::ZERO,
            );
            sources::update::update_source(
                sources.store(),
                rule,
                false,
                &mut ctx,
                &Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
        };
        sources.import_script_source(&raw, 1, code, false).unwrap();
        update(&sources);
        let old = sources.registry().index_candidates(&game).remove(0);
        let input = super::super::service::ManualDownloadInput {
            source_id: "custom".into(),
            candidate_id: old.candidate_id,
            url: "https://download.example.com/manual.zip".into(),
        };
        assert!(resolve_download(sources.registry(), &game, &input).is_ok());
        sources
            .import_script_source(&raw, 1, &format!("{code}\n-- parser B"), true)
            .unwrap();
        assert_eq!(
            resolve_download(sources.registry(), &game, &input)
                .unwrap_err()
                .code,
            "selection_not_found"
        );
        update(&sources);
        assert_eq!(
            resolve_download(sources.registry(), &game, &input)
                .unwrap_err()
                .code,
            "selection_not_found"
        );
        let current = sources.registry().index_candidates(&game).remove(0);
        assert!(resolve_download(
            sources.registry(),
            &game,
            &super::super::service::ManualDownloadInput {
                candidate_id: current.candidate_id,
                ..input
            }
        )
        .is_ok());
    }
}
