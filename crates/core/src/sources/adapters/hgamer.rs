//! HGamer fallback-index adapter (`hgamer_html`), porting `discovery.py::_hgamer`:
//! search via `query_terms`, numeric result-count requirement, Steam-appid
//! identity, alias extraction, format report and mirror collection.

use super::super::http::RequestContext;
use super::super::model::{QueryContext, SourceCandidate, SourceRule};
use super::super::{identity, validate, Result};
use super::dom;
use super::{fetch_page, hostname_in, Adapter, DiscoverOutcome, DiscoverPrevious};
use scraper::ElementRef;

pub struct HgamerAdapter;

impl Adapter for HgamerAdapter {
    fn discover(
        &self,
        rule: &SourceRule,
        query: &QueryContext,
        ctx: &mut RequestContext,
        _previous: &DiscoverPrevious,
    ) -> Result<DiscoverOutcome> {
        let (candidates, identities, diagnostics) = hgamer(rule, query, ctx);
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
            index: None,
            checkpoint: None,
            identities,
        })
    }
}

fn hgamer(
    rule: &SourceRule,
    query: &QueryContext,
    ctx: &mut RequestContext,
) -> (Vec<SourceCandidate>, Vec<identity::Identity>, Vec<String>) {
    let mut diagnostics: Vec<String> = Vec::new();
    let base = rule
        .url
        .clone()
        .or_else(|| rule.pages.first().cloned())
        .unwrap_or_default();

    let mut games: Vec<(String, String)> = Vec::new();
    let mut patches: Vec<(String, String)> = Vec::new();

    for term in identity::query_terms(&query.name, &query.name_hints) {
        let Some(search_url) = super::join_relative(&base, "/zh-CN/search") else {
            diagnostics.push("discovery_search_structure_changed".to_string());
            continue;
        };
        let Ok(mut url) = url::Url::parse(&search_url) else {
            diagnostics.push("discovery_search_structure_changed".to_string());
            continue;
        };
        url.query_pairs_mut().append_pair("q", term.as_str());
        let (html, final_url) = match fetch_page(ctx, url.as_str(), &rule.page_hosts) {
            Ok(value) => value,
            Err(error) => {
                diagnostics.push(error.to_string());
                continue;
            }
        };
        let document = dom::document(&html);
        let root = dom::root(&document);
        let header = dom::first(&root, |node| dom::tag(node) == "h1");
        let summary = header
            .as_ref()
            .and_then(dom::parent)
            .map(|parent| dom::text(&parent))
            .unwrap_or_default();
        let count = result_count(&summary);
        if !summary.contains(term.as_str()) || count.is_none() {
            diagnostics.push("discovery_search_structure_changed".to_string());
            continue;
        }
        let count = count.unwrap_or(0);

        let mut identified = 0_u32;
        for heading in dom::walk(&root) {
            if dom::tag(&heading) != "h2" {
                continue;
            }
            let heading_text = dom::text(&heading);
            let kind = match heading_text.as_str() {
                "游戏" | "遊戲" => "games",
                "补丁" | "補丁" => "patches",
                "文章" | "新聞" | "新闻" | "Articles" => "posts",
                _ => continue,
            };
            let Some(parent) = dom::parent(&heading) else {
                continue;
            };
            let Some(group) = dom::parent(&parent) else {
                continue;
            };
            for (label, page) in dom::links(&group, &final_url, |_, url| {
                hostname_in(url, &rule.page_hosts)
                    && matches_detail_path(&validate::split_url(url).path, kind)
            }) {
                identified += 1;
                if kind == "games" {
                    push_ordered(&mut games, page, label);
                } else if kind == "patches" {
                    push_ordered(&mut patches, page, label);
                }
            }
        }
        if identified < count {
            diagnostics.push("discovery_search_structure_changed".to_string());
        }
    }

    let mut identities: Vec<identity::Identity> = Vec::new();
    for (page, _) in &games {
        let (html, final_url) = match fetch_page(ctx, page, &rule.page_hosts) {
            Ok(value) => value,
            Err(error) => {
                diagnostics.push(error.to_string());
                continue;
            }
        };
        let document = dom::document(&html);
        let root = dom::root(&document);
        let body = dom::first(&root, |node| dom::attr(node, "data-game-body").is_some());
        let heading = body
            .as_ref()
            .and_then(|body| dom::first(body, |node| dom::tag(node) == "h1"));
        let Some(heading) = heading else {
            diagnostics.push("discovery_structure_changed".to_string());
            continue;
        };
        let Some(primary) = dom::parent(&heading) else {
            diagnostics.push("discovery_structure_changed".to_string());
            continue;
        };
        let ids = identity::steam_ids(&dom::link_urls(&primary));
        if !ids.contains(&query.appid) {
            continue;
        }
        let aliases = identity::aliases(&dom::text(&heading), &dom::element_texts(&primary));
        identities.push(identity::Identity {
            appid: query.appid,
            page: final_url.clone(),
            aliases,
        });
        if let Some(body) = &body {
            for (label, patch) in dom::links(body, &final_url, |_, url| {
                hostname_in(url, &rule.page_hosts)
                    && matches_detail_path(&validate::split_url(url).path, "patches")
            }) {
                push_ordered(&mut patches, patch, label);
            }
        }
    }

    let mut candidates = Vec::new();
    for (page, _) in &patches {
        let (html, final_url) = match fetch_page(ctx, page, &rule.page_hosts) {
            Ok(value) => value,
            Err(error) => {
                diagnostics.push(error.to_string());
                continue;
            }
        };
        let document = dom::document(&html);
        let root = dom::root(&document);
        let body = dom::first(&root, |node| dom::attr(node, "data-patch-body").is_some());
        let heading = body
            .as_ref()
            .and_then(|body| dom::first(body, |node| dom::tag(node) == "h1"));
        let Some(heading) = heading else {
            diagnostics.push("discovery_structure_changed".to_string());
            continue;
        };
        let Some(primary) = dom::parent(&heading) else {
            diagnostics.push("discovery_structure_changed".to_string());
            continue;
        };
        let mut ids = identity::steam_ids(&dom::link_urls(&primary));
        let descriptions = description_blocks(&body);
        for description in &descriptions {
            ids.extend(identity::steam_ids(&dom::link_urls(description)));
        }
        for description in &descriptions {
            if let Some(column) = dom::parent(description) {
                for child in dom::children(&column) {
                    if matches!(dom::tag(&child), "iframe" | "a") {
                        ids.extend(identity::steam_ids(&dom::link_urls(&child)));
                    }
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();

        let primary_text = dom::text(&primary);
        let reported = if contains_word_ci(&primary_text, "ZIP") {
            "zip"
        } else if contains_word_ci(&primary_text, "EXE") {
            "exe"
        } else {
            "unknown"
        };
        let mirrors = dom::mirrors(&primary, &final_url);
        let mut aliases = identity::aliases(&dom::text(&heading), &dom::element_texts(&primary));
        if identity::game_title(&dom::text(&heading)).is_empty() {
            if let Some(found) = identities.iter().find(|entry| ids.contains(&entry.appid)) {
                aliases = found.aliases.clone();
            }
        }
        let mut found = identity::build_candidates(
            rule,
            &dom::text(&heading),
            &final_url,
            &primary_text,
            &ids,
            &aliases,
            reported,
            &mirrors,
            query,
        );
        for candidate in &mut found {
            candidate.upstream_page = mirrors
                .iter()
                .find(|mirror| !hostname_in(&mirror.url, &rule.page_hosts))
                .map(|mirror| mirror.url.clone());
        }
        candidates.extend(found);
    }

    (candidates, identities, diagnostics)
}

/// Python `discovery.py::_hgamer` uses `description = h.parent.parent`, i.e. the
/// content-column block that owns the `<h2>` heading, not the heading itself.
/// The official Steam widget (`<iframe>`/`<a>`) is a sibling of that block in
/// the same column, so the caller's `description.parent` direct-child scan can
/// only reach it when the grandparent is returned.
fn description_blocks<'a>(body: &Option<ElementRef<'a>>) -> Vec<ElementRef<'a>> {
    let Some(body) = body else {
        return Vec::new();
    };
    dom::walk(body)
        .into_iter()
        .filter(|node| {
            dom::tag(node) == "h2"
                && matches!(
                    dom::text(node).as_str(),
                    "補丁說明" | "补丁说明" | "安裝教學" | "安装教学"
                )
        })
        .filter_map(|node| dom::parent(&dom::parent(&node)?))
        .collect()
}

fn push_ordered(list: &mut Vec<(String, String)>, page: String, label: String) {
    if let Some(position) = list.iter().position(|(existing, _)| existing == &page) {
        list[position].1 = label;
    } else {
        list.push((page, label));
    }
}

/// `/zh-[A-Z]{2}/<kind>/[^/]+/?$`
fn matches_detail_path(path: &str, kind: &str) -> bool {
    let Some(rest) = path.strip_prefix("/zh-") else {
        return false;
    };
    let bytes = rest.as_bytes();
    if bytes.len() < 2 || !bytes[0].is_ascii_uppercase() || !bytes[1].is_ascii_uppercase() {
        return false;
    }
    let rest = &rest[2..];
    let Some(rest) = rest.strip_prefix('/') else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(kind) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix('/') else {
        return false;
    };
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    !rest.is_empty() && !rest.contains('/')
}

/// `(?:找到|found)\s*(\d+)\s*(?:笔|筆|results?)`
fn result_count(summary: &str) -> Option<u32> {
    for marker in ["找到", "found"] {
        let mut search = 0;
        while let Some(start) = find_ci_from(summary, marker, search) {
            let absolute = start + marker.len();
            let rest = summary[absolute..].trim_start();
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                let after = rest[digits.len()..].trim_start();
                if after.starts_with('笔')
                    || after.starts_with('筆')
                    || after.to_ascii_lowercase().starts_with("result")
                {
                    if let Ok(count) = digits.parse::<u32>() {
                        return Some(count);
                    }
                }
            }
            search = absolute;
        }
    }
    None
}

