//! Generic HTML-table ZIP-index collector (port of `sources.py::_collect_table`).

use super::super::http::RequestContext;
use super::super::model::{SourceCache, SourceRule};
use super::super::{invalid, validate, Result};
use super::{parse_rows, require_ok_text, response_base, row_entry, Adapter, CollectOutcome};

pub struct TableAdapter;

impl Adapter for TableAdapter {
    fn collect(
        &self,
        rule: &SourceRule,
        previous: &SourceCache,
        ctx: &mut RequestContext,
    ) -> Result<CollectOutcome> {
        let columns = rule
            .columns
            .as_ref()
            .ok_or_else(|| invalid("invalid_source_columns"))?;
        // Conditional requests are only safe for a single-page table.
        let single_page = rule.pages.len() == 1;
        let language = rule.language.clone().unwrap_or_else(|| "en".to_string());
        let mut entries = Vec::new();
        let mut etag = None;
        let mut modified = None;

        for page in &rule.pages {
            let (request_etag, request_modified) = if single_page {
                (previous.etag.as_deref(), previous.modified.as_deref())
            } else {
                (None, None)
            };
            let response =
                ctx.fetch_conditional(page, &rule.page_hosts, request_etag, request_modified)?;
            if response.status == 304 {
                if previous.entries.is_empty() {
                    return Err(invalid("source_cache_missing"));
                }
                return Ok(CollectOutcome {
                    entries: previous.entries.clone(),
                    etag: previous.etag.clone(),
                    modified: previous.modified.clone(),
                    not_modified: true,
                });
            }
            let html = require_ok_text(&response)?;
            let base = response_base(&response, page);
            for row in parse_rows(&html, rule.table_id.as_deref()) {
                if let Some(entry) = row_entry(&row, columns, &base, &language)? {
                    entries.push(entry);
                }
            }
            if single_page {
                etag = response.etag.clone();
                modified = response.last_modified.clone();
            }
        }

        let entries = validate::validate_entries(rule, &entries)?;
        Ok(CollectOutcome {
            entries,
            etag,
            modified,
            not_modified: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{builtin_adapter, Adapter};
    use super::*;
    use crate::sources::http::{FakeFetcher, FetchResponse, RequestContext};
    use crate::sources::model::{Columns, RootLayout, SourceEntry, SourceRole};
    use std::sync::Arc;
    use std::time::Duration;

    fn rule(columns: Option<Columns>, table_id: Option<&str>) -> SourceRule {
        SourceRule {
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
        let outcome = TableAdapter
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
        assert_eq!(
            TableAdapter
                .collect(&rule(None, None), &SourceCache::default(), &mut ctx)
                .unwrap_err()
                .to_string(),
            "invalid_source_columns"
        );

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
        assert_eq!(
            TableAdapter
                .collect(
                    &rule(Some(appid_columns), None),
                    &SourceCache::default(),
                    &mut ctx
                )
                .unwrap_err()
                .to_string(),
            "invalid_source_appid"
        );
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
        assert_eq!(
            TableAdapter
                .collect(
                    &rule(Some(columns()), Some("missing")),
                    &SourceCache::default(),
                    &mut ctx
                )
                .unwrap_err()
                .to_string(),
            "empty_or_oversized_source"
        );
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
        let outcome = TableAdapter
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
        assert_eq!(
            TableAdapter
                .collect(
                    &rule(Some(columns()), None),
                    &SourceCache::default(),
                    &mut ctx
                )
                .unwrap_err()
                .to_string(),
            "source_cache_missing"
        );
    }
}
