//! Kagura discovery adapters (`kagura_wordpress` / `kagura_woocommerce`),
//! porting `discovery.py::_official`: shared-index build/pagination, category
//! and product/article extraction, the CN "two pages before yielding" rule and
//! the `detail_unavailable` fallback.

use super::super::http::RequestContext;
use super::super::model::{DiscoveryIndex, IndexRecord, QueryContext, SourceCandidate, SourceRule};
use super::super::{identity, invalid, validate, Result, PARSER_VERSION, SOURCES_ENCODING_VERSION};
use super::dom;
use super::{fetch_page, hostname_in, Adapter, DiscoverOutcome, DiscoverPrevious};
use scraper::ElementRef;
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct KaguraAdapter;

impl Adapter for KaguraAdapter {
    fn discover(
        &self,
        rule: &SourceRule,
        query: &QueryContext,
        ctx: &mut RequestContext,
        previous: &DiscoverPrevious,
    ) -> Result<DiscoverOutcome> {
        let adapter = match rule.adapter.as_str() {
            "kagura_wordpress" => "kagura_cn_html",
            "kagura_woocommerce" => "kagura_en_html",
            other => other,
        };
        let (candidates, index, diagnostics) = official(rule, adapter, query, ctx, previous);
        Ok(DiscoverOutcome {
            status: String::new(),
            candidates,
            diagnostics: diagnostics
                .into_iter()
                .map(|code| super::super::model::Diagnostic {
                    source_id: Some(rule.id.clone()),
                    code,
                    detail: None,
                })
                .collect(),
            index: Some(index),
            checkpoint: None,
            identities: Vec::new(),
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
struct IndexItem {
    title: String,
    page: String,
    kind: String,
}

fn official(
    rule: &SourceRule,
    adapter: &str,
    query: &QueryContext,
    ctx: &mut RequestContext,
    previous: &DiscoverPrevious,
) -> (Vec<SourceCandidate>, DiscoveryIndex, Vec<String>) {
    let mut diagnostics: Vec<String> = Vec::new();
    let entry_url = rule
        .url
        .clone()
        .or_else(|| rule.pages.first().cloned())
        .unwrap_or_default();

    let mut direct: Vec<(Option<String>, String)> = Vec::new();
    for url in &previous.upstream_pages {
        if validate::navigation_url(url).is_err() {
            continue;
        }
        let path = validate::split_url(url).path.clone();
        if !path.is_empty() && path != "/" && path != "/patch/" {
            direct.push((None, url.clone()));
        }
    }

    let previous_index = if query.force {
        None
    } else {
        previous.index.as_ref()
    };
    let mut records: Vec<IndexItem> = previous_index
        .map(|index| {
            index
                .records
                .iter()
                .map(|record| IndexItem {
                    title: record.title.clone(),
                    page: record.page.clone(),
                    kind: record.kind.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut pages: Vec<String> = previous_index
        .map(|index| index.pending.clone())
        .unwrap_or_default();
    let mut checked_at = previous_index.map(|index| index.checked_at).unwrap_or(0);

    if direct.is_empty() {
        let target_category_known = adapter == "kagura_cn_html"
            && records.iter().any(|record| {
                record.kind == "category" && identity::title_relevant(&record.title, query)
            });

        if records.is_empty() || (!pages.is_empty() && !target_category_known) {
            if pages.is_empty() {
                pages.push(entry_url.clone());
            }
            let mut seen: HashSet<String> = HashSet::new();
            while !pages.is_empty() && seen.len() < 100 {
                let url = pages.remove(0);
                if seen.contains(&url) {
                    diagnostics.push("discovery_pagination_cycle".to_string());
                    pages.insert(0, url);
                    break;
                }
                seen.insert(url.clone());
                let (html, final_url) = match fetch_page(ctx, &url, &rule.page_hosts) {
                    Ok(value) => value,
                    Err(error) => {
                        let message = error.to_string();
                        diagnostics.push(message.clone());
                        if adapter == "kagura_en_html"
                            && url == entry_url
                            && message.contains("html_too_large")
                        {
                            if let Some(alternative) =
                                super::join_relative(&entry_url, "/product-category/patches/patch/")
                            {
                                pages.insert(0, alternative);
                            }
                            continue;
                        }
                        pages.insert(0, url);
                        break;
                    }
                };
                checked_at = now_unix();
                let document = dom::document(&html);
                let root = dom::root(&document);
                let mut found: Vec<IndexItem> = Vec::new();

                if adapter == "kagura_en_html" {
                    let cards: Vec<ElementRef> = dom::walk(&root)
                        .into_iter()
                        .filter(|node| {
                            dom::has_class(node, "product")
                                && dom::has_class(node, "type-product")
                                && !dom::ancestors(node).iter().any(|ancestor| {
                                    dom::has_class(ancestor, "widget")
                                        || dom::has_class(ancestor, "related")
                                })
                        })
                        .collect();
                    for card in &cards {
                        let title_node = dom::first(card, |node| {
                            dom::has_class(node, "woocommerce-loop-product__title")
                                || dom::has_class(node, "name")
                        });
                        let links = dom::links(card, &final_url, |_, url| {
                            validate::split_url(url).path.contains("/product/")
                        });
                        if !links.is_empty() {
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
                                found.push(IndexItem {
                                    title,
                                    page: links[0].1.clone(),
                                    kind: "product".to_string(),
                                });
                            }
                        }
                    }
                    if cards.is_empty() {
                        diagnostics.push("discovery_structure_changed".to_string());
                    }
                } else {
                    for article in dom::walk(&root) {
                        if dom::tag(&article) != "article" {
                            continue;
                        }
                        let title_node =
                            dom::first(&article, |node| dom::has_class(node, "entry-title"));
                        if let Some(title_node) = title_node {
                            let links = dom::links(&title_node, &final_url, |_, url| {
                                hostname_in(url, &rule.page_hosts)
                            });
                            if !links.is_empty() {
                                found.push(IndexItem {
                                    title: dom::text(&title_node),
                                    page: links[0].1.clone(),
                                    kind: "article".to_string(),
                                });
                            }
                        }
                    }
                    for (label, link) in dom::links(&root, &final_url, |_, url| {
                        validate::split_url(url).path.contains("/category/")
                    }) {
                        let record = IndexItem {
                            title: label,
                            page: link,
                            kind: "category".to_string(),
                        };
                        if !records.contains(&record) {
                            records.push(record);
                        }
                    }
                    if found.is_empty()
                        && dom::first(&root, |node| dom::has_class(node, "widget_categories"))
                            .is_none()
                    {
                        diagnostics.push("discovery_structure_changed".to_string());
                    }
                }

                let mut merged: Vec<IndexItem> = Vec::new();
                let mut positions: HashMap<String, usize> = HashMap::new();
                for record in records.drain(..).chain(found.into_iter()) {
                    match positions.get(&record.page) {
                        Some(&position) => merged[position] = record,
                        None => {
                            positions.insert(record.page.clone(), merged.len());
                            merged.push(record);
                        }
                    }
                }
                merged.truncate(4096);
                records = merged;

                if let Some(next) = next_page(&root, &final_url) {
                    pages.push(next);
                }
                // A target's category is enough; avoid crawling the full archive.
                if adapter == "kagura_cn_html"
                    && records.iter().any(|record| {
                        record.kind == "category" && identity::title_relevant(&record.title, query)
                    })
                {
                    break;
                }
                if adapter == "kagura_cn_html" && seen.len() >= 2 {
                    break;
                }
            }
            if !pages.is_empty()
                && !records.iter().any(|record| {
                    record.kind == "category" && identity::title_relevant(&record.title, query)
                })
            {
                diagnostics.push("discovery_index_incomplete".to_string());
            }
        }

        direct = records
            .iter()
            .filter(|record| {
                record.kind != "category" && identity::title_relevant(&record.title, query)
            })
            .map(|record| (Some(record.title.clone()), record.page.clone()))
            .collect();

        let categories: Vec<(String, String)> = records
            .iter()
            .filter(|record| {
                record.kind == "category" && identity::title_relevant(&record.title, query)
            })
            .map(|record| (record.title.clone(), record.page.clone()))
            .collect();
        for (_, category) in categories {
            match crawl_category(rule, query, ctx, &category) {
                Ok(mut articles) => direct.append(&mut articles),
                Err(error) => diagnostics.push(error.to_string()),
            }
        }
    }

    let mut candidates = Vec::new();
    for (page, title) in dedup_by_page(direct) {
        match detail_candidate(rule, adapter, query, ctx, &page, title.as_deref()) {
            Ok(found) => candidates.extend(found),
            Err(error) => {
                diagnostics.push(error.to_string());
                if let Some(title) = title {
                    if !title.is_empty() {
                        let mut fallback = identity::build_candidates(
                            rule,
                            &title,
                            &page,
                            "",
                            &[],
                            &[],
                            "unknown",
                            &[],
                            query,
                        );
                        for candidate in &mut fallback {
                            candidate.warnings.push("detail_unavailable".to_string());
                        }
                        candidates.extend(fallback);
                    }
                }
            }
        }
    }

    let index = DiscoveryIndex {
        parser_version: PARSER_VERSION,
        encoding_version: SOURCES_ENCODING_VERSION,
        records: records
            .into_iter()
            .map(|record| IndexRecord {
                title: record.title,
                page: record.page,
                kind: record.kind,
            })
            .collect(),
        pending: pages,
        checked_at,
    };
    (candidates, index, diagnostics)
}

fn detail_candidate(
    rule: &SourceRule,
    adapter: &str,
    query: &QueryContext,
    ctx: &mut RequestContext,
    page: &str,
    fallback_title: Option<&str>,
) -> Result<Vec<SourceCandidate>> {
    let (html, final_url) = fetch_page(ctx, page, &rule.page_hosts)?;
    let document = dom::document(&html);
    let root = dom::root(&document);
    let heading = dom::first(&root, |node| {
        dom::tag(node) == "h1"
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
        dom::first(&root, |node| dom::attr(node, "id") == Some("tab-patch"))
    } else {
        dom::first(&root, |node| {
            dom::has_class(node, "entry-content")
                && dom::ancestors(node)
                    .iter()
                    .any(|ancestor| dom::tag(ancestor) == "article")
        })
    };
    let Some(body) = body else {
        return Err(invalid("discovery_structure_changed"));
    };
    let ids = identity::steam_ids(&dom::link_urls(&body));
    let mirrors = dom::mirrors(&body, &final_url);
    let body_text = dom::text(&body);
    let mut found = identity::build_candidates(
        rule,
        &title,
        &final_url,
        &body_text,
        &ids,
        &[],
        "unknown",
        &mirrors,
        query,
    );
    let article = dom::ancestors(&body)
        .into_iter()
        .find(|ancestor| dom::tag(ancestor) == "article");
    if let Some(article) = article {
        let published = dom::first(&article, |node| {
            dom::tag(node) == "time"
                && (dom::has_class(node, "published") || dom::has_class(node, "entry-date"))
        });
        if let Some(published) = published {
            let published_at = identity::date(dom::attr(&published, "datetime").unwrap_or(""));
            for candidate in &mut found {
                candidate.published_at = published_at.clone();
            }
        }
    }
    Ok(found)
}

fn crawl_category(
    rule: &SourceRule,
    query: &QueryContext,
    ctx: &mut RequestContext,
    category: &str,
) -> Result<Vec<(Option<String>, String)>> {
    let mut articles = Vec::new();
    let mut url = Some(category.to_string());
    let mut visited: HashSet<String> = HashSet::new();
    while let Some(current) = url.clone() {
        if visited.len() >= 100 {
            break;
        }
        if visited.contains(&current) {
            return Err(invalid("discovery_pagination_cycle"));
        }
        visited.insert(current.clone());
        let (html, final_url) = fetch_page(ctx, &current, &rule.page_hosts)?;
        let document = dom::document(&html);
        let root = dom::root(&document);
        let mut article_count = 0;
        for article in dom::walk(&root) {
            if dom::tag(&article) != "article" {
                continue;
            }
            article_count += 1;
            if let Some(title) = dom::first(&article, |node| dom::has_class(node, "entry-title")) {
                let text = dom::text(&title);
                let links = dom::links(&title, &final_url, |_, link| {
                    hostname_in(link, &rule.page_hosts)
                });
                if !links.is_empty() && identity::title_relevant(&text, query) {
                    articles.push((Some(text), links[0].1.clone()));
                }
            }
        }
        if article_count == 0
            && dom::first(&root, |node| dom::has_class(node, "no-results")).is_none()
        {
            return Err(invalid("discovery_structure_changed"));
        }
        url = next_page(&root, &final_url);
    }
    if url.is_some() {
        return Err(invalid("discovery_index_incomplete"));
    }
    Ok(articles)
}

fn dedup_by_page(direct: Vec<(Option<String>, String)>) -> Vec<(String, Option<String>)> {
    let mut order: Vec<String> = Vec::new();
    let mut titles: HashMap<String, Option<String>> = HashMap::new();
    for (title, page) in direct {
        if !titles.contains_key(&page) {
            order.push(page.clone());
        }
        titles.insert(page, title);
    }
    order
        .into_iter()
        .map(|page| {
            let title = titles.get(&page).cloned().flatten();
            (page, title)
        })
        .collect()
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

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::super::{builtin_adapter, Adapter, DiscoverPrevious};
    use super::*;
    use crate::sources::http::{FakeFetcher, FetchResponse, RequestContext};
    use crate::sources::model::{SourceRole, Variant};
    use std::sync::Arc;
    use std::time::Duration;

    fn rule(adapter: &str) -> SourceRule {
        SourceRule {
            id: "kagura-cn".to_string(),
            name: "Kagura".to_string(),
            adapter: adapter.to_string(),
            role: SourceRole::Publisher,
            discovery_only: true,
            priority: 10,
            enabled: true,
            pages: vec!["https://kagurafan.com/".to_string()],
            page_hosts: vec!["kagurafan.com".to_string()],
            download_hosts: vec!["kaguraserver.com".to_string()],
            root_layout: None,
            columns: None,
            table_id: None,
            language: None,
            url: None,
        }
    }

    fn query(appid: u32, name: &str) -> QueryContext {
        QueryContext {
            appid,
            name: name.to_string(),
            name_hints: Vec::new(),
            preferred_language: None,
            legacy_match: false,
            force: false,
            source_context: Vec::new(),
            parser_version: 2,
            query_version: 1,
            settings_revision: 1,
            generation: None,
        }
    }

    fn response(html: &str, final_url: &str) -> FetchResponse {
        FetchResponse {
            status: 200,
            body: html.as_bytes().to_vec(),
            final_url: final_url.to_string(),
            ..FetchResponse::default()
        }
    }

    fn context(fake: &Arc<FakeFetcher>) -> RequestContext {
        RequestContext::with_settings(
            Box::new(Arc::clone(fake)),
            20,
            Duration::from_secs(30),
            Duration::ZERO,
        )
    }

    const INDEX: &str = r#"<html><body>
<a href="/category/patches" rel="category">Other</a>
<article><h2 class="entry-title"><a href="/game/">Game</a></h2></article>
</body></html>"#;

    const ARTICLE: &str = r#"<html><body><article>
<h1 class="entry-title">Game</h1>
<div class="entry-content">
<p>中文 补丁 v1.2.3 ZIP</p>
<a href="https://kaguraserver.com/game_v1.2.3.zip">下载 Mirror 1</a>
</div>
</article></body></html>"#;

    #[test]
    fn index_build_and_detail_candidate() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(INDEX, "https://kagurafan.com/"));
        fake.push(response(ARTICLE, "https://kagurafan.com/game/"));
        let mut ctx = context(&fake);
        let adapter = builtin_adapter("kagura_wordpress").unwrap();
        let outcome = adapter
            .discover(
                &rule("kagura_wordpress"),
                &query(42, "Game"),
                &mut ctx,
                &DiscoverPrevious::default(),
            )
            .unwrap();
        assert_eq!(outcome.candidates.len(), 1);
        assert_eq!(outcome.candidates[0].variant, Variant::Base);
        let index = outcome.index.unwrap();
        assert_eq!(index.parser_version, PARSER_VERSION);
        assert!(index.records.iter().any(|record| record.kind == "category"));
        assert!(index.records.iter().any(|record| record.kind == "article"));
    }

    #[test]
    fn index_is_reused_from_previous() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(ARTICLE, "https://kagurafan.com/game/"));
        let mut ctx = context(&fake);
        let previous = DiscoverPrevious {
            index: Some(DiscoveryIndex {
                parser_version: PARSER_VERSION,
                encoding_version: SOURCES_ENCODING_VERSION,
                records: vec![IndexRecord {
                    title: "Game".to_string(),
                    page: "https://kagurafan.com/game/".to_string(),
                    kind: "article".to_string(),
                }],
                pending: Vec::new(),
                checked_at: 123,
            }),
            upstream_pages: Vec::new(),
        };
        let outcome = KaguraAdapter
            .discover(
                &rule("kagura_wordpress"),
                &query(42, "Game"),
                &mut ctx,
                &previous,
            )
            .unwrap();
        assert_eq!(outcome.candidates.len(), 1);
        // Only the detail page was fetched; the index page was reused.
        assert_eq!(fake.call_count(), 1);
        assert_eq!(outcome.index.unwrap().checked_at, 123);
    }

    #[test]
    fn detail_failure_yields_unavailable_fallback() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(INDEX, "https://kagurafan.com/"));
        // Detail page without an entry-content body -> structure error.
        fake.push(response(
            "<html><body><article><h1 class=\"entry-title\">Game</h1></article></body></html>",
            "https://kagurafan.com/game/",
        ));
        let mut ctx = context(&fake);
        let outcome = KaguraAdapter
            .discover(
                &rule("kagura_wordpress"),
                &query(42, "Game"),
                &mut ctx,
                &DiscoverPrevious::default(),
            )
            .unwrap();
        assert_eq!(outcome.candidates.len(), 1);
        assert!(outcome.candidates[0]
            .warnings
            .iter()
            .any(|warning| warning == "detail_unavailable"));
    }
}