fn find_ci_from(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    if from >= haystack.len() {
        return None;
    }
    if needle.is_ascii() {
        haystack[from..]
            .to_ascii_lowercase()
            .find(&needle.to_ascii_lowercase())
            .map(|position| position + from)
    } else {
        haystack[from..]
            .find(needle)
            .map(|position| position + from)
    }
}

fn contains_word_ci(text: &str, word: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let word = word.to_ascii_lowercase();
    let mut search = 0;
    while let Some(position) = lower[search..].find(&word) {
        let absolute = search + position;
        let before_ok = !text[..absolute]
            .chars()
            .next_back()
            .is_some_and(is_word_char);
        let after = absolute + word.len();
        let after_ok = !text[after..].chars().next().is_some_and(is_word_char);
        if before_ok && after_ok {
            return true;
        }
        search = absolute + word.len();
    }
    false
}

fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

#[cfg(test)]
mod tests {
    use super::super::{builtin_adapter, Adapter, DiscoverPrevious};
    use super::*;
    use crate::sources::http::{FakeFetcher, FetchResponse, RequestContext};
    use crate::sources::model::{ArtifactFormat, MatchBasis, SourceRole};
    use std::sync::Arc;
    use std::time::Duration;

    fn rule() -> SourceRule {
        SourceRule {
            id: "hgamer".to_string(),
            name: "HGamer".to_string(),
            adapter: "hgamer_html".to_string(),
            role: SourceRole::FallbackIndexer,
            discovery_only: true,
            priority: 100,
            enabled: true,
            pages: vec!["https://hgamer.tw/zh-CN/patches".to_string()],
            page_hosts: vec!["hgamer.tw".to_string()],
            download_hosts: Vec::new(),
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
        RequestContext::with_settings(Box::new(Arc::clone(fake)), Duration::ZERO)
    }

    const SEARCH: &str = r#"<html><body>
<div class="summary"><h1>Search</h1><p>找到 1 笔 results for Game</p></div>
<div><h2>游戏</h2><ul><li><a href="/zh-CN/games/1">Game</a></li></ul></div>
</body></html>"#;

    const GAME: &str = r#"<html><body>
<div data-game-body>
<h1>Game</h1>
<div><a href="https://store.steampowered.com/app/42/Game/">Steam</a>
<a href="/zh-CN/patches/patch-1">Patch</a></div>
</div>
</body></html>"#;

    const PATCH: &str = r#"<html><body>
<div data-patch-body>
<h1>Patch</h1>
<div><a href="https://store.steampowered.com/app/42/Game/">Steam</a>
<p>ZIP</p>
<a href="https://mirror.example/patch.zip">下载 Mirror 1</a></div>
</div>
</body></html>"#;

    #[test]
    fn search_and_patch_candidate() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(SEARCH, "https://hgamer.tw/zh-CN/search?q=Game"));
        fake.push(response(GAME, "https://hgamer.tw/zh-CN/games/1"));
        fake.push(response(PATCH, "https://hgamer.tw/zh-CN/patches/patch-1"));
        let mut ctx = context(&fake);
        let adapter = builtin_adapter("hgamer_html").unwrap();
        let outcome = adapter
            .discover(
                &rule(),
                &query(42, "Game"),
                &mut ctx,
                &DiscoverPrevious::default(),
            )
            .unwrap();
        assert_eq!(outcome.identities.len(), 1);
        assert_eq!(outcome.candidates.len(), 1);
        let candidate = &outcome.candidates[0];
        assert_eq!(candidate.game_appid, Some(42));
        assert_eq!(candidate.match_basis, MatchBasis::Appid);
        assert_eq!(candidate.format_reported, ArtifactFormat::Zip);
        assert_eq!(candidate.mirrors.len(), 1);
        assert_eq!(
            candidate.upstream_page.as_deref(),
            Some("https://mirror.example/patch.zip")
        );
    }

    const SEARCH_WITH_DESCRIPTION_PATCH: &str = r#"<html><body>
