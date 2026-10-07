"""Bounded public-HTML discovery. Metadata never grants download authority.

html.parser is imported lazily: Decky's frozen interpreter uses this module's
models, while the isolated system-Python worker performs HTML parsing.
"""
import hashlib
import ipaddress
import json
from pathlib import Path
import re
import time
import unicodedata
from datetime import datetime, timezone
from urllib.parse import parse_qs, unquote, urlencode, urljoin, urlsplit, urlunsplit
from urllib.request import HTTPRedirectHandler, HTTPSHandler, Request, build_opener
from urllib.error import HTTPError, URLError

MAX_HTML = 4 * 1024 * 1024
PARSER_VERSION = 2
MATCHED_BASES = {"appid", "verified_alias", "official_title"}


def canonical_url(url):
    p = urlsplit(url)
    return urlunsplit((p.scheme, p.netloc.lower(), p.path, "", ""))


def safe_url(url, hosts=None):
    if not isinstance(url, str) or url != url.strip() or any(ord(c) < 33 for c in url):
        raise ValueError("invalid_discovery_url")
    p = urlsplit(url)
    try:
        port = p.port
        ipaddress.ip_address(p.hostname or "")
    except ValueError:
        port = p.port
    else:
        raise ValueError("invalid_discovery_host")
    host = p.hostname or ""
    if (p.scheme != "https" or not re.fullmatch(r"[a-z0-9.-]+", host)
            or "." not in host or host.endswith((".local", ".localhost", "."))
            or ".." in host or not re.search(r"[a-z]", host) or p.username or p.password or port not in (None, 443)
            or (hosts is not None and host not in hosts)):
        raise ValueError("invalid_discovery_url")
    return url


class _Redirect(HTTPRedirectHandler):
    def __init__(self, hosts):
        self.hosts = hosts

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        safe_url(newurl, self.hosts)
        # Ubuntu 22.04's system Python lacks urllib's HTTP 308 handler.
        # Our discovery requests are GET; preserve them as for HTTP 307.
        return super().redirect_request(req, fp, 307 if code == 308 else code, msg, headers, newurl)

    http_error_308 = HTTPRedirectHandler.http_error_302


class RequestContext:
    """One game's shared request budget, including upstream delegation."""
    def __init__(self, fetcher=None, budget=18, deadline=None, interval=1.0):
        self.fetcher = fetcher
        self.budget = budget
        self.deadline = deadline if deadline is not None else time.monotonic() + 30
        self.interval = interval
        self.visited = set()
        self.responses = {}
        self.last_request = {}
        self.requests = 0
        self.detail_requests = 0

    def fetch(self, url, rule):
        hosts = rule.get("page_hosts", [])
        safe_url(url, hosts)
        if url in self.responses:
            return self.responses[url]
        if url in self.visited:
            raise ValueError("discovery_request_cycle")
        remaining = self.deadline - time.monotonic()
        if self.requests >= self.budget or remaining <= 0:
            raise ValueError("discovery_budget_exhausted")
        host = urlsplit(url).hostname
        wait = self.interval - (time.monotonic() - self.last_request.get(host, 0))
        if wait > 0:
            if wait >= remaining:
                raise ValueError("discovery_budget_exhausted")
            time.sleep(wait)
        self.visited.add(url)
        self.requests += 1
        self.last_request[host] = time.monotonic()
        timeout = min(15, self.deadline - time.monotonic())
        for attempt in range(2):
            retry_headers = {}
            try:
                if self.fetcher:
                    result = self.fetcher(url, rule)
                else:
                    # urllib's default HTTPSHandler verifies the certificate chain.
                    opener = build_opener(_Redirect(hosts), HTTPSHandler())
                    req = Request(url, headers={"User-Agent": "Mozilla/5.0 SteamPatchDiscovery/1",
                                                "Accept": "text/html", "Accept-Encoding": "identity"})
                    with opener.open(req, timeout=timeout) as response:
                        raw = response.read(MAX_HTML + 1)
                        if len(raw) > MAX_HTML:
                            raise ValueError("discovery_html_too_large")
                        if response.headers.get("Content-Encoding", "identity").lower() not in ("", "identity"):
                            raise ValueError("discovery_compressed_response")
                        result = (response.status, raw.decode("utf-8", errors="replace"),
                                  dict(response.headers), response.geturl())
                if result[0] not in (429, 500, 502, 503, 504):
                    break
                retry_headers = result[2]
            except HTTPError as error:
                if error.code not in (429, 500, 502, 503, 504):
                    raise
                retry_headers = dict(error.headers)
                if attempt:
                    raise
            except (URLError, TimeoutError, OSError):
                if attempt:
                    raise
            if attempt or self.requests >= self.budget:
                raise ValueError("discovery_retry_exhausted")
            retry_after = next((v for k, v in retry_headers.items() if k.lower() == "retry-after"), "1")
            try:
                wait = max(self.interval, float(retry_after))
            except (ValueError, TypeError):
                from email.utils import parsedate_to_datetime
                try:
                    wait = max(self.interval, parsedate_to_datetime(retry_after).timestamp() - time.time())
                except (ValueError, TypeError):
                    wait = max(self.interval, 1)
            if wait >= self.deadline - time.monotonic():
                raise ValueError("discovery_retry_deferred")
            time.sleep(wait)
            self.requests += 1
            self.last_request[host] = time.monotonic()
            timeout = min(15, self.deadline - time.monotonic())
        status, html, headers, final = result
        safe_url(final, hosts)
        if status != 200:
            raise ValueError("discovery_http_" + str(status))
        if not isinstance(html, str) or len(html.encode("utf-8")) > MAX_HTML:
            raise ValueError("discovery_html_too_large")
        if re.search(r"<title[^>]*>\s*(?:Just a moment|Access denied|Captcha)", html, re.I):
            raise ValueError("discovery_challenge")
        if time.monotonic() > self.deadline:
            raise ValueError("discovery_budget_exhausted")
        self.responses[url] = result
        return result


