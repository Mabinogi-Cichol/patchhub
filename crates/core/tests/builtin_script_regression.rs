//! Behavior assertions migrated from the retired website Rust modules.
use patchhub_core::sources::{
    self,
    adapters::{source_adapter, DiscoverOutcome, DiscoverPrevious},
    http::RequestContext,
    model::*,
};
use std::sync::{atomic::AtomicBool, Arc};
struct CompatAdapter;
fn builtin_adapter(name: &str) -> Option<CompatAdapter> {
    sources::builtin::code(name).map(|_| CompatAdapter)
}
#[derive(Debug)]
pub struct Collection {
    pub entries: Vec<SourceEntry>,
    pub etag: Option<String>,
    pub modified: Option<String>,
    pub not_modified: bool,
}
impl CompatAdapter {
    fn collect(
        &self,
        rule: &SourceRule,
        previous: &SourceCache,
        ctx: &mut RequestContext,
    ) -> sources::Result<Collection> {
        let mut rule = rule.clone();
        sources::builtin::bind(&mut rule);
        let index = SourceIndex {
            schema_version: 1,
            source_id: rule.id.clone(),
            parser_fingerprint: sources::index::parser_fingerprint(&rule),
            revision: 1,
            updated_at: 0,
            checked_at: 0,
            complete: true,
            pending: vec![],
            etag: previous.etag.clone(),
            modified: previous.modified.clone(),
            entries: previous
                .entries
                .iter()
                .map(|e| SourceIndexEntry {
                    title: e.title.clone(),
                    aliases: e.aliases.clone(),
                    page: e.source_page.clone(),
                    kind: "entry".into(),
                    appid: e.appid,
                    url: Some(e.url.clone()),
                    language: Some(e.language.clone()),
                    date: e.date.clone(),
                })
                .collect(),
        };
        let result = source_adapter(&rule, None)?.collect_index(
            &rule,
            Some(&index),
            ctx,
            &Arc::new(AtomicBool::new(false)),
        )?;
        Ok(Collection {
            entries: result
                .entries
                .into_iter()
                .map(|e| SourceEntry {
                    title: e.title,
                    aliases: e.aliases,
                    source_page: e.page,
                    url: e.url.unwrap(),
                    language: e.language.unwrap(),
                    date: e.date,
                    appid: e.appid,
                })
                .collect(),
            etag: result.etag,
            modified: result.modified,
            not_modified: result.not_modified,
        })
    }
    fn discover(
        &self,
        rule: &SourceRule,
        query: &QueryContext,
        ctx: &mut RequestContext,
        previous: &DiscoverPrevious,
    ) -> sources::Result<DiscoverOutcome> {
        let mut rule = rule.clone();
        sources::builtin::bind(&mut rule);
        source_adapter(&rule, None)?.discover(
            &rule,
            query,
            ctx,
            previous,
            &Arc::new(AtomicBool::new(false)),
        )
    }
}
mod patchmoe {
    use super::*;
    use patchhub_core::sources::http::{FakeFetcher, FetchResponse, RequestContext};
    use patchhub_core::sources::model::SourceEntry;
    use std::sync::Arc;
    use std::time::Duration;

    fn rule() -> SourceRule {
        SourceRule {
            script: None,
            id: "patchmoe".to_string(),
            name: "patch.moe".to_string(),
            adapter: "patchmoe".to_string(),
            role: patchhub_core::sources::model::SourceRole::Publisher,
            discovery_only: false,
            priority: 100,
            enabled: true,
            pages: vec!["https://www.patch.moe/?flag=1".to_string()],
            page_hosts: vec!["www.patch.moe".to_string()],
            download_hosts: vec!["download.patch.moe".to_string()],
            root_layout: Some(patchhub_core::sources::model::RootLayout::GameRoot),
            columns: None,
            table_id: None,
            language: None,
            url: None,
        }
    }

    fn response(status: u16, html: &str, final_url: &str) -> FetchResponse {
        FetchResponse {
            status,
            body: html.as_bytes().to_vec(),
            final_url: final_url.to_string(),
            ..FetchResponse::default()
        }
    }

    fn context(fake: &Arc<FakeFetcher>) -> RequestContext {
        RequestContext::with_settings(Box::new(Arc::clone(fake)), Duration::ZERO)
    }

    const INITIAL: &str = r#"<html><body>
<form name="form_zh_cn" method="post" action="/zh"><input type="hidden" name="lang" value="zh-CN"></form>
<form name="form_en" method="post" action="/en"><input type="hidden" name="lang" value="en"></form>
</body></html>"#;

