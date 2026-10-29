//! Pure (no IO) validation and URL / host / name primitives.
//!
//! Faithful ports of `sources.py::_validate_rule`, `_validate_entries`,
//! `_validate_candidates`, `_validate_overrides`, `_host`, `_allowed_url`,
//! `_normal`, `_split_names` plus a std `urlsplit` replacement (design §2.6,
//! §8). Every failure uses the frozen `invalid_*` / `*_source*` error codes.

use super::invalid;
use super::model::*;
use super::DISCOVERY_ADAPTERS;
use std::collections::HashSet;
use std::io;

// ---- URL / host ----------------------------------------------------------

/// A minimal std replacement for Python's `urllib.parse.urlsplit` covering the
/// fields the source engine reads. `urlunsplit` in [`super::canonical`] consumes
/// these components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitUrl {
    pub scheme: String,
    pub netloc: String,
    pub path: String,
    pub query: String,
    pub fragment: String,
}

impl SplitUrl {
    /// Python `SplitResult._hostinfo`: strip `userinfo@`, then split the host
    /// from the port (honouring `[IPv6]` brackets).
    fn hostinfo(&self) -> (&str, Option<&str>) {
        let (_, hostinfo) = rpartition(&self.netloc, '@');
        if hostinfo.contains('[') {
            let (_, bracketed) = partition(hostinfo, '[');
            let (hostname, after) = partition(bracketed, ']');
            let (_, port) = partition(after, ':');
            (hostname, non_empty(port))
        } else {
            let (hostname, port) = partition(hostinfo, ':');
            (hostname, non_empty(port))
        }
    }

    /// Python `SplitResult.hostname` (lowercased), `None` when absent.
    pub fn hostname(&self) -> Option<String> {
        let (hostname, _) = self.hostinfo();
        if hostname.is_empty() {
            None
        } else {
            Some(hostname.to_lowercase())
        }
    }

    /// Python `SplitResult.username`.
    pub fn username(&self) -> Option<&str> {
        if self.netloc.contains('@') {
            let (userinfo, _) = rpartition(&self.netloc, '@');
            let (user, _) = partition(userinfo, ':');
            Some(user)
        } else {
            None
        }
    }

    /// Python `SplitResult.password`.
    pub fn password(&self) -> Option<&str> {
        if self.netloc.contains('@') {
            let (userinfo, _) = rpartition(&self.netloc, '@');
            if let Some((_, password)) = userinfo.split_once(':') {
                return Some(password);
            }
        }
        None
    }

    /// Python `SplitResult.port`; `Err(())` mirrors its `ValueError`.
    #[allow(clippy::result_unit_err)] // unit error intentionally mirrors CPython's ValueError
    pub fn port(&self) -> std::result::Result<Option<u32>, ()> {
        let (_, port) = self.hostinfo();
        let Some(port) = port else {
            return Ok(None);
        };
        let parsed: u32 = port.parse().map_err(|_| ())?;
        if parsed > 65535 {
            return Err(());
        }
        Ok(Some(parsed))
    }
}

fn non_empty(value: &str) -> Option<&str> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Mimics `str.partition`: `(head, tail)` with `tail == ""` when absent.
fn partition(value: &str, delimiter: char) -> (&str, &str) {
    match value.split_once(delimiter) {
        Some((head, tail)) => (head, tail),
        None => (value, ""),
    }
}

/// Mimics `str.rpartition`: `(head, tail)` with `head == ""` when absent.
fn rpartition(value: &str, delimiter: char) -> (&str, &str) {
    match value.rsplit_once(delimiter) {
        Some((head, tail)) => (head, tail),
        None => ("", value),
    }
}

