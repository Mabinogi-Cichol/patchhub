//! Candidate identity, title/version/language primitives and merge/enrich
//! (discovery.py P2). Deliberately DOM-free: adapters pass strings and link
//! hrefs in, so every helper here is pure and unit-testable offline.

use super::canonical;
use super::model::*;
use super::validate;
use super::{invalid, Result};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

// ---- script conversion (OpenCC) ------------------------------------------

static ST_CHARACTERS: OnceLock<HashMap<char, char>> = OnceLock::new();
static TS_CHARACTERS: OnceLock<HashMap<char, char>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dictionary {
    /// Simplified -> Traditional (`STCharacters.txt`).
    St,
    /// Traditional -> Simplified (`TSCharacters.txt`).
    Ts,
}

/// Port of `discovery.py::_convert`: each character's first listed conversion.
pub fn convert(text: &str, dictionary: Dictionary) -> String {
    let table = match dictionary {
        Dictionary::St => ST_CHARACTERS
            .get_or_init(|| parse_table(include_str!("../../resources/opencc/STCharacters.txt"))),
        Dictionary::Ts => TS_CHARACTERS
            .get_or_init(|| parse_table(include_str!("../../resources/opencc/TSCharacters.txt"))),
    };
    text.chars()
        .map(|character| *table.get(&character).unwrap_or(&character))
        .collect()
}

fn parse_table(content: &str) -> HashMap<char, char> {
    let mut table = HashMap::new();
    for line in content.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, targets)) = line.split_once('\t') else {
            continue;
        };
        // Only single-character keys are reachable through per-character lookup.
        if key.chars().count() != 1 {
            continue;
        }
        let key = key.chars().next().expect("single char key");
        // Python uses `targets.split()[0]` (the first whitespace-delimited token).
        let target = targets
            .split_whitespace()
            .next()
            .and_then(|target| target.chars().next());
        if let Some(target) = target {
            table.insert(key, target);
        }
    }
    table
}

/// Port of `discovery.py::query_terms`: at most three terms (name, a Latin
/// alias, then script conversions), deduped by [`validate::discovery_normal`].
pub fn query_terms(name: &str, aliases: &[String]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    names.push(name.to_string());
    names.extend(aliases.iter().cloned());
    let names: Vec<String> = names
        .into_iter()
        .filter(|name| !name.trim().is_empty())
        .map(|name| name.trim().chars().take(256).collect::<String>())
        .collect();

    let mut terms: Vec<String> = Vec::new();
    let Some(first) = names.first().cloned() else {
        return terms;
    };
    add_term(&mut terms, first.clone());
    for name in &names[1..] {
        if contains_ascii_alpha(name) {
            add_term(&mut terms, name.clone());
            break;
        }
    }
    add_term(&mut terms, convert(&first, Dictionary::St));
    add_term(&mut terms, convert(&first, Dictionary::Ts));
    for name in &names[1..] {
        add_term(&mut terms, name.clone());
    }
    terms
}

fn add_term(terms: &mut Vec<String>, candidate: String) {
    if candidate.is_empty() {
        return;
    }
    let key = validate::discovery_normal(&candidate);
    if terms
        .iter()
        .any(|term| validate::discovery_normal(term) == key)
    {
        return;
    }
    if terms.len() < 3 {
        terms.push(candidate);
    }
}

// ---- title / variant -----------------------------------------------------

/// Port of `discovery.py::game_title`: strip one established descriptive suffix.
pub fn game_title(title: &str) -> String {
    let chars: Vec<char> = title.chars().collect();
    for start in 0..=chars.len() {
        let head = skip_ws(&chars, start);
        // The optional separator is tried before the bare alternative (greedy).
        if let Some(separator) = chars.get(head) {
            if matches!(*separator, '-' | '–' | '—' | '：' | ':') {
                let after = skip_ws(&chars, head + 1);
                if let Some(end) = match_title_suffix(&chars, after) {
                    if skip_ws(&chars, end) == chars.len() {
                        return chars[..start].iter().collect::<String>().trim().to_string();
                    }
                }
            }
        }
        if let Some(end) = match_title_suffix(&chars, head) {
            if skip_ws(&chars, end) == chars.len() {
                return chars[..start].iter().collect::<String>().trim().to_string();
            }
        }
    }
    title.trim().to_string()
}

fn match_title_suffix(chars: &[char], index: usize) -> Option<usize> {
    // 官方工口[补補]丁
    if seq_equals(chars, index, "官方工口") {
        if let Some(bu) = chars.get(index + 4) {
            if matches!(*bu, '补' | '補') && chars.get(index + 5) == Some(&'丁') {
                return Some(index + 6);
            }
        }
    }
    // 中文[无無][码碼][补補]丁(包)?
    if seq_equals(chars, index, "中文") {
        if let Some(end) = match_wuma_suffix(chars, index + 2) {
            return Some(end);
        }
    }
    // [无無][码碼][补補]丁(包)?
    if let Some(end) = match_wuma_suffix(chars, index) {
        return Some(end);
    }
    // [补補]丁
    if let Some(bu) = chars.get(index) {
        if matches!(*bu, '补' | '補') && chars.get(index + 1) == Some(&'丁') {
            return Some(index + 2);
        }
    }
    // \bpatch
    if seq_equals_ci(chars, index, "patch") && (index == 0 || !is_word_char(chars[index - 1])) {
        return Some(index + 5);
    }
    None
}

fn match_wuma_suffix(chars: &[char], index: usize) -> Option<usize> {
    match chars.get(index) {
        Some(c) if matches!(*c, '无' | '無') => {}
        _ => return None,
    }
    match chars.get(index + 1) {
        Some(c) if matches!(*c, '码' | '碼') => {}
        _ => return None,
    }
    match chars.get(index + 2) {
        Some(c) if matches!(*c, '补' | '補') => {}
        _ => return None,
    }
    if chars.get(index + 3) != Some(&'丁') {
        return None;
    }
    if chars.get(index + 4) == Some(&'包') {
        Some(index + 5)
    } else {
        Some(index + 4)
    }
}

/// Port of `discovery.py::_variant`.
pub fn variant(title: &str) -> Variant {
    if demo_title(title) {
        return Variant::Demo;
    }
    if dlc_title(title) {
        return Variant::Dlc;
    }
    Variant::Base
}

fn demo_title(title: &str) -> bool {
    if title.contains("体验版") || title.contains("体驗版") {
        return true;
    }
    word_bounded_ci(title, "demo")
}