def _context(value):
    if isinstance(value, RequestContext):
        return value
    if isinstance(value, dict):
        if "_context" not in value:
            value["_context"] = RequestContext(value.get("fetcher"), value.get("budget", 18),
                                                value.get("deadline"), value.get("interval", 1.0))
        return value["_context"]
    return RequestContext()


class Node:
    def __init__(self, tag, attrs=(), parent=None):
        self.tag, self.attrs, self.parent, self.children = tag, dict(attrs), parent, []

    def walk(self):
        yield self
        for child in self.children:
            if isinstance(child, Node):
                yield from child.walk()

    def text(self):
        if self.tag in ("script", "style"):
            return ""
        return " ".join(c.text() if isinstance(c, Node) else c for c in self.children).strip()

    def has(self, cls):
        return cls in self.attrs.get("class", "").split()

    def links(self):
        return [(n.text(), n.attrs["href"]) for n in self.walk()
                if n.tag == "a" and "href" in n.attrs]


def parse_html(html):
    from html.parser import HTMLParser
    class Parser(HTMLParser):
        VOID = set("area base br col embed hr img input link meta param source track wbr".split())

        def __init__(self):
            super().__init__(convert_charrefs=True)
            self.root = self.cur = Node("root")

        def handle_starttag(self, tag, attrs):
            n = Node(tag, attrs, self.cur)
            self.cur.children.append(n)
            if tag not in self.VOID:
                self.cur = n

        def handle_startendtag(self, tag, attrs):
            self.handle_starttag(tag, attrs)
            if tag not in self.VOID:
                self.handle_endtag(tag)

        def handle_endtag(self, tag):
            n = self.cur
            while n.parent is not None:
                if n.tag == tag:
                    self.cur = n.parent
                    return
                n = n.parent

        def handle_data(self, data):
            self.cur.children.append(data)
    parser = Parser()
    parser.feed(html)
    return parser.root


def _first(node, predicate):
    return next((n for n in node.walk() if predicate(n)), None)


def _normal(text):
    text = unicodedata.normalize("NFKC", text).casefold()
    return "".join(c for c in text if c == "+" or unicodedata.category(c)[0] in {"L", "N"})


def game_title(title):
    # Remove only established descriptive suffixes. +, DLC and Demo survive.
    return re.sub(r"\s*(?:[-–—：:]\s*)?(?:官方工口[补補]丁|中文[无無][码碼][补補]丁(?:包)?|[无無][码碼][补補]丁(?:包)?|[补補]丁|\bpatch)\s*$",
                  "", title, flags=re.I).strip()


def _variant(title):
    if re.search(r"(?<![A-Za-z0-9])demo(?![A-Za-z0-9])|体[验驗]版", title, re.I):
        return "demo"
    if re.search(r"\+|(?<![A-Za-z0-9])(?:dlc|expansion|append|plus)(?![A-Za-z0-9])", title, re.I):
        return "dlc"
    return "base"


