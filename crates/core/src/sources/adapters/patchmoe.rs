//! `patchmoe` ZIP-index collector (port of `sources.py::_collect_patchmoe`).
//!
//! The per-language form submission is issued as an
//! `application/x-www-form-urlencoded` `POST` through the shared
//! [`RequestContext`], which also carries the age-gate cookie jar across the
//! initial redirect and the localized fetches.

use super::super::http::RequestContext;
use super::super::model::{Columns, SourceCache, SourceIndex, SourceRule};
use super::super::{invalid, validate, Result};
use super::{
    index_entry_from_source, join_relative, now_unix, parse_rows, require_ok_text, response_base,
    row_entry, selector, source_cache_from_index, Adapter, CollectOutcome, IndexOutcome,
};
use scraper::Html;
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::AtomicBool;

pub struct PatchmoeAdapter;

impl Adapter for PatchmoeAdapter {
    fn collect(
        &self,
        rule: &SourceRule,
        previous: &SourceCache,
        ctx: &mut RequestContext,
    ) -> Result<CollectOutcome> {
        let page = rule
            .pages
            .first()
            .ok_or_else(|| invalid("invalid_source_pages"))?;
        let response = ctx.fetch_conditional(
            page,
            &rule.page_hosts,
            previous.etag.as_deref(),
            previous.modified.as_deref(),
        )?;
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
        let etag = response.etag.clone();
        let modified = response.last_modified.clone();

        let forms: Vec<Form> = parse_forms(&html)
            .into_iter()
            .filter(|form| matches!(form.name.as_str(), "form_en" | "form_zh_cn" | "form_zh_tw"))
            .collect();
        if forms.is_empty() {
            return Err(invalid("patchmoe_language_forms_missing"));
        }

        let mut entries = Vec::new();
        for form in &forms {
            if form.method != "post" || !form.inputs.iter().any(|(name, _)| name == "lang") {
                return Err(invalid("patchmoe_language_form_invalid"));
            }
            let mut lang = form
                .inputs
                .iter()
                .find(|(name, _)| name == "lang")
                .map(|(_, value)| value.clone())
                .unwrap_or_default();
            if lang.is_empty() {
                if let Some(found) = extract_lang_assignment(&html, &form.name) {
                    lang = found;
                }
            }
            if lang.is_empty() {
                continue;
            }

            let action: &str = if form.action.is_empty() {
                "/"
            } else {
                form.action.as_str()
            };
            let action =
                join_relative(&base, action).ok_or_else(|| invalid("invalid_source_url"))?;
            let action = validate::allowed_url(&action, &rule.page_hosts, false)?;

            // patch.moe ignores GET `?lang=`; submit the hidden language form as
            // a form-encoded POST, reusing the context cookie jar for the gate.
            let localized = ctx.fetch_form(&action, &rule.page_hosts, &form.inputs)?;
            let localized_html = require_ok_text(&localized)?;
            let localized_base = response_base(&localized, &action);
            let language = match form.name.as_str() {
                "form_en" => "en",
                "form_zh_cn" => "zh-CN",
                _ => "zh-TW",
            };

            for row in parse_rows(&localized_html, None) {
                let classes = class_index(&row);
                let (Some(&title), Some(&release), Some(&patch)) = (
                    classes.get("title"),
                    classes.get("release"),
                    classes.get("patch"),
                ) else {
                    continue;
                };
                let columns = Columns {
                    title: title as u8,
                    download: patch as u8,
                    date: Some(release as u8),
                    language: None,
                    appid: None,
                };
                if let Some(mut entry) = row_entry(&row, &columns, &localized_base, language)? {
                    if matches!(
                        entry.date.as_deref(),
                        Some("待定") | Some("即将推出") | Some("即將推出")
                    ) {
                        entry.date = Some("COMING SOON".to_string());
                    }
                    entries.push(entry);
                }
            }
        }
        if entries.is_empty() {
            return Err(invalid("patchmoe_rows_missing"));
        }

        // Only an identical archive URL proves the localized titles describe the
        // same release; never infer pairs from row position or filename.
        let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            match groups.iter().position(|(url, _)| url == &entry.url) {
                Some(position) => groups[position].1.push(index),
                None => groups.push((entry.url.clone(), vec![index])),
            }
        }
        let mut merged = Vec::new();
        for (_, indices) in groups {
            let chosen = indices
                .iter()
                .map(|&index| &entries[index])
                .find(|entry| entry.language == "zh-CN")
                .unwrap_or(&entries[indices[0]]);
            let mut chosen = chosen.clone();
            let aliases: BTreeSet<String> = indices
                .iter()
                .map(|&index| &entries[index])
                .filter(|entry| entry.title != chosen.title)
                .map(|entry| entry.title.clone())
                .collect();
            chosen.aliases = aliases.into_iter().collect();
            merged.push(chosen);
        }