fn dlc_title(title: &str) -> bool {
    if title.contains('+') {
        return true;
    }
    ["dlc", "expansion", "append", "plus"]
        .iter()
        .any(|keyword| word_bounded_ci(title, keyword))
}

/// Port of `discovery.py::_base_title`: replace `+`, dashes and DLC keywords
/// with spaces, then trim.
pub fn base_title(title: &str) -> String {
    let cleaned = game_title(title);
    let chars: Vec<char> = cleaned.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if matches!(character, '+' | '-' | '–' | '—') {
            out.push(' ');
            index += 1;
            continue;
        }
        let mut consumed = None;
        for keyword in ["expansion", "dlc", "plus", "append"] {
            if seq_equals_ci(&chars, index, keyword)
                && (index == 0 || !is_word_char(chars[index - 1]))
                && (index + keyword.len() == chars.len()
                    || !is_word_char(chars[index + keyword.len()]))
            {
                consumed = Some(keyword.len());
                break;
            }
        }
        if let Some(len) = consumed {
            out.push(' ');
            index += len;
            continue;
        }
        out.push(character);
        index += 1;
    }
    out.trim().to_string()
}

/// Port of `discovery.py::_title_relevant`. `QueryContext` has no `aliases`
/// field, so only `name` + `name_hints` participate.
pub fn title_relevant(title: &str, query: &QueryContext) -> bool {
    let mut target: HashSet<String> = HashSet::new();
    for name in std::iter::once(&query.name).chain(query.name_hints.iter()) {
        if !name.is_empty() {
            target.insert(validate::discovery_normal(name));
        }
    }
    target.contains(&validate::discovery_normal(&game_title(title)))
        || target.contains(&validate::discovery_normal(&base_title(title)))
}

