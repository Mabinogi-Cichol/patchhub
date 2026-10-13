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
        let (candidates, index, diagnostics, checkpoint) =
            official(rule, adapter, query, ctx, previous);
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
            checkpoint,
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
) -> (
    Vec<SourceCandidate>,
    DiscoveryIndex,
    Vec<String>,
    Option<serde_json::Value>,
) {
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

    let previous_index = previous.index.as_ref();
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
    // Explicit refresh also revisits the entry page, after saved pending work.
    // Never throw away progress just because this game has no known detail yet.
    if query.force && direct.is_empty() && !pages.contains(&entry_url) {
        pages.push(entry_url.clone());
    }
    let mut checked_at = previous_index.map(|index| index.checked_at).unwrap_or(0);

    if direct.is_empty() {
        let target_category_known = adapter == "kagura_cn_html"
            && records.iter().any(|record| {
                record.kind == "category" && lead_relevant(&record.title, query, previous)
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
                        record.kind == "category" && lead_relevant(&record.title, query, previous)
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
                    record.kind == "category" && lead_relevant(&record.title, query, previous)
                })
            {
                diagnostics.push("discovery_index_incomplete".to_string());
            }
        }

        direct = records
            .iter()
            .filter(|record| {
                record.kind != "category" && lead_relevant(&record.title, query, previous)
            })
            .map(|record| (Some(record.title.clone()), record.page.clone()))
            .collect();
    }

    // Detail priority must never turn into detail-only discovery. Work is local
    // to this game and resumes before repeating already completed requests.
    let binding = serde_json::json!({"appid":query.appid,"name":query.name,"hints":query.name_hints,
        "language":query.preferred_language,"settings":query.settings_revision,"rule":rule});
    let resumed = previous
        .checkpoint
        .as_ref()
        .filter(|c| c.get("binding") == Some(&binding));
    let mut work: std::collections::VecDeque<Work> = resumed
        .and_then(|c| serde_json::from_value(c.get("work")?.clone()).ok())
        .unwrap_or_default();
    let mut candidates: Vec<SourceCandidate> = if !work.is_empty() {
        resumed
            .and_then(|c| serde_json::from_value(c.get("candidates")?.clone()).ok())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    if work.is_empty() {
        for (page, title) in dedup_by_page(direct) {
            work.push_back(Work {
                kind: "detail".into(),
                page,
                title,
            });
        }
        let mut categories: Vec<_> = records
            .iter()
            .filter(|r| r.kind == "category" && lead_relevant(&r.title, query, previous))
            .collect();
        // Expand missing variants before spending the bounded pass on the known base category.
        categories.sort_by_key(|r| {
            (
                previous
                    .known_candidates
                    .iter()
                    .any(|c| c.patch_title == r.title),
                identity::variant(&r.title) == super::super::model::Variant::Dlc,
            )
        });
        for r in categories {
            work.push_back(Work {
                kind: "category".into(),
                page: r.page.clone(),
                title: Some(r.title.clone()),
            });
        }
        for r in records
            .iter()
            .filter(|r| r.kind != "category" && lead_relevant(&r.title, query, previous))
        {
            if !work.iter().any(|w| w.page == r.page) {
                work.push_back(Work {
                    kind: "detail".into(),
                    page: r.page.clone(),
                    title: Some(r.title.clone()),
                });
            }
        }
    }
    let mut related_previous = previous.clone();
    related_previous.known_candidates.extend(candidates.clone());
    related_previous.related_pages.extend(
        resumed
            .and_then(|c| {
                serde_json::from_value::<Vec<String>>(c.get("related_pages")?.clone()).ok()
            })
            .unwrap_or_default(),
    );
    let mut visited = HashSet::new();
    let mut retry = Vec::new();
    let mut deferred = Vec::new();
    let mut resolving_deferred = false;
    while let Some(task) = work.pop_front() {
        let needs_base = task.kind == "detail"
            && task.title.as_ref().is_some_and(|title| {
                identity::variant(title) == super::super::model::Variant::Dlc
                    && !identity::title_relevant(title, query)
            })
            && !related_previous
                .known_candidates
                .iter()
                .any(|c| c.variant == super::super::model::Variant::Base);
        if needs_base && !resolving_deferred {
            deferred.push(task);
            if work.is_empty() {
                work.extend(deferred.drain(..));
                resolving_deferred = true;
            }
            continue;
        }
        if !visited.insert(task.page.clone()) {
            continue;
        }
        let fetched = fetch_page(ctx, &task.page, &rule.page_hosts);
        let (html, final_url) = match fetched {
            Ok(v) => v,
            Err(e) => {
                diagnostics.push(e.to_string());
                retry.push(task);
                if ctx.requests >= ctx.budget || std::time::Instant::now() >= ctx.deadline {
                    break;
                } else {
                    continue;
                }
            }
        };
        if task.kind == "category" {
            let document = dom::document(&html);
            let root = dom::root(&document);
            let mut articles = 0;
            let mut detail_tasks = Vec::new();
            let mut article_nodes: Vec<_> = dom::walk(&root)
                .into_iter()
                .filter(|n| dom::tag(n) == "article")
                .collect();
            article_nodes.sort_by_key(|n| {
                dom::first(n, |h| dom::has_class(h, "entry-title")).is_some_and(|h| {
                    identity::variant(&dom::text(&h)) == super::super::model::Variant::Dlc
                })
            });
            for article in article_nodes {
                articles += 1;
                let Some(heading) = dom::first(&article, |n| dom::has_class(n, "entry-title"))
                else {
                    continue;
                };
                let title = dom::text(&heading);
                if !lead_relevant(&title, query, previous) {
                    continue;
                }
                let links = dom::links(&heading, &final_url, |_, url| {
                    hostname_in(url, &rule.page_hosts)
                });
                let Some((_, page)) = links.first() else {
                    continue;
                };
                // WordPress category pages often contain the whole article. Parse
                // each article separately, retaining its canonical detail URL.
                if dom::first(&article, |n| dom::has_class(n, "entry-content")).is_some() {
                    match candidate_from_root(
                        rule,
                        adapter,
                        query,
                        &article,
                        page,
                        Some(&title),
                        &related_previous,
                    ) {
                        Ok(found) => {
                            if found.is_empty()
                                && identity::variant(&title) == super::super::model::Variant::Dlc
                                && !visited.contains(page)
                            {
                                deferred.push(Work {
                                    kind: "detail".into(),
                                    page: page.clone(),
                                    title: Some(title.clone()),
                                });
                            }
                            related_previous.known_candidates.extend(found.clone());
                            candidates.extend(found);
                        }
                        Err(e) => diagnostics.push(e.to_string()),
                    }
                } else if !visited.contains(page) && !work.iter().any(|w| &w.page == page) {
                    detail_tasks.push(Work {
                        kind: "detail".into(),
                        page: page.clone(),
                        title: Some(title),
                    });
                }
            }
            detail_tasks.sort_by_key(|w| {
                w.title.as_ref().is_some_and(|title| {
                    identity::variant(title) == super::super::model::Variant::Dlc
                })
            });
            for detail in detail_tasks.into_iter().rev() {
                work.push_front(detail);
            }
            if articles == 0 && dom::first(&root, |n| dom::has_class(n, "no-results")).is_none() {
                diagnostics.push("discovery_structure_changed".into());
                retry.push(task.clone());
            }
            if let Some(next) = next_page(&root, &final_url) {
                if visited.contains(&next) {
                    diagnostics.push("discovery_pagination_cycle".into());
                } else {
                    work.push_back(Work { page: next, ..task });
                }
            }
        } else {
            let document = dom::document(&html);
            let root = dom::root(&document);
            match candidate_from_root(
                rule,
                adapter,
                query,
                &root,
                &final_url,
                task.title.as_deref(),
                &related_previous,
            ) {
                Ok(found) => {
                    if found.is_empty()
                        && previous
                            .known_candidates
                            .iter()
                            .any(|c| c.source_page == task.page)
                    {
                        diagnostics.push("discovery_candidate_revalidation_incomplete".into());
                    }
                    candidates.extend(found);
                }
                Err(e) => {
                    diagnostics.push(e.to_string());
                    retry.push(task.clone());
                    if let Some(title) = task.title {
                        let mut fallback = identity::build_candidates(
                            rule,
                            &title,
                            &task.page,
                            "",
                            &[],
                            &[],
                            "unknown",
                            &[],
                            query,
                        );
                        for c in &mut fallback {
                            c.warnings.push("detail_unavailable".into());
                        }
                        candidates.extend(fallback);
                    }
                }
            }
        }
        // Only body links labelled as related DLC leads, on the publisher's
        // allowed page hosts, become work. Sidebars/download mirrors do not.
        let document = dom::document(&html);
        let root = dom::root(&document);
        if let Some(body) = dom::first(&root, |n| dom::has_class(n, "entry-content")) {
            for (label, page) in dom::links(&body, &final_url, |_, url| {
                hostname_in(url, &rule.page_hosts)
            }) {
                if identity::variant(&label) == super::super::model::Variant::Dlc
                    && lead_relevant(&label, query, &related_previous)
                    && !visited.contains(&page)
                    && !work.iter().any(|w| w.page == page)
                {
                    related_previous.related_pages.push(page.clone());
                    work.push_back(Work {
                        kind: "detail".into(),
                        page,
                        title: Some(label),
                    });
                }
            }
        }
        related_previous.known_candidates.extend(
            candidates
                .iter()
                .filter(|c| {
                    !related_previous
                        .known_candidates
                        .iter()
                        .any(|old| old.candidate_id == c.candidate_id)
                })
                .cloned()
                .collect::<Vec<_>>(),
        );
        if work.is_empty() && !deferred.is_empty() {
            work.extend(deferred.drain(..));
            resolving_deferred = true;
        }
    }
    // Append failures behind unvisited work so a repeated force makes progress.
    work.extend(retry);
    work.extend(deferred);
    for old in &previous.known_candidates {
        if !candidates.iter().any(|c| c.source_page == old.source_page) {
            diagnostics.push("discovery_candidate_revalidation_incomplete".into());
        }
    }
    if !work.is_empty() {
        diagnostics.push("discovery_index_incomplete".into());
    }
    let checkpoint = (!work.is_empty())
        .then(|| serde_json::json!({"binding":binding,"work":work,"candidates":candidates,"related_pages":related_previous.related_pages}));
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
    (candidates, index, diagnostics, checkpoint)
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Work {
    kind: String,
    page: String,
    title: Option<String>,
}

// This is only a discovery lead. A new DLC still needs an explicit body link
// to the known base page before its short title can enter candidate building.
fn lead_relevant(title: &str, query: &QueryContext, previous: &DiscoverPrevious) -> bool {
    if identity::title_relevant(title, query)
        || previous
            .known_candidates
            .iter()
            .any(|c| c.patch_title == title)
    {
        return true;
    }
    if identity::variant(title) != super::super::model::Variant::Dlc {
        return false;
    }
    let stem = super::super::validate::discovery_normal(&identity::base_title(title));
    std::iter::once(&query.name)
        .chain(query.name_hints.iter())
        .chain(previous.known_candidates.iter().map(|c| &c.game_title))
        .any(|name| {
            let base = name.split(['：', ':']).next().unwrap_or(name);
            stem == super::super::validate::discovery_normal(&identity::base_title(base))
        })
}

fn candidate_from_root(
    rule: &SourceRule,
    adapter: &str,
    query: &QueryContext,
    root: &ElementRef<'_>,
    final_url: &str,
    fallback_title: Option<&str>,
    previous: &DiscoverPrevious,
) -> Result<Vec<SourceCandidate>> {
    let heading = dom::first(&root, |node| {
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
    let mut related_query = query.clone();
    let known = previous.known_candidates.iter().any(|c| {
        c.source_page == final_url
            && c.patch_title == title
            && c.game_appid.is_none_or(|id| id == query.appid)
    });
    let base_link = dom::link_urls(&body)
        .iter()
        .filter_map(|url| super::join_relative(final_url, url))
        .any(|url| {
            previous
                .known_candidates
                .iter()
                .any(|c| c.variant == super::super::model::Variant::Base && c.source_page == url)
        });
    if known
        || (lead_relevant(&title, query, previous)
            && (base_link || previous.related_pages.iter().any(|p| p == final_url)))
    {
        related_query.name_hints.push(title.clone());
    }
    let mut found = identity::build_candidates(
        rule,
        &title,
        &final_url,
        &body_text,
        &ids,
        &[],
        "unknown",
        &mirrors,
        &related_query,
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

    const SUMMER_BASE: &str = r#"<html><body><article><h1 class="entry-title">夏日狂想曲：乡间的难忘回忆</h1><div class="entry-content"><p>中文 v1.03 Steam 补丁</p><a href="https://kaguraserver.com/Summer-Memories-CN-Patch-v1.03.exe">下载</a></div></article></body></html>"#;
    const SUMMER_DLC: &str = r#"<html><body><article><h1 class="entry-title">夏日狂想曲+</h1><div class="entry-content"><p>中文 v2.02.1 只有购买过DLC扩展包才能使用 Steam</p><a href="/base/">夏日狂想曲本体补丁</a><a href="https://kaguraserver.com/Summer-Memories-Plus-DLC-CN-Patch-v2.02.1.exe">下载</a></div></article></body></html>"#;
    fn summer_previous(include_dlc: bool) -> DiscoverPrevious {
        let r = rule("kagura_wordpress");
        let q = query(1227890, "夏日狂想曲：乡间的难忘回忆");
        let f = Arc::new(FakeFetcher::new());
        let mut ctx = context(&f);
        let doc = dom::document(SUMMER_BASE);
        let base = candidate_from_root(
            &r,
            "kagura_cn_html",
            &q,
            &dom::root(&doc),
            "https://kagurafan.com/base/",
            None,
            &DiscoverPrevious::default(),
        )
        .unwrap()
        .remove(0);
        let mut previous = DiscoverPrevious {
            index: Some(DiscoveryIndex {
                parser_version: PARSER_VERSION,
                encoding_version: SOURCES_ENCODING_VERSION,
                records: vec![IndexRecord {
                    title: "夏日狂想曲+".into(),
                    page: "https://kagurafan.com/category/dlc/".into(),
                    kind: "category".into(),
                }],
                ..Default::default()
            }),
            upstream_pages: vec![base.source_page.clone()],
            known_candidates: vec![base],
            ..Default::default()
        };
        if include_dlc {
            let doc = dom::document(SUMMER_DLC);
            let dlc = candidate_from_root(
                &r,
                "kagura_cn_html",
                &q,
                &dom::root(&doc),
                "https://kagurafan.com/dlc/",
                None,
                &previous,
            )
            .unwrap()
            .remove(0);
            previous.upstream_pages.push(dlc.source_page.clone());
            previous.known_candidates.push(dlc);
        }
        // avoid unused fetch context: construction is DOM-only
        let _ = &mut ctx;
        previous
    }
    #[test]
    fn old_base_and_dlc_survive_force_with_long_base_name() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(SUMMER_BASE, "https://kagurafan.com/base/"));
        fake.push(response(SUMMER_DLC, "https://kagurafan.com/dlc/"));
        fake.push(response(
            "<html><body><div class='no-results'></div></body></html>",
            "https://kagurafan.com/category/dlc/",
        ));
        let mut ctx = context(&fake);
        let mut q = query(1227890, "夏日狂想曲：乡间的难忘回忆");
        q.force = true;
        let out = super::super::discover(
            &rule("kagura_wordpress"),
            &q,
            &mut ctx,
            &summer_previous(true),
        )
        .unwrap();
        assert_eq!(out.status, "complete");
        assert_eq!(out.candidates.len(), 2);
        assert!(out.candidates.iter().any(|c| c.variant == Variant::Dlc));
        assert_eq!(fake.call_count(), 3);
    }
    #[test]
    fn base_only_cache_expands_dlc_category_body_without_duplicate_detail_fetch() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(SUMMER_BASE, "https://kagurafan.com/base/"));
        let category = SUMMER_DLC.replace(
            "<h1 class=\"entry-title\">夏日狂想曲+</h1>",
            "<h2 class=\"entry-title\"><a href='/dlc/'>夏日狂想曲+</a></h2>",
        );
        fake.push(response(&category, "https://kagurafan.com/category/dlc/"));
        let mut ctx = context(&fake);
        let q = query(1227890, "夏日狂想曲：乡间的难忘回忆");
        let out = super::super::discover(
            &rule("kagura_wordpress"),
            &q,
            &mut ctx,
            &summer_previous(false),
        )
        .unwrap();
        assert_eq!(out.status, "complete");
        assert_eq!(out.candidates.len(), 2);
        assert_eq!(fake.call_count(), 2);
        let dlc = out
            .candidates
            .iter()
            .find(|c| c.variant == Variant::Dlc)
            .unwrap();
        assert_eq!(dlc.game_appid, None);
        assert_eq!(dlc.match_basis, crate::sources::model::MatchBasis::Title);
    }
    #[test]
    fn cold_category_base_and_short_dlc_share_relationship_context() {
        let fake = Arc::new(FakeFetcher::new());
        let base = SUMMER_BASE.replace(
            "<h1 class=\"entry-title\">夏日狂想曲：乡间的难忘回忆</h1>",
            "<h2 class=\"entry-title\"><a href='/base/'>夏日狂想曲：乡间的难忘回忆</a></h2>",
        );
        let dlc = SUMMER_DLC.replace(
            "<h1 class=\"entry-title\">夏日狂想曲+</h1>",
            "<h2 class=\"entry-title\"><a href='/dlc/'>夏日狂想曲+</a></h2>",
        );
        fake.push(response(
            &format!("{dlc}{base}"),
            "https://kagurafan.com/category/game/",
        ));
        let previous = DiscoverPrevious {
            index: Some(DiscoveryIndex {
                parser_version: PARSER_VERSION,
                encoding_version: SOURCES_ENCODING_VERSION,
                records: vec![IndexRecord {
                    title: "夏日狂想曲：乡间的难忘回忆".into(),
                    page: "https://kagurafan.com/category/game/".into(),
                    kind: "category".into(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut ctx = context(&fake);
        let out = super::super::discover(
            &rule("kagura_wordpress"),
            &query(1227890, "夏日狂想曲：乡间的难忘回忆"),
            &mut ctx,
            &previous,
        )
        .unwrap();
        assert_eq!(out.status, "complete");
        assert_eq!(out.candidates.len(), 2);
    }

    #[test]
    fn known_base_body_link_expands_missing_dlc_without_index_entry() {
        let fake = Arc::new(FakeFetcher::new());
        let base = SUMMER_BASE.replace("</div>", "<a href='/dlc/'>夏日狂想曲+</a></div>");
        fake.push(response(&base, "https://kagurafan.com/base/"));
        fake.push(response(SUMMER_DLC, "https://kagurafan.com/dlc/"));
        let mut previous = summer_previous(false);
        previous.index.as_mut().unwrap().records.clear();
        let mut ctx = context(&fake);
        let out = super::super::discover(
            &rule("kagura_wordpress"),
            &query(1227890, "夏日狂想曲：乡间的难忘回忆"),
            &mut ctx,
            &previous,
        )
        .unwrap();
        assert_eq!(out.status, "complete");
        assert_eq!(out.candidates.len(), 2);
        assert_eq!(fake.call_count(), 2);
    }

    #[test]
    fn cold_summary_category_fetches_base_before_short_dlc() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response("<html><body><article><h2 class='entry-title'><a href='/dlc/'>夏日狂想曲+</a></h2></article><article><h2 class='entry-title'><a href='/base/'>夏日狂想曲：乡间的难忘回忆</a></h2></article></body></html>","https://kagurafan.com/category/game/"));
        fake.push(response(SUMMER_BASE, "https://kagurafan.com/base/"));
        fake.push(response(SUMMER_DLC, "https://kagurafan.com/dlc/"));
        let previous = DiscoverPrevious {
            index: Some(DiscoveryIndex {
                parser_version: PARSER_VERSION,
                encoding_version: SOURCES_ENCODING_VERSION,
                records: vec![IndexRecord {
                    title: "夏日狂想曲：乡间的难忘回忆".into(),
                    page: "https://kagurafan.com/category/game/".into(),
                    kind: "category".into(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut ctx = context(&fake);
        let out = super::super::discover(
            &rule("kagura_wordpress"),
            &query(1227890, "夏日狂想曲：乡间的难忘回忆"),
            &mut ctx,
            &previous,
        )
        .unwrap();
        assert_eq!(out.status, "complete");
        assert_eq!(out.candidates.len(), 2);
        assert_eq!(fake.calls()[1].url, "https://kagurafan.com/base/");
    }

    #[test]
    fn cold_base_summary_establishes_context_before_next_dlc_category() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response("<html><body><article><h2 class='entry-title'><a href='/base/'>夏日狂想曲：乡间的难忘回忆</a></h2></article></body></html>","https://kagurafan.com/category/base/"));
        fake.push(response(SUMMER_BASE, "https://kagurafan.com/base/"));
        let category = SUMMER_DLC.replace(
            "<h1 class=\"entry-title\">夏日狂想曲+</h1>",
            "<h2 class=\"entry-title\"><a href='/dlc/'>夏日狂想曲+</a></h2>",
        );
        fake.push(response(&category, "https://kagurafan.com/category/dlc/"));
        let previous = DiscoverPrevious {
            index: Some(DiscoveryIndex {
                parser_version: PARSER_VERSION,
                encoding_version: SOURCES_ENCODING_VERSION,
                records: vec![
                    IndexRecord {
                        title: "夏日狂想曲：乡间的难忘回忆".into(),
                        page: "https://kagurafan.com/category/base/".into(),
                        kind: "category".into(),
                    },
                    IndexRecord {
                        title: "夏日狂想曲+".into(),
                        page: "https://kagurafan.com/category/dlc/".into(),
                        kind: "category".into(),
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut ctx = context(&fake);
        let out = super::super::discover(
            &rule("kagura_wordpress"),
            &query(1227890, "夏日狂想曲：乡间的难忘回忆"),
            &mut ctx,
            &previous,
        )
        .unwrap();
        assert_eq!(out.status, "complete");
        assert_eq!(out.candidates.len(), 2);
        assert_eq!(fake.calls()[1].url, "https://kagurafan.com/base/");
    }

    #[test]
    fn category_pagination_never_overtakes_current_base_detail() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response("<html><body><article><h2 class='entry-title'><a href='/base/'>夏日狂想曲：乡间的难忘回忆</a></h2></article><a rel='next' href='/category/game/page/2/'>Next</a></body></html>","https://kagurafan.com/category/game/"));
        fake.push(response(SUMMER_BASE, "https://kagurafan.com/base/"));
        let category = SUMMER_DLC.replace(
            "<h1 class=\"entry-title\">夏日狂想曲+</h1>",
            "<h2 class=\"entry-title\"><a href='/dlc/'>夏日狂想曲+</a></h2>",
        );
        fake.push(response(
            &category,
            "https://kagurafan.com/category/game/page/2/",
        ));
        let previous = DiscoverPrevious {
            index: Some(DiscoveryIndex {
                parser_version: PARSER_VERSION,
                encoding_version: SOURCES_ENCODING_VERSION,
                records: vec![IndexRecord {
                    title: "夏日狂想曲：乡间的难忘回忆".into(),
                    page: "https://kagurafan.com/category/game/".into(),
                    kind: "category".into(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut ctx = context(&fake);
        let out = super::super::discover(
            &rule("kagura_wordpress"),
            &query(1227890, "夏日狂想曲：乡间的难忘回忆"),
            &mut ctx,
            &previous,
        )
        .unwrap();
        assert_eq!(out.status, "complete");
        assert_eq!(out.candidates.len(), 2);
        assert_eq!(fake.calls()[1].url, "https://kagurafan.com/base/");
    }

    #[test]
    fn dlc_first_page_is_reconsidered_after_base_on_later_page() {
        let fake = Arc::new(FakeFetcher::new());
        let dlc = SUMMER_DLC
            .replace(
                "<h1 class=\"entry-title\">夏日狂想曲+</h1>",
                "<h2 class=\"entry-title\"><a href='/dlc/'>夏日狂想曲+</a></h2>",
            )
            .replace(
                "</body>",
                "<a rel='next' href='/category/game/page/2/'>Next</a></body>",
            );
        let base = SUMMER_BASE.replace(
            "<h1 class=\"entry-title\">夏日狂想曲：乡间的难忘回忆</h1>",
            "<h2 class=\"entry-title\"><a href='/base/'>夏日狂想曲：乡间的难忘回忆</a></h2>",
        );
        fake.push(response(&dlc, "https://kagurafan.com/category/game/"));
        fake.push(response(
            &base,
            "https://kagurafan.com/category/game/page/2/",
        ));
        fake.push(response(SUMMER_DLC, "https://kagurafan.com/dlc/"));
        let previous = DiscoverPrevious {
            index: Some(DiscoveryIndex {
                parser_version: PARSER_VERSION,
                encoding_version: SOURCES_ENCODING_VERSION,
                records: vec![IndexRecord {
                    title: "夏日狂想曲：乡间的难忘回忆".into(),
                    page: "https://kagurafan.com/category/game/".into(),
                    kind: "category".into(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut ctx = context(&fake);
        let out = super::super::discover(
            &rule("kagura_wordpress"),
            &query(1227890, "夏日狂想曲：乡间的难忘回忆"),
            &mut ctx,
            &previous,
        )
        .unwrap();
        assert_eq!(out.status, "complete");
        assert_eq!(out.candidates.len(), 2);
        assert_eq!(
            fake.calls()[1].url,
            "https://kagurafan.com/category/game/page/2/"
        );
    }

    #[test]
    fn missing_body_relationship_does_not_admit_unrelated_plus_title() {
        let previous = summer_previous(false);
        let q = query(1227890, "夏日狂想曲：乡间的难忘回忆");
        let html = SUMMER_DLC.replace("/base/", "/other-game/");
        let doc = dom::document(&html);
        assert!(candidate_from_root(
            &rule("kagura_wordpress"),
            "kagura_cn_html",
            &q,
            &dom::root(&doc),
            "https://kagurafan.com/unrelated/",
            None,
            &previous
        )
        .unwrap()
        .is_empty());
    }
    #[test]
    fn budget_checkpoint_resumes_missing_dlc_before_repeating_base() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(SUMMER_BASE, "https://kagurafan.com/base/"));
        let mut ctx = context(&fake);
        ctx.budget = 1;
        let q = query(1227890, "夏日狂想曲：乡间的难忘回忆");
        let mut previous = summer_previous(true);
        previous.index.as_mut().unwrap().records.clear();
        let first =
            super::super::discover(&rule("kagura_wordpress"), &q, &mut ctx, &previous).unwrap();
        assert_eq!(first.status, "partial");
        assert!(first.checkpoint.is_some());
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(SUMMER_DLC, "https://kagurafan.com/dlc/"));
        let mut ctx = context(&fake);
        previous.checkpoint = first.checkpoint;
        let second =
            super::super::discover(&rule("kagura_wordpress"), &q, &mut ctx, &previous).unwrap();
        assert_eq!(second.status, "complete");
        assert_eq!(second.candidates.len(), 2);
        assert_eq!(fake.calls()[0].url, "https://kagurafan.com/dlc/");
    }

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
    fn repeated_force_queries_continue_pending_pages_before_revisiting_entry() {
        let mut previous = DiscoverPrevious {
            index: Some(DiscoveryIndex {
                parser_version: PARSER_VERSION,
                encoding_version: SOURCES_ENCODING_VERSION,
                records: vec![IndexRecord {
                    title: "Other".into(),
                    page: "https://kagurafan.com/other/".into(),
                    kind: "article".into(),
                }],
                pending: vec!["https://kagurafan.com/page/3/".into()],
                checked_at: 123,
            }),
            upstream_pages: vec![],
            ..Default::default()
        };
        let mut q = query(42, "Missing");
        q.force = true;
        for page in [3, 4] {
            let fake = Arc::new(FakeFetcher::new());
            let html = format!("<html><body><article><h2 class='entry-title'><a href='/other/'>Other</a></h2></article><a rel='next' href='/page/{}/'>Next</a></body></html>", page+1);
            fake.push(response(
                &html,
                &format!("https://kagurafan.com/page/{page}/"),
            ));
            fake.push(response(INDEX, "https://kagurafan.com/"));
            let mut ctx = context(&fake);
            let outcome = KaguraAdapter
                .discover(&rule("kagura_wordpress"), &q, &mut ctx, &previous)
                .unwrap();
            assert_eq!(
                fake.calls()[0].url,
                format!("https://kagurafan.com/page/{page}/")
            );
            let index = outcome.index.unwrap();
            assert!(index
                .pending
                .contains(&format!("https://kagurafan.com/page/{}/", page + 1)));
            assert!(index.records.iter().any(|r| r.title == "Other"));
            previous.index = Some(index);
        }
    }

    #[test]
    fn force_direct_revalidation_preserves_shared_index_and_checkpoint() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(ARTICLE, "https://kagurafan.com/game/"));
        let mut ctx = context(&fake);
        let previous = DiscoverPrevious {
            index: Some(DiscoveryIndex {
                parser_version: PARSER_VERSION,
                encoding_version: SOURCES_ENCODING_VERSION,
                records: vec![IndexRecord {
                    title: "Other".into(),
                    page: "https://kagurafan.com/other/".into(),
                    kind: "article".into(),
                }],
                pending: vec!["https://kagurafan.com/page/3/".into()],
                checked_at: 123,
            }),
            upstream_pages: vec!["https://kagurafan.com/game/".into()],
            ..Default::default()
        };
        let mut q = query(42, "Game");
        q.force = true;
        let outcome = KaguraAdapter
            .discover(&rule("kagura_wordpress"), &q, &mut ctx, &previous)
            .unwrap();
        assert_eq!(fake.call_count(), 1);
        let index = outcome.index.unwrap();
        assert_eq!(index.records, previous.index.as_ref().unwrap().records);
        assert_eq!(index.pending, previous.index.as_ref().unwrap().pending);
        assert_eq!(outcome.candidates.len(), 1);
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
            ..Default::default()
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
