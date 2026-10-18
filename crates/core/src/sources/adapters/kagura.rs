//! Kagura adapters (`kagura_wordpress` / `kagura_woocommerce`).
//!
//! Source updates walk only the list/catalog pages into the shared
//! [`SourceIndex`]; matched games resolve individual article/product/category
//! pages on demand through [`resolve_page`]. No detail page is fetched by the
//! index update.

use super::super::canonical;
use super::super::http::RequestContext;
use super::super::model::{
    Diagnostic, FactLink, PageFact, SourceIndex, SourceIndexEntry, SourceRule,
};
use super::super::{identity, invalid, validate, Result};
use super::dom;
use super::{fetch_page, hostname_in, now_unix, Adapter, IndexOutcome};
use scraper::ElementRef;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct KaguraAdapter;

impl Adapter for KaguraAdapter {
    fn collect_index(
        &self,
        rule: &SourceRule,
        previous: Option<&SourceIndex>,
        ctx: &mut RequestContext,
        cancel: &AtomicBool,
    ) -> Result<IndexOutcome> {
        collect_index(rule, adapter_name(rule), previous, ctx, cancel)
    }
}

/// Internal parser name for a Kagura rule (`kagura_wordpress` → CN HTML,
/// `kagura_woocommerce` → EN HTML). Shared with the P3 match task so detail
/// parsing always uses the source's own adapter.
pub fn adapter_name(rule: &SourceRule) -> &str {
    match rule.adapter.as_str() {
        "kagura_wordpress" => "kagura_cn_html",
        "kagura_woocommerce" => "kagura_en_html",
        other => other,
    }
}

fn diagnostic(rule: &SourceRule, code: impl Into<String>) -> Diagnostic {
    Diagnostic {
        source_id: Some(rule.id.clone()),
        code: code.into(),
        detail: None,
    }
}

/// Builds the source's complete list/catalog index by walking only its list
/// pages (design §4.1, §9). Detail and category page contents are never
/// fetched here: P3 resolves those on demand for matched games.
///
/// A `previous` snapshot carrying `pending` pages is resumed; anything else
/// restarts from the rule entry page so withdrawn entries are not carried over.
fn collect_index(
    rule: &SourceRule,
    adapter: &str,
    previous: Option<&SourceIndex>,
    ctx: &mut RequestContext,
    cancel: &AtomicBool,
) -> Result<IndexOutcome> {
    let mut entries: Vec<SourceIndexEntry> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    match previous.filter(|index| !index.pending.is_empty()) {
        Some(index) => {
            entries = index.entries.clone();
            for page in &index.pending {
                let page = canonical::canonical_url(page);
                if seen.insert(page.clone()) {
                    pending.push(page);
                }
            }
        }
        None => {
            // Every configured entry is an independent list root (for example
            // the WooCommerce patch listing plus its product-category view);
            // each one is crawled through its own `next_page` chain.
            let mut roots: Vec<String> = Vec::new();
            if let Some(url) = &rule.url {
                roots.push(canonical::canonical_url(url));
            }
            for page in &rule.pages {
                roots.push(canonical::canonical_url(page));
            }
            for root in roots {
                if seen.insert(root.clone()) {
                    pending.push(root);
                }
            }
        }
    }

    let mut checked_at = 0;
    let mut incomplete = false;
    while !pending.is_empty() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let url = pending[0].clone();
        let (html, final_url) = match fetch_page(ctx, &url, &rule.page_hosts) {
            Ok(value) => value,
            Err(error) => {
                // A cancelled fetch keeps the unfinished page queued; any other
                // failure does the same while recording the diagnostic.
                if error.to_string() != "task_cancelled" && !cancel.load(Ordering::Relaxed) {
                    diagnostics.push(diagnostic(rule, error.to_string()));
                }
                incomplete = true;
                break;
            }
        };
        pending.remove(0);
        // Track the redirect-aware final URL as well so a page reached twice
        // through different spellings is not fetched a second time.
        seen.insert(canonical::canonical_url(&final_url));
        checked_at = now_unix();
        let document = dom::document(&html);
        let root = dom::root(&document);
        let (found, next) = parse_index_page(rule, adapter, &root, &final_url);
        if found.is_empty() {
            diagnostics.push(diagnostic(rule, "discovery_structure_changed"));
            incomplete = true;
        }
        entries.extend(found);
        if let Some(next) = next {
            let next = canonical::canonical_url(&next);
            if seen.insert(next.clone()) {
                pending.push(next);
            } else {
                // A repeated page is already covered; it does not make the
                // index incomplete.
                diagnostics.push(diagnostic(rule, "discovery_pagination_cycle"));
            }
        }
    }
    // The index keys entries by canonical page: a page referenced twice (for
    // example by a wrapper listing) appears once, first occurrence winning.
    let mut pages = HashSet::new();
    entries.retain(|entry| pages.insert(entry.page.clone()));
    Ok(IndexOutcome {
        entries,
        complete: pending.is_empty() && !incomplete,
        pending,
        etag: None,
        modified: None,
        not_modified: false,
        checked_at,
        diagnostics,
    })
}