    /// Cropped excerpt of the live localized index: `table.tbl.new` (recent
    /// updates) plus `table.tbl.regular` (full list), each with a header row.
    const INDEX_FIXTURE: &str = include_str!("fixtures/sources/patchmoe-index.html");

    fn table(title: &str) -> String {
        format!(
            r#"<table><tr><th class="title">Title</th><th class="release">Date</th><th class="patch">Patch</th></tr>
<tr><td class="title">{title}</td><td class="release">2024/1/2</td><td class="patch"><a href="https://download.patch.moe/patch.zip">dl</a></td></tr></table>"#
        )
    }

    #[test]
    fn pairs_language_forms_by_identical_url() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(200, INITIAL, "https://www.patch.moe/?flag=1"));
        fake.push(response(200, &table("Game"), "https://www.patch.moe/zh"));
        fake.push(response(
            200,
            &table("Game English"),
            "https://www.patch.moe/en",
        ));
        let mut ctx = context(&fake);
        let adapter = builtin_adapter("patchmoe").unwrap();
        let outcome = adapter
            .collect(&rule(), &SourceCache::default(), &mut ctx)
            .unwrap();
        assert!(!outcome.not_modified);
        assert_eq!(outcome.entries.len(), 1);
        let entry = &outcome.entries[0];
        assert_eq!(entry.title, "Game");
        assert_eq!(entry.language, "zh-CN");
        assert_eq!(entry.aliases, vec!["Game English"]);
        assert_eq!(entry.url, "https://download.patch.moe/patch.zip");
        assert_eq!(entry.date.as_deref(), Some("2024/1/2"));
    }

    #[test]
    fn parses_rows_from_new_and_regular_tables() {
        // The live page ships two tables (`tbl new` + `tbl regular`); rows from
        // both must be collected, with each `th` header row skipped.
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(200, INITIAL, "https://www.patch.moe/?flag=1"));
        fake.push(response(200, INDEX_FIXTURE, "https://www.patch.moe/zh"));
        fake.push(response(200, INDEX_FIXTURE, "https://www.patch.moe/en"));
        let mut ctx = context(&fake);
        let outcome = CompatAdapter
            .collect(&rule(), &SourceCache::default(), &mut ctx)
            .unwrap();
        // 3 `new` rows + 3 `regular` rows; both header rows carry no `.zip`
        // link and are dropped.
        assert_eq!(outcome.entries.len(), 6);
        assert!(!outcome.entries.iter().any(|entry| entry.title == "Title"));
        assert!(outcome
            .entries
            .iter()
            .any(|entry| entry.url == "https://download.patch.moe/Fusty_Dungeon_patch.zip"));
        let kudo = outcome
            .entries
            .iter()
            .find(|entry| entry.url == "https://download.patch.moe/Kudos_Kuppa_patch.zip")
            .expect("Kudo's Kuppa from the regular table");
        assert_eq!(kudo.title, "Kudo’s Kuppa");
        // Both language passes returned identical URLs, so zh-CN is chosen and
        // the matching en title yields no alias.
        assert_eq!(kudo.language, "zh-CN");
        assert!(kudo.aliases.is_empty());
    }

    #[test]
    fn missing_or_invalid_language_forms_error() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(200, "<html><body>no forms</body></html>", ""));
        let mut ctx = context(&fake);
        let adapter = CompatAdapter;
        assert!(adapter
            .collect(&rule(), &SourceCache::default(), &mut ctx)
            .unwrap_err()
            .to_string()
            .contains("patchmoe_language_forms_missing"));

        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(
            200,
            r#"<form name="form_en" method="get" action="/en"><input name="lang" value="en"></form>"#,
            "",
        ));
        let mut ctx = context(&fake);
        assert!(adapter
            .collect(&rule(), &SourceCache::default(), &mut ctx)
            .unwrap_err()
            .to_string()
            .contains("patchmoe_language_form_invalid"));
    }

    #[test]
    fn conditional_304_reuses_previous_entries() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(FetchResponse {
            status: 304,
            ..FetchResponse::default()
        });
        let previous = SourceCache {
            entries: vec![SourceEntry {
                title: "Game".to_string(),
                aliases: Vec::new(),
                url: "https://download.patch.moe/patch.zip".to_string(),
                source_page: "https://www.patch.moe/zh".to_string(),
                language: "zh-CN".to_string(),
                date: None,
                appid: None,
            }],
            etag: Some("\"etag\"".to_string()),
            modified: Some("date".to_string()),
            cached_at: Some(1),
            checked_at: Some(1),
        };
        let mut ctx = context(&fake);
        let adapter = CompatAdapter;
        let outcome = adapter.collect(&rule(), &previous, &mut ctx).unwrap();
        assert!(outcome.not_modified);
        assert_eq!(outcome.entries, previous.entries);
        assert_eq!(outcome.etag.as_deref(), Some("\"etag\""));
        let calls = fake.calls();
        assert!(calls[0]
            .headers
            .iter()
            .any(|(name, value)| name == "If-None-Match" && value == "\"etag\""));
    }

    #[test]
    fn coming_soon_dates_are_mapped() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(
            200,
            r#"<form name="form_en" method="post" action="/en"><input name="lang" value="en"></form>"#,
            "",
        ));
        fake.push(response(
            200,
            r#"<table><tr><td class="title">Game</td><td class="release">即将推出</td><td class="patch"><a href="https://download.patch.moe/patch.zip">dl</a></td></tr></table>"#,
            "https://www.patch.moe/en",
        ));
        let mut ctx = context(&fake);
        let outcome = CompatAdapter
            .collect(&rule(), &SourceCache::default(), &mut ctx)
            .unwrap();
        assert_eq!(outcome.entries[0].date.as_deref(), Some("COMING SOON"));
    }

    #[test]
    fn language_forms_are_posted_with_cookie_jar() {
        let fake = Arc::new(FakeFetcher::new());
        // The index 302s to the age gate, which sets the session cookie; the
        // localized fetches must reuse it and use POST.
        let mut redirect = response(302, "", "https://www.patch.moe/gate");
        redirect.location = Some("https://www.patch.moe/gate".to_string());
        redirect.headers.push((
            "Set-Cookie".to_string(),
            "PHPSESSID=abc; Path=/".to_string(),
        ));
        fake.push(redirect);
        fake.push(response(200, INITIAL, "https://www.patch.moe/gate"));
        fake.push(response(200, &table("Game"), "https://www.patch.moe/zh"));
        fake.push(response(
            200,
            &table("Game English"),
            "https://www.patch.moe/en",
        ));

        let mut ctx = context(&fake);
        let outcome = CompatAdapter
            .collect(&rule(), &SourceCache::default(), &mut ctx)
            .unwrap();
        assert_eq!(outcome.entries.len(), 1);

        let calls = fake.calls();
        assert!(calls[1]
            .headers
            .iter()
            .any(|(name, value)| name == "Cookie" && value == "PHPSESSID=abc"));
        let zh = calls
            .iter()
            .find(|call| call.body.as_deref() == Some(&b"lang=zh-CN"[..]))
            .expect("zh-CN form POST");
        assert_eq!(zh.method, "POST");
        assert_eq!(
            zh.content_type.as_deref(),
            Some("application/x-www-form-urlencoded")
        );
        assert!(zh
            .headers
            .iter()
            .any(|(name, value)| name == "Cookie" && value == "PHPSESSID=abc"));
    }
}