/// Port of `discovery.py::_aliases`, taking text blocks instead of DOM nodes.
pub fn aliases(title: &str, blocks: &[String]) -> Vec<String> {
    let mut names: Vec<String> = vec![game_title(title)];
    for block in blocks {
        if block.chars().count() >= 1000 {
            continue;
        }
        if let Some(rest) = strip_alias_prefix(block) {
            for part in rest.split(['·', '・']) {
                names.push(part.trim().to_string());
            }
            break;
        }
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for name in names {
        if !name.is_empty() && name.chars().count() <= 256 && seen.insert(name.clone()) {
            out.push(name);
        }
    }
    out
}

fn strip_alias_prefix(block: &str) -> Option<&str> {
    for prefix in ["又称", "又稱"] {
        if let Some(rest) = block.strip_prefix(prefix) {
            let rest = rest.strip_prefix(['：', ':'])?;
            return Some(rest.trim_start());
        }
    }
    None
}

/// Port of `discovery.py::_steam_ids`, taking link hrefs (and iframe srcs).
pub fn steam_ids(links: &[String]) -> Vec<u32> {
    let mut ids = Vec::new();
    for link in links {
        let parsed = validate::split_url(link);
        if parsed.scheme != "https"
            || parsed.hostname().as_deref() != Some("store.steampowered.com")
        {
            continue;
        }
        let path = parsed.path.trim_start_matches('/');
        let tail = if let Some(tail) = path.strip_prefix("app/") {
            tail
        } else if let Some(tail) = path.strip_prefix("widget/") {
            tail
        } else {
            continue;
        };
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            continue;
        }
        let after = &tail[digits.len()..];
        if !after.is_empty() && !after.starts_with('/') {
            continue;
        }
        if let Ok(id) = digits.parse::<u32>() {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

/// Port of `discovery.py::_format`.
pub fn format(url: &str) -> String {
    let path = validate::split_url(url).path.to_lowercase();
    if path.ends_with(".exe") {
        "exe".to_string()
    } else if path.ends_with(".zip") {
        "zip".to_string()
    } else {
        "web".to_string()
    }
}

/// Port of `discovery.py::_versions` (share passwords are stripped first).
pub fn versions(text: &str) -> Vec<String> {
    let cleaned = strip_passwords(text);
    let chars: Vec<char> = cleaned.chars().collect();
    let mut result: Vec<String> = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        // Python's negative lookbehind is `(?<![A-Za-z0-9])` (no underscore).
        if index > 0 && chars[index - 1].is_ascii_alphanumeric() {
            index += 1;
            continue;
        }
        if let Some((token, end)) = match_version(&chars, index) {
            if !result.contains(&token) {
                result.push(token);
            }
            index = end;
            continue;
        }
        index += 1;
    }
    result
}

fn match_version(chars: &[char], index: usize) -> Option<(String, usize)> {
    let mut cursor = None;
    if seq_equals_ci(chars, index, "ver") {
        let mut after = index + 3;
        if chars.get(after) == Some(&'.') {
            after += 1;
        }
        cursor = Some(skip_ws(chars, after));
    } else if matches!(chars.get(index), Some(c) if *c == 'v' || *c == 'V') {
        cursor = Some(skip_ws(chars, index + 1));
    }
    let start = cursor?;
    let (token, end) = parse_version_number(chars, start)?;
    Some((token, end))
}

fn parse_version_number(chars: &[char], start: usize) -> Option<(String, usize)> {
    let mut index = start;
    let mut token = String::new();
    let mut groups = 0;
    loop {
        let digits_start = index;
        while index < chars.len() && chars[index].is_ascii_digit() {
            token.push(chars[index]);
            index += 1;
        }
        if index == digits_start {
            return None;
        }
        groups += 1;
        if groups >= 5 {
            break;
        }
        if chars.get(index) == Some(&'.')
            && matches!(chars.get(index + 1), Some(next) if next.is_ascii_digit())
        {
            token.push('.');
            index += 1;
        } else {
            break;
        }
    }
    Some((token, index))
}

fn strip_passwords(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if let Some(end) = password_end(&chars, index) {
            index = end;
            continue;
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

fn password_end(chars: &[char], index: usize) -> Option<usize> {
    let marker_len = if seq_equals(chars, index, "提取") {
        2
    } else if chars.get(index) == Some(&'密') {
        1
    } else {
        return None;
    };
    let mut cursor = index + marker_len;
    if !matches!(chars.get(cursor), Some(c) if *c == '码' || *c == '碼') {
        return None;
    }
    cursor += 1;
    if matches!(chars.get(cursor), Some(c) if *c == '：' || *c == ':') {
        cursor += 1;
    }
    cursor = skip_ws(chars, cursor);
    let value_start = cursor;
    while cursor < chars.len() && (chars[cursor].is_ascii_alphanumeric() || chars[cursor] == '.') {
        cursor += 1;
    }
    if cursor > value_start {
        Some(cursor)
    } else {
        None
    }
}

/// Port of `discovery.py::_date`.
pub fn date(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index + 4 <= chars.len() {
        if chars[index] == '2'
            && chars[index + 1] == '0'
            && chars[index + 2].is_ascii_digit()
            && chars[index + 3].is_ascii_digit()
            && (index == 0 || !is_word_char(chars[index - 1]))
        {
            if let Some(value) = parse_date_at(&chars, index) {
                return Some(value);
            }
        }
        index += 1;
    }
    None
}

fn parse_date_at(chars: &[char], index: usize) -> Option<String> {
    let year: u32 = chars[index..index + 4]
        .iter()
        .collect::<String>()
        .parse()
        .ok()?;
    if !matches!(chars.get(index + 4), Some(c) if matches!(*c, '.' | '/' | '-')) {
        return None;
    }
    let (month, after_month) = digits_at(chars, index + 5)?;
    if !matches!(chars.get(after_month), Some(c) if matches!(*c, '.' | '/' | '-')) {
        return None;
    }
    let (day, after_day) = digits_at(chars, after_month + 1)?;
    if after_day < chars.len() && is_word_char(chars[after_day]) {
        return None;
    }
    if !valid_date(year, month, day) {
        return None;
    }
    Some(format!("{:04}-{:02}-{:02}", year, month, day))
}

fn digits_at(chars: &[char], start: usize) -> Option<(u32, usize)> {
    let mut index = start;
    let mut value = 0_u32;
    while index < chars.len() && chars[index].is_ascii_digit() && index - start < 2 {
        value = value * 10 + (chars[index] as u32 - '0' as u32);
        index += 1;
    }
    if index == start {
        None
    } else {
        Some((value, index))
    }
}

fn valid_date(year: u32, month: u32, day: u32) -> bool {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return false;
    }
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    day <= days
}

/// Port of `discovery.py::_language`: fixed order, case-insensitive.
pub fn language(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    if find_ci(text, "/CN/").is_some()
        || cn_patch(text, "cn")
        || chinese_without_traditional_lookbehind(text)
    {
        result.push("zh-CN".to_string());
    }
    if find_ci(text, "/EN/").is_some() || cn_patch(text, "en") || find_ci(text, "english").is_some()
    {
        result.push("en".to_string());
    }
    if text.contains("繁體中文") || text.contains("繁体中文") || word_bounded_ci(text, "tcn")
    {
        result.push("zh-TW".to_string());
    }
    if find_ci(text, "/JP/").is_some() || find_ci(text, "japanese").is_some() {
        result.push("ja".to_string());
    }
    result
}

fn chinese_without_traditional_lookbehind(text: &str) -> bool {
    let mut search = 0;
    while let Some(position) = text[search..].find("中文") {
        let absolute = search + position;
        let before = &text[..absolute];
        if !(before.ends_with("繁體") || before.ends_with("繁体")) {
            return true;
        }
        search = absolute + "中文".len();
    }
    false
}

// ---- candidate construction ----------------------------------------------

#[derive(Clone, PartialEq, Eq)]
struct GroupKey {
    languages: Vec<String>,
    version: Option<String>,
    filename: Option<String>,
}

/// Port of `discovery.py::_candidate`.
#[allow(clippy::too_many_arguments)]
pub fn build_candidates(
    rule: &SourceRule,
    title: &str,
    page: &str,
    text: &str,
    ids: &[u32],
    aliases: &[String],
    reported: &str,
    mirrors: &[Mirror],
    query: &QueryContext,
) -> Vec<SourceCandidate> {
    let appid = query.appid;
    let has_ids = !ids.is_empty();
    if has_ids && !ids.contains(&appid) {
        return Vec::new();
    }
    let mut alias_list: Vec<String> = if aliases.is_empty() {
        vec![game_title(title)]
    } else {
        aliases.to_vec()
    };
    alias_list.retain(|alias| !alias.is_empty());
    if !has_ids && !alias_list.iter().any(|alias| title_relevant(alias, query)) {
        return Vec::new();
    }
    let variant = variant(title);
    if variant == Variant::Demo || (!has_ids && gog_only(text)) {
        return Vec::new();
    }
    let platforms: Vec<String> = if has_ids || word_bounded_ci(text, "steam") {
        vec!["steam".to_string()]
    } else {
        Vec::new()
    };
    let all_versions = versions(text);

    let mut groups: Vec<(GroupKey, Vec<Mirror>)> = Vec::new();
    for mirror in mirrors {
        let key = mirror_key(mirror);
        match groups.iter().position(|entry| entry.0 == key) {
            Some(position) => groups[position].1.push(mirror.clone()),
            None => groups.push((key, vec![mirror.clone()])),
        }
    }
    let unversioned: Vec<GroupKey> = groups
        .iter()
        .filter(|entry| entry.0.version.is_none() && entry.0.filename.is_none())
        .map(|entry| entry.0.clone())
        .collect();
    let versioned_count = groups.len() - unversioned.len();
    if versioned_count == 1 && !unversioned.is_empty() {
        let versioned_key = groups
            .iter()
            .find(|entry| !(entry.0.version.is_none() && entry.0.filename.is_none()))
            .map(|entry| entry.0.clone());
        if let Some(versioned_key) = versioned_key {
            for key in unversioned {
                let position = groups.iter().position(|entry| entry.0 == key);
                if let Some(position) = position {
                    let (_, mut entries) = groups.remove(position);
                    let target = groups.iter().position(|entry| entry.0 == versioned_key);
                    if let Some(target) = target {
                        groups[target].1.append(&mut entries);
                    }
                }
            }
        }
    }
    if groups.is_empty() {
        let key = GroupKey {
            languages: language(text),
            version: if all_versions.len() == 1 {
                Some(all_versions[0].clone())
            } else {
                None
            },
            filename: None,
        };
        groups.push((key, Vec::new()));
    }

    let mut result = Vec::new();
    for (key, entries) in groups {
        let mut version = key.version.clone();
        if version.is_none() && all_versions.len() == 1 {
            version = Some(all_versions[0].clone());
        }
        let mut group_languages = key.languages.clone();
        if group_languages.is_empty() {
            group_languages = language(text);
        }
        let observed = observed_format(&entries);
        let mut warnings = Vec::new();
        if !matches!(reported, "unknown" | "web")
            && !matches!(observed.as_str(), "unknown" | "web")
            && observed != reported
        {
            warnings.push("artifact_format_conflict".to_string());
        }
        if all_versions.len() > 1 && versioned_count <= 1 {
            warnings.push("version_conflict".to_string());
        }
        if variant == Variant::Dlc || dlc_conditions_unknown(text) {
            warnings.push("dlc_conditions_unknown".to_string());
        }
        if find_ci(text, "outside of japan").is_some() {
            warnings.push("outside_japan_only".to_string());
        }

        let version_identity = match (&version, &key.filename) {
            (Some(version), Some(filename)) if !filename.is_empty() => {
                format!("{version}:{filename}")
            }
            (Some(version), _) => version.clone(),
            (None, Some(filename)) if !filename.is_empty() => {
                format!("page-unknown:{filename}")
            }
            _ => "page-unknown".to_string(),
        };
        let identity = serde_json::Value::Array(vec![
            serde_json::Value::String(rule.id.clone()),
            serde_json::Value::String(canonical::canonical_url(page)),
            serde_json::Value::String(variant_str(variant).to_string()),
            serde_json::Value::Array(
                group_languages
                    .iter()
                    .cloned()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
            serde_json::Value::String(version_identity.clone()),
        ]);
        let candidate_id = canonical::candidate_id(&identity);

        let game_title_value = {
            let value = game_title(title);
            if !value.is_empty() {
                value
            } else if let Some(first) = alias_list.first() {
                first.clone()
            } else {
                title.to_string()
            }
        };
        let mut candidate = SourceCandidate {
            candidate_id,
            source_id: rule.id.clone(),
            source_record_id: canonical::canonical_url(page),
            game_appid: if has_ids { Some(appid) } else { None },
            parent_appid: if has_ids && variant == Variant::Dlc {
                Some(appid)
            } else {
                None
            },
            aliases: alias_list.clone(),
            game_title: game_title_value,
            patch_title: title.to_string(),
            variant,
            requires_dlc: None,
            excludes_dlc: None,
            platforms: platforms.clone(),
            patch_kind: "adult_content".to_string(),
            languages: group_languages.clone(),
            version_raw: version.clone(),
            version_evidence: all_versions.clone(),
            version_identity,
            published_at: None,
            patch_updated_at: if find_ci(text, "updated").is_some() || text.contains("更新") {
                date(text)
            } else {
                None
            },
            source_page: page.to_string(),
            upstream_page: if rule.role != SourceRole::FallbackIndexer {
                Some(page.to_string())
            } else {
                None
            },
            mirrors: entries.clone(),
            format_reported: parse_format(reported),
            format_observed: parse_format(&observed),
            capability: Capability::DiscoverOnly,
            identity_evidence: if has_ids {
                vec![IdentityEvidence {
                    kind: "steam_appid".to_string(),
                    appid: Some(appid),
                    page: Some(page.to_string()),
                }]
            } else {
                Vec::new()
            },
            provenance: vec![Provenance {
                source_id: rule.id.clone(),
                page: page.to_string(),
            }],
            match_basis: if has_ids {
                MatchBasis::Appid
            } else {
                MatchBasis::Title
            },
            observed_at: now_iso8601(),
            cache_status: CacheStatus::Fresh,
            warnings,
            fingerprint: String::new(),
            related_index_pages: Vec::new(),
        };
        if appid == 1227890 && rule.id == "kagura-cn" {
            if variant == Variant::Base && excludes_dlc_condition(text) {
                candidate.excludes_dlc = Some(vec![1390150]);
            } else if variant == Variant::Dlc && requires_dlc_condition(text) {
                candidate.requires_dlc = Some(vec![1390150]);
            }
        }
        if let Ok(value) = serde_json::to_value(&candidate) {
            candidate.fingerprint = canonical::fingerprint(&value);
        }
        result.push(candidate);
    }
    result
}

fn mirror_key(mirror: &Mirror) -> GroupKey {
    let decoded = percent_decode(&mirror.url);
    let languages = language(&decoded);
    let artifact = mirror.format == "zip" || mirror.format == "exe";
    let filename = if artifact {
        let path = validate::split_url(&mirror.url).path;
        let base = path.rsplit('/').next().unwrap_or("").to_string();
        Some(percent_decode(&base))
    } else {
        None
    };
    let version = match &filename {
        Some(filename) if !filename.is_empty() => {
            let found = versions(filename);
            if found.len() == 1 {
                Some(found[0].clone())
            } else {
                None
            }
        }
        _ => None,
    };
    GroupKey {
        languages,
        version,
        filename,
    }
}

fn observed_format(entries: &[Mirror]) -> String {
    let mut formats: Vec<String> = Vec::new();
    for entry in entries {
        if entry.format != "web" && !formats.contains(&entry.format) {
            formats.push(entry.format.clone());
        }
    }
    match formats.len() {
        1 => formats[0].clone(),
        count if count > 1 => "mixed".to_string(),
        _ if entries.is_empty() => "unknown".to_string(),
        _ => "web".to_string(),
    }
}

fn parse_format(value: &str) -> ArtifactFormat {
    match value {
        "zip" => ArtifactFormat::Zip,
        "exe" => ArtifactFormat::Exe,
        "web" => ArtifactFormat::Web,
        "mixed" => ArtifactFormat::Mixed,
        _ => ArtifactFormat::Unknown,
    }
}

fn variant_str(variant: Variant) -> &'static str {
    match variant {
        Variant::Base => "base",
        Variant::Dlc => "dlc",
        Variant::Demo => "demo",
        Variant::Unknown => "unknown",
    }
}

fn dlc_conditions_unknown(text: &str) -> bool {
    dlc_line_condition(text, &["没", "沒", "未"])
}

fn excludes_dlc_condition(text: &str) -> bool {
    dlc_line_condition(text, &["没", "沒"])
}

fn requires_dlc_condition(text: &str) -> bool {
    dlc_line_condition(text, &["已经", "已經", "必须", "必須", "已", "有"])
}

/// Shared `(?:markers).*?(?:购买|購買).*?DLC` matcher (`.*?` stays on one line).
fn dlc_line_condition(text: &str, markers: &[&str]) -> bool {
    for marker in markers {
        let mut search = 0;
        while let Some(position) = text[search..].find(marker) {
            let absolute = search + position;
            let rest = &text[absolute + marker.len()..];
            let line_end = rest.find('\n').unwrap_or(rest.len());
            let line = &rest[..line_end];
            if let Some(buy) = find_buy(line) {
                if find_ci(&line[buy..], "DLC").is_some() {
                    return true;
                }
            }
            search = absolute + marker.len();
        }
    }
    if let Some(position) = find_ci(text, "without") {
        let rest = &text[position..];
        let line_end = rest.find('\n').unwrap_or(rest.len());
        if find_ci(&rest[..line_end], "DLC").is_some() {
            return true;
        }
    }
    false
}

fn find_buy(text: &str) -> Option<usize> {
    text.find("购买").or_else(|| text.find("購買"))
}

fn gog_only(text: &str) -> bool {
    for marker in ["GOG", "Android", "DLsite"] {
        let mut search = 0;
        while let Some(position) = find_ci(&text[search..], marker) {
            let absolute = search + position;
            let rest = &text[absolute + marker.len()..];
            let line_end = rest.find('\n').unwrap_or(rest.len());
            let line = &rest[..line_end];
            if find_ci(line, "only").is_some() || line.contains("专用") || line.contains("專用")
            {
                return true;
            }
            search = absolute + marker.len();
        }
    }
    false
}

// ---- validate / enrich / merge / fallback --------------------------------

/// Port of `discovery.py::validate_candidate` for a single candidate.
pub fn validate_candidate(candidate: &SourceCandidate, rule: &SourceRule) -> Result<()> {
    if !valid_candidate_id(&candidate.candidate_id) {
        return Err(invalid("invalid_discovery_candidate"));
    }
    for value in [
        &candidate.source_id,
        &candidate.source_record_id,
        &candidate.game_title,
        &candidate.patch_title,
        &candidate.version_identity,
    ] {
        if value.is_empty() || value.chars().count() > 2048 {
            return Err(invalid("invalid_discovery_text"));
        }
    }
    for values in [
        &candidate.aliases,
        &candidate.languages,
        &candidate.platforms,
        &candidate.warnings,
        &candidate.version_evidence,
    ] {
        if values.len() > 64 || values.iter().any(|value| value.chars().count() > 1024) {
            return Err(invalid("invalid_discovery_list"));
        }
    }
    for values in [&candidate.requires_dlc, &candidate.excludes_dlc]
        .into_iter()
        .flatten()
    {
        if values.len() > 32 || values.contains(&0) {
            return Err(invalid("invalid_discovery_dlc"));
        }
    }
    if candidate.source_id != rule.id {
        return Err(invalid("invalid_discovery_source"));
    }
    validate::allowed_url(&candidate.source_page, &rule.page_hosts, false)
        .map_err(|_| invalid("invalid_discovery_url"))?;
    if let Some(upstream) = &candidate.upstream_page {
        if !upstream.is_empty() {
            validate::navigation_url(upstream).map_err(|_| invalid("invalid_discovery_url"))?;
        }
    }
    if candidate.mirrors.len() > 64 {
        return Err(invalid("invalid_discovery_mirrors"));
    }
    for mirror in &candidate.mirrors {
        if !matches!(mirror.format.as_str(), "zip" | "exe" | "web" | "unknown") {
            return Err(invalid("invalid_discovery_mirror"));
        }
        validate::navigation_url(&mirror.url).map_err(|_| invalid("invalid_discovery_url"))?;
    }
    if candidate.game_appid == Some(0) || candidate.parent_appid == Some(0) {
        return Err(invalid("invalid_discovery_appid"));
    }
    if matched_basis(candidate.match_basis) && candidate.game_appid.is_none() {
        return Err(invalid("invalid_discovery_identity"));
    }
    Ok(())
}

fn valid_candidate_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("candidate-") else {
        return false;
    };
    rest.len() == 24
        && rest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A verified upstream identity used to enrich title-only candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub appid: u32,
    pub page: String,
    pub aliases: Vec<String>,
}

/// Port of `discovery.py::enrich_candidates`.
pub fn enrich_candidates(
    candidates: &[SourceCandidate],
    identities: &[Identity],
) -> Vec<SourceCandidate> {
    let mut result = Vec::new();
    for original in candidates {
        let mut candidate = original.clone();
        if candidate.game_appid.is_none() {
            for identity in identities {
                let alias_set: HashSet<String> = identity
                    .aliases
                    .iter()
                    .map(|alias| validate::discovery_normal(alias))
                    .collect();
                if alias_set.contains(&validate::discovery_normal(&candidate.game_title)) {
                    candidate.game_appid = Some(identity.appid);
                    candidate.match_basis = MatchBasis::VerifiedAlias;
                    candidate.identity_evidence.push(IdentityEvidence {
                        kind: "verified_alias".to_string(),
                        appid: Some(identity.appid),
                        page: Some(identity.page.clone()),
                    });
                    break;
                }
            }
        }
        result.push(candidate);
    }
    result
}

/// Port of `discovery.py::merge_candidates`: dedupe by `candidate_id`, unioning
/// `provenance`/`identity_evidence`/`warnings` (sorted-JSON dedupe).
pub fn merge_candidates(candidates: &[SourceCandidate]) -> Vec<SourceCandidate> {
    let mut order: Vec<SourceCandidate> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for candidate in candidates {
        if let Some(&position) = index.get(&candidate.candidate_id) {
            let existing = &mut order[position];
            let mut provenance = existing.provenance.clone();
            provenance.extend(candidate.provenance.iter().cloned());
            existing.provenance = dedupe_json(provenance);
            let mut evidence = existing.identity_evidence.clone();
            evidence.extend(candidate.identity_evidence.iter().cloned());
            existing.identity_evidence = dedupe_json(evidence);
            let mut warnings = existing.warnings.clone();
            warnings.extend(candidate.warnings.iter().cloned());
            existing.warnings = dedupe_json(warnings);
        } else {
            index.insert(candidate.candidate_id.clone(), order.len());
            order.push(candidate.clone());
        }
    }
    order
}

fn dedupe_json<T: serde::Serialize + Clone>(items: Vec<T>) -> Vec<T> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let key = serde_json::to_value(&item)
            .map(|value| canonical::python_json(&value, true, false))
            .unwrap_or_default();
        if seen.insert(key) {
            out.push(item);
        }
    }
    out
}

