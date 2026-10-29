//! Adapter trait/registry plus the per-source index collectors.
//!
//! An adapter may support a ZIP-index `collect` (patchmoe/table, feeding
//! [`Adapter::collect_index`]) and/or a per-game `discover` used by the HGamer
//! fallback. Both default to `unsupported_source_adapter`. HTML parsing lives
//! here and in the concrete adapters; the identity/validation logic is reused
//! from `identity`/`validate`.

use super::canonical;
use super::http::{FetchResponse, RequestContext};
use super::identity::{self, Identity};
use super::model::{
    Columns, Diagnostic, QueryContext, SourceCache, SourceCandidate, SourceEntry, SourceIndex,
    SourceIndexEntry, SourceRule,
};
use super::validate;
use super::{invalid, Result};
use scraper::{ElementRef, Html, Selector};
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub mod dom;
pub mod hgamer;
pub mod kagura;
pub mod patchmoe;
pub mod script;
pub mod table;

/// Result of a ZIP-index refresh, shaped for the registry's cache/status write.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CollectOutcome {
    pub entries: Vec<SourceEntry>,
    pub etag: Option<String>,
    pub modified: Option<String>,
    /// `true` on a 304: keep the previous cache record and only bump `checked_at`.
    pub not_modified: bool,
}

/// Result of one per-source index update pass (P2, non-wire).
///
/// A `304` sets `not_modified`; the published snapshot then only changes its
/// `checked_at`/validators. `complete` is only true when every list page was
/// fetched and parsed, so the caller may replace the official snapshot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IndexOutcome {
    pub entries: Vec<SourceIndexEntry>,
    /// List pages still to fetch; a non-empty queue implies `complete = false`.
    pub pending: Vec<String>,
    pub complete: bool,
    pub etag: Option<String>,
    pub modified: Option<String>,
    /// `304`: keep the previous snapshot, only update `checked_at`.
    pub not_modified: bool,
    pub checked_at: u64,
    pub diagnostics: Vec<Diagnostic>,
}

/// Result of one per-game fallback (HGamer) discovery pass.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiscoverOutcome {
    pub status: String,
    pub candidates: Vec<SourceCandidate>,
    pub diagnostics: Vec<Diagnostic>,
    pub identities: Vec<Identity>,
}

/// Inputs supplied to the per-game fallback `discover`: candidates from earlier
/// passes that seed DLC/lead relevance.
#[derive(Debug, Clone, Default)]
pub struct DiscoverPrevious {
    pub known_candidates: Vec<SourceCandidate>,
}

/// A maintained source adapter. `previous` is required for conditional requests
/// (Python `_collect_table(rule, previous, force)`).
pub trait Adapter {
    fn collect(
        &self,
        _rule: &SourceRule,
        _previous: &SourceCache,
        _ctx: &mut RequestContext,
    ) -> Result<CollectOutcome> {
        Err(invalid("unsupported_source_adapter"))
    }

    fn discover(
        &self,
        _rule: &SourceRule,
        _query: &QueryContext,
        _ctx: &mut RequestContext,
        _previous: &DiscoverPrevious,
    ) -> Result<DiscoverOutcome> {
        Err(invalid("unsupported_source_adapter"))
    }

    /// Collects the source's own list/catalog index without fetching detail
    /// pages. `previous` is the official snapshot when no update progress is
    /// pending (conditional validators live there), or the saved progress when
    /// an earlier attempt was left unfinished.
    fn collect_index(
        &self,
        _rule: &SourceRule,
        _previous: Option<&SourceIndex>,
        _ctx: &mut RequestContext,
        _cancel: &AtomicBool,
    ) -> Result<IndexOutcome> {
        Err(invalid("unsupported_source_adapter"))
    }
}

