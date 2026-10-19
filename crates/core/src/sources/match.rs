//! Game match records: input digest and top-level status derivation
//! (p1-contract §7.1/§7.2).
//!
//! Only real matching inputs participate in [`input_key`]; viewing state,
//! cache age and update timestamps deliberately do not, so they never trigger a
//! rematch.

use super::canonical;
use super::model::{
    FallbackMatchStatus, GameMatchRecord, GameMatchStatus, MatchBasis, SourceCandidate,
    SourceMatchStatus,
};
use std::collections::BTreeMap;

/// `sha256(canonical_json({name, localized_name, name_hints,
/// preferred_language, subscriptions}))`.
///
/// `name_hints`/`preferred_language` are real matching inputs: the trusted
/// localized name, frontend name hints and the preferred patch language change
/// the publisher/fallback query context, so a change must trigger a rematch.
/// `subscriptions` is the per-game, matching-relevant source snapshot
/// (`source_id -> active`), not the full settings document. `BTreeMap` keeps the
/// key stable across insertion order.
pub fn input_key(
    name: &str,
    localized_name: Option<&str>,
    name_hints: &[String],
    preferred_language: Option<&str>,
    subscriptions: &BTreeMap<String, bool>,
) -> String {
    #[derive(serde::Serialize)]
    struct MatchingInput<'a> {
        name: &'a str,
        localized_name: Option<&'a str>,
        name_hints: &'a [String],
        preferred_language: Option<&'a str>,
        subscriptions: &'a BTreeMap<String, bool>,
    }
    canonical::canonical_sha256(&MatchingInput {
        name,
        localized_name,
        name_hints,
        preferred_language,
        subscriptions,
    })
}

/// The identity bases that count as a confirmed source match; title-only
/// candidates are leads that must not suppress the fallback query.
pub fn candidate_is_credible(candidate: &SourceCandidate) -> bool {
    matches!(
        candidate.match_basis,
        MatchBasis::Appid | MatchBasis::VerifiedAlias | MatchBasis::OfficialTitle
    )
}

