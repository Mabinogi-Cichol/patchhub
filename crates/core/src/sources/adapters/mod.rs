//! One source executor. Legacy names only select bundled Lua text.
use super::http::RequestContext;
use super::identity::Identity;
use super::model::{
    Diagnostic, QueryContext, SourceCandidate, SourceIndex, SourceIndexEntry, SourceRule,
};
use super::{identity, invalid, Result};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::{SystemTime, UNIX_EPOCH};
pub mod script;
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IndexOutcome {
    pub entries: Vec<SourceIndexEntry>,
    pub pending: Vec<String>,
    pub complete: bool,
    pub etag: Option<String>,
    pub modified: Option<String>,
    pub not_modified: bool,
    pub checked_at: u64,
    pub diagnostics: Vec<Diagnostic>,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiscoverOutcome {
    pub status: String,
    pub candidates: Vec<SourceCandidate>,
    pub diagnostics: Vec<Diagnostic>,
    pub identities: Vec<identity::Identity>,
}
#[derive(Debug, Clone, Default)]
pub struct DiscoverPrevious {
    pub known_candidates: Vec<SourceCandidate>,
}
pub struct SourceAdapter<'a> {
    code: &'a str,
}
pub fn source_adapter<'a>(rule: &SourceRule, code: Option<&'a str>) -> Result<SourceAdapter<'a>> {
    let bundled = super::builtin::code(&rule.adapter);
    if rule.adapter != "lua" && bundled.is_none() {
        return Err(invalid("unknown_source_adapter"));
    }
    let code = code
        .or(bundled)
        .ok_or_else(|| invalid(format!("script_source_missing: {}", rule.id)))?;
    Ok(SourceAdapter { code })
}
impl SourceAdapter<'_> {
    pub fn discover(
        &self,
        rule: &SourceRule,
        query: &QueryContext,
        ctx: &mut RequestContext,
        previous: &DiscoverPrevious,
        cancel: &Arc<AtomicBool>,
    ) -> Result<DiscoverOutcome> {
        let code = self.code;
        let facts = script::discover(rule, code, query, previous, ctx, Arc::clone(cancel), |_| {})?;
        let identities: Vec<Identity> = facts
            .identities
            .into_iter()
            .filter(|identity| identity.appid == query.appid)
            .map(|identity| Identity {
                appid: identity.appid,
                page: identity.page,
                aliases: identity.aliases,
            })
            .collect();
        let mut identity_query = query.clone();
        identity_query.name_hints.extend(
            identities
                .iter()
                .flat_map(|identity| identity.aliases.iter().cloned()),
        );
        let mut candidates = Vec::new();
        for page in facts.pages {
            let mut pending = vec![page.fact];
            while let Some(fact) = pending.pop() {
                let mut found = identity::build_candidates(
                    rule,
                    &fact.title,
                    &fact.page,
                    &fact.text,
                    &fact.steam_ids,
                    &fact.aliases,
                    &page.reported_format,
                    &fact.mirrors,
                    &identity_query,
                );
                for candidate in &mut found {
                    candidate.published_at = fact.published_at.clone();
                    candidate.upstream_page = page.upstream_page.clone();
                }
                candidates.extend(found);
                pending.extend(fact.article_facts.into_iter().rev());
            }
        }
        let candidates = identity::enrich_candidates(&candidates, &identities);
        let mut candidates = identity::merge_candidates(&candidates);
        for candidate in &mut candidates {
            super::index::bind_candidate(rule, candidate);
            identity::validate_candidate(candidate, rule)?;
        }
        Ok(DiscoverOutcome {
            status: if facts.complete {
                "complete"
            } else {
                "partial"
            }
            .into(),
            candidates,
            diagnostics: facts.diagnostics,
            identities,
        })
    }
    pub fn collect_index(
        &self,
        rule: &SourceRule,
        previous: Option<&SourceIndex>,
        ctx: &mut RequestContext,
        cancel: &Arc<AtomicBool>,
    ) -> Result<IndexOutcome> {
        script::collect_index(rule, self.code, previous, ctx, Arc::clone(cancel), |_| {})
    }
    pub fn resolve_page(
        &self,
        rule: &SourceRule,
        page: &str,
        ctx: &mut RequestContext,
        cancel: &Arc<AtomicBool>,
    ) -> Result<super::model::PageFact> {
        script::resolve_page(rule, self.code, page, ctx, Arc::clone(cancel), |_| {})
    }
}
pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .unwrap_or(0)
}