def _base_title(title):
    return re.sub(r"(?:\+|\b(?:expansion|dlc|plus|append)\b|[–—-])", " ", game_title(title), flags=re.I).strip()


_CONVERSIONS = {}
def _convert(text, dictionary):
    if dictionary not in _CONVERSIONS:
        table = {}
        path = Path(__file__).with_name("vendor") / "opencc" / (dictionary + ".txt")
        for line in path.read_text(encoding="utf-8").splitlines():
            if line and not line.startswith("#"):
                key, targets = line.split("\t", 1)
                table[key] = targets.split()[0]
        _CONVERSIONS[dictionary] = table
    return "".join(_CONVERSIONS[dictionary].get(char, char) for char in text)


def query_terms(query):
    names = [query.get("name", "")] + query.get("name_hints", []) + query.get("aliases", [])
    names = [n.strip()[:256] for n in names if isinstance(n, str) and n.strip()]
    terms = []
    def add(n):
        if n and _normal(n) not in {_normal(t) for t in terms} and len(terms) < 3:
            terms.append(n)
    if names:
        add(names[0])
        # Verified / supplied Latin aliases precede script variants.
        for name in names[1:]:
            if re.search(r"[A-Za-z]", name):
                add(name)
                break
        for dictionary in ("STCharacters", "TSCharacters"):
            add(_convert(names[0], dictionary))
        for name in names[1:]:
            add(name)
    return terms


def _names(query):
    return [query.get("name", "")] + query.get("name_hints", []) + query.get("aliases", [])


def _title_relevant(title, query):
    target = {_normal(n) for n in _names(query) if n}
    return _normal(game_title(title)) in target or _normal(_base_title(title)) in target


def _steam_ids(node):
    ids = []
    for n in node.walk():
        url = n.attrs.get("href") if n.tag == "a" else n.attrs.get("src") if n.tag == "iframe" else None
        if url:
            p = urlsplit(url)
            if p.scheme == "https" and p.hostname == "store.steampowered.com":
                m = re.match(r"/(?:app|widget)/(\d+)(?:/|$)", p.path)
                if m and int(m[1]) not in ids:
                    ids.append(int(m[1]))
    return ids


def _aliases(node, title):
    names = [game_title(title)]
    for n in node.walk():
        text = n.text()
        if re.match(r"^(?:又称|又稱)[：:]", text) and len(text) < 1000:
            names.extend(t.strip() for t in re.split(r"[·・]", re.sub(r"^(?:又称|又稱)[：:]\s*", "", text)))
            break
    return list(dict.fromkeys(n for n in names if n and len(n) <= 256))


def _format(url):
    path = urlsplit(url).path.lower()
    return "exe" if path.endswith(".exe") else "zip" if path.endswith(".zip") else "web"


def _versions(text):
    # Share passwords such as '提取码: v103' are not release versions.
    text = re.sub(r"(?:提取[码碼]|密[码碼])[：:]?\s*[A-Za-z0-9.]+", "", text)
    return list(dict.fromkeys(re.findall(r"(?i)(?<![A-Za-z0-9])(?:ver\.?\s*|v\s*)(\d+(?:\.\d+){0,4})", text)))


def _date(text):
    match = re.search(r"\b(20\d{2})[./-](\d{1,2})[./-](\d{1,2})\b", text)
    if match:
        try:
            return datetime(*(int(n) for n in match.groups())).date().isoformat()
        except ValueError:
            pass
    return None


def _language(text):
    result = []
    for pattern, language in [(r"/CN/|\bCN[ -]Patch\b|(?<!繁體)(?<!繁体)中文|簡體中文|简体中文", "zh-CN"),
                              (r"/EN/|\bEN[ -]Patch\b|English", "en"),
                              (r"繁體中文|繁体中文|\bTCN\b", "zh-TW"),
                              (r"/JP/|Japanese", "ja")]:
        if re.search(pattern, text, re.I):
            result.append(language)
    return result