/// Port of `urllib.parse.urlsplit(url)` for the default `scheme=""`,
/// `allow_fragments=True` call shape (CPython 3.10: strips only tab/CR/LF, and
/// accepts any scheme made of `scheme_chars`).
///
/// The exotic `_checknetloc` NFKC guard and the malformed-`[`/`]` `ValueError`
/// are not replicated; such inputs are still rejected downstream by
/// [`host`] / [`allowed_url`] and the Unicode dependency is deferred to P2.
pub fn split_url(url: &str) -> SplitUrl {
    // Python removes tabs/CR/LF (but not spaces) before parsing.
    let cleaned: String = url
        .chars()
        .filter(|c| !matches!(*c, '\t' | '\r' | '\n'))
        .collect();
    let mut rest: &str = &cleaned;

    let mut scheme = String::new();
    if let Some(index) = rest.find(':') {
        if index > 0 {
            let candidate = &rest[..index];
            if candidate
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
            {
                scheme = candidate.to_ascii_lowercase();
                rest = &rest[index + 1..];
            }
        }
    }

    let mut netloc: &str = "";
    if let Some(after) = rest.strip_prefix("//") {
        let end = after
            .find(|c| ['/', '?', '#'].contains(&c))
            .unwrap_or(after.len());
        netloc = &after[..end];
        rest = &after[end..];
    }

    let mut fragment: &str = "";
    if let Some(index) = rest.find('#') {
        fragment = &rest[index + 1..];
        rest = &rest[..index];
    }

    let mut query: &str = "";
    if let Some(index) = rest.find('?') {
        query = &rest[index + 1..];
        rest = &rest[..index];
    }

    SplitUrl {
        scheme,
        netloc: netloc.to_string(),
        path: rest.to_string(),
        query: query.to_string(),
        fragment: fragment.to_string(),
    }
}

/// Port of `sources.py::_host`.
pub fn host(value: &str) -> io::Result<String> {
    let valid_chars = value
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-');
    if value.is_empty() || !valid_chars {
        return Err(invalid("invalid_source_host"));
    }
    if !value.contains('.')
        || value.starts_with('.')
        || value.ends_with('.')
        || value.contains("..")
        || value == "localhost"
        || value.ends_with(".localhost")
        || value.ends_with(".local")
    {
        return Err(invalid("invalid_source_host"));
    }
    if is_ip_address(value) {
        return Err(invalid("invalid_source_host"));
    }
    Ok(value.to_string())
}

/// Mirrors `ipaddress.ip_address` for the strings `_host` can reach: the caller's
/// regex already rejects `:`, so only dotted-quad IPv4 matters. A leading-zero
/// dotted quad is not a valid IPv4 address in the stdlib, so it is treated as a
/// hostname (and accepted by [`host`]).
fn is_ip_address(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|part| {
        !part.is_empty()
            && part.len() <= 3
            && part.bytes().all(|b| b.is_ascii_digit())
            && (part.len() == 1 || !part.starts_with('0'))
            && part.parse::<u16>().is_ok_and(|octet| octet <= 255)
    })
}

/// Port of `sources.py::_allowed_url`.
///
/// `download` marks a download URL: such a URL must not carry a fragment, whose
/// presence would make the response identity ambiguous. The path extension is
/// not admission evidence; the downloaded bytes are validated as an archive.
pub fn allowed_url(url: &str, hosts: &[String], download: bool) -> io::Result<String> {
    if url.is_empty() || url != url.trim() || url.chars().any(|c| (c as u32) < 33) {
        return Err(invalid("invalid_source_url"));
    }
    let parsed = split_url(url);
    let port = parsed.port().map_err(|_| invalid("invalid_source_url"))?;
    let hostname_allowed = parsed
        .hostname()
        .is_some_and(|name| hosts.iter().any(|host| host == &name));
    let credentials = parsed.username().is_some_and(|user| !user.is_empty())
        || parsed
            .password()
            .is_some_and(|password| !password.is_empty());
    if parsed.scheme != "https"
        || !hostname_allowed
        || credentials
        || !matches!(port, None | Some(443))
        || !parsed.path.starts_with('/')
        || (download && !parsed.fragment.is_empty())
    {
        return Err(invalid("invalid_source_url"));
    }
    Ok(url.trim().to_string())
}

/// Port of `sources.py::SourceRegistry._navigation_url`.
pub fn navigation_url(url: &str) -> io::Result<String> {
    let hostname = split_url(url).hostname().unwrap_or_default();
    let host = host(&hostname)?;
    allowed_url(url, &[host], false)
}

// ---- names ---------------------------------------------------------------

use unicode_general_category::{get_general_category, GeneralCategory};
use unicode_normalization::UnicodeNormalization;

/// `unicodedata.normalize("NFKC", text).casefold()`.
fn normal_form(text: &str) -> String {
    let nfkc: String = text.nfkc().collect();
    caseless::default_case_fold_str(&nfkc)
}

fn is_letter_or_number(c: char) -> bool {
    matches!(
        get_general_category(c),
        GeneralCategory::UppercaseLetter
            | GeneralCategory::LowercaseLetter
            | GeneralCategory::TitlecaseLetter
            | GeneralCategory::ModifierLetter
            | GeneralCategory::OtherLetter
            | GeneralCategory::DecimalNumber
            | GeneralCategory::LetterNumber
            | GeneralCategory::OtherNumber
    )
}

