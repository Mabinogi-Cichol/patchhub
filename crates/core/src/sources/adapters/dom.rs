//! Small `scraper`-backed DOM helpers shared by the discovery adapters.
//!
//! These mirror the subset of `discovery.py`'s `Node` API the adapters use
//! (`walk`/`has`/`text`/`links`/`parent`/`children`/ancestors). Everything is
//! collected into `Vec`s to keep lifetimes simple.

use super::super::model::Mirror;
use super::super::{identity, validate};
use super::{join_relative, selector};
use scraper::{ElementRef, Html};

pub(crate) fn document(html: &str) -> Html {
    Html::parse_document(html)
}

pub(crate) fn root(document: &Html) -> ElementRef<'_> {
    document.root_element()
}

pub(crate) fn tag<'a>(element: &ElementRef<'a>) -> &'a str {
    element.value().name()
}

pub(crate) fn attr<'a>(element: &ElementRef<'a>, name: &str) -> Option<&'a str> {
    element.value().attr(name)
}

pub(crate) fn has_class(element: &ElementRef, class: &str) -> bool {
    element
        .value()
        .attr("class")
        .unwrap_or("")
        .split_whitespace()
        .any(|token| token == class)
}

pub(crate) fn text(element: &ElementRef) -> String {
    collapse(element.text().collect::<String>())
}

pub(crate) fn collapse(value: String) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn children<'a>(element: &ElementRef<'a>) -> Vec<ElementRef<'a>> {
    element.children().filter_map(ElementRef::wrap).collect()
}

pub(crate) fn parent<'a>(element: &ElementRef<'a>) -> Option<ElementRef<'a>> {
    element.parent().and_then(ElementRef::wrap)
}

pub(crate) fn ancestors<'a>(element: &ElementRef<'a>) -> Vec<ElementRef<'a>> {
    let mut out = Vec::new();
    let mut current = parent(element);
    while let Some(node) = current {
        current = parent(&node);
        out.push(node);
    }
    out
}

pub(crate) fn walk<'a>(element: &ElementRef<'a>) -> Vec<ElementRef<'a>> {
    let mut out = Vec::new();
    collect(element, &mut out);
    out
}

fn collect<'a>(element: &ElementRef<'a>, out: &mut Vec<ElementRef<'a>>) {
    out.push(*element);
    for child in element.children().filter_map(ElementRef::wrap) {
        collect(&child, out);
    }
}

pub(crate) fn first<'a>(
    element: &ElementRef<'a>,
    predicate: impl Fn(&ElementRef<'a>) -> bool,
) -> Option<ElementRef<'a>> {
    walk(element)
        .into_iter()
        .find(|candidate| predicate(candidate))
}

/// Resolved, validated `<a href>` links with collapsed labels, deduped by URL.
pub(crate) fn links(
    element: &ElementRef,
    page: &str,
    predicate: impl Fn(&str, &str) -> bool,
) -> Vec<(String, String)> {
    let mut result: Vec<(String, String)> = Vec::new();
    for anchor in element.select(&selector("a[href]")) {
        let label = text(&anchor);
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let Some(url) = join_relative(page, href) else {
            continue;
        };
        if !predicate(&label, &url) {
            continue;
        }
        if validate::navigation_url(&url).is_err() {
            continue;
        }
        if result.iter().any(|(_, existing)| existing == &url) {
            continue;
        }
        result.push((label, url));
    }
    result
}

/// Raw `href`/`src` values (a/iframe) for `identity::steam_ids`.
pub(crate) fn link_urls(element: &ElementRef) -> Vec<String> {
    let mut urls = Vec::new();
    for node in walk(element) {
        let url = match tag(&node) {
            "a" => attr(&node, "href"),
            "iframe" => attr(&node, "src"),
            _ => None,
        };
        if let Some(url) = url {
            urls.push(url.to_string());
        }
    }
    urls
}

/// Collapsed text of every element in the subtree (for `identity::aliases`).
pub(crate) fn element_texts(element: &ElementRef) -> Vec<String> {
    walk(element).iter().map(text).collect()
}

/// `_mirrors`: download-labelled links that are not install instructions.
pub(crate) fn mirrors(element: &ElementRef, page: &str) -> Vec<Mirror> {
    let mut result: Vec<Mirror> = Vec::new();
    for anchor in element.select(&selector("a[href]")) {
        let label = text(&anchor);
        if !is_mirror_label(&label) {
            continue;
        }
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let Some(url) = join_relative(page, href) else {
            continue;
        };
        if validate::navigation_url(&url).is_err() {
            continue;
        }
        if result.iter().any(|mirror| mirror.url == url) {
            continue;
        }
        let format = identity::format(&url);
        result.push(Mirror {
            url,
            label: Some(label),
            format,
        });
    }
    result
}

fn is_mirror_label(label: &str) -> bool {
    let positive = mirror_word_number(label)
        || label.contains("下载")
        || label.contains("下載")
        || label.contains("载点")
        || label.contains("載点")
        || label.contains("载點")
        || label.contains("載點")
        || contains_ci(label, "download");
    if !positive {
        return false;
    }
    !(label.contains("安装说明")
        || label.contains("安裝說明")
        || contains_ci(label, "instructions"))
}

/// `\bMirror\s*\d`
fn mirror_word_number(label: &str) -> bool {
    let lower = label.to_ascii_lowercase();
    let mut search = 0;
    while let Some(position) = lower[search..].find("mirror") {
        let absolute = search + position;
        let before_ok = absolute == 0
            || !label[..absolute]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if before_ok {
            let after = label[absolute + "mirror".len()..].trim_start();
            if after.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                return true;
            }
        }
        search = absolute + "mirror".len();
    }
    false
}

pub(crate) fn contains_ci(haystack: &str, needle: &str) -> bool {
    if needle.is_ascii() {
        haystack
            .to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase())
    } else {
        haystack.contains(needle)
    }
}