/// Port of `discovery.py::needs_fallback`.
pub fn needs_fallback(candidates: &[SourceCandidate], query: &QueryContext) -> bool {
    let official: Vec<&SourceCandidate> = candidates
        .iter()
        .filter(|candidate| {
            candidate.source_id != "hgamer"
                && matched_basis(candidate.match_basis)
                && candidate.cache_status != CacheStatus::Stale
                && candidate.variant == Variant::Base
                && !candidate
                    .warnings
                    .iter()
                    .any(|warning| warning == "dlc_conditions_unknown")
        })
        .collect();
    let official: Vec<&SourceCandidate> =
        if let Some(preferred) = query.preferred_language.as_deref() {
            official
                .into_iter()
                .filter(|candidate| {
                    candidate
                        .languages
                        .iter()
                        .any(|language| language == preferred)
                })
                .collect()
        } else {
            official
        };
    if official.is_empty() {
        return true;
    }
    let mut counts: HashMap<(String, Vec<String>), usize> = HashMap::new();
    for candidate in &official {
        *counts
            .entry((candidate.source_id.clone(), candidate.languages.clone()))
            .or_insert(0) += 1;
    }
    if counts.values().any(|count| *count > 1) {
        return true;
    }
    candidates.iter().any(|candidate| {
        candidate.source_id != "hgamer"
            && (candidate.variant == Variant::Dlc
                || candidate
                    .warnings
                    .iter()
                    .any(|warning| warning == "dlc_conditions_unknown"))
    })
}