mod table {
    use super::*;
    use patchhub_core::sources::http::{FakeFetcher, FetchResponse, RequestContext};
    use patchhub_core::sources::model::{Columns, RootLayout, SourceEntry, SourceRole};
    use std::sync::Arc;
    use std::time::Duration;

    fn rule(columns: Option<Columns>, table_id: Option<&str>) -> SourceRule {
        SourceRule {
            script: None,
            id: "example".to_string(),
            name: "Example".to_string(),
            adapter: "table_html".to_string(),
            role: SourceRole::Publisher,
            discovery_only: false,
            priority: 100,
            enabled: true,
            pages: vec!["https://example.com/list".to_string()],
            page_hosts: vec!["example.com".to_string()],
            download_hosts: vec!["download.example.com".to_string()],
            root_layout: Some(RootLayout::GameRoot),
            columns,
            table_id: table_id.map(str::to_string),
            language: None,
            url: None,
        }
    }

    fn columns() -> Columns {
        Columns {
            title: 0,
            download: 1,
            date: Some(2),
            language: Some(3),
            appid: None,
        }
    }

    fn response(status: u16, html: &str, final_url: &str, etag: Option<&str>) -> FetchResponse {
        FetchResponse {
            status,
            body: html.as_bytes().to_vec(),
            final_url: final_url.to_string(),
            etag: etag.map(str::to_string),
            ..FetchResponse::default()
        }
    }

    fn context(fake: &Arc<FakeFetcher>) -> RequestContext {
        RequestContext::with_settings(Box::new(Arc::clone(fake)), Duration::ZERO)
    }

