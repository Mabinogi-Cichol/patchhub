"""Live, bounded source subscriptions. Catalog signing and installation stay in core."""

import asyncio
from datetime import date
import ipaddress
import json
import os
import sys
from pathlib import Path
import re
import threading
import time
import unicodedata
from urllib.error import HTTPError
from urllib.parse import urlencode, urljoin, urlsplit
from urllib.request import HTTPCookieProcessor, HTTPRedirectHandler, HTTPSHandler, Request, build_opener
import uuid

MAX_HTML = 4 * 1024 * 1024
MAX_ENTRIES = 4096
MAX_SOURCES = 32


def _normal(text):
    text = unicodedata.normalize("NFKC", text).casefold().replace("+", "plus")
    return "".join(c for c in text if unicodedata.category(c)[0] in {"L", "N"})


def _host(host):
    if not isinstance(host, str) or host != host.lower() or not re.fullmatch(r"[a-z0-9.-]+", host):
        raise ValueError("invalid_source_host")
    if ("." not in host or host.startswith(".") or host.endswith(".") or ".." in host
            or host == "localhost" or host.endswith((".localhost", ".local"))):
        raise ValueError("invalid_source_host")
    try:
        ipaddress.ip_address(host)
    except ValueError:
        return host
    raise ValueError("invalid_source_host")


def _allowed_url(url, hosts, *, download=False):
    if not isinstance(url, str) or not url or url != url.strip() or any(ord(c) < 33 for c in url):
        raise ValueError("invalid_source_url")
    parsed = urlsplit(url)
    try:
        port = parsed.port
    except ValueError as error:
        raise ValueError("invalid_source_url") from error
    if (parsed.scheme != "https" or parsed.hostname not in hosts or parsed.username or parsed.password
            or port not in (None, 443) or not parsed.path.startswith("/")
            or (download and (not parsed.path.lower().endswith(".zip") or parsed.fragment))):
        raise ValueError("invalid_source_url")
    return url.strip()


def _no_links(path):
    return all(not p.is_symlink() and not (hasattr(p, "is_junction") and p.is_junction())
               for p in (path, *path.parents))


def _write_json(path, data):
    if not _no_links(path):
        raise ValueError("linked_source_state")
    path.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
    if not _no_links(path.parent):
        raise ValueError("linked_source_state")
    temp = path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    try:
        with os.fdopen(os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w", encoding="utf-8") as stream:
            json.dump(data, stream, ensure_ascii=False, indent=2)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp, path)
    finally:
        temp.unlink(missing_ok=True)


class _BoundedRedirect(HTTPRedirectHandler):
    def __init__(self, hosts):
        self.hosts = hosts

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        _allowed_url(newurl, self.hosts)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def _Page(table_id=None):
    # Decky's frozen interpreter omits html.parser. This is only called by
    # the system-Python source worker, or normal Python tests.
    from html.parser import HTMLParser
    class _Page(HTMLParser):
        """Collect forms and table rows without executing page scripts."""

        def __init__(self, table_id=None):
            super().__init__(convert_charrefs=True)
            self.table_id = table_id
            self.forms = []
            self.rows = []
            self._form = None
            self._table_depth = 0
            self._row = None
            self._cell = None
            self._anchor = None

        def handle_starttag(self, tag, attrs):
            attrs = dict(attrs)
            if tag == "form":
                self._form = {"name": attrs.get("name", ""), "action": attrs.get("action", ""),
                              "method": attrs.get("method", "get").lower(), "inputs": {}}
            elif tag == "input" and self._form is not None:
                if attrs.get("name") and attrs.get("type", "hidden").lower() == "hidden":
                    self._form["inputs"][attrs["name"]] = attrs.get("value", "")
            if tag == "table":
                if self._table_depth:
                    self._table_depth += 1
                elif self.table_id is None or attrs.get("id") == self.table_id:
                    self._table_depth = 1
            elif tag == "tr" and self._table_depth:
                self._row = []
            elif tag in ("td", "th") and self._row is not None:
                self._cell = {"class": attrs.get("class", ""), "text": "", "links": []}
            elif tag == "a" and self._cell is not None:
                self._anchor = {"href": attrs.get("href", ""), "text": ""}

        def handle_data(self, data):
            if self._cell is not None:
                self._cell["text"] += data
            if self._anchor is not None:
                self._anchor["text"] += data

        def handle_endtag(self, tag):
            if tag == "a" and self._anchor is not None:
                self._cell["links"].append(self._anchor)
                self._anchor = None
            elif tag in ("td", "th") and self._cell is not None:
                self._cell["text"] = " ".join(self._cell["text"].split())
                self._row.append(self._cell)
                self._cell = None
            elif tag == "tr" and self._row is not None:
                if self._row:
                    self.rows.append(self._row)
                self._row = None
            elif tag == "table" and self._table_depth:
                self._table_depth -= 1
            elif tag == "form" and self._form is not None:
                self.forms.append(self._form)
                self._form = None

    return _Page(table_id)