fn matched_basis(basis: MatchBasis) -> bool {
    matches!(
        basis,
        MatchBasis::Appid | MatchBasis::VerifiedAlias | MatchBasis::OfficialTitle
    )
}

// ---- small string primitives ---------------------------------------------

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn skip_ws(chars: &[char], mut index: usize) -> usize {
    while index < chars.len() && chars[index].is_whitespace() {
        index += 1;
    }
    index
}

fn seq_equals(chars: &[char], index: usize, pattern: &str) -> bool {
    pattern
        .chars()
        .enumerate()
        .all(|(offset, expected)| chars.get(index + offset) == Some(&expected))
}

fn seq_equals_ci(chars: &[char], index: usize, pattern: &str) -> bool {
    pattern.chars().enumerate().all(|(offset, expected)| {
        chars
            .get(index + offset)
            .is_some_and(|actual| actual.eq_ignore_ascii_case(&expected))
    })
}

fn contains_ascii_alpha(text: &str) -> bool {
    text.bytes().any(|b| b.is_ascii_alphabetic())
}

fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_ascii() {
        haystack
            .to_ascii_lowercase()
            .find(&needle.to_ascii_lowercase())
    } else {
        haystack.find(needle)
    }
}

fn word_bounded_ci(text: &str, keyword: &str) -> bool {
    let mut search = 0;
    while let Some(position) = find_ci(&text[search..], keyword) {
        let absolute = search + position;
        let before = text[..absolute].chars().next_back();
        let after = text[absolute + keyword.len()..].chars().next();
        if before.is_none_or(|c| !is_word_char(c)) && after.is_none_or(|c| !is_word_char(c)) {
            return true;
        }
        search = absolute + keyword.len();
    }
    false
}