    const ROW: &str = r#"<tr><td>Game</td><td><a href="https://download.example.com/g.zip">dl</a></td><td>2024/1/2</td><td>zh-CN</td></tr>"#;

    #[test]
    fn parses_table_rows_and_etag() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(
            200,
            &format!("<table>{ROW}</table>"),
            "https://example.com/list",
            Some("etag-1"),
        ));
        let mut ctx = context(&fake);
        let adapter = builtin_adapter("table_html").unwrap();
        let outcome = adapter
            .collect(
                &rule(Some(columns()), None),
                &SourceCache::default(),
                &mut ctx,
            )
            .unwrap();
        assert_eq!(outcome.entries.len(), 1);
        assert_eq!(outcome.entries[0].title, "Game");
        assert_eq!(outcome.entries[0].language, "zh-CN");
        assert_eq!(outcome.entries[0].date.as_deref(), Some("2024/1/2"));
        assert_eq!(outcome.etag.as_deref(), Some("etag-1"));
        assert!(!outcome.not_modified);
    }

    #[test]
    fn skips_malformed_rows_and_non_zip_links() {
        let fake = Arc::new(FakeFetcher::new());
        let html = format!(
            "<table>\
             <tr><td>No link</td><td></td><td></td><td></td></tr>\
             <tr><td>Rar</td><td><a href=\"https://download.example.com/g.rar\">dl</a></td><td></td><td></td></tr>\
             {ROW}\
             </table>"
        );
        fake.push(response(200, &html, "https://example.com/list", None));
        let mut ctx = context(&fake);
        let outcome = CompatAdapter
            .collect(
                &rule(Some(columns()), None),
                &SourceCache::default(),
                &mut ctx,
            )
            .unwrap();
        assert_eq!(outcome.entries.len(), 1);
        assert_eq!(outcome.entries[0].title, "Game");
    }

    #[test]
    fn rejects_missing_columns_and_invalid_appid() {
        let fake = Arc::new(FakeFetcher::new());
        let mut ctx = context(&fake);
        assert!(CompatAdapter
            .collect(&rule(None, None), &SourceCache::default(), &mut ctx)
            .unwrap_err()
            .to_string()
            .contains("invalid_source_columns"));

        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(
            200,
            "<table><tr><td>Game</td><td><a href=\"https://download.example.com/g.zip\">dl</a></td><td>2024/1/2</td><td>en</td><td>abc</td></tr></table>",
            "https://example.com/list",
            None,
        ));
        let mut ctx = context(&fake);
        let appid_columns = Columns {
            title: 0,
            download: 1,
            date: Some(2),
            language: Some(3),
            appid: Some(4),
        };
        assert!(CompatAdapter
            .collect(
                &rule(Some(appid_columns), None),
                &SourceCache::default(),
                &mut ctx
            )
            .unwrap_err()
            .to_string()
            .contains("invalid_source_appid"));
    }

    #[test]
    fn empty_table_is_rejected() {
        let fake = Arc::new(FakeFetcher::new());
        // table_id selects a table that does not exist → no rows.
        fake.push(response(
            200,
            "<table></table>",
            "https://example.com/list",
            None,
        ));
        let mut ctx = context(&fake);
        assert!(CompatAdapter
            .collect(
                &rule(Some(columns()), Some("missing")),
                &SourceCache::default(),
                &mut ctx
            )
            .unwrap_err()
            .to_string()
            .contains("empty_or_oversized_source"));
    }

    #[test]
    fn conditional_304_reuses_or_requires_previous_cache() {
        let entry = SourceEntry {
            title: "Game".to_string(),
            aliases: Vec::new(),
            url: "https://download.example.com/g.zip".to_string(),
            source_page: "https://example.com/list".to_string(),
            language: "en".to_string(),
            date: None,
            appid: None,
        };
        let previous = SourceCache {
            entries: vec![entry],
            etag: Some("etag-1".to_string()),
            modified: None,
            cached_at: Some(1),
            checked_at: Some(1),
        };

        let fake = Arc::new(FakeFetcher::new());
        fake.push(FetchResponse {
            status: 304,
            ..FetchResponse::default()
        });
        let mut ctx = context(&fake);
        let outcome = CompatAdapter
            .collect(&rule(Some(columns()), None), &previous, &mut ctx)
            .unwrap();
        assert!(outcome.not_modified);
        assert_eq!(outcome.entries, previous.entries);

        let fake = Arc::new(FakeFetcher::new());
        fake.push(FetchResponse {
            status: 304,
            ..FetchResponse::default()
        });
        let mut ctx = context(&fake);
        assert!(CompatAdapter
            .collect(
                &rule(Some(columns()), None),
                &SourceCache::default(),
                &mut ctx
            )
            .unwrap_err()
            .to_string()
            .contains("source_cache_missing"));
    }
}