/// Parses one list page into index entries plus its `rel=next` URL.
fn parse_index_page(
    rule: &SourceRule,
    adapter: &str,
    root: &ElementRef<'_>,
    final_url: &str,
) -> (Vec<SourceIndexEntry>, Option<String>) {
    let mut found: Vec<SourceIndexEntry> = Vec::new();
    if adapter == "kagura_en_html" {
        for card in dom::walk(root).into_iter().filter(|node| {
            dom::has_class(node, "product")
                && dom::has_class(node, "type-product")
                && !dom::ancestors(node).iter().any(|ancestor| {
                    dom::has_class(ancestor, "widget") || dom::has_class(ancestor, "related")
                })
        }) {
            let title_node = dom::first(&card, |node| {
                dom::has_class(node, "woocommerce-loop-product__title")
                    || dom::has_class(node, "name")
            });
            let links = dom::links(&card, final_url, |_, url| {
                validate::split_url(url).path.contains("/product/")
            });
            if links.is_empty() {
                continue;
            }
            let title = title_node
                .as_ref()
                .map(dom::text)
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| {
                    links
                        .iter()
                        .map(|(label, _)| label.clone())
                        .find(|label| !label.is_empty())
                        .unwrap_or_default()
                });
            if !title.is_empty() {
                found.push(index_entry(title, &links[0].1, "product"));
            }
        }
    } else {
        for article in dom::walk(root) {
            if dom::tag(&article) != "article" {
                continue;
            }
            let Some(title_node) = dom::first(&article, |node| dom::has_class(node, "entry-title"))
            else {
                continue;
            };
            let links = dom::links(&title_node, final_url, |_, url| {
                hostname_in(url, &rule.page_hosts)
            });
            let Some((_, page)) = links.first() else {
                continue;
            };
            let title = dom::text(&title_node);
            if !title.is_empty() {
                found.push(index_entry(title, page, "article"));
            }
        }
        for (label, link) in dom::links(root, final_url, |_, url| {
            validate::split_url(url).path.contains("/category/")
        }) {
            if !label.is_empty() {
                found.push(index_entry(label, &link, "category"));
            }
        }
    }
    (found, next_page(root, final_url))
}

fn index_entry(title: String, page: &str, kind: &str) -> SourceIndexEntry {
    SourceIndexEntry {
        title,
        aliases: Vec::new(),
        page: canonical::canonical_url(page),
        kind: kind.to_string(),
        appid: None,
        url: None,
        language: None,
        date: None,
    }
}

/// Query-independent facts of one article/product page. The P3 match task
/// persists these facts in the shared detail cache.
fn page_fact_from_root(
    _rule: &SourceRule,
    adapter: &str,
    root: &ElementRef<'_>,
    final_url: &str,
    fallback_title: Option<&str>,
) -> Result<PageFact> {
    let heading = dom::first(root, |node| {
        matches!(dom::tag(node), "h1" | "h2")
            && (dom::has_class(node, "entry-title") || adapter == "kagura_en_html")
    });
    let title = heading
        .as_ref()
        .map(dom::text)
        .or_else(|| fallback_title.map(str::to_string))
        .unwrap_or_default();
    if title.is_empty() {
        return Err(invalid("discovery_structure_changed"));
    }
    let body = if adapter == "kagura_en_html" {
        dom::first(root, |node| dom::attr(node, "id") == Some("tab-patch"))
    } else {
        dom::first(root, |node| {
            dom::has_class(node, "entry-content")
                && dom::ancestors(node)
                    .iter()
                    .any(|ancestor| dom::tag(ancestor) == "article")
        })
    };
    let Some(body) = body else {
        return Err(invalid("discovery_structure_changed"));
    };
    let body_links: Vec<String> = dom::link_urls(&body)
        .iter()
        .filter_map(|url| super::join_relative(final_url, url))
        .collect();
    Ok(PageFact {
        title,
        page: canonical::canonical_url(final_url),
        kind: if adapter == "kagura_en_html" {
            "product"
        } else {
            "article"
        }
        .to_string(),
        steam_ids: identity::steam_ids(&dom::link_urls(&body)),
        aliases: Vec::new(),
        mirrors: dom::mirrors(&body, final_url),
        text: dom::text(&body),
        published_at: article_published_at(&body),
        body_links,
        article_links: Vec::new(),
        article_facts: Vec::new(),
        next_page: next_page(root, final_url).map(|page| canonical::canonical_url(&page)),
    })
}