/// Returns an adapter for a maintained `adapter` id, or `None`.
pub fn builtin_adapter(id: &str) -> Option<Box<dyn Adapter>> {
    match id {
        "patchmoe" => Some(Box::new(patchmoe::PatchmoeAdapter)),
        "table_html" | "table" => Some(Box::new(table::TableAdapter)),
        "kagura_wordpress" | "kagura_woocommerce" => Some(Box::new(kagura::KaguraAdapter)),
        "hgamer_html" => Some(Box::new(hgamer::HgamerAdapter)),
        _ => None,
    }
}

/// Captured source implementation. Scripts never fall back to a website parser.
pub enum SourceAdapter<'a> {
    Builtin(Box<dyn Adapter>),
    Script(&'a str),
}

pub fn source_adapter<'a>(rule: &SourceRule, code: Option<&'a str>) -> Result<SourceAdapter<'a>> {
    if rule.adapter == "lua" {
        return code
            .map(SourceAdapter::Script)
            .ok_or_else(|| invalid(format!("script_source_missing: {}", rule.id)));
    }
    builtin_adapter(&rule.adapter)
        .map(SourceAdapter::Builtin)
        .ok_or_else(|| invalid("unknown_source_adapter"))
}

impl SourceAdapter<'_> {
    pub fn collect_index(
        &self,
        rule: &SourceRule,
        previous: Option<&SourceIndex>,
        ctx: &mut RequestContext,
        cancel: &Arc<AtomicBool>,
    ) -> Result<IndexOutcome> {
        match self {
            Self::Builtin(adapter) => adapter.collect_index(rule, previous, ctx, cancel),
            Self::Script(code) => {
                script::collect_index(rule, code, previous, ctx, Arc::clone(cancel), |_| {})
            }
        }
    }

    pub fn resolve_page(
        &self,
        rule: &SourceRule,
        page: &str,
        ctx: &mut RequestContext,
        cancel: &Arc<AtomicBool>,
    ) -> Result<super::model::PageFact> {
        match self {
            Self::Script(code) => {
                script::resolve_page(rule, code, page, ctx, Arc::clone(cancel), |_| {})
            }
            Self::Builtin(_) => kagura::resolve_page(rule, kagura::adapter_name(rule), page, ctx),
        }
    }
}

/// Dispatch a discovery pass and apply the wrapper from `discovery.discover`:
/// fingerprint resave, `validate_candidate` + `merge_candidates`, diagnostic
/// dedupe and complete/partial/failed status. Unknown adapters error.
pub fn discover(
    rule: &SourceRule,
    query: &QueryContext,
    ctx: &mut RequestContext,
    previous: &DiscoverPrevious,
) -> Result<DiscoverOutcome> {
    let adapter =
        builtin_adapter(&rule.adapter).ok_or_else(|| invalid("unknown_discovery_adapter"))?;
    let mut outcome = adapter.discover(rule, query, ctx, previous)?;

    // Adapters mutate `published_at`/`upstream_page` after construction, so the
    // fingerprint must be resaved (Python `discover` does this before merging).
    for candidate in &mut outcome.candidates {
        if let Ok(value) = serde_json::to_value(&*candidate) {
            candidate.fingerprint = canonical::fingerprint(&value);
        }
    }

    let merged = identity::merge_candidates(&outcome.candidates);
    let mut validated = Vec::with_capacity(merged.len());
    let mut failure = None;
    for candidate in merged {
        match identity::validate_candidate(&candidate, rule) {
            Ok(()) => validated.push(candidate),
            Err(error) => {
                failure = Some(error.to_string());
                break;
            }
        }
    }
    if let Some(code) = failure {
        // Python keeps the unmerged candidates when validation raises.
        outcome.diagnostics.push(Diagnostic {
            source_id: Some(rule.id.clone()),
            code,
            detail: None,
        });
    } else {
        outcome.candidates = validated;
    }

    let mut seen = HashSet::new();
    let mut diagnostics = Vec::new();
    for diagnostic in outcome.diagnostics {
        if seen.insert(diagnostic.code.clone()) {
            diagnostics.push(Diagnostic {
                source_id: Some(rule.id.clone()),
                code: diagnostic.code,
                detail: None,
            });
        }
    }
    outcome.diagnostics = diagnostics;

    let incomplete = outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code.contains("incomplete"));
    let has_diagnostics = !outcome.diagnostics.is_empty();
    outcome.status = if has_diagnostics && (!outcome.candidates.is_empty() || incomplete) {
        "partial"
    } else if has_diagnostics {
        "failed"
    } else {
        "complete"
    }
    .to_string();
    Ok(outcome)
}