<div class="summary"><h1>Search</h1><p>找到 1 笔 results for Game</p></div>
<div><h2>补丁</h2><ul><li><a href="/zh-CN/patches/patch-desc">Game patch</a></li></ul></div>
</body></html>"#;

    // The Steam widget is a sibling of the `<h2>`'s grandparent (content
    // column), not of the heading. `heading.parent` (the hero) cannot reach it.
    const PATCH_WITH_DESCRIPTION: &str = r#"<html><body>
<div data-patch-body="true">
<section class="hero"><h1>Game 补丁</h1><p>v4</p></section>
<section class="content">
<iframe src="https://store.steampowered.com/widget/42/" title="Steam"></iframe>
<div class="block"><div class="head"><h2>补丁说明</h2></div><div class="text"><p>ZIP</p></div></div>
</section>
</div>
</body></html>"#;

    #[test]
    fn description_block_widget_yields_steam_identity() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(
            SEARCH_WITH_DESCRIPTION_PATCH,
            "https://hgamer.tw/zh-CN/search?q=Game",
        ));
        fake.push(response(
            PATCH_WITH_DESCRIPTION,
            "https://hgamer.tw/zh-CN/patches/patch-desc",
        ));
        let mut ctx = context(&fake);
        let adapter = builtin_adapter("hgamer_html").unwrap();
        let outcome = adapter
            .discover(
                &rule(),
                &query(42, "Game"),
                &mut ctx,
                &DiscoverPrevious::default(),
            )
            .unwrap();
        assert_eq!(outcome.candidates.len(), 1);
        let candidate = &outcome.candidates[0];
        assert_eq!(candidate.game_appid, Some(42));
        assert_eq!(candidate.match_basis, MatchBasis::Appid);
        assert_eq!(candidate.platforms, vec!["steam".to_string()]);
        assert!(candidate
            .identity_evidence
            .iter()
            .any(|evidence| evidence.kind == "steam_appid" && evidence.appid == Some(42)));
    }

    #[test]
    fn missing_numeric_result_count_errors() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(
            r#"<html><body><div><h1>Search</h1><p>no results for Game</p></div></body></html>"#,
            "https://hgamer.tw/zh-CN/search?q=Game",
        ));
        let mut ctx = context(&fake);
        let outcome = HgamerAdapter
            .discover(
                &rule(),
                &query(42, "Game"),
                &mut ctx,
                &DiscoverPrevious::default(),
            )
            .unwrap();
        assert!(outcome.candidates.is_empty());
        assert!(outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "discovery_search_structure_changed"));
    }

    const SEARCH_MANY: &str = r#"<html><body>