/// Port of `sources.py::_normal`: NFKC, casefold, `+` -> `"plus"`, keep only
/// Unicode Letter/Number categories.
pub fn normal(text: &str) -> String {
    normal_form(text)
        .replace('+', "plus")
        .chars()
        .filter(|c| is_letter_or_number(*c))
        .collect()
}

/// Port of `discovery.py::_normal`: NFKC, casefold, keep `+` and Letter/Number.
pub fn discovery_normal(text: &str) -> String {
    normal_form(text)
        .chars()
        .filter(|c| *c == '+' || is_letter_or_number(*c))
        .collect()
}

/// Port of `sources.py::_split_names`.
pub fn split_names(title: &str) -> Vec<String> {
    let mut values = vec![title.to_string()];
    if title.contains(" / ") {
        values.extend(title.split(" / ").map(|part| part.trim().to_string()));
    }
    values.retain(|value| !value.is_empty());
    values
}

// ---- rules ---------------------------------------------------------------

fn valid_source_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    if !matches!(bytes.next(), Some(first) if first.is_ascii_lowercase()) {
        return false;
    }
    value.len() <= 48 && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_adapter_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    if !matches!(bytes.next(), Some(first) if first.is_ascii_lowercase()) {
        return false;
    }
    value.len() <= 48 && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn valid_table_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Port of `sources.py::_validate_rule`, preserving the Python check order and
/// error codes.
pub fn validate_rule(raw: &RawSourceRule) -> io::Result<SourceRule> {
    if !valid_source_id(&raw.id) {
        return Err(invalid("invalid_source_id"));
    }
    if raw.name.trim().is_empty() || raw.name.len() > 128 {
        return Err(invalid("invalid_source_name"));
    }
    if !valid_adapter_id(&raw.adapter) {
        return Err(invalid("unsupported_source_adapter"));
    }
    let role = match raw.role.as_deref().unwrap_or("publisher") {
        "publisher" => SourceRole::Publisher,
        "fallback_indexer" => SourceRole::FallbackIndexer,
        _ => return Err(invalid("invalid_source_rule")),
    };
    let discovery_only = raw.discovery_only.unwrap_or(false);
    let priority = raw.priority.unwrap_or(100);
    if !(0..=1000).contains(&priority) {
        return Err(invalid("invalid_source_rule"));
    }
    if discovery_only {
        if raw.root_layout.is_some()
            || !(DISCOVERY_ADAPTERS.contains(&raw.adapter.as_str()) || raw.adapter == "lua")
        {
            return Err(invalid("invalid_source_rule"));
        }
    } else if raw.root_layout.as_deref() != Some("game_root") || role != SourceRole::Publisher {
        return Err(invalid("invalid_source_rule"));
    }
    let download_hosts = raw.download_hosts.clone().unwrap_or_default();
    if role == SourceRole::FallbackIndexer && (!discovery_only || !download_hosts.is_empty()) {
        return Err(invalid("invalid_source_rule"));
    }
    if raw.pages.is_empty() {
        return Err(invalid("invalid_source_pages"));
    }
    if raw.page_hosts.is_empty() {
        return Err(invalid("invalid_source_hosts"));
    }
    for value in &raw.page_hosts {
        host(value)?;
    }
    let minimum_download_hosts = if discovery_only { 0 } else { 1 };
    if download_hosts.len() < minimum_download_hosts {
        return Err(invalid("invalid_source_hosts"));
    }
    for value in &download_hosts {
        host(value)?;
    }
    for page in &raw.pages {
        allowed_url(page, &raw.page_hosts, false)?;
    }
    if raw.adapter == "table_html" {
        let Some(columns) = &raw.columns else {
            return Err(invalid("invalid_source_columns"));
        };
        if columns.title > 20
            || columns.download > 20
            || columns.date.is_some_and(|v| v > 20)
            || columns.language.is_some_and(|v| v > 20)
            || columns.appid.is_some_and(|v| v > 20)
        {
            return Err(invalid("invalid_source_columns"));
        }
        if let Some(table_id) = &raw.table_id {
            if !valid_table_id(table_id) {
                return Err(invalid("invalid_table_id"));
            }
        }
    }
    Ok(SourceRule {
        id: raw.id.clone(),
        name: raw.name.clone(),
        adapter: raw.adapter.clone(),
        role,
        discovery_only,
        priority: priority as u16,
        enabled: raw.enabled,
        pages: raw.pages.clone(),
        page_hosts: raw.page_hosts.clone(),
        download_hosts,
        root_layout: if discovery_only {
            None
        } else {
            Some(RootLayout::GameRoot)
        },
        columns: raw.columns.clone(),
        table_id: raw.table_id.clone(),
        language: raw.language.clone(),
        url: raw.url.clone(),
        script: None,
    })
}