/// Current Unix time for index `checked_at` values.
pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// A cancelled-before-start outcome for collectors that cannot expose per-page
/// progress: every configured list page stays queued.
pub(crate) fn cancelled_index_outcome(rule: &SourceRule) -> IndexOutcome {
    let mut pending: Vec<String> = rule
        .pages
        .iter()
        .map(|page| canonical::canonical_url(page))
        .collect();
    pending.sort();
    pending.dedup();
    IndexOutcome {
        entries: Vec::new(),
        pending,
        complete: false,
        etag: None,
        modified: None,
        not_modified: false,
        checked_at: now_unix(),
        diagnostics: Vec::new(),
    }
}

/// Maps one validated table/patch.moe row to its index entry. The list page is
/// all the index can point at; the artifact URL lives in `url`.
pub(crate) fn index_entry_from_source(entry: &SourceEntry) -> SourceIndexEntry {
    SourceIndexEntry {
        title: entry.title.clone(),
        aliases: entry.aliases.clone(),
        page: canonical::canonical_url(&entry.source_page),
        kind: "entry".to_string(),
        appid: entry.appid,
        url: Some(entry.url.clone()),
        language: Some(entry.language.clone()),
        date: entry.date.clone(),
    }
}

/// Projects a published index back to the legacy `SourceCache` shape so the
/// table/patch.moe collectors can issue their existing conditional requests.
pub(crate) fn source_cache_from_index(index: Option<&SourceIndex>) -> SourceCache {
    let Some(index) = index else {
        return SourceCache::default();
    };
    SourceCache {
        entries: index
            .entries
            .iter()
            .map(|entry| SourceEntry {
                title: entry.title.clone(),
                aliases: entry.aliases.clone(),
                url: entry.url.clone().unwrap_or_default(),
                source_page: entry.page.clone(),
                language: entry.language.clone().unwrap_or_default(),
                date: entry.date.clone(),
                appid: entry.appid,
            })
            .collect(),
        etag: index.etag.clone(),
        modified: index.modified.clone(),
        cached_at: index.complete.then_some(index.updated_at),
        checked_at: Some(index.checked_at),
    }
}

/// Fetches a discovery page through the gated `fetch_html` and returns the
/// decoded text plus the response's redirect-aware final URL.
pub(crate) fn fetch_page(
    ctx: &mut RequestContext,
    url: &str,
    hosts: &[String],
) -> Result<(String, String)> {
    let html = ctx.fetch_html(url, hosts)?;
    let response = ctx.fetch(url, hosts)?;
    Ok((html, response_base(&response, url)))
}

/// True when `url`'s hostname is one of the (validated) rule hosts.
pub(crate) fn hostname_in(url: &str, hosts: &[String]) -> bool {
    validate::split_url(url)
        .hostname()
        .map(|hostname| hosts.iter().any(|host| host == &hostname))
        .unwrap_or(false)
}

// ---- shared HTML primitives ----------------------------------------------

/// One table cell: first class token, collapsed text and link hrefs.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Cell {
    pub class: String,
    pub text: String,
    pub links: Vec<String>,
}

pub(crate) fn selector(css: &str) -> Selector {
    Selector::parse(css).expect("static CSS selector must be valid")
}