def _candidate(rule, title, page, body, query, *, ids=None, aliases=None, reported="unknown", mirrors=None):
    text = body.text() if body else ""
    ids = ids if ids is not None else _steam_ids(body) if body else []
    appid = int(query["appid"])
    if ids and appid not in ids:
        return []
    aliases = [a for a in (aliases or [game_title(title)]) if a]
    if not ids and not any(_title_relevant(a, query) for a in aliases):
        return []
    mirrors = mirrors or []
    variant = _variant(title)
    platforms = ["steam"] if ids or re.search(r"\bsteam\b", text, re.I) else []
    if variant == "demo" or (not ids and re.search(r"(?:GOG|Android|DLsite).*?(?:only|专用|專用)", text, re.I)):
        return []
    all_versions = _versions(text)
    groups = {}
    for mirror in mirrors:
        # The same version can have distinct artifacts. Mirrors with the same
        # stable basename share a group, independent of host/signature/order.
        filename = unquote(urlsplit(mirror["url"]).path.rsplit("/", 1)[-1]) if mirror["format"] in ("zip", "exe") else None
        versions = _versions(filename) if filename else []
        key = (tuple(_language(unquote(mirror["url"]))), versions[0] if len(versions) == 1 else None, filename)
        groups.setdefault(key, []).append(mirror)
    # Web mirrors with no version evidence attach only to a single version group.
    unversioned = [k for k in groups if k[1] is None and k[2] is None]
    versioned = [k for k in groups if k not in unversioned]
    if len(versioned) == 1:
        for key in unversioned:
            groups[versioned[0]].extend(groups.pop(key))
    if not groups:
        groups[(tuple(_language(text)), all_versions[0] if len(all_versions) == 1 else None, None)] = []
    result = []
    for (languages, version, filename), entries in groups.items():
        if version is None and len(all_versions) == 1:
            version = all_versions[0]
        if not languages:
            languages = tuple(_language(text))
        observed = list(dict.fromkeys(e["format"] for e in entries if e["format"] != "web"))
        observed = observed[0] if len(observed) == 1 else "mixed" if observed else "web" if entries else "unknown"
        warnings = []
        if reported not in ("unknown", "web") and observed not in ("unknown", "web", reported):
            warnings.append("artifact_format_conflict")
        if len(all_versions) > 1 and (len(versioned) <= 1):
            warnings.append("version_conflict")
        if variant == "dlc" or re.search(r"(?:没|沒|未).*?(?:购买|購買).*?DLC|without.*?DLC", text, re.I):
            warnings.append("dlc_conditions_unknown")
        if re.search(r"outside of Japan", text, re.I):
            warnings.append("outside_japan_only")
        version_identity = (version or "page-unknown") + (":" + filename if filename else "")
        identity = [rule["id"], canonical_url(page), variant, list(languages), version_identity]
        now = datetime.now(timezone.utc).isoformat()
        c = {"candidate_id": "candidate-" + hashlib.sha256(json.dumps(identity, ensure_ascii=False).encode()).hexdigest()[:24],
             "source_id": rule["id"], "source_record_id": canonical_url(page),
             "game_appid": appid if ids else None, "parent_appid": appid if ids and variant == "dlc" else None,
             "aliases": aliases, "game_title": game_title(title) or (aliases[0] if aliases else title), "patch_title": title,
             "variant": variant, "requires_dlc": None, "excludes_dlc": None, "platforms": platforms,
             "patch_kind": "adult_content", "languages": list(languages), "version_raw": version,
             "version_evidence": all_versions, "version_identity": version_identity,
             "published_at": None, "patch_updated_at": _date(text) if re.search(r"更新|updated", text, re.I) else None,
             "source_page": page, "upstream_page": page if rule.get("role") != "fallback_indexer" else None,
             "mirrors": entries, "format_reported": reported, "format_observed": observed,
             "capability": "discover_only", "identity_evidence": [{"kind": "steam_appid", "appid": appid, "page": page}] if ids else [],
             "provenance": [{"source_id": rule["id"], "page": page}],
             "match_basis": "appid" if ids else "title", "observed_at": now,
             "cache_status": "fresh", "warnings": warnings}
        # These are the design's verified sample conditions, anchored to the
        # exact game identity AND explicit article wording; never generic DLC rules.
        if appid == 1227890 and rule["id"] == "kagura-cn":
            if variant == "base" and re.search(r"(?:没|沒).*?(?:购买|購買).*?DLC", text, re.I):
                c["excludes_dlc"] = [1390150]
            elif variant == "dlc" and re.search(r"(?:已经|已經|必须|必須|已|有).*?(?:购买|購買).*?DLC", text, re.I):
                c["requires_dlc"] = [1390150]
        c["fingerprint"] = hashlib.sha256(json.dumps({k: v for k, v in c.items() if k != "observed_at"}, sort_keys=True).encode()).hexdigest()
        result.append(c)
    return result