<div class="summary"><h1>Search</h1><p>找到 7 笔 results for Game</p></div>
<div><h2>游戏</h2><ul>
<li><a href="/zh-CN/games/1">G1</a></li>
<li><a href="/zh-CN/games/2">G2</a></li>
<li><a href="/zh-CN/games/3">G3</a></li>
<li><a href="/zh-CN/games/4">G4</a></li>
<li><a href="/zh-CN/games/5">G5</a></li>
<li><a href="/zh-CN/games/6">G6</a></li>
<li><a href="/zh-CN/games/7">G7</a></li>
</ul></div>
</body></html>"#;

    #[test]
    fn detail_requests_are_not_capped() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(
            SEARCH_MANY,
            "https://hgamer.tw/zh-CN/search?q=Game",
        ));
        for index in 1..=7 {
            fake.push(response(
                "<html><body></body></html>",
                &format!("https://hgamer.tw/zh-CN/games/{index}"),
            ));
        }
        let mut ctx = context(&fake);
        let outcome = HgamerAdapter
            .discover(
                &rule(),
                &query(42, "Game"),
                &mut ctx,
                &DiscoverPrevious::default(),
            )
            .unwrap();
        // Every game detail page is fetched; no request-count cap stops the
        // traversal.
        assert_eq!(fake.call_count(), 8);
        assert!(!outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.contains("budget")));
    }
}