mod hgamer {
    use super::*;
    use patchhub_core::sources::http::{FakeFetcher, FetchResponse, RequestContext};
    use patchhub_core::sources::model::{ArtifactFormat, MatchBasis, SourceRole};
    use std::sync::Arc;
    use std::time::Duration;

    fn rule() -> SourceRule {
        SourceRule {
            script: None,
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

    /// Current site layout: the header `<h1>` and the result-count paragraph are
    /// siblings in different sections, so the old `h1.parent()` scan sees an
    /// empty summary.
    const SEARCH_SIBLING_SUMMARY: &str = r#"<html><body>
<header class="search-header"><h1>搜尋結果</h1></header>
<section class="results"><p>「Game」找到 1 筆結果</p></section>
<section><h2>補丁</h2><ul><li><a href="/zh-CN/patches/patch-1">Game patch</a></li></ul></section>
</body></html>"#;

    #[test]
    fn result_summary_in_a_sibling_section_is_parsed() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(
            SEARCH_SIBLING_SUMMARY,
            "https://hgamer.tw/zh-CN/search?q=Game",
        ));
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
        assert_eq!(outcome.candidates.len(), 1);
        assert!(
            !outcome
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "discovery_search_structure_changed"),
            "sibling-section summary must not be treated as a structure change"
        );
        assert_eq!(fake.call_count(), 2);
    }

    /// No `<p>` carries the summary at all: the document-text fallback applies.
    const SEARCH_ROOT_TEXT_SUMMARY: &str = r#"<html><body>
<header><h1>搜尋結果</h1></header>
<section><span>「Game」找到 1 筆結果</span></section>
<section><h2>補丁</h2><ul><li><a href="/zh-CN/patches/patch-1">Game patch</a></li></ul></section>
</body></html>"#;

    #[test]
    fn result_summary_falls_back_to_the_document_text() {
        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(
            SEARCH_ROOT_TEXT_SUMMARY,
            "https://hgamer.tw/zh-CN/search?q=Game",
        ));
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
        assert_eq!(outcome.candidates.len(), 1);
        assert!(!outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "discovery_search_structure_changed"));
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
        let outcome = CompatAdapter
            .discover(
                &rule(),
                &query(42, "Game"),
                &mut ctx,
                &DiscoverPrevious::default(),
            )
            .unwrap_err();
        assert!(outcome
            .to_string()
            .contains("discovery_search_structure_changed"));
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
        let outcome = CompatAdapter
            .discover(
                &rule(),
                &query(42, "Game"),
                &mut ctx,
                &DiscoverPrevious::default(),
            )
            .unwrap_err();
        // Every game detail page is fetched; no request-count cap stops the
        // traversal.
        assert_eq!(fake.call_count(), 8);
        assert!(outcome.to_string().contains("discovery_structure_changed"));
    }
}

#[test]
fn table_accepts_column_beyond_twenty_and_literal_unicode_table_id() {
    let raw: RawSourceRule=serde_json::from_value(serde_json::json!({"id":"wide-table","name":"Table","adapter":"table_html","enabled":true,"pages":["https://example.com/list"],"page_hosts":["example.com"],"download_hosts":["download.example.com"],"root_layout":"game_root","table_id":"补丁:列表","columns":{"title":21,"download":22}})).unwrap();
    let rule = sources::validate::validate_rule(&raw).unwrap();
    let fake = Arc::new(sources::http::FakeFetcher::new());
    fake.push(sources::http::FetchResponse { status:200,final_url:rule.pages[0].clone(),body:format!("<table id='补丁:列表'><tr>{}<td>Game</td><td><a href='https://download.example.com/game.zip'>Patch</a></td></tr></table>","<td></td>".repeat(21)).into_bytes(),..Default::default() });
    let mut ctx =
        RequestContext::with_settings(Box::new(Arc::clone(&fake)), std::time::Duration::ZERO);
    let result = source_adapter(&rule, None)
        .unwrap()
        .collect_index(&rule, None, &mut ctx, &Arc::new(AtomicBool::new(false)))
        .unwrap();
    assert!(result.complete);
    assert_eq!(result.entries.len(), 1);
    assert_eq!(result.entries[0].title, "Game");
}