/// Port of `sources.py::_validate_entries`.
pub fn validate_entries(
    source: &SourceRule,
    entries: &[SourceEntry],
) -> io::Result<Vec<SourceEntry>> {
    if entries.is_empty() {
        return Err(invalid("empty_or_oversized_source"));
    }
    let mut clean = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.title.trim().is_empty() {
            return Err(invalid("invalid_source_entry"));
        }
        // `aliases` is already `Vec<String>`; Python's element type check cannot fail.
        let mut item = entry.clone();
        item.url = allowed_url(&entry.url, &source.download_hosts, true)?;
        item.source_page = allowed_url(&entry.source_page, &source.page_hosts, false)?;
        if entry.language.trim().is_empty() {
            return Err(invalid("invalid_source_language"));
        }
        if entry.appid == Some(0) {
            return Err(invalid("invalid_source_appid"));
        }
        if let Some(date) = &entry.date {
            item.date = Some(normalize_date(date)?);
        }
        clean.push(item);
    }
    Ok(clean)
}

fn normalize_date(raw: &str) -> io::Result<String> {
    let stripped = raw.trim();
    let value = if stripped.to_uppercase() == "COMING SOON" {
        String::from("COMING SOON")
    } else {
        stripped.to_string()
    };
    if value == "COMING SOON" {
        return Ok(value);
    }
    let parts: Vec<&str> = value.split('/').collect();
    if parts.len() != 3
        || !is_digits(parts[0], 4, 4)
        || !is_digits(parts[1], 1, 2)
        || !is_digits(parts[2], 1, 2)
    {
        return Err(invalid("invalid_source_date"));
    }
    let year: u32 = parts[0]
        .parse()
        .map_err(|_| invalid("invalid_source_date"))?;
    let month: u32 = parts[1]
        .parse()
        .map_err(|_| invalid("invalid_source_date"))?;
    let day: u32 = parts[2]
        .parse()
        .map_err(|_| invalid("invalid_source_date"))?;
    if !valid_ymd(year, month, day) {
        return Err(invalid("invalid_source_date"));
    }
    Ok(value)
}

fn is_digits(value: &str, minimum: usize, maximum: usize) -> bool {
    (minimum..=maximum).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit())
}

fn valid_ymd(year: u32, month: u32, day: u32) -> bool {
    if !(1..=9999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return false;
    }
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    day <= days_in_month
}

// ---- candidates ----------------------------------------------------------

fn valid_candidate_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("candidate-") else {
        return false;
    };
    rest.len() == 24
        && rest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Port of `sources.py::SourceRegistry._validate_candidates`.
pub fn validate_candidates(
    rule: &SourceRule,
    appid: u32,
    candidates: &[SourceCandidate],
) -> io::Result<Vec<SourceCandidate>> {
    for candidate in candidates {
        if !valid_candidate_id(&candidate.candidate_id)
            || candidate.source_id != rule.id
            || candidate.capability != Capability::DiscoverOnly
        {
            return Err(invalid("invalid_discovery_candidate"));
        }
        if candidate.game_appid.is_some_and(|value| value != appid) {
            return Err(invalid("invalid_discovery_identity"));
        }
        if matches!(
            candidate.match_basis,
            MatchBasis::Appid | MatchBasis::VerifiedAlias | MatchBasis::OfficialTitle
        ) && candidate.game_appid != Some(appid)
        {
            return Err(invalid("invalid_discovery_identity"));
        }
        for title in [&candidate.game_title, &candidate.patch_title] {
            if title.trim().is_empty() || title.len() > 1024 {
                return Err(invalid("invalid_discovery_candidate"));
            }
        }
        for list in [
            &candidate.aliases,
            &candidate.languages,
            &candidate.warnings,
        ] {
            if list.iter().any(|item| item.len() > 1024) {
                return Err(invalid("invalid_discovery_candidate"));
            }
        }
        for list in [&candidate.requires_dlc, &candidate.excludes_dlc] {
            // u32 cannot exceed the Python 9999999999 ceiling, so only zero is invalid.
            if list.as_ref().is_some_and(|values| values.contains(&0)) {
                return Err(invalid("invalid_discovery_candidate"));
            }
        }
        allowed_url(&candidate.source_page, &rule.page_hosts, false)?;
        if let Some(upstream) = &candidate.upstream_page {
            if !upstream.is_empty() {
                navigation_url(upstream)?;
            }
        }
        for mirror in &candidate.mirrors {
            navigation_url(&mirror.url)?;
        }
        for evidence in &candidate.identity_evidence {
            if let Some(page) = &evidence.page {
                if !page.is_empty() {
                    navigation_url(page)?;
                }
            }
        }
        for evidence in &candidate.provenance {
            if !evidence.page.is_empty() {
                navigation_url(&evidence.page)?;
            }
        }
        for evidence in &candidate.related_index_pages {
            if !evidence.page.is_empty() {
                navigation_url(&evidence.page)?;
            }
        }
    }
    Ok(candidates.to_vec())
}