/// Derives the persisted top-level status from the per-source and fallback
/// results (p1-contract §7.1, design §3.2):
///
/// 1. a credible publisher match wins and hides the fallback;
/// 2. complete publisher coverage + fallback match → fallback match;
/// 3. complete publisher coverage, no match + fallback no-match/disabled → no match;
/// 4. any uninitialized/incomplete/failed publisher coverage or an unfinished
///    fallback query → incomplete (a positive result may still be displayed);
/// 5. only when every attempt failed → failed.
pub fn derive_game_status(record: &GameMatchRecord) -> GameMatchStatus {
    if record
        .sources
        .values()
        .any(|source| source.status == SourceMatchStatus::Matched)
    {
        return GameMatchStatus::PublisherMatch;
    }
    if record.sources.is_empty() && record.fallback.is_none() {
        return GameMatchStatus::Pending;
    }

    let sources_incomplete = record.sources.values().any(|source| {
        matches!(
            source.status,
            SourceMatchStatus::NoIndex
                | SourceMatchStatus::IndexIncomplete
                | SourceMatchStatus::DetailIncomplete
                | SourceMatchStatus::DetailFailed
        )
    });
    let all_sources_failed = !record.sources.is_empty()
        && record
            .sources
            .values()
            .all(|source| source.status == SourceMatchStatus::DetailFailed);
    let fallback = record.fallback.as_ref().map(|fallback| fallback.status);

    if sources_incomplete || fallback == Some(FallbackMatchStatus::Failed) {
        let failed = fallback == Some(FallbackMatchStatus::Failed)
            && (record.sources.is_empty() || all_sources_failed);
        return if failed {
            GameMatchStatus::Failed
        } else {
            GameMatchStatus::Incomplete
        };
    }

    match fallback {
        Some(FallbackMatchStatus::Matched) => GameMatchStatus::FallbackMatch,
        Some(FallbackMatchStatus::NoMatch) | Some(FallbackMatchStatus::Disabled) => {
            GameMatchStatus::NoMatch
        }
        // `not_needed` is only legitimate when a publisher source matched (rule 1);
        // reaching here means the fallback was never actually covered.
        Some(
            FallbackMatchStatus::NotQueried
            | FallbackMatchStatus::Incomplete
            | FallbackMatchStatus::NotNeeded,
        )
        | None => GameMatchStatus::Incomplete,
        Some(FallbackMatchStatus::Failed) => GameMatchStatus::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::model::{FallbackMatchRecord, SourceMatchRecord};

    fn source(status: SourceMatchStatus) -> SourceMatchRecord {
        SourceMatchRecord {
            index_revision: 1,
            status,
            candidates: Vec::new(),
            detail_pages: Vec::new(),
            checked_at: 1,
            diagnostics: Vec::new(),
        }
    }

    fn fallback(status: FallbackMatchStatus) -> FallbackMatchRecord {
        FallbackMatchRecord {
            query_key: "q".to_string(),
            status,
            candidates: Vec::new(),
            checked_at: 1,
            diagnostics: Vec::new(),
        }
    }

    fn record(
        sources: &[(&str, SourceMatchStatus)],
        fallback_status: Option<FallbackMatchStatus>,
    ) -> GameMatchRecord {
        GameMatchRecord {
            schema_version: 1,
            appid: 42,
            name: "Game".to_string(),
            localized_name: None,
            input_key: "key".to_string(),
            status: GameMatchStatus::Pending,
            sources: sources
                .iter()
                .map(|(id, status)| (id.to_string(), source(*status)))
                .collect(),
            fallback: fallback_status.map(fallback),
            updated_at: 1,
        }
    }

    #[test]
    fn input_key_changes_only_with_matching_inputs() {
        let subscriptions = BTreeMap::from([("patchmoe".to_string(), true)]);
        let base = input_key("Game", Some("游戏"), &[], None, &subscriptions);
        assert_eq!(
            base,
            input_key("Game", Some("游戏"), &[], None, &subscriptions)
        );
        assert_ne!(
            base,
            input_key("Game 2", Some("游戏"), &[], None, &subscriptions)
        );
        assert_ne!(base, input_key("Game", None, &[], None, &subscriptions));
        // Frontend name hints and the preferred language are matching inputs.
        let hints = vec!["Alias".to_string()];
        assert_ne!(
            base,
            input_key("Game", Some("游戏"), &hints, None, &subscriptions)
        );
        assert_ne!(
            base,
            input_key("Game", Some("游戏"), &[], Some("en"), &subscriptions)
        );
        let other = BTreeMap::from([("patchmoe".to_string(), false)]);
        assert_ne!(base, input_key("Game", Some("游戏"), &[], None, &other));
        // A source added to the snapshot changes the key; map order does not.
        let mut reordered = BTreeMap::from([
            ("patchmoe".to_string(), true),
            ("hgamer".to_string(), false),
        ]);
        assert_eq!(
            input_key("Game", Some("游戏"), &[], None, &reordered),
            input_key("Game", Some("游戏"), &[], None, &reordered.clone())
        );
        reordered.remove("hgamer");
        assert_eq!(base, input_key("Game", Some("游戏"), &[], None, &reordered));
    }

    #[test]
    fn publisher_match_wins_over_fallback() {
        assert_eq!(
            derive_game_status(&record(
                &[("patchmoe", SourceMatchStatus::Matched)],
                Some(FallbackMatchStatus::Matched)
            )),
            GameMatchStatus::PublisherMatch
        );
    }

    #[test]
    fn complete_coverage_with_fallback_match_is_fallback_match() {
        assert_eq!(
            derive_game_status(&record(
                &[("kagura-cn", SourceMatchStatus::NoCandidate)],
                Some(FallbackMatchStatus::Matched)
            )),
            GameMatchStatus::FallbackMatch
        );
    }

    #[test]
    fn complete_coverage_with_fallback_no_match_is_no_match() {
        assert_eq!(
            derive_game_status(&record(
                &[("kagura-cn", SourceMatchStatus::NoCandidate)],
                Some(FallbackMatchStatus::NoMatch)
            )),
            GameMatchStatus::NoMatch
        );
        // A disabled fallback means "not found in enabled sources", not "HGamer
        // confirmed no patch".
        assert_eq!(
            derive_game_status(&record(
                &[("kagura-cn", SourceMatchStatus::NoCandidate)],
                Some(FallbackMatchStatus::Disabled)
            )),
            GameMatchStatus::NoMatch
        );
    }

    #[test]
    fn incomplete_coverage_or_query_is_incomplete() {
        // Publisher source never initialized.
        assert_eq!(
            derive_game_status(&record(
                &[("patchmoe", SourceMatchStatus::NoIndex)],
                Some(FallbackMatchStatus::Matched)
            )),
            GameMatchStatus::Incomplete
        );
        // Index incomplete.
        assert_eq!(
            derive_game_status(&record(
                &[("kagura-cn", SourceMatchStatus::IndexIncomplete)],
                None
            )),
            GameMatchStatus::Incomplete
        );
        // Detail work not finished.
        assert_eq!(
            derive_game_status(&record(
                &[("kagura-cn", SourceMatchStatus::DetailIncomplete)],
                Some(FallbackMatchStatus::NoMatch)
            )),
            GameMatchStatus::Incomplete
        );
        // Fallback never queried despite complete publisher coverage.
        assert_eq!(
            derive_game_status(&record(
                &[("kagura-cn", SourceMatchStatus::NoCandidate)],
                None
            )),
            GameMatchStatus::Incomplete
        );
        // Fallback failed.
        assert_eq!(
            derive_game_status(&record(
                &[("kagura-cn", SourceMatchStatus::NoCandidate)],
                Some(FallbackMatchStatus::Failed)
            )),
            GameMatchStatus::Incomplete
        );
    }

    #[test]
    fn all_failed_is_failed() {
        assert_eq!(
            derive_game_status(&record(
                &[("kagura-cn", SourceMatchStatus::DetailFailed)],
                Some(FallbackMatchStatus::Failed)
            )),
            GameMatchStatus::Failed
        );
        assert_eq!(
            derive_game_status(&record(&[], Some(FallbackMatchStatus::Failed))),
            GameMatchStatus::Failed
        );
    }

    #[test]
    fn fresh_record_is_pending() {
        assert_eq!(
            derive_game_status(&record(&[], None)),
            GameMatchStatus::Pending
        );
    }
}