def validate_candidate(candidate, rule=None):
    if not isinstance(candidate, dict) or candidate.get("capability") != "discover_only":
        raise ValueError("invalid_discovery_candidate")
    if not re.fullmatch(r"candidate-[0-9a-f]{24}", candidate.get("candidate_id", "")):
        raise ValueError("invalid_discovery_candidate")
    if candidate.get("match_basis") not in MATCHED_BASES | {"title", "none"}:
        raise ValueError("invalid_discovery_match_basis")
    if candidate.get("variant") not in {"base", "dlc", "demo", "unknown"}:
        raise ValueError("invalid_discovery_variant")
    for key in ("source_id", "source_record_id", "game_title", "patch_title", "version_identity"):
        if not isinstance(candidate.get(key), str) or not candidate[key] or len(candidate[key]) > 2048:
            raise ValueError("invalid_discovery_text")
    for key in ("aliases", "languages", "platforms", "warnings", "version_evidence"):
        values = candidate.get(key)
        if not isinstance(values, list) or len(values) > 64 or any(not isinstance(v, str) or len(v) > 1024 for v in values):
            raise ValueError("invalid_discovery_list")
    for key in ("requires_dlc", "excludes_dlc"):
        if candidate.get(key) is not None and (not isinstance(candidate[key], list) or len(candidate[key]) > 32
                                               or any(type(v) is not int or v <= 0 for v in candidate[key])):
            raise ValueError("invalid_discovery_dlc")
    if candidate.get("format_reported") not in {"zip", "exe", "web", "unknown", "mixed"} or candidate.get("format_observed") not in {"zip", "exe", "web", "unknown", "mixed"}:
        raise ValueError("invalid_discovery_format")
    if rule and candidate.get("source_id") != rule["id"]:
        raise ValueError("invalid_discovery_source")
    safe_url(candidate.get("source_page"), rule.get("page_hosts") if rule else None)
    if candidate.get("upstream_page"):
        safe_url(candidate["upstream_page"])
    mirrors = candidate.get("mirrors")
    if not isinstance(mirrors, list) or len(mirrors) > 64:
        raise ValueError("invalid_discovery_mirrors")
    for entry in mirrors:
        if not isinstance(entry, dict) or entry.get("format") not in {"zip", "exe", "web", "unknown"}:
            raise ValueError("invalid_discovery_mirror")
        safe_url(entry["url"])
    for key in ("game_appid", "parent_appid"):
        if candidate.get(key) is not None and (type(candidate[key]) is not int or candidate[key] <= 0):
            raise ValueError("invalid_discovery_appid")
    if candidate.get("match_basis") in MATCHED_BASES and candidate.get("game_appid") is None:
        raise ValueError("invalid_discovery_identity")
    return candidate


def enrich_candidates(candidates, identities):
    result = []
    for original in candidates:
        c = dict(original)
        if c.get("game_appid") is None:
            for identity in identities:
                aliases = {_normal(a) for a in identity.get("aliases", [])}
                if _normal(c["game_title"]) in aliases:
                    c["game_appid"] = identity["appid"]
                    c["match_basis"] = "verified_alias"
                    c["identity_evidence"] = c.get("identity_evidence", []) + [{"kind": "verified_alias", "page": identity["page"], "appid": identity["appid"]}]
                    break
        result.append(c)
    return result


def merge_candidates(candidates):
    merged = {}
    for c in candidates:
        key = c["candidate_id"]
        if key not in merged:
            merged[key] = dict(c)
        else:
            for field in ("provenance", "identity_evidence", "warnings"):
                merged[key][field] = list({json.dumps(x, sort_keys=True): x for x in merged[key].get(field, []) + c.get(field, [])}.values())
    return list(merged.values())


def needs_fallback(candidates, query):
    official = [c for c in candidates if c.get("source_id") != "hgamer" and c.get("match_basis") in MATCHED_BASES
                and c.get("cache_status") != "stale" and c.get("variant") == "base"
                and "dlc_conditions_unknown" not in c.get("warnings", [])]
    preferred = query.get("preferred_language")
    if preferred:
        official = [c for c in official if preferred in c.get("languages", [])]
    if not official:
        return True
    counts = {}
    for c in official:
        key = (c["source_id"], tuple(c.get("languages", [])))
        counts[key] = counts.get(key, 0) + 1
    return any(n > 1 for n in counts.values()) or any(
        c.get("variant") == "dlc" or "dlc_conditions_unknown" in c.get("warnings", [])
        for c in candidates if c.get("source_id") != "hgamer")