// ---- overrides -----------------------------------------------------------

fn valid_game_appid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 10
        && value.bytes().all(|b| b.is_ascii_digit())
        && !value.starts_with('0')
}

/// Structural validation of a parsed [`Overrides`] (port of the `sources.py`
/// `_reload` games validation). Cross-rule uniqueness against builtins and
/// enabled-flag merging are enforced by the registry merge (later task).
pub fn validate_overrides(overrides: &Overrides) -> io::Result<()> {
    let mut ids = HashSet::new();
    for rule in &overrides.added {
        if !valid_source_id(&rule.id) || !ids.insert(rule.id.as_str()) {
            return Err(invalid("invalid_source_overrides"));
        }
    }
    for (appid, choices) in &overrides.games {
        if !valid_game_appid(appid) {
            return Err(invalid("invalid_source_overrides"));
        }
        for source_id in choices.keys() {
            if !valid_source_id(source_id) {
                return Err(invalid("invalid_source_overrides"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_rule() -> RawSourceRule {
        RawSourceRule {
            id: "example".into(),
            name: "Example".into(),
            adapter: "patchmoe".into(),
            role: None,
            discovery_only: None,
            priority: None,
            enabled: true,
            pages: vec!["https://example.com/".into()],
            page_hosts: vec!["example.com".into()],
            download_hosts: Some(vec!["download.example.com".into()]),
            root_layout: Some("game_root".into()),
            columns: None,
            table_id: None,
            language: None,
            url: None,
        }
    }

    fn entry() -> SourceEntry {
        SourceEntry {
            title: "Game".into(),
            aliases: vec![],
            url: "https://download.example.com/patch.zip".into(),
            source_page: "https://example.com/game/".into(),
            language: "en".into(),
            date: Some("2024/1/2".into()),
            appid: Some(42),
        }
    }

    #[test]
    fn rule_validation_applies_defaults_and_roles() {
        let rule = validate_rule(&raw_rule()).unwrap();
        assert_eq!(rule.role, SourceRole::Publisher);
        assert!(!rule.discovery_only);
        assert_eq!(rule.priority, 100);
        assert_eq!(rule.root_layout, Some(RootLayout::GameRoot));

        let mut discovery = raw_rule();
        discovery.id = "hgamer".into();
        discovery.adapter = "hgamer_html".into();
        discovery.role = Some("fallback_indexer".into());
        discovery.discovery_only = Some(true);
        discovery.root_layout = None;
        discovery.download_hosts = Some(vec![]);
        let rule = validate_rule(&discovery).unwrap();
        assert_eq!(rule.role, SourceRole::FallbackIndexer);
        assert!(rule.discovery_only);
        assert_eq!(rule.root_layout, None);
    }

    #[test]
    fn rules_config_schema2_required_defaults_and_rejections() {
        // Minimal schema 2 publisher: required fields only, defaults applied.
        let schema2 = r#"{"schema_version":2,"sources":[{"id":"patchmoe","name":"patch.moe","adapter":"patchmoe","enabled":true,"pages":["https://www.patch.moe/?flag=1"],"page_hosts":["www.patch.moe"],"download_hosts":["download.patch.moe"],"root_layout":"game_root"}]}"#;
        let config: RulesConfig = serde_json::from_str(schema2).unwrap();
        assert_eq!(config.schema_version, 2);
        let rule = validate_rule(&config.sources[0]).unwrap();
        assert_eq!(rule.role, SourceRole::Publisher);
        assert!(!rule.discovery_only);
        assert_eq!(rule.priority, 100);
        assert_eq!(rule.root_layout, Some(RootLayout::GameRoot));

        // A publisher must carry `root_layout == "game_root"`.
        let missing_root = r#"{"schema_version":2,"sources":[{"id":"patchmoe","name":"patch.moe","adapter":"patchmoe","enabled":true,"pages":["https://www.patch.moe/"],"page_hosts":["www.patch.moe"],"download_hosts":["download.patch.moe"]}]}"#;
        let config: RulesConfig = serde_json::from_str(missing_root).unwrap();
        assert_eq!(
            validate_rule(&config.sources[0]).unwrap_err().to_string(),
            "invalid_source_rule"
        );

        // A `discovery_only` rule must leave `root_layout` absent.
        let discovery_root = r#"{"schema_version":2,"sources":[{"id":"hgamer","name":"HGamer","adapter":"hgamer_html","enabled":true,"role":"fallback_indexer","discovery_only":true,"pages":["https://hgamer.tw/zh-CN/patches"],"page_hosts":["hgamer.tw"],"download_hosts":[],"root_layout":"game_root"}]}"#;
        let config: RulesConfig = serde_json::from_str(discovery_root).unwrap();
        assert_eq!(
            validate_rule(&config.sources[0]).unwrap_err().to_string(),
            "invalid_source_rule"
        );

        // Required fields `enabled`/`pages`/`page_hosts` cannot be omitted.
        for incomplete in [
            r#"{"schema_version":2,"sources":[{"id":"patchmoe","name":"patch.moe","adapter":"patchmoe","pages":["https://www.patch.moe/"],"page_hosts":["www.patch.moe"]}]}"#,
            r#"{"schema_version":2,"sources":[{"id":"patchmoe","name":"patch.moe","adapter":"patchmoe","enabled":true,"page_hosts":["www.patch.moe"]}]}"#,
            r#"{"schema_version":2,"sources":[{"id":"patchmoe","name":"patch.moe","adapter":"patchmoe","enabled":true,"pages":["https://www.patch.moe/"]}]}"#,
        ] {
            assert!(
                serde_json::from_str::<RulesConfig>(incomplete).is_err(),
                "missing required field must fail: {incomplete}"
            );
        }
    }

    #[test]
    fn builtin_rules_validate_and_keep_roles() {
        let config: RulesConfig = serde_json::from_str(crate::sources::DEFAULT_RULES_JSON).unwrap();
        assert_eq!(config.schema_version, 2);
        assert_eq!(config.sources.len(), 4);
        let rules: Vec<SourceRule> = config
            .sources
            .iter()
            .map(|raw| validate_rule(raw).unwrap())
            .collect();
        assert_eq!(rules.iter().filter(|r| r.discovery_only).count(), 3);
        let hgamer = rules.iter().find(|r| r.id == "hgamer").unwrap();
        assert_eq!(hgamer.role, SourceRole::FallbackIndexer);
        assert!(hgamer.discovery_only);
        assert!(hgamer.download_hosts.is_empty());
    }

    #[test]
    fn rule_validation_rejects_unsupported_adapter_and_bad_shapes() {
        let mut bad = raw_rule();
        bad.id = "Bad".into();
        assert_eq!(
            validate_rule(&bad).unwrap_err().to_string(),
            "invalid_source_id"
        );

        let mut bad = raw_rule();
        bad.adapter = "bad!".into();
        assert_eq!(
            validate_rule(&bad).unwrap_err().to_string(),
            "unsupported_source_adapter"
        );

        // discovery_only requires a discovery adapter and no root layout.
        let mut bad = raw_rule();
        bad.discovery_only = Some(true);
        assert_eq!(
            validate_rule(&bad).unwrap_err().to_string(),
            "invalid_source_rule"
        );

        // Non-discovery publishers must be game_root publishers.
        let mut bad = raw_rule();
        bad.root_layout = None;
        assert_eq!(
            validate_rule(&bad).unwrap_err().to_string(),
            "invalid_source_rule"
        );

        // Bounds: pages 1..=5, page_hosts 1..=8, download_hosts 1..=8.
        let mut bad = raw_rule();
        bad.pages = vec![];
        assert_eq!(
            validate_rule(&bad).unwrap_err().to_string(),
            "invalid_source_pages"
        );
        let mut bad = raw_rule();
        bad.pages = (0..6).map(|i| format!("https://example.com/{i}")).collect();
        assert_eq!(validate_rule(&bad).unwrap().pages.len(), 6);
        let mut bad = raw_rule();
        bad.page_hosts = vec![];
        assert_eq!(
            validate_rule(&bad).unwrap_err().to_string(),
            "invalid_source_hosts"
        );
        let mut bad = raw_rule();
        bad.download_hosts = Some(vec![]);
        assert_eq!(
            validate_rule(&bad).unwrap_err().to_string(),
            "invalid_source_hosts"
        );
    }

    #[test]
    fn rule_validation_table_columns_and_table_id() {
        let mut table = raw_rule();
        table.adapter = "table_html".into();
        table.columns = Some(Columns {
            title: 0,
            download: 1,
            date: None,
            language: None,
            appid: None,
        });
        assert!(validate_rule(&table).is_ok());

        table.columns = None;
        assert_eq!(
            validate_rule(&table).unwrap_err().to_string(),
            "invalid_source_columns"
        );

        table.columns = Some(Columns {
            title: 21,
            download: 1,
            date: None,
            language: None,
            appid: None,
        });
        assert_eq!(
            validate_rule(&table).unwrap_err().to_string(),
            "invalid_source_columns"
        );

        table.columns = Some(Columns {
            title: 0,
            download: 1,
            date: None,
            language: None,
            appid: None,
        });
        table.table_id = Some("p".repeat(65));
        assert_eq!(
            validate_rule(&table).unwrap_err().to_string(),
            "invalid_table_id"
        );
    }

    #[test]
    fn host_and_allowed_url_reject_hostile() {
        assert!(host("example.com").is_ok());
        for bad in [
            "localhost",
            "example",       // no dot
            ".example.com",  // leading dot
            "example.com.",  // trailing dot
            "a..b.com",      // empty label
            "Example.com",   // uppercase
            "example_com",   // underscore
            "1.2.3.4",       // IPv4 literal
            "evil.local",    // .local suffix
            "sub.localhost", // .localhost suffix
            "",
        ] {
            assert!(host(bad).is_err(), "host {bad:?} must be rejected");
        }
        // Numeric dotted strings that are not valid IPs are accepted, matching Python.
        assert!(host("999.1.1.1").is_ok());
        assert!(host("01.2.3.4").is_ok());
        assert!(host("1.2.3").is_ok());

        let hosts = vec!["example.com".to_string()];
        assert!(allowed_url("https://example.com/p", &hosts, false).is_ok());
        for bad in [
            "http://example.com/p",            // scheme
            "https://other.example/p",         // host not allowed
            "https://user@example.com/p",      // credentials
            "https://user:pass@example.com/p", // credentials
            "https://example.com:8443/p",      // non-443 port
            "https://example.com",             // path must start with /
            "https://example.com/p ",          // trailing space
            "https://example.com/p\t",         // control character
        ] {
            assert!(allowed_url(bad, &hosts, false).is_err(), "allow {bad:?}");
        }
        assert!(allowed_url("https://example.com/patch.zip", &hosts, true).is_ok());
        // Download admission does not depend on the path extension (S11); the
        // fragment rule stays.
        assert!(allowed_url("https://example.com/patch.rar", &hosts, true).is_ok());
        assert!(allowed_url("https://example.com/download?id=7", &hosts, true).is_ok());
        assert!(allowed_url("https://example.com/patch.zip#frag", &hosts, true).is_err());
    }

    #[test]
    fn navigation_url_guards_mirror_hosts() {
        assert!(navigation_url("https://mirror.example/page").is_ok());
        assert!(navigation_url("https://mirror.example/page#x").is_ok());
        assert!(navigation_url("http://mirror.example/page").is_err());
        assert!(navigation_url("https://user@mirror.example/page").is_err());
        assert!(navigation_url("https://10.0.0.1/page").is_err());
        assert!(navigation_url("not a url").is_err());
    }

    #[test]
    fn validate_entries_rejects_bad_date_and_appid() {
        let rule = validate_rule(&raw_rule()).unwrap();
        assert_eq!(validate_entries(&rule, &[entry()]).unwrap().len(), 1);

        assert_eq!(
            validate_entries(&rule, &[]).unwrap_err().to_string(),
            "empty_or_oversized_source"
        );

        let mut bad = entry();
        bad.date = Some("2024/13/01".into());
        assert_eq!(
            validate_entries(&rule, &[bad]).unwrap_err().to_string(),
            "invalid_source_date"
        );

        let mut bad = entry();
        bad.date = Some("2023/2/29".into());
        assert_eq!(
            validate_entries(&rule, &[bad]).unwrap_err().to_string(),
            "invalid_source_date"
        );

        let mut bad = entry();
        bad.appid = Some(0);
        assert_eq!(
            validate_entries(&rule, &[bad]).unwrap_err().to_string(),
            "invalid_source_appid"
        );

        let mut bad = entry();
        bad.language = "  ".into();
        assert_eq!(
            validate_entries(&rule, &[bad]).unwrap_err().to_string(),
            "invalid_source_language"
        );

        let mut bad = entry();
        bad.url = "http://download.example.com/patch.rar".into();
        assert_eq!(
            validate_entries(&rule, &[bad]).unwrap_err().to_string(),
            "invalid_source_url"
        );

        let mut good = entry();
        good.date = Some("coming soon".into());
        assert_eq!(
            validate_entries(&rule, &[good]).unwrap().remove(0).date,
            Some("COMING SOON".into())
        );
    }

    #[test]
    fn split_names_and_normal_primitives() {
        assert_eq!(
            split_names("A / B"),
            vec!["A / B".to_string(), "A".to_string(), "B".to_string()]
        );
        assert_eq!(split_names("Solo"), vec!["Solo".to_string()]);
        assert_eq!(split_names(""), Vec::<String>::new());
        assert_eq!(normal("Pokémon + DLC!"), "pokémonplusdlc");
        assert_eq!(discovery_normal("A+B!"), "a+b");
    }

    #[test]
    fn normal_uses_nfkc_full_casefold_and_categories() {
        // NFKC: circled / full-width forms collapse to ASCII.
        assert_eq!(normal("①"), "1");
        assert_eq!(normal("ＡＢＣ１２３"), "abc123");
        assert_eq!(discovery_normal("Ⅳ"), "iv");
        // Full casefold (not just lowercase): ß expands, ẞ folds too.
        assert_eq!(normal("ß"), "ss");
        assert_eq!(normal("Straße"), "strasse");
        assert_eq!(normal("ẞ"), "ss");
        // Category filtering drops punctuation/symbols and keeps L/N.
        assert_eq!(normal("a-b c!?+"), "abcplus");
        assert_eq!(discovery_normal("a-b c!?+"), "abc+");
        // The R5 distinction: `normal` maps '+' to "plus" before filtering.
        assert_eq!(normal("Fate/Extra+"), "fateextraplus");
        assert_eq!(discovery_normal("Fate/Extra+"), "fateextra+");
    }

    #[test]
    fn validate_overrides_rejects_bad_game_keys() {
        let mut overrides = Overrides::default();
        overrides
            .games
            .insert("42".into(), [("example".to_string(), true)].into());
        assert!(validate_overrides(&overrides).is_ok());

        let mut bad = Overrides::default();
        bad.games
            .insert("0".into(), [("example".to_string(), true)].into());
        assert_eq!(
            validate_overrides(&bad).unwrap_err().to_string(),
            "invalid_source_overrides"
        );

        let mut bad = Overrides::default();
        bad.games
            .insert("42".into(), [("Bad".to_string(), true)].into());
        assert_eq!(
            validate_overrides(&bad).unwrap_err().to_string(),
            "invalid_source_overrides"
        );
    }

    #[test]
    fn split_url_matches_urlsplit_fields() {
        let parsed = split_url("https://user:pass@Host.Example:8443/p?a=1#frag");
        assert_eq!(parsed.scheme, "https");
        assert_eq!(parsed.netloc, "user:pass@Host.Example:8443");
        assert_eq!(parsed.path, "/p");
        assert_eq!(parsed.query, "a=1");
        assert_eq!(parsed.fragment, "frag");
        assert_eq!(parsed.hostname().as_deref(), Some("host.example"));
        assert_eq!(parsed.port().unwrap(), Some(8443));
        assert_eq!(parsed.username(), Some("user"));
        assert_eq!(parsed.password(), Some("pass"));

        let plain = split_url("https://example.com/path");
        assert_eq!(plain.scheme, "https");
        assert_eq!(plain.netloc, "example.com");
        assert_eq!(plain.path, "/path");
        assert_eq!(plain.hostname().as_deref(), Some("example.com"));
        assert_eq!(plain.port().unwrap(), None);
    }
}