fn article_published_at(body: &ElementRef<'_>) -> Option<String> {
    let article = dom::ancestors(body)
        .into_iter()
        .find(|ancestor| dom::tag(ancestor) == "article")?;
    let published = dom::first(&article, |node| {
        dom::tag(node) == "time"
            && (dom::has_class(node, "published") || dom::has_class(node, "entry-date"))
    })?;
    identity::date(dom::attr(&published, "datetime").unwrap_or(""))
}

/// Fetches and parses one source page into query-independent facts for the P3
/// game match task (never called from a source index update).
///
/// `page` is canonicalized before the request so query/fragment variants of the
/// same page reuse one response inside the task's `RequestContext`.
pub fn resolve_page(
    rule: &SourceRule,
    adapter: &str,
    page: &str,
    ctx: &mut RequestContext,
) -> Result<PageFact> {
    let page = canonical::canonical_url(page);
    if !hostname_in(&page, &rule.page_hosts) {
        return Err(invalid("invalid_discovery_url"));
    }
    let (html, final_url) = fetch_page(ctx, &page, &rule.page_hosts)?;
    let document = dom::document(&html);
    let root = dom::root(&document);
    if is_category_page(adapter, &final_url) {
        return category_fact(rule, adapter, &root, &final_url);
    }
    page_fact_from_root(rule, adapter, &root, &final_url, None)
}

fn is_category_page(adapter: &str, url: &str) -> bool {
    adapter != "kagura_en_html" && validate::split_url(url).path.contains("/category/")
}

/// Parses a CN category page: inline full articles become [`PageFact`]s, and
/// every linked article is recorded in `article_links`.
fn category_fact(
    rule: &SourceRule,
    adapter: &str,
    root: &ElementRef<'_>,
    final_url: &str,
) -> Result<PageFact> {
    let title = dom::first(root, |node| dom::tag(node) == "h1")
        .map(|node| dom::text(&node))
        .or_else(|| dom::first(root, |node| dom::tag(node) == "title").map(|node| dom::text(&node)))
        .unwrap_or_default();
    let mut article_links = Vec::new();
    let mut article_facts = Vec::new();
    for article in dom::walk(root) {
        if dom::tag(&article) != "article" {
            continue;
        }
        let Some(heading) = dom::first(&article, |node| dom::has_class(node, "entry-title")) else {
            continue;
        };
        let links = dom::links(&heading, final_url, |_, url| {
            hostname_in(url, &rule.page_hosts)
        });
        let Some((_, page)) = links.first() else {
            continue;
        };
        let page = canonical::canonical_url(page);
        article_links.push(FactLink {
            title: dom::text(&heading),
            page: page.clone(),
        });
        if dom::first(&article, |node| dom::has_class(node, "entry-content")).is_some() {
            let fallback = dom::text(&heading);
            if let Ok(fact) = page_fact_from_root(rule, adapter, &article, &page, Some(&fallback)) {
                article_facts.push(fact);
            }
        }
    }
    Ok(PageFact {
        title,
        page: canonical::canonical_url(final_url),
        kind: "category".to_string(),
        steam_ids: Vec::new(),
        aliases: Vec::new(),
        mirrors: Vec::new(),
        text: String::new(),
        published_at: None,
        body_links: Vec::new(),
        article_links,
        article_facts,
        next_page: next_page(root, final_url).map(|page| canonical::canonical_url(&page)),
    })
}

fn next_page(root: &ElementRef, page: &str) -> Option<String> {
    for anchor in root.select(&super::selector("a")) {
        let rel_next = dom::attr(&anchor, "rel")
            .map(|rel| rel.split_whitespace().any(|token| token == "next"))
            .unwrap_or(false);
        if rel_next || dom::has_class(&anchor, "next") || dom::has_class(&anchor, "nextpostslink") {
            let href = dom::attr(&anchor, "href").unwrap_or("");
            return super::join_relative(page, href);
        }
    }
    None
}