fn cn_patch(text: &str, letters: &str) -> bool {
    let mut search = 0;
    while let Some(position) = find_ci(&text[search..], letters) {
        let absolute = search + position;
        let before_ok = !text[..absolute]
            .chars()
            .next_back()
            .is_some_and(is_word_char);
        if before_ok {
            let after = &text[absolute + letters.len()..];
            let after = after
                .strip_prefix(' ')
                .or_else(|| after.strip_prefix('-'))
                .unwrap_or(after);
            if let Some(rest) = strip_prefix_ci(after, "patch") {
                if !rest.chars().next().is_some_and(is_word_char) {
                    return true;
                }
            }
        }
        search = absolute + letters.len();
    }
    false
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let candidate = text.get(..prefix.len())?;
    if candidate.eq_ignore_ascii_case(prefix) {
        Some(&text[prefix.len()..])
    } else {
        None
    }
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                out.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn now_iso8601() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    iso8601_utc(seconds)
}

fn iso8601_utc(seconds: u64) -> String {
    let days = seconds / 86_400;
    let remainder = seconds % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00",
        year,
        month,
        day,
        remainder / 3600,
        (remainder % 3600) / 60,
        remainder % 60
    )
}

/// Howard Hinnant's `civil_from_days` (days since 1970-01-01).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule() -> SourceRule {
        SourceRule {
            id: "kagura-cn".to_string(),
            name: "Kagura".to_string(),
            adapter: "kagura_wordpress".to_string(),
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

    fn mirror(url: &str, format: &str) -> Mirror {
        Mirror {
            url: url.to_string(),
            label: None,
            format: format.to_string(),
        }
    }

    fn sample(
        id: char,
        source_id: &str,
        appid: Option<u32>,
        basis: MatchBasis,
        variant: Variant,
        warnings: Vec<String>,
    ) -> SourceCandidate {
        SourceCandidate {
            candidate_id: format!("candidate-{}", id.to_string().repeat(24)),
            source_id: source_id.to_string(),
            source_record_id: "https://kagurafan.com/game/".to_string(),
            game_appid: appid,
            parent_appid: None,
            aliases: Vec::new(),
            game_title: "Game".to_string(),
            patch_title: "Game".to_string(),
            variant,
            requires_dlc: None,
            excludes_dlc: None,
            platforms: vec!["steam".to_string()],
            patch_kind: "adult_content".to_string(),
            languages: vec!["en".to_string()],
            version_raw: Some("1".to_string()),
            version_evidence: vec!["1".to_string()],
            version_identity: "1".to_string(),
            published_at: None,
            patch_updated_at: None,
            source_page: "https://kagurafan.com/game/".to_string(),
            upstream_page: None,
            mirrors: Vec::new(),
            format_reported: ArtifactFormat::Unknown,
            format_observed: ArtifactFormat::Unknown,
            capability: Capability::DiscoverOnly,
            identity_evidence: Vec::new(),
            provenance: vec![Provenance {
                source_id: source_id.to_string(),
                page: "https://kagurafan.com/game/".to_string(),
            }],
            match_basis: basis,
            observed_at: "2026-01-01T00:00:00+00:00".to_string(),
            cache_status: CacheStatus::Fresh,
            warnings,
            fingerprint: "fp".to_string(),
            related_index_pages: Vec::new(),
        }
    }

    #[test]
    fn script_conversion_and_query_terms() {
        assert_eq!(convert("游戏", Dictionary::St), "遊戲");
        assert_eq!(convert("遊戲", Dictionary::Ts), "游戏");
        assert_eq!(convert("没有", Dictionary::St), "沒有");

        assert_eq!(query_terms("遊戲", &[]), vec!["遊戲", "游戏"]);
        assert_eq!(query_terms("游戏", &[]), vec!["游戏", "遊戲"]);
        assert_eq!(
            query_terms("遊戲", &["Game".to_string()]),
            vec!["遊戲", "Game", "游戏"]
        );
        let long = "x".repeat(300);
        assert_eq!(query_terms(&long, &[])[0].chars().count(), 256);
        assert!(query_terms("", &[]).is_empty());
    }

    #[test]
    fn title_variant_and_base_title_match_python() {
        assert_eq!(game_title("Game 官方工口补丁"), "Game");
        assert_eq!(game_title("Game patch"), "Game");
        assert_eq!(game_title("Game - 补丁"), "Game");
        assert_eq!(game_title("Game 中文无码补丁包"), "Game");
        assert_eq!(game_title("Game DLC"), "Game DLC");

        assert_eq!(variant("Game + DLC"), Variant::Dlc);
        assert_eq!(variant("Game Demo"), Variant::Demo);
        assert_eq!(variant("Game"), Variant::Base);
        // Japanese 験 is not in 体[验驗]版, so Python keeps it base.
        assert_eq!(variant("体験版"), Variant::Base);
        assert_eq!(variant("EXPANSION"), Variant::Dlc);
        assert_eq!(variant("Gameplus"), Variant::Base);
        assert_eq!(variant("DemoGame"), Variant::Base);

        assert_eq!(base_title("Game + DLC"), "Game");
        assert_eq!(base_title("Game Demo"), "Game Demo");
        assert_eq!(base_title("Game – Expansion"), "Game");
        assert_eq!(base_title("Game-Plus"), "Game");
    }

    #[test]
    fn versions_dates_and_languages_match_python() {
        assert_eq!(versions("ver1.2.3"), vec!["1.2.3"]);
        assert_eq!(versions("v 1.2"), vec!["1.2"]);
        assert_eq!(versions("提取码: v103"), Vec::<String>::new());
        assert_eq!(versions("sv1.0"), Vec::<String>::new());
        assert_eq!(versions("Ver. 2.0 released"), vec!["2.0"]);
        assert_eq!(versions("v1.2.3.4.5.6"), vec!["1.2.3.4.5"]);
        assert_eq!(versions("v1"), vec!["1"]);

        assert_eq!(date("updated 2024.1.2").as_deref(), Some("2024-01-02"));
        assert_eq!(date("2024-13-01"), None);
        assert_eq!(date("2024/02/29").as_deref(), Some("2024-02-29"));
        assert_eq!(date("2023/02/29"), None);
        assert_eq!(date("2024.1.2 and 2025.3.4").as_deref(), Some("2024-01-02"));

        assert_eq!(language("/CN/ patch"), vec!["zh-CN"]);
        assert_eq!(language("CN Patch"), vec!["zh-CN"]);
        assert_eq!(language("简体中文"), vec!["zh-CN"]);
        assert_eq!(language("繁體中文"), vec!["zh-TW"]);
        assert_eq!(language("/EN/ English"), vec!["en"]);
        assert_eq!(language("TCN"), vec!["zh-TW"]);
        assert_eq!(language("/JP/ Japanese"), vec!["ja"]);

        assert_eq!(format("https://x/a.zip"), "zip");
        assert_eq!(format("https://x/a.EXE"), "exe");
        assert_eq!(format("https://x/a.html"), "web");
        assert_eq!(
            steam_ids(&[
                "https://store.steampowered.com/app/1227890/Game/".to_string(),
                "https://store.steampowered.com/widget/42/".to_string(),
                "https://store.steampowered.com/app/1227890/Game/".to_string(),
                "https://other.example/app/1".to_string(),
            ]),
            vec![1227890, 42]
        );
    }

    #[test]
    fn title_relevant_and_aliases() {
        let q = query(42, "Game");
        assert!(title_relevant("Game", &q));
        assert!(title_relevant("Game patch", &q));
        assert!(title_relevant("Game + DLC", &q));
        assert!(!title_relevant("Other", &q));

        assert_eq!(
            aliases("Game 官方工口补丁", &["又称：A·B・C".to_string()]),
            vec!["Game", "A", "B", "C"]
        );
        assert_eq!(aliases("Game", &["no alias".to_string()]), vec!["Game"]);
    }

    #[test]
    fn build_candidates_identity_and_fingerprint() {
        let rule = rule();
        let page = "https://kagurafan.com/game/";
        let mirror = mirror("https://kagurafan.com/dl/game_v1.2.3.zip", "zip");
        let built = build_candidates(
            &rule,
            "Game",
            page,
            "steam",
            &[1227890],
            &["Game".to_string()],
            "zip",
            &[mirror],
            &query(1227890, "Game"),
        );
        assert_eq!(built.len(), 1);
        let candidate = &built[0];
        assert_eq!(candidate.variant, Variant::Base);
        assert_eq!(candidate.game_appid, Some(1227890));
        assert_eq!(candidate.match_basis, MatchBasis::Appid);
        assert_eq!(candidate.version_raw.as_deref(), Some("1.2.3"));
        assert_eq!(candidate.version_identity, "1.2.3:game_v1.2.3.zip");
        assert_eq!(candidate.capability, Capability::DiscoverOnly);
        assert_eq!(candidate.fingerprint, {
            let value = serde_json::to_value(candidate).unwrap();
            canonical::fingerprint(&value)
        });
        assert!(candidate.candidate_id.starts_with("candidate-"));
        assert_eq!(candidate.candidate_id.len(), 10 + 24);
    }

    #[test]
    fn build_candidates_groups_artifacts_and_attaches_web_mirrors() {
        let rule = rule();
        let built = build_candidates(
            &rule,
            "Game",
            "https://kagurafan.com/game/",
            "steam v1.0",
            &[1227890],
            &["Game".to_string()],
            "zip",
            &[
                mirror("https://a.example/game_v1.0.zip", "zip"),
                mirror("https://b.example/other/game_v1.0.zip", "zip"),
                mirror("https://a.example/page", "web"),
            ],
            &query(1227890, "Game"),
        );
        assert_eq!(built.len(), 1);
        assert_eq!(built[0].mirrors.len(), 3);
        assert_eq!(built[0].version_raw.as_deref(), Some("1.0"));
        assert_eq!(built[0].version_identity, "1.0:game_v1.0.zip");
        assert_eq!(built[0].format_observed, ArtifactFormat::Zip);
    }

    #[test]
    fn build_candidates_warnings() {
        let rule = rule();
        let built = build_candidates(
            &rule,
            "Game",
            "https://kagurafan.com/game/",
            "v1.0 v2.0 outside of Japan",
            &[1227890],
            &["Game".to_string()],
            "exe",
            &[mirror("https://kagurafan.com/dl/game_v1.0.zip", "zip")],
            &query(1227890, "Game"),
        );
        assert_eq!(built.len(), 1);
        let warnings = &built[0].warnings;
        assert!(warnings.iter().any(|w| w == "artifact_format_conflict"));
        assert!(warnings.iter().any(|w| w == "version_conflict"));
        assert!(warnings.iter().any(|w| w == "outside_japan_only"));
    }

    #[test]
    fn build_candidates_dlc_and_kagura_special_case() {
        let rule = rule();
        let excludes = build_candidates(
            &rule,
            "Game",
            "https://kagurafan.com/game/",
            "本作沒有购买DLC",
            &[1227890],
            &["Game".to_string()],
            "zip",
            &[mirror("https://kagurafan.com/dl/game.zip", "zip")],
            &query(1227890, "Game"),
        );
        assert_eq!(excludes[0].excludes_dlc, Some(vec![1390150]));
        assert!(excludes[0]
            .warnings
            .iter()
            .any(|w| w == "dlc_conditions_unknown"));

        let requires = build_candidates(
            &rule,
            "Game + DLC",
            "https://kagurafan.com/game/",
            "已经购买DLC",
            &[1227890],
            &["Game".to_string()],
            "zip",
            &[mirror("https://kagurafan.com/dl/game.zip", "zip")],
            &query(1227890, "Game"),
        );
        assert_eq!(requires[0].variant, Variant::Dlc);
        assert_eq!(requires[0].requires_dlc, Some(vec![1390150]));
    }

    #[test]
    fn build_candidates_rejects_mismatched_appid_and_demo() {
        let rule = rule();
        assert!(build_candidates(
            &rule,
            "Game",
            "https://kagurafan.com/game/",
            "steam",
            &[42],
            &["Game".to_string()],
            "unknown",
            &[],
            &query(999, "Game"),
        )
        .is_empty());
        assert!(build_candidates(
            &rule,
            "Game Demo",
            "https://kagurafan.com/game/",
            "",
            &[999],
            &["Game".to_string()],
            "unknown",
            &[],
            &query(999, "Game"),
        )
        .is_empty());
    }

    #[test]
    fn enrich_merge_and_validate_candidate() {
        let rule = rule();
        let title_only = sample(
            'a',
            "kagura-cn",
            None,
            MatchBasis::Title,
            Variant::Base,
            Vec::new(),
        );
        let enriched = enrich_candidates(
            &[title_only],
            &[Identity {
                appid: 42,
                page: "https://kagurafan.com/game/".to_string(),
                aliases: vec!["Game".to_string()],
            }],
        );
        assert_eq!(enriched[0].game_appid, Some(42));
        assert_eq!(enriched[0].match_basis, MatchBasis::VerifiedAlias);
        assert_eq!(enriched[0].identity_evidence.len(), 1);

        let first = sample(
            'b',
            "kagura-cn",
            Some(42),
            MatchBasis::Appid,
            Variant::Base,
            vec!["one".to_string()],
        );
        let mut second = first.clone();
        second.warnings = vec!["two".to_string()];
        second.provenance = vec![Provenance {
            source_id: "hgamer".to_string(),
            page: "https://hgamer.tw/zh-CN/patches/x".to_string(),
        }];
        let merged = merge_candidates(&[first.clone(), second]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].warnings, vec!["one", "two"]);
        assert_eq!(merged[0].provenance.len(), 2);

        assert!(validate_candidate(&first, &rule).is_ok());
        let mut invalid = first.clone();
        invalid.candidate_id = "candidate-zz".to_string();
        assert_eq!(
            validate_candidate(&invalid, &rule).unwrap_err().to_string(),
            "invalid_discovery_candidate"
        );
        let mut no_appid = first.clone();
        no_appid.game_appid = None;
        assert_eq!(
            validate_candidate(&no_appid, &rule)
                .unwrap_err()
                .to_string(),
            "invalid_discovery_identity"
        );
        let wrong_source = sample(
            'c',
            "hgamer",
            Some(42),
            MatchBasis::Appid,
            Variant::Base,
            Vec::new(),
        );
        assert_eq!(
            validate_candidate(&wrong_source, &rule)
                .unwrap_err()
                .to_string(),
            "invalid_discovery_source"
        );
        // Capability is always discover_only: candidates never grant installability.
        assert_eq!(first.capability, Capability::DiscoverOnly);
    }

    #[test]
    fn hgamer_identity_upgrades_kagura_candidate_without_changing_id() {
        let kagura = sample(
            'd',
            "kagura-cn",
            None,
            MatchBasis::Title,
            Variant::Base,
            Vec::new(),
        );
        let original_id = kagura.candidate_id.clone();
        let enriched = enrich_candidates(
            &[kagura],
            &[Identity {
                appid: 2303800,
                page: "https://hgamer.tw/zh-TW/games/my-life-as-a-cult-leader".to_string(),
                aliases: vec!["Game".to_string(), "妃神会秘史".to_string()],
            }],
        );
        assert_eq!(enriched.len(), 1);
        // Enrichment must never rewrite the artifact identity.
        assert_eq!(enriched[0].candidate_id, original_id);
        assert_eq!(enriched[0].game_appid, Some(2303800));
        assert_eq!(enriched[0].match_basis, MatchBasis::VerifiedAlias);
        assert_eq!(enriched[0].identity_evidence.len(), 1);
        assert_eq!(enriched[0].identity_evidence[0].kind, "verified_alias");
        assert_eq!(enriched[0].identity_evidence[0].appid, Some(2303800));
        assert_eq!(
            enriched[0].identity_evidence[0].page.as_deref(),
            Some("https://hgamer.tw/zh-TW/games/my-life-as-a-cult-leader")
        );
    }

    #[test]
    fn needs_fallback_matches_python() {
        let query = query(42, "Game");
        assert!(needs_fallback(&[], &query));
        let official = sample(
            'a',
            "kagura-cn",
            Some(42),
            MatchBasis::Appid,
            Variant::Base,
            Vec::new(),
        );
        assert!(!needs_fallback(std::slice::from_ref(&official), &query));
        let dlc = sample(
            'b',
            "kagura-cn",
            Some(42),
            MatchBasis::Appid,
            Variant::Dlc,
            Vec::new(),
        );
        assert!(needs_fallback(&[official.clone(), dlc], &query));
        // A gated official (dlc_conditions_unknown) always needs fallback.
        let gated = sample(
            'c',
            "kagura-cn",
            Some(42),
            MatchBasis::Appid,
            Variant::Base,
            vec!["dlc_conditions_unknown".to_string()],
        );
        assert!(!needs_fallback(&[official], &query));
        assert!(needs_fallback(&[gated], &query));
    }
}