        let entries = validate::validate_entries(rule, &merged)?;
        Ok(CollectOutcome {
            entries,
            etag,
            modified,
            not_modified: false,
        })
    }

    fn collect_index(
        &self,
        rule: &SourceRule,
        previous: Option<&SourceIndex>,
        ctx: &mut RequestContext,
        cancel: &AtomicBool,
    ) -> Result<IndexOutcome> {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(super::cancelled_index_outcome(rule));
        }
        let cache = source_cache_from_index(previous);
        let outcome = self.collect(rule, &cache, ctx)?;
        let checked_at = now_unix();
        if outcome.not_modified {
            return Ok(IndexOutcome {
                entries: previous
                    .map(|index| index.entries.clone())
                    .unwrap_or_default(),
                pending: previous
                    .map(|index| index.pending.clone())
                    .unwrap_or_default(),
                complete: previous.is_some_and(|index| index.complete),
                etag: outcome.etag,
                modified: outcome.modified,
                not_modified: true,
                checked_at,
                diagnostics: Vec::new(),
            });
        }
        // patch.moe returns the full list in one pass: two localized form
        // submissions, no pagination queue.
        Ok(IndexOutcome {
            entries: outcome
                .entries
                .iter()
                .map(index_entry_from_source)
                .collect(),
            pending: Vec::new(),
            complete: true,
            etag: outcome.etag,
            modified: outcome.modified,
            not_modified: false,
            checked_at,
            diagnostics: Vec::new(),
        })
    }
}

struct Form {
    name: String,
    method: String,
    action: String,
    inputs: Vec<(String, String)>,
}

fn parse_forms(html: &str) -> Vec<Form> {
    let document = Html::parse_document(html);
    let mut forms = Vec::new();
    for form in document.select(&selector("form")) {
        let mut inputs = Vec::new();
        for input in form.select(&selector("input[name]")) {
            let name = input.attr("name").unwrap_or("").to_string();
            let input_type = input.attr("type").unwrap_or("hidden").to_ascii_lowercase();
            if !name.is_empty() && input_type == "hidden" {
                inputs.push((name, input.attr("value").unwrap_or("").to_string()));
            }
        }
        forms.push(Form {
            name: form.attr("name").unwrap_or("").to_string(),
            method: form.attr("method").unwrap_or("get").to_ascii_lowercase(),
            action: form.attr("action").unwrap_or("").to_string(),
            inputs,
        });
    }
    forms
}

fn class_index(row: &[super::Cell]) -> HashMap<String, usize> {
    let mut classes = HashMap::new();
    for (index, cell) in row.iter().enumerate() {
        if !cell.class.is_empty() {
            classes.insert(cell.class.clone(), index);
        }
    }
    classes
}

/// Port of the `(?:document\.)?<form>\.lang\.value\s*=\s*(['"])([^'"<>]{1,64})\1`
/// fallback assignment scan.
fn extract_lang_assignment(html: &str, form_name: &str) -> Option<String> {
    let marker = format!("{form_name}.lang.value");
    let mut search = 0;
    while let Some(position) = html[search..].find(marker.as_str()) {
        let absolute = search + position;
        let after = &html[absolute + marker.len()..];
        let after = after.trim_start();
        if let Some(after) = after.strip_prefix('=') {
            let after = after.trim_start();
            if let Some(quote) = after.chars().next() {
                if quote == '"' || quote == '\'' {
                    let rest = &after[quote.len_utf8()..];
                    if let Some(end) = rest.find(quote) {
                        let value = &rest[..end];
                        let length = value.chars().count();
                        if (1..=64).contains(&length) && !value.contains(['"', '\'', '<', '>']) {
                            return Some(value.to_string());
                        }
                    }
                }
            }
        }
        search = absolute + marker.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::{builtin_adapter, Adapter};
    use super::*;
    use crate::sources::http::{FakeFetcher, FetchResponse, RequestContext};
    use crate::sources::model::SourceEntry;
    use std::sync::Arc;
    use std::time::Duration;

    fn rule() -> SourceRule {
        SourceRule {
            script: None,
            id: "patchmoe".to_string(),
            name: "patch.moe".to_string(),
            adapter: "patchmoe".to_string(),
            role: crate::sources::model::SourceRole::Publisher,
            discovery_only: false,
            priority: 100,
            enabled: true,
            pages: vec!["https://www.patch.moe/?flag=1".to_string()],
            page_hosts: vec!["www.patch.moe".to_string()],
            download_hosts: vec!["download.patch.moe".to_string()],
            root_layout: Some(crate::sources::model::RootLayout::GameRoot),
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
    const INDEX_FIXTURE: &str = include_str!("../../../tests/fixtures/sources/patchmoe-index.html");

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
        let outcome = PatchmoeAdapter
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
        let adapter = PatchmoeAdapter;
        assert_eq!(
            adapter
                .collect(&rule(), &SourceCache::default(), &mut ctx)
                .unwrap_err()
                .to_string(),
            "patchmoe_language_forms_missing"
        );

        let fake = Arc::new(FakeFetcher::new());
        fake.push(response(
            200,
            r#"<form name="form_en" method="get" action="/en"><input name="lang" value="en"></form>"#,
            "",
        ));
        let mut ctx = context(&fake);
        assert_eq!(
            adapter
                .collect(&rule(), &SourceCache::default(), &mut ctx)
                .unwrap_err()
                .to_string(),
            "patchmoe_language_form_invalid"
        );
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
        let adapter = PatchmoeAdapter;
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
        let outcome = PatchmoeAdapter
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
        let outcome = PatchmoeAdapter
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