def _links(node, page, predicate):
    result = []
    for label, href in node.links():
        url = urljoin(page, href)
        if predicate(label, url):
            try:
                safe_url(url)
            except ValueError:
                continue
            if not any(u == url for _, u in result):
                result.append((label, url))
    return result


def _mirrors(node, page):
    return [{"label": label, "url": url, "format": _format(url)} for label, url in _links(
        node, page, lambda label, url: bool(re.search(r"\bMirror\s*\d|下[载載]|[载載]点|[载載]點|Download", label, re.I))
        and not re.search(r"安装说明|安裝說明|instructions", label, re.I))]


def _next(root, page):
    for n in root.walk():
        if n.tag == "a" and ("next" in n.attrs.get("rel", "").split() or n.has("next") or n.has("nextpostslink")):
            return urljoin(page, n.attrs.get("href", ""))
    return None


def _official(rule, query, previous, context, diagnostics):
    adapter = {"kagura_wordpress": "kagura_cn_html", "kagura_woocommerce": "kagura_en_html"}.get(rule["adapter"], rule["adapter"])
    entry_url = rule.get("url") or rule["pages"][0]
    direct = []
    for url in query.get("upstream_pages", []):
        safe_url(url, rule["page_hosts"])
        if urlsplit(url).path not in ("", "/", "/patch/"):
            direct.append((None, url))
    index = previous.get("index", {}) if isinstance(previous, dict) and not query.get("force") else {}
    records = list(index.get("records", []))
    pages = list(index.get("pending", []))
    checked_at = index.get("checked_at", 0)
    if not direct:
        target_category_known = adapter == "kagura_cn_html" and any(
            r["kind"] == "category" and _title_relevant(r["title"], query) for r in records)
        if not records or (pages and not target_category_known):
            pages = pages or [entry_url]
            seen = set()
            while pages and len(seen) < 100:
                url = pages.pop(0)
                if url in seen:
                    diagnostics.append("discovery_pagination_cycle")
                    pages.insert(0, url)
                    break
                seen.add(url)
                try:
                    _, html, _, final = context.fetch(url, rule)
                    root = parse_html(html)
                    checked_at = int(time.time())
                except Exception as error:
                    diagnostics.append(str(error))
                    if adapter == "kagura_en_html" and url == entry_url and "html_too_large" in str(error):
                        pages.insert(0, urljoin(entry_url, "/product-category/patches/patch/"))
                        continue
                    pages.insert(0, url)
                    break
                found = []
                if adapter == "kagura_en_html":
                    cards = [n for n in root.walk() if n.has("product") and n.has("type-product")
                             and not any(a.has("widget") or a.has("related") for a in _ancestors(n))]
                    for card in cards:
                        title_node = _first(card, lambda n: n.has("woocommerce-loop-product__title") or n.has("name"))
                        links = _links(card, final, lambda label, u: "/product/" in urlsplit(u).path)
                        if links:
                            title = title_node.text() if title_node else next((label for label, _ in links if label), "")
                            if title:
                                found.append({"title": title, "page": links[0][1], "kind": "product"})
                    if not cards:
                        diagnostics.append("discovery_structure_changed")
                else:
                    for article in root.walk():
                        if article.tag != "article":
                            continue
                        title_node = _first(article, lambda n: n.has("entry-title"))
                        if title_node:
                            links = _links(title_node, final, lambda label, u: urlsplit(u).hostname in rule["page_hosts"])
                            if links:
                                found.append({"title": title_node.text(), "page": links[0][1], "kind": "article"})
                    for label, link in _links(root, final, lambda label, u: "/category/" in urlsplit(u).path):
                        record = {"title": label, "page": link, "kind": "category"}
                        if record not in records:
                            records.append(record)
                    if not found and not _first(root, lambda n: n.has("widget_categories")):
                        diagnostics.append("discovery_structure_changed")
                records = list({r["page"]: r for r in records + found}.values())[:4096]
                next_page = _next(root, final)
                if next_page:
                    pages.append(next_page)
                # A target's category is enough; avoid crawling the complete archive.
                if adapter == "kagura_cn_html" and any(r["kind"] == "category" and _title_relevant(r["title"], query) for r in records):
                    break
                # Do not spend a selected game's entire deadline sweeping an
                # unrelated archive. Preserve next for subsequent index refresh
                # and let the fallback indexer establish aliases first.
                if adapter == "kagura_cn_html" and len(seen) >= 2:
                    break
            if pages and not any(r["kind"] == "category" and _title_relevant(r["title"], query) for r in records):
                diagnostics.append("discovery_index_incomplete")
        direct = [(r["title"], r["page"]) for r in records if r["kind"] != "category" and _title_relevant(r["title"], query)]
        for r in records:
            if r["kind"] == "category" and _title_relevant(r["title"], query):
                try:
                    url = r["page"]
                    visited = set()
                    while url and len(visited) < 100:
                        if url in visited:
                            raise ValueError("discovery_pagination_cycle")
                        visited.add(url)
                        _, html, _, final = context.fetch(url, rule)
                        root = parse_html(html)
                        article_count = 0
                        for article in root.walk():
                            if article.tag == "article":
                                article_count += 1
                                title = _first(article, lambda n: n.has("entry-title"))
                                if title:
                                    links = _links(title, final, lambda label, u: urlsplit(u).hostname in rule["page_hosts"])
                                    if links and _title_relevant(title.text(), query):
                                        direct.append((title.text(), links[0][1]))
                        if not article_count and not _first(root, lambda n: n.has("no-results")):
                            raise ValueError("discovery_structure_changed")
                        url = _next(root, final)
                    if url:
                        raise ValueError("discovery_index_incomplete")
                except Exception as error:
                    diagnostics.append(str(error))
    candidates = []
    for page, title in dict((page, title) for title, page in direct).items():
        try:
            _, html, _, final = context.fetch(page, rule)
            root = parse_html(html)
            h = _first(root, lambda n: n.tag == "h1" and (n.has("entry-title") or adapter == "kagura_en_html"))
            title = h.text() if h else title
            if not title:
                raise ValueError("discovery_structure_changed")
            body = _first(root, lambda n: n.attrs.get("id") == "tab-patch") if adapter == "kagura_en_html" else _first(root, lambda n: n.has("entry-content") and any(a.tag == "article" for a in _ancestors(n)))
            if body is None:
                raise ValueError("discovery_structure_changed")
            found = _candidate(rule, title, final, body, query, mirrors=_mirrors(body, final))
            article = next((a for a in _ancestors(body) if a.tag == "article"), None)
            published = _first(article, lambda n: n.tag == "time" and (n.has("published") or n.has("entry-date"))) if article else None
            if published:
                for candidate in found:
                    candidate["published_at"] = _date(published.attrs.get("datetime", ""))
            candidates.extend(found)
        except Exception as error:
            diagnostics.append(str(error))
            if title:
                fallback = _candidate(rule, title, page, None, query)
                for c in fallback:
                    c["warnings"].append("detail_unavailable")
                candidates.extend(fallback)
    return candidates, {"records": records, "pending": pages, "parser_version": PARSER_VERSION, "checked_at": checked_at}