/// Parses table rows into ordered rows of ordered `td`/`th` cells. Mirrors
/// `_Page` table/row/cell collection: with `table_id=None` rows are streamed
/// from every top-level table in document order (legacy `_Page(None)` — e.g.
/// patch.moe ships both a `tbl new` and a `tbl regular` table); otherwise only
/// the table carrying that id is parsed.
pub(crate) fn parse_rows(html: &str, table_id: Option<&str>) -> Vec<Vec<Cell>> {
    let document = Html::parse_document(html);
    let mut tables: Vec<ElementRef> = Vec::new();
    match table_id {
        Some(id) => {
            if let Ok(selector) = Selector::parse(&format!("table#{id}")) {
                if let Some(table) = document.select(&selector).next() {
                    tables.push(table);
                }
            }
        }
        None => {
            // Only top-level tables are iterated: a nested table's rows are
            // already reached through its ancestor's descendant `tr` selection,
            // so re-selecting them would double count.
            let table_selector = selector("table");
            for table in document.select(&table_selector) {
                let nested = table
                    .ancestors()
                    .any(|node| node.value().as_element().map(|el| el.name()) == Some("table"));
                if !nested {
                    tables.push(table);
                }
            }
        }
    }
    let mut rows = Vec::new();
    for table in tables {
        for row in table.select(&selector("tr")) {
            let mut cells = Vec::new();
            for cell in row.select(&selector("td, th")) {
                cells.push(Cell {
                    class: cell
                        .attr("class")
                        .unwrap_or("")
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_string(),
                    text: collapse(cell.text().collect::<String>()),
                    links: cell
                        .select(&selector("a[href]"))
                        .filter_map(|anchor| anchor.attr("href").map(str::to_string))
                        .collect(),
                });
            }
            if !cells.is_empty() {
                rows.push(cells);
            }
        }
    }
    rows
}

fn collapse(text: String) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Port of `_row_entry`: reads title/download cells, requires a `.zip` link, and
/// applies the optional date/language/appid columns.
pub(crate) fn row_entry(
    row: &[Cell],
    columns: &Columns,
    page: &str,
    language: &str,
) -> Result<Option<SourceEntry>> {
    let title = row
        .get(columns.title as usize)
        .map(|cell| cell.text.trim().to_string())
        .unwrap_or_default();
    let download = row
        .get(columns.download as usize)
        .and_then(|cell| cell.links.first())
        .map(|href| href.trim().to_string())
        .unwrap_or_default();
    if title.is_empty() || download.is_empty() {
        return Ok(None);
    }
    let Some(url) = join_relative(page, &download) else {
        return Ok(None);
    };
    if !validate::split_url(&url)
        .path
        .to_lowercase()
        .ends_with(".zip")
    {
        return Ok(None);
    }
    let mut entry = SourceEntry {
        title,
        aliases: Vec::new(),
        url,
        source_page: page.to_string(),
        language: language.to_string(),
        date: None,
        appid: None,
    };
    if let Some(index) = columns.date {
        if let Some(cell) = row.get(index as usize) {
            let value = cell.text.trim();
            if !value.is_empty() {
                entry.date = Some(value.to_string());
            }
        }
    }
    if let Some(index) = columns.language {
        if let Some(cell) = row.get(index as usize) {
            let value = cell.text.trim();
            if !value.is_empty() {
                entry.language = value.to_string();
            }
        }
    }
    if let Some(index) = columns.appid {
        if let Some(cell) = row.get(index as usize) {
            let value = cell.text.trim();
            if !valid_appid(value) {
                return Err(invalid("invalid_source_appid"));
            }
            entry.appid = value.parse::<u32>().ok();
        }
    }
    Ok(Some(entry))
}

fn valid_appid(value: &str) -> bool {
    let mut bytes = value.bytes();
    match bytes.next() {
        Some(b'1'..=b'9') => {}
        _ => return false,
    }
    bytes.all(|byte| byte.is_ascii_digit())
}

/// `urllib.parse.urljoin` over validated absolute bases.
pub(crate) fn join_relative(base: &str, reference: &str) -> Option<String> {
    let base = url::Url::parse(base).ok()?;
    base.join(reference).ok().map(|joined| joined.to_string())
}