def _split_names(title):
    values = [title]
    if " / " in title:
        values.extend(x.strip() for x in title.split(" / "))
    return [x for x in values if x]


def _row_entry(row, columns, page, language):
    try:
        title = row[columns["title"]]["text"].strip()
        download = row[columns["download"]]["links"][0]["href"].strip()
    except (IndexError, KeyError, TypeError):
        return None
    if not title or not download:
        return None
    if not urlsplit(urljoin(page, download)).path.lower().endswith(".zip"):
        return None
    entry = {"title": title, "aliases": [], "url": urljoin(page, download),
             "language": language, "date": None, "source_page": page}
    for key in ("date", "language", "appid"):
        if key not in columns:
            continue
        try:
            value = row[columns[key]]["text"].strip()
        except (IndexError, KeyError, TypeError):
            continue
        if key == "appid":
            if not re.fullmatch(r"[1-9][0-9]*", value):
                raise ValueError("invalid_source_appid")
            entry["appid"] = int(value)
        elif value:
            entry[key] = value
    return entry


def _validate_rule(rule):
    if not isinstance(rule, dict):
        raise ValueError("invalid_source_rule")
    source_id = rule.get("id")
    if not isinstance(source_id, str) or not re.fullmatch(r"[a-z][a-z0-9-]{0,47}", source_id):
        raise ValueError("invalid_source_id")
    if not isinstance(rule.get("name"), str) or not rule["name"].strip() or len(rule["name"]) > 128:
        raise ValueError("invalid_source_name")
    if not isinstance(rule.get("adapter"), str) or not re.fullmatch(r"[a-z][a-z0-9_]{0,47}", rule["adapter"]):
        raise ValueError("unsupported_source_adapter")
    if type(rule.get("enabled")) is not bool or rule.get("root_layout") != "game_root":
        raise ValueError("invalid_source_rule")
    pages = rule.get("pages")
    if not isinstance(pages, list) or not 1 <= len(pages) <= 5:
        raise ValueError("invalid_source_pages")
    for key in ("page_hosts", "download_hosts"):
        hosts = rule.get(key)
        if not isinstance(hosts, list) or not 1 <= len(hosts) <= 8:
            raise ValueError("invalid_source_hosts")
        for host in hosts:
            _host(host)
    for page in pages:
        _allowed_url(page, rule["page_hosts"])
    if rule["adapter"] == "table_html":
        columns = rule.get("columns")
        if (not isinstance(columns, dict) or not all(k in columns for k in ("title", "download"))
                or any(k not in ("title", "download", "date", "language", "appid")
                       or type(v) is not int or not 0 <= v <= 20 for k, v in columns.items())):
            raise ValueError("invalid_source_columns")
        table_id = rule.get("table_id")
        if table_id is not None and (not isinstance(table_id, str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", table_id)):
            raise ValueError("invalid_table_id")
    return dict(rule)


def _validate_entries(source, entries):
    if not isinstance(entries, list) or not 0 < len(entries) <= MAX_ENTRIES:
        raise ValueError("empty_or_oversized_source")
    clean = []
    for entry in entries:
        if not isinstance(entry, dict) or not isinstance(entry.get("title"), str) or not entry["title"].strip():
            raise ValueError("invalid_source_entry")
        aliases = entry.get("aliases", [])
        if not isinstance(aliases, list) or any(not isinstance(a, str) for a in aliases):
            raise ValueError("invalid_source_entry")
        item = dict(entry)
        item["url"] = _allowed_url(item.get("url"), source["download_hosts"], download=True)
        item["source_page"] = _allowed_url(item.get("source_page"), source["page_hosts"])
        if not isinstance(item.get("language"), str) or not item["language"].strip():
            raise ValueError("invalid_source_language")
        if item.get("appid") is not None and (type(item["appid"]) is not int or item["appid"] <= 0):
            raise ValueError("invalid_source_appid")
        if item.get("date") is not None:
            value = item["date"]
            if not isinstance(value, str):
                raise ValueError("invalid_source_date")
            value = value.strip().upper() if value.strip().upper() == "COMING SOON" else value.strip()
            if value != "COMING SOON":
                if not re.fullmatch(r"[0-9]{4}/[0-9]{1,2}/[0-9]{1,2}", value):
                    raise ValueError("invalid_source_date")
                try:
                    year, month, day = map(int, value.split("/"))
                    date(year, month, day)
                except ValueError as error:
                    raise ValueError("invalid_source_date") from error
            item["date"] = value
        clean.append(item)
    return clean


class SourceRegistry:
    def __init__(self, state_dir: Path, config_path: Path):
        self.state_dir = Path(state_dir).absolute() / "sources"
        self.config_path = Path(config_path)
        if not _no_links(self.state_dir):
            raise ValueError("linked_source_state")
        data = json.loads(self.config_path.read_text(encoding="utf-8"))
        if data.get("schema_version") != 1 or not isinstance(data.get("sources"), list):
            raise ValueError("invalid_sources_config")
        self.builtins = [_validate_rule(rule) for rule in data["sources"]]
        self.override_path = self.state_dir / "overrides.json"
        self.adapters = {"patchmoe": self._collect_patchmoe, "table_html": self._collect_table}
        self._cache = {}
        self._status = {}
        self._rules = []
        self.diagnostics = []
        self._sync_lock = threading.RLock()
        self._refresh_lock = asyncio.Lock()
        self._reload()

    def register_adapter(self, adapter_id, collector):
        if not re.fullmatch(r"[a-z][a-z0-9_]{0,47}", adapter_id) or not callable(collector):
            raise ValueError("invalid_adapter")
        self.adapters[adapter_id] = collector

    def _reload(self):
        overrides = {"enabled": {}, "added": []}
        if self.override_path.exists():
            if not _no_links(self.override_path) or self.override_path.stat().st_size > 256 * 1024:
                raise ValueError("invalid_source_overrides")
            overrides = json.loads(self.override_path.read_text(encoding="utf-8"))
        if not isinstance(overrides.get("enabled"), dict) or not isinstance(overrides.get("added"), list):
            raise ValueError("invalid_source_overrides")
        games = overrides.setdefault("games", {})
        if not isinstance(games, dict) or any(
            not isinstance(appid, str) or not re.fullmatch(r"[1-9][0-9]{0,9}", appid)
            or not isinstance(choices, dict) or any(
                not isinstance(source_id, str) or not re.fullmatch(r"[a-z][a-z0-9-]{0,47}", source_id)
                or type(value) is not bool for source_id, value in choices.items())
            for appid, choices in games.items()
        ):
            raise ValueError("invalid_source_overrides")
        rules = [dict(rule) for rule in self.builtins]
        rules.extend(_validate_rule(rule) for rule in overrides["added"])
        if len(rules) > MAX_SOURCES or len({r["id"] for r in rules}) != len(rules):
            raise ValueError("invalid_source_overrides")
        for rule in rules:
            if rule["id"] in overrides["enabled"]:
                value = overrides["enabled"][rule["id"]]
                if type(value) is not bool:
                    raise ValueError("invalid_source_overrides")
                rule["enabled"] = value
        self._rules = rules
        self._overrides = overrides
        self._cache = {}
        self._status = {}
        for rule in rules:
            status_path = self._status_path(rule["id"])
            if status_path.is_file() and _no_links(status_path) and status_path.stat().st_size <= 4096:
                try:
                    status = json.loads(status_path.read_text(encoding="utf-8"))
                    if isinstance(status, dict) and type(status.get("checked_at")) is int and isinstance(status.get("code"), str):
                        self._status[rule["id"]] = status
                except (OSError, ValueError, TypeError, json.JSONDecodeError):
                    pass
            path = self._cache_path(rule["id"])
            if not path.is_file():
                continue
            try:
                if not _no_links(path) or path.stat().st_size > 4 * 1024 * 1024:
                    raise ValueError("invalid_source_cache")
                record = json.loads(path.read_text(encoding="utf-8"))
                self._cache[rule["id"]] = {"entries": _validate_entries(rule, record["entries"]),
                                           "etag": record.get("etag"), "modified": record.get("modified"),
                                           "cached_at": record.get("cached_at"), "checked_at": record.get("checked_at")}
            except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError):
                self.diagnostics.append({"source_id": rule["id"], "code": "invalid_source_cache"})

    def _cache_path(self, source_id):
        return self.state_dir / "cache" / (source_id + ".json")

    def _status_path(self, source_id):
        return self.state_dir / "status" / (source_id + ".json")

    def list_sources(self):
        statuses = {d["source_id"]: d["code"] for d in self.diagnostics if "source_id" in d}
        return [{**rule, "status": statuses.get(rule["id"], "cached" if rule["id"] in self._cache else "not_refreshed"),
                 "entry_count": len(self._cache.get(rule["id"], {}).get("entries", [])),
                 "cached_at": self._cache.get(rule["id"], {}).get("cached_at"),
                 "checked_at": self._status.get(rule["id"], {}).get("checked_at",
                                self._cache.get(rule["id"], {}).get("checked_at"))} for rule in self._rules]

    def set_enabled(self, source_id, enabled):
        if type(enabled) is not bool or not any(r["id"] == source_id for r in self._rules):
            raise ValueError("unknown_source")
        with self._sync_lock:
            updated = {**self._overrides, "enabled": {**self._overrides["enabled"], source_id: enabled}}
            _write_json(self.override_path, updated)
            self._reload()

    def add_source(self, rule):
        rule = _validate_rule(rule)
        if rule["adapter"] not in self.adapters or any(r["id"] == rule["id"] for r in self._rules):
            raise ValueError("source_exists_or_adapter_unavailable")
        if len(self._rules) >= MAX_SOURCES:
            raise ValueError("source_limit")
        with self._sync_lock:
            updated = {**self._overrides, "added": [*self._overrides["added"], rule]}
            _write_json(self.override_path, updated)
            self._reload()

    def game_subscribed(self, appid, source_id):
        return self._overrides["games"].get(str(appid), {}).get(source_id, True)

    def list_game_sources(self, game):
        matched = {entry["source_id"] for entry in self.matches(game, include_disabled=True)}
        return [{**source, "subscribed": self.game_subscribed(game["appid"], source["id"]),
                 "matched": source["id"] in matched} for source in self.list_sources()]

    def set_game_source(self, appid, source_id, subscribed):
        if type(appid) is not int or not 0 < appid <= 9999999999:
            raise ValueError("invalid_appid")
        if type(subscribed) is not bool or not any(r["id"] == source_id for r in self._rules):
            raise ValueError("unknown_source")
        with self._sync_lock:
            games = dict(self._overrides["games"])
            choices = {**games.get(str(appid), {}), source_id: subscribed}
            games[str(appid)] = choices
            updated = {**self._overrides, "games": games}
            if len(json.dumps(updated).encode("utf-8")) > 256 * 1024:
                raise ValueError("source_limit")
            _write_json(self.override_path, updated)
            self._reload()

    def source_for_release(self, release_id):
        if not isinstance(release_id, str):
            return None
        for rule in self._rules:
            prefix = "patchmoe-" if rule["id"] == "patchmoe" else f"source-{rule['id']}-"
            if release_id.startswith(prefix) and re.fullmatch(r"[1-9][0-9]*", release_id[len(prefix):]):
                return dict(rule)
        return None

    def validate_download(self, source_id, url):
        rule = next((r for r in self._rules if r["id"] == source_id and r["enabled"]), None)
        if rule is None:
            raise ValueError("source_not_enabled")
        return _allowed_url(url, rule["download_hosts"], download=True)

    def matches(self, game, *, include_disabled=False):
        if not isinstance(game.get("name"), str) or type(game.get("appid")) is not int or game["appid"] <= 0:
            return []
        name = _normal(game["name"])
        result = []
        for rule in self._rules:
            if not include_disabled and (not rule["enabled"] or not self.game_subscribed(game["appid"], rule["id"])):
                continue
            candidates = []
            for entry in self._cache.get(rule["id"], {}).get("entries", []):
                names = set(_normal(n) for title in (entry["title"], *entry["aliases"]) for n in _split_names(title))
                if entry.get("appid") == game["appid"] or (entry.get("appid") is None and name and name in names):
                    candidates.append(entry)
            if not candidates or len({e["url"] for e in candidates}) != 1:
                continue
            selected = next((e for e in candidates if e["language"].lower() in ("zh-cn", "zh_cn")), candidates[0])
            release_id = (f"patchmoe-{game['appid']}" if rule["id"] == "patchmoe"
                          else f"source-{rule['id']}-{game['appid']}")
            result.append({**selected, "source_id": rule["id"], "release_id": release_id,
                           "root_layout": rule["root_layout"]})
        return result

    def _fetch(self, url, rule, *, data=None, etag=None, modified=None, cookies=None):
        import http.cookiejar
        headers = {"User-Agent": "Mozilla/5.0 (X11; Linux x86_64) SteamPatchSearcher/1",
                   "Accept": "text/html,application/xhtml+xml"}
        if etag:
            headers["If-None-Match"] = etag
        if modified:
            headers["If-Modified-Since"] = modified
        if data is not None:
            headers["Content-Type"] = "application/x-www-form-urlencoded"
        opener = build_opener(HTTPSHandler(), HTTPCookieProcessor(cookies if cookies is not None else http.cookiejar.CookieJar()),
                              _BoundedRedirect(rule["page_hosts"]))
        try:
            with opener.open(Request(_allowed_url(url, rule["page_hosts"]), data=data, headers=headers), timeout=20) as response:
                final = _allowed_url(response.geturl(), rule["page_hosts"])
                size = response.headers.get("Content-Length")
                if size and int(size) > MAX_HTML:
                    raise ValueError("source_html_too_large")
                body = response.read(MAX_HTML + 1)
                if len(body) > MAX_HTML:
                    raise ValueError("source_html_too_large")
                return response.status, body.decode("utf-8", "replace"), dict(response.headers), final
        except HTTPError as error:
            if error.code == 304:
                return 304, "", dict(error.headers), url
            raise

    def _collect_patchmoe(self, rule, previous, force):
        import http.cookiejar
        cookies = http.cookiejar.CookieJar()
        page = rule["pages"][0]
        status, html, headers, final = self._fetch(page, rule, cookies=cookies)
        parser = _Page()
        parser.feed(html)
        forms = [f for f in parser.forms if f["name"] in {"form_en", "form_zh_cn", "form_zh_tw"}]
        if not forms:
            raise ValueError("patchmoe_language_forms_missing")
        entries = []
        for form in forms:
            if form["method"] != "post" or "lang" not in form["inputs"]:
                raise ValueError("patchmoe_language_form_invalid")
            if not form["inputs"]["lang"]:
                escaped = re.escape(form["name"])
                assignment = re.search(r"(?:document\.)?" + escaped + r"\.lang\.value\s*=\s*(['\"])([^'\"<>]{1,64})\1", html)
                if assignment:
                    form["inputs"]["lang"] = assignment.group(2)
            if not form["inputs"]["lang"]:
                continue
            action = _allowed_url(urljoin(final, form["action"] or "/"), rule["page_hosts"])
            _, localized, _, localized_url = self._fetch(action, rule,
                data=urlencode(form["inputs"]).encode("utf-8"), cookies=cookies)
            table = _Page()
            table.feed(localized)
            language = {"form_en": "en", "form_zh_cn": "zh-CN", "form_zh_tw": "zh-TW"}[form["name"]]
            for row in table.rows:
                classes = {cell["class"].split()[0]: i for i, cell in enumerate(row) if cell["class"]}
                if not all(k in classes for k in ("title", "release", "patch")):
                    continue
                entry = _row_entry(row, {"title": classes["title"], "date": classes["release"],
                                         "download": classes["patch"]}, localized_url, language)
                if entry:
                    if entry.get("date") in ("待定", "即将推出", "即將推出"):
                        entry["date"] = "COMING SOON"
                    entries.append(entry)
        if not entries:
            raise ValueError("patchmoe_rows_missing")
        # Only an identical archive URL verifies that localized titles describe
        # the same release. Never infer pairs from row position or filename.
        by_url = {}
        for entry in entries:
            by_url.setdefault(entry["url"], []).append(entry)
        merged = []
        for group in by_url.values():
            chosen = next((e for e in group if e["language"] == "zh-CN"), group[0]).copy()
            chosen["aliases"] = sorted({e["title"] for e in group if e["title"] != chosen["title"]})
            merged.append(chosen)
        return merged, None, None, False

    def _collect_table(self, rule, previous, force):
        entries = []
        etag = modified = None
        for page in rule["pages"]:
            status, html, headers, final = self._fetch(page, rule,
                etag=previous.get("etag") if len(rule["pages"]) == 1 else None,
                modified=previous.get("modified") if len(rule["pages"]) == 1 else None)
            if status == 304:
                if not previous.get("entries"):
                    raise ValueError("source_cache_missing")
                return previous["entries"], previous.get("etag"), previous.get("modified"), True
            parser = _Page(rule.get("table_id"))
            parser.feed(html)
            for row in parser.rows:
                entry = _row_entry(row, rule["columns"], final, rule.get("language", "en"))
                if entry:
                    entries.append(entry)
            if len(rule["pages"]) == 1:
                etag, modified = headers.get("ETag"), headers.get("Last-Modified")
        return entries, etag, modified, False

    def _refresh_one(self, rule, force):
        with self._sync_lock:
            old = self._cache.get(rule["id"], {}).copy()
        try:
            entries, etag, modified, unchanged = self.adapters[rule["adapter"]](rule, old, force)
            entries = _validate_entries(rule, entries)
            checked_at = int(time.time())
            if not unchanged:
                record = {"entries": entries, "etag": etag, "modified": modified,
                          "cached_at": checked_at, "checked_at": checked_at}
            else:
                record = {**old, "checked_at": checked_at}
            with self._sync_lock:
                _write_json(self._cache_path(rule["id"]), record)
                self._cache[rule["id"]] = record
                self._status[rule["id"]] = {"checked_at": checked_at, "code": "source_not_modified" if unchanged else "source_refreshed"}
                _write_json(self._status_path(rule["id"]), self._status[rule["id"]])
            return {"source_id": rule["id"], "code": "source_not_modified" if unchanged else "source_refreshed",
                    "entries": len(entries), "cached_at": record["cached_at"], "checked_at": checked_at}
        except Exception as error:
            checked_at = int(time.time())
            with self._sync_lock:
                try:
                    self._status[rule["id"]] = {"checked_at": checked_at,
                        "code": "source_offline_cached" if old.get("entries") else "source_refresh_failed"}
                    _write_json(self._status_path(rule["id"]), self._status[rule["id"]])
                except (OSError, ValueError):
                    pass
            return {"source_id": rule["id"], "code": "source_offline_cached" if old.get("entries") else "source_refresh_failed",
                    "detail": str(error)[:200], "entries": len(old.get("entries", [])),
                    "cached_at": old.get("cached_at"), "checked_at": checked_at}

    async def refresh(self, force=False):
        async with self._refresh_lock:
            if getattr(sys, "frozen", False):
                # Use the OS interpreter for the standard-library HTML/cookie
                # modules omitted from Decky's PyInstaller bundle. Fixed argv,
                # no shell or source-provided executable.
                worker_env = os.environ.copy()
                # PyInstaller's library path targets its bundled OpenSSL and
                # Python libraries, which are incompatible with OS Python.
                for key in ("PYTHONHOME", "PYTHONPATH", "LD_LIBRARY_PATH"):
                    worker_env.pop(key, None)
                if worker_env.get("LD_LIBRARY_PATH_ORIG"):
                    worker_env["LD_LIBRARY_PATH"] = worker_env["LD_LIBRARY_PATH_ORIG"]
                try:
                    process = await asyncio.create_subprocess_exec(
                        "/usr/bin/python3", "-I", str(Path(__file__).absolute()), "--refresh",
                        str(self.state_dir.parent), str(self.config_path.absolute()),
                        "1" if force else "0", stdout=asyncio.subprocess.PIPE,
                        stderr=asyncio.subprocess.PIPE, env=worker_env)
                except OSError as error:
                    return self._failed_worker(str(error))
                try:
                    stdout, stderr = await asyncio.wait_for(process.communicate(), 180)
                except asyncio.TimeoutError:
                    if process.returncode is None:
                        process.kill()
                    await process.wait()
                    return self._failed_worker("source_worker_timeout")
                except BaseException:
                    if process.returncode is None:
                        process.kill()
                    await process.wait()
                    raise
                if process.returncode or len(stdout) > 256 * 1024:
                    return self._failed_worker("source_worker_failed:" + stderr.decode(errors="replace")[:200])
                try:
                    diagnostics = json.loads(stdout)
                    if not isinstance(diagnostics, list):
                        raise ValueError("invalid_worker_response")
                except (ValueError, UnicodeError):
                    return self._failed_worker("invalid_worker_response")
                self._reload()
                self.diagnostics = diagnostics
                return diagnostics
            semaphore = asyncio.Semaphore(4)
            async def one(rule):
                async with semaphore:
                    return await asyncio.to_thread(self._refresh_one, rule, force)
            rules = [dict(rule) for rule in self._rules if rule["enabled"]]
            diagnostics = await asyncio.gather(*(one(rule) for rule in rules))
            self.diagnostics = diagnostics
            return diagnostics

    def _failed_worker(self, detail):
        # An OS interpreter failure should still leave the installed game list
        # and previously validated source cache available to the frontend.
        self._reload()
        self.diagnostics = [{"source_id": rule["id"],
            "code": "source_offline_cached" if self._cache.get(rule["id"], {}).get("entries") else "source_refresh_failed",
            "detail": detail[:200]} for rule in self._rules if rule["enabled"]]
        return self.diagnostics


if __name__ == "__main__":
    if len(sys.argv) != 5 or sys.argv[1] != "--refresh" or sys.argv[4] not in ("0", "1"):
        raise SystemExit("invalid_source_worker_arguments")
    registry = SourceRegistry(Path(sys.argv[2]), Path(sys.argv[3]))
    result = asyncio.run(registry.refresh(force=sys.argv[4] == "1"))
    print(json.dumps(result, ensure_ascii=True))