def _ancestors(node):
    node = node.parent
    while node is not None:
        yield node
        node = node.parent


def _hgamer(rule, query, context, diagnostics):
    games, patches, identities, candidates = {}, {}, [], []
    for term in query_terms(query):
        try:
            url = urljoin(rule.get("url") or rule["pages"][0], "/zh-CN/search") + "?" + urlencode({"q": term})
            _, html, _, final = context.fetch(url, rule)
            root = parse_html(html)
            header = _first(root, lambda n: n.tag == "h1")
            summary = header.parent.text() if header else ""
            # Require both the actual query and a numeric result marker, including zero.
            count = re.search(r"(?:找到|found)\s*(\d+)\s*(?:笔|筆|results?)", summary, re.I)
            if term not in summary or not count:
                raise ValueError("discovery_search_structure_changed")
            identified_results = 0
            for heading in root.walk():
                if heading.tag != "h2" or heading.text() not in ("游戏", "遊戲", "补丁", "補丁", "文章", "新聞", "新闻", "Articles"):
                    continue
                group = heading.parent.parent
                kind = "games" if heading.text() in ("游戏", "遊戲") else "patches" if heading.text() in ("补丁", "補丁") else "posts"
                target = games if kind == "games" else patches
                for label, page in _links(group, final, lambda label, u: urlsplit(u).hostname in rule["page_hosts"]
                                          and bool(re.match(r"/zh-[A-Z]{2}/" + kind + r"/[^/]+/?$", urlsplit(u).path))):
                    identified_results += 1
                    if kind != "posts":
                        target[page] = label
            if identified_results < int(count[1]):
                raise ValueError("discovery_search_structure_changed")
        except Exception as error:
            diagnostics.append(str(error))
    for page in list(games):
        try:
            if context.detail_requests >= 6:
                raise ValueError("discovery_detail_budget_exhausted")
            context.detail_requests += 1
            _, html, _, final = context.fetch(page, rule)
            root = parse_html(html)
            body = _first(root, lambda n: "data-game-body" in n.attrs)
            heading = _first(body, lambda n: n.tag == "h1") if body else None
            if heading is None:
                raise ValueError("discovery_structure_changed")
            primary = heading.parent
            ids = _steam_ids(primary)
            if int(query["appid"]) not in ids:
                continue
            aliases = _aliases(primary, heading.text())
            identities.append({"appid": int(query["appid"]), "aliases": aliases, "page": final})
            for label, patch in _links(body, final, lambda label, u: urlsplit(u).hostname in rule["page_hosts"]
                                        and bool(re.match(r"/zh-[A-Z]{2}/patches/[^/]+/?$", urlsplit(u).path))):
                patches[patch] = label
        except Exception as error:
            diagnostics.append(str(error))
    for page in patches:
        try:
            if context.detail_requests >= 6:
                raise ValueError("discovery_detail_budget_exhausted")
            context.detail_requests += 1
            _, html, _, final = context.fetch(page, rule)
            root = parse_html(html)
            body = _first(root, lambda n: "data-patch-body" in n.attrs)
            heading = _first(body, lambda n: n.tag == "h1") if body else None
            if heading is None:
                raise ValueError("discovery_structure_changed")
            primary = heading.parent
            # AppID widgets in description are primary; related articles are excluded.
            ids = _steam_ids(primary)
            descriptions = [h.parent.parent for h in body.walk() if h.tag == "h2" and h.text() in ("補丁說明", "补丁说明", "安裝教學", "安装教学")]
            for description in descriptions:
                ids.extend(_steam_ids(description))
            # The official Steam widget is a sibling of description blocks in
            # the site's primary content column. Restrict to direct widgets
            # there, never widgets in related-card descendants.
            for description in descriptions:
                column = description.parent
                for child in column.children:
                    if isinstance(child, Node) and child.tag in ("iframe", "a"):
                        ids.extend(_steam_ids(child))
            reported = "zip" if re.search(r"\bZIP\b", primary.text(), re.I) else "exe" if re.search(r"\bEXE\b", primary.text(), re.I) else "unknown"
            mirrors = _mirrors(primary, final)
            aliases = _aliases(primary, heading.text())
            if not game_title(heading.text()):
                aliases = next((identity["aliases"] for identity in identities if identity["appid"] in ids), aliases)
            found = _candidate(rule, heading.text(), final, primary, query, ids=list(set(ids)),
                               aliases=aliases, reported=reported, mirrors=mirrors)
            for c in found:
                c["upstream_page"] = next((m["url"] for m in mirrors if urlsplit(m["url"]).hostname not in rule["page_hosts"]), None)
            candidates.extend(found)
        except Exception as error:
            diagnostics.append(str(error))
    return candidates, identities