/// Requires a 200 body and decodes it (challenge detection stays in discovery).
pub(crate) fn require_ok_text(response: &FetchResponse) -> Result<String> {
    if response.status != 200 {
        return Err(invalid(format!("source_http_{}", response.status)));
    }
    Ok(response.text())
}

/// Redirect-aware base URL for relative links.
pub(crate) fn response_base(response: &FetchResponse, fallback: &str) -> String {
    if response.final_url.is_empty() {
        fallback.to_string()
    } else {
        response.final_url.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::http::{FakeFetcher, RequestContext};
    use crate::sources::model::SourceRole;
    use std::sync::Arc;
    use std::time::Duration;

    fn rule(adapter: &str) -> SourceRule {
        SourceRule {
            script: None,
            id: "example".to_string(),
            name: "Example".to_string(),
            adapter: adapter.to_string(),
            role: SourceRole::Publisher,
            discovery_only: true,
            priority: 100,
            enabled: true,
            pages: vec!["https://example.com/".to_string()],
            page_hosts: vec!["example.com".to_string()],
            download_hosts: Vec::new(),
            root_layout: None,
            columns: None,
            table_id: None,
            language: None,
            url: None,
        }
    }

    fn query() -> QueryContext {
        QueryContext {
            appid: 42,
            name: "Game".to_string(),
            name_hints: Vec::new(),

            legacy_match: false,
            force: false,
            source_context: Vec::new(),
            parser_version: 2,
            query_version: 1,
            settings_revision: 1,
            generation: None,
        }
    }

    fn context(fake: &Arc<FakeFetcher>) -> RequestContext {
        RequestContext::with_settings(Box::new(Arc::clone(fake)), Duration::ZERO)
    }

    #[test]
    fn unknown_adapter_is_rejected() {
        let fake = Arc::new(FakeFetcher::new());
        let mut ctx = context(&fake);
        let error = discover(
            &rule("nope"),
            &query(),
            &mut ctx,
            &DiscoverPrevious::default(),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "unknown_discovery_adapter");
    }

    #[test]
    fn parse_rows_collects_every_top_level_table() {
        // Legacy `_Page(None)` streams rows from every table in document order.
        let html = r#"<html><body>
            <table><tr><th class="title">Title</th><th class="patch">Patch</th></tr>
            <tr><td class="title">A</td><td class="patch"><a href="a.zip">dl</a></td></tr></table>
            <table><tr><th class="title">Title</th><th class="patch">Patch</th></tr>
            <tr><td class="title">B</td><td class="patch"><a href="b.zip">dl</a></td></tr></table>
            </body></html>"#;
        let rows = parse_rows(html, None);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1][0].text, "A");
        assert_eq!(rows[1][1].links, vec!["a.zip".to_string()]);
        assert_eq!(rows[3][0].text, "B");
        assert_eq!(rows[3][1].links, vec!["b.zip".to_string()]);
    }

    #[test]
    fn parse_rows_with_id_selects_one_table() {
        let html = r#"<table class="a"><tr><td>x</td></tr></table>
            <table id="pick"><tr><td>y</td></tr></table>"#;
        let rows = parse_rows(html, Some("pick"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0].text, "y");
        assert!(parse_rows(html, Some("missing")).is_empty());
    }

    #[test]
    fn parse_rows_does_not_double_count_nested_tables() {
        let html = r#"<table><tr><td><table><tr><td>inner</td></tr></table></td></tr></table>"#;
        let rows = parse_rows(html, None);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn builtin_adapters_are_registered() {
        for id in [
            "patchmoe",
            "table_html",
            "table",
            "kagura_wordpress",
            "kagura_woocommerce",
            "hgamer_html",
        ] {
            assert!(builtin_adapter(id).is_some(), "{id}");
        }
        assert!(builtin_adapter("nope").is_none());
    }
}