def discover(rule, game_query, previous=None, request_context=None):
    context = _context(request_context)
    diagnostics, candidates, identities, index = [], [], [], None
    try:
        if rule.get("adapter") == "hgamer_html":
            candidates, identities = _hgamer(rule, game_query, context, diagnostics)
        elif rule.get("adapter") in ("kagura_cn_html", "kagura_en_html", "kagura_wordpress", "kagura_woocommerce"):
            candidates, index = _official(rule, game_query, previous or {}, context, diagnostics)
        else:
            raise ValueError("unknown_discovery_adapter")
        for candidate in candidates:
            candidate["fingerprint"] = hashlib.sha256(json.dumps(
                {k: v for k, v in candidate.items() if k not in ("fingerprint", "observed_at", "cache_status")},
                sort_keys=True, ensure_ascii=False).encode()).hexdigest()
        candidates = [validate_candidate(c, rule) for c in merge_candidates(candidates)]
    except Exception as error:
        diagnostics.append(str(error))
    incomplete = any("budget" in d or "incomplete" in d or "retry_deferred" in d for d in diagnostics)
    status = "partial" if diagnostics and (candidates or incomplete) else "failed" if diagnostics else "complete"
    result = {"candidates": candidates, "diagnostics": [{"source_id": rule["id"], "code": code} for code in dict.fromkeys(diagnostics)], "status": status, "identities": identities}
    if index is not None:
        result["index"] = index
    return result
