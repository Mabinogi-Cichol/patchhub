"""Live, bounded source subscriptions. Catalog signing and installation stay in core."""

import asyncio
import hashlib
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
MAX_DISCOVERY_INPUT = 32 * 1024
MAX_DISCOVERY_OUTPUT = 512 * 1024
DISCOVERY_ADAPTERS = {"kagura_wordpress", "kagura_woocommerce", "hgamer_html"}
DISCOVERY_PARSER_VERSION = 2
DISCOVERY_QUERY_VERSION = 1


def _discovery_module():
    # -I workers and Decky's frozen interpreter do not add this module's
    # directory to sys.path. Load a fixed sibling, never a supplied module path.
    import importlib.util
    name = "steam_patch_discovery"
    if name not in sys.modules:
        spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name("discovery.py"))
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
    return sys.modules[name]


def _worker_environment():
    environment = os.environ.copy()
    for key in ("PYTHONHOME", "PYTHONPATH", "LD_LIBRARY_PATH"):
        environment.pop(key, None)
    if environment.get("LD_LIBRARY_PATH_ORIG"):
        environment["LD_LIBRARY_PATH"] = environment["LD_LIBRARY_PATH_ORIG"]
    return environment


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
    rule = dict(rule)
    rule.setdefault("role", "publisher")
    rule.setdefault("discovery_only", False)
    rule.setdefault("priority", 100)
    if (type(rule.get("enabled")) is not bool or type(rule["discovery_only"]) is not bool
            or rule["role"] not in ("publisher", "fallback_indexer")
            or type(rule["priority"]) is not int or not 0 <= rule["priority"] <= 1000):
        raise ValueError("invalid_source_rule")
    if rule["discovery_only"]:
        if rule.get("root_layout") is not None or rule["adapter"] not in DISCOVERY_ADAPTERS:
            raise ValueError("invalid_source_rule")
    elif rule.get("root_layout") != "game_root" or rule["role"] != "publisher":
        raise ValueError("invalid_source_rule")
    if rule["role"] == "fallback_indexer" and (not rule["discovery_only"] or rule.get("download_hosts")):
        raise ValueError("invalid_source_rule")
    pages = rule.get("pages")
    if not isinstance(pages, list) or not 1 <= len(pages) <= 5:
        raise ValueError("invalid_source_pages")
    for key in ("page_hosts", "download_hosts"):
        hosts = rule.get(key)
        minimum = 0 if key == "download_hosts" and rule["discovery_only"] else 1
        if not isinstance(hosts, list) or not minimum <= len(hosts) <= 8:
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
        if data.get("schema_version") not in (1, 2) or not isinstance(data.get("sources"), list):
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
        self.settings_revision = 0
        self._discovery_cache = {}
        self._discovery_indexes = {}
        self._discovery_tasks = {}
        self._discovery_semaphore = asyncio.Semaphore(3)
        self._source_discovery_locks = {}
        self._worker_processes = set()
        self._closed = False
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
        added = [_validate_rule(rule) for rule in overrides["added"]]
        added_ids = {rule["id"] for rule in added}
        # Newly shipped defaults must not overwrite a previously configured
        # user source or make the upgraded registry fail on a duplicate ID.
        rules = [dict(rule) for rule in self.builtins if rule["id"] not in added_ids]
        rules.extend(added)
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
        self._discovery_cache = {}
        for rule in rules:
            status_path = self._status_path(rule["id"])
            if status_path.is_file() and _no_links(status_path) and status_path.stat().st_size <= 4096:
                try:
                    status = json.loads(status_path.read_text(encoding="utf-8"))
                    if isinstance(status, dict) and type(status.get("checked_at")) is int and isinstance(status.get("code"), str):
                        self._status[rule["id"]] = status
                except (OSError, ValueError, TypeError, json.JSONDecodeError):
                    pass
            if rule["discovery_only"]:
                continue
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
        return [{**rule, "status": statuses.get(rule["id"], self._status.get(rule["id"], {}).get("code", "cached" if rule["id"] in self._cache else "not_refreshed")),
                 "entry_count": self._status.get(rule["id"], {}).get("entry_count", 0) if rule["discovery_only"] else len(self._cache.get(rule["id"], {}).get("entries", [])),
                 "cached_at": self._status.get(rule["id"], {}).get("cached_at") if rule["discovery_only"] else self._cache.get(rule["id"], {}).get("cached_at"),
                 "checked_at": self._status.get(rule["id"], {}).get("checked_at",
                                self._cache.get(rule["id"], {}).get("checked_at"))} for rule in self._rules]

    def set_enabled(self, source_id, enabled):
        if type(enabled) is not bool or not any(r["id"] == source_id for r in self._rules):
            raise ValueError("unknown_source")
        with self._sync_lock:
            updated = {**self._overrides, "enabled": {**self._overrides["enabled"], source_id: enabled}}
            _write_json(self.override_path, updated)
            self.settings_revision += 1
            self._reload()

    def add_source(self, rule):
        rule = _validate_rule(rule)
        if (rule["adapter"] not in self.adapters and rule["adapter"] not in DISCOVERY_ADAPTERS) or any(r["id"] == rule["id"] for r in self._rules):
            raise ValueError("source_exists_or_adapter_unavailable")
        if len(self._rules) >= MAX_SOURCES:
            raise ValueError("source_limit")
        with self._sync_lock:
            updated = {**self._overrides, "added": [*self._overrides["added"], rule]}
            _write_json(self.override_path, updated)
            self.settings_revision += 1
            self._reload()

    def game_subscribed(self, appid, source_id):
        return self._overrides["games"].get(str(appid), {}).get(source_id, True)

    def list_game_sources(self, game):
        matched = {entry["source_id"] for entry in self.matches(game, include_disabled=True)}
        for rule in self._rules:
            if rule["discovery_only"]:
                record = self._game_record(rule, game["appid"])
                if any(c.get("match_basis") in ("appid", "verified_alias", "official_title")
                       for c in [*record.get("success", {}).get("candidates", []), *record.get("attempt", {}).get("candidates", [])]):
                    matched.add(rule["id"])
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
            self.settings_revision += 1
            self._reload()

    def source_for_release(self, release_id):
        if not isinstance(release_id, str):
            return None
        for rule in self._rules:
            if rule["discovery_only"]:
                continue
            prefix = "patchmoe-" if rule["id"] == "patchmoe" else f"source-{rule['id']}-"
            if release_id.startswith(prefix) and re.fullmatch(r"[1-9][0-9]*", release_id[len(prefix):]):
                return dict(rule)
        return None

    def validate_download(self, source_id, url):
        rule = next((r for r in self._rules if r["id"] == source_id and r["enabled"]), None)
        if rule is None or rule["discovery_only"]:
            raise ValueError("source_not_enabled")
        return _allowed_url(url, rule["download_hosts"], download=True)

    def matches(self, game, *, include_disabled=False):
        if not isinstance(game.get("name"), str) or type(game.get("appid")) is not int or game["appid"] <= 0:
            return []
        name = _normal(game["name"])
        result = []
        for rule in self._rules:
            if rule["discovery_only"]:
                continue
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

    @staticmethod
    def _fresh(record, ttl):
        checked = record.get("checked_at")
        return type(checked) is int and 0 <= time.time() - checked < ttl

    def _discovery_fresh(self, record, ttl):
        return (record.get("parser_version") == DISCOVERY_PARSER_VERSION
                and record.get("query_version") == DISCOVERY_QUERY_VERSION and self._fresh(record, ttl))

    def _discovery_path(self, source_id, appid):
        return self.state_dir / "discovery" / source_id / (str(appid) + ".json")

    def _index_path(self, source_id):
        return self.state_dir / "discovery-index" / (source_id + ".json")

    def _game_previous(self, rule, appid):
        record = self._game_record(rule, appid)
        source_id = rule["id"]
        if source_id not in self._discovery_indexes:
            path = self._index_path(source_id)
            index = {}
            try:
                if path.is_file() and _no_links(path) and path.stat().st_size <= MAX_DISCOVERY_OUTPUT:
                    data = json.loads(path.read_text(encoding="utf-8"))
                    if (data.get("schema_version") == 1 and self._fresh(data, 24 * 3600)
                            and isinstance(data.get("index"), dict) and data["index"].get("parser_version") == DISCOVERY_PARSER_VERSION):
                        index = data
            except (OSError, ValueError, TypeError):
                pass
            self._discovery_indexes[source_id] = index
        source_record = self._discovery_indexes[source_id]
        index = source_record.get("index", {}) if self._fresh(source_record, 24 * 3600) else {}
        if index.get("parser_version") != DISCOVERY_PARSER_VERSION:
            index = {}
        # Old per-game index copies must not bypass the shared index TTL.
        record = {**record, "attempt": {k: v for k, v in record.get("attempt", {}).items() if k != "index"}}
        if index:
            record = {**record, "attempt": {**record.get("attempt", {}), "index": index}}
        return record

    def _effective_discovery_rules(self, appid):
        return sorted((dict(r) for r in self._rules if r["discovery_only"]
                       and r["enabled"] and self.game_subscribed(appid, r["id"])),
                      key=lambda r: (r["priority"], r["id"]))

    def _validate_candidates(self, rule, appid, candidates):
        if not isinstance(candidates, list) or len(candidates) > 128:
            raise ValueError("invalid_discovery_candidates")
        if len(json.dumps(candidates, ensure_ascii=False).encode()) > MAX_DISCOVERY_OUTPUT:
            raise ValueError("oversized_discovery_candidates")
        clean = []
        for candidate in candidates:
            if not isinstance(candidate, dict):
                raise ValueError("invalid_discovery_candidate")
            item = dict(candidate)
            if (not isinstance(item.get("candidate_id"), str)
                    or not re.fullmatch(r"candidate-[a-f0-9]{24}", item["candidate_id"])
                    or item.get("source_id") != rule["id"] or item.get("capability") != "discover_only"
                    or item.get("variant") not in ("base", "dlc", "demo", "unknown")
                    or item.get("match_basis") not in ("appid", "verified_alias", "official_title", "title", "none")):
                raise ValueError("invalid_discovery_candidate")
            if item.get("game_appid") is not None and (type(item["game_appid"]) is not int or item["game_appid"] != appid):
                raise ValueError("invalid_discovery_identity")
            if item["match_basis"] in ("appid", "verified_alias", "official_title") and item.get("game_appid") != appid:
                raise ValueError("invalid_discovery_identity")
            if item["match_basis"] in ("appid", "verified_alias", "official_title") and item.get("game_appid") != appid:
                raise ValueError("invalid_discovery_identity")
            for key in ("game_title", "patch_title"):
                if not isinstance(item.get(key), str) or not item[key].strip() or len(item[key]) > 1024:
                    raise ValueError("invalid_discovery_candidate")
            for key in ("aliases", "languages", "warnings"):
                if not isinstance(item.get(key, []), list) or any(not isinstance(x, str) or len(x) > 1024 for x in item.get(key, [])):
                    raise ValueError("invalid_discovery_candidate")
            for key in ("requires_dlc", "excludes_dlc"):
                values = item.get(key)
                if values is not None and (not isinstance(values, list) or any(type(x) is not int or not 0 < x <= 9999999999 for x in values)):
                    raise ValueError("invalid_discovery_candidate")
            item["source_page"] = _allowed_url(item.get("source_page"), rule["page_hosts"])
            if item.get("upstream_page"):
                self._navigation_url(item["upstream_page"])
            mirrors = item.get("mirrors", [])
            if not isinstance(mirrors, list) or len(mirrors) > 32:
                raise ValueError("invalid_discovery_candidate")
            for mirror in mirrors:
                if not isinstance(mirror, dict):
                    raise ValueError("invalid_discovery_candidate")
                self._navigation_url(mirror.get("url"))
            for field in ("identity_evidence", "provenance", "related_index_pages"):
                evidence = item.get(field, [])
                if not isinstance(evidence, list) or len(evidence) > 64 or any(not isinstance(e, dict) for e in evidence):
                    raise ValueError("invalid_discovery_candidate")
                for evidence_item in evidence:
                    if evidence_item.get("page"):
                        self._navigation_url(evidence_item["page"])
            clean.append(item)
        return clean

    @staticmethod
    def _navigation_url(url):
        if not isinstance(url, str):
            raise ValueError("invalid_source_url")
        host = _host(urlsplit(url).hostname or "")
        return _allowed_url(url, [host])

    def _game_record(self, rule, appid):
        key = (rule["id"], appid)
        if key not in self._discovery_cache:
            path = self._discovery_path(*key)
            record = {}
            if path.is_file():
                try:
                    if not _no_links(path) or path.stat().st_size > MAX_DISCOVERY_OUTPUT:
                        raise ValueError("invalid_discovery_cache")
                    record = json.loads(path.read_text(encoding="utf-8"))
                    if (record.get("schema_version") != 1 or record.get("appid") != appid
                            or record.get("source_id") != rule["id"]):
                        raise ValueError("invalid_discovery_cache")
                    for section in ("success", "attempt"):
                        if record.get(section):
                            self._validate_candidates(rule, appid, record[section].get("candidates", []))
                except (OSError, ValueError, TypeError, KeyError):
                    record = {}
                    self.diagnostics.append({"source_id": rule["id"], "code": "invalid_discovery_cache"})
            self._discovery_cache[key] = record
        return self._discovery_cache[key]

    def cached_candidates(self, game):
        candidates = []
        for rule in self._effective_discovery_rules(game["appid"]):
            record = self._game_record(rule, game["appid"])
            success = record.get("success", {})
            attempt = record.get("attempt", {})
            values = success.get("candidates", [])
            if attempt.get("status") == "partial":
                values = [*values, *attempt.get("candidates", [])]
            by_id = {c["candidate_id"]: {**c, "cache_status": "cached" if self._discovery_fresh(success, 6 * 3600) else "stale"}
                     for c in values}
            candidates.extend(by_id.values())
        return candidates

    def cached_discovery(self, game):
        rules = self._effective_discovery_rules(game["appid"])
        records = [self._game_record(r, game["appid"]) for r in rules]
        attempts = [r.get("attempt", {}) for r in records]
        candidates = self.cached_candidates(game)
        statuses = [a.get("status", "not_queried") for a in attempts]
        if any(s == "failed" for s in statuses):
            status = "source_offline_cached" if candidates else "source_refresh_failed"
        elif "partial" in statuses:
            status = "cache_partial"
        elif not statuses or "not_queried" in statuses:
            status = "not_queried"
        else:
            status = "cache_complete"
        return {"appid": game["appid"], "status": status, "candidates": candidates,
                "diagnostics": [d for a in attempts for d in a.get("diagnostics", [])],
                "settings_revision": self.settings_revision}

    @staticmethod
    def _query_key(query):
        # New aliases and language preferences invalidate only negative results.
        data = {k: query.get(k) for k in ("appid", "name", "name_hints", "preferred_language", "legacy_match", "source_context")}
        data.update({"parser_version": DISCOVERY_PARSER_VERSION, "query_version": DISCOVERY_QUERY_VERSION})
        return hashlib.sha256(json.dumps(data, ensure_ascii=False, sort_keys=True).encode()).hexdigest()

    def _discovery_sync(self, query, rules, previous):
        module = _discovery_module()
        context = module.RequestContext(deadline=time.monotonic() + 29, budget=24)
        results = []
        all_candidates = [{**c, "cache_status": "cached" if self._discovery_fresh(record.get("success", {}), 6 * 3600) else "stale"}
                          for record in previous.values() for c in record.get("success", {}).get("candidates", [])
                          if any(r["id"] == c.get("source_id") and r["role"] == "publisher" for r in self._rules)]
        officials = [r for r in rules if r["role"] == "publisher"]
        effective_officials = [r for r in self._effective_discovery_rules(query["appid"]) if r["role"] == "publisher"]
        fallback = [r for r in rules if r["role"] == "fallback_indexer"]

        def collect(rule, current_query):
            try:
                cached = previous.get(rule["id"], {})
                old = {**cached.get("success", {}), **{k: v for k, v in cached.get("attempt", {}).items()
                       if k in ("index", "checkpoint", "queries", "query_cache")}}
                outcome = module.discover(rule, current_query, old, context)
                if not isinstance(outcome, dict) or outcome.get("status") not in ("complete", "empty", "partial", "failed"):
                    raise ValueError("invalid_discovery_response")
                outcome = {**outcome, "source_id": rule["id"], "status": "complete" if outcome["status"] == "empty" else outcome["status"],
                    "diagnostics": [d if isinstance(d, dict) else {"source_id": rule["id"], "code": str(d)[:200]}
                                    for d in outcome.get("diagnostics", [])]}
            except Exception as error:
                outcome = {"source_id": rule["id"], "status": "failed", "candidates": [],
                           "diagnostics": [{"source_id": rule["id"], "code": "source_refresh_failed", "detail": str(error)[:200]}]}
            return outcome

        for rule in officials:
            # Keep most of the single-game deadline for fallback. A publisher
            # archive without this title should not exhaust every HGamer query.
            deadline, budget = context.deadline, context.budget
            context.deadline = min(deadline, time.monotonic() + 6)
            context.budget = min(budget, context.requests + 3)
            try:
                result = collect(rule, query)
            finally:
                context.deadline, context.budget = deadline, budget
            results.append(result)
            all_candidates = [c for c in all_candidates if c.get("source_id") != rule["id"]]
            all_candidates.extend(result.get("candidates", []))
        if query.get("force") or ((module.needs_fallback(all_candidates, query) or any(r["status"] in ("failed", "partial") for r in results)) and not query.get("legacy_match")):
            for rule in fallback:
                result = collect(rule, query)
                results.append(result)
                identities = [i for i in result.get("identities", []) if i.get("appid") == query["appid"]]
                if identities or any(c.get("upstream_page") for c in result.get("candidates", [])):
                    supplemented = {**query, "identities": identities,
                        "name_hints": list(dict.fromkeys([*query.get("name_hints", []),
                            *(alias for i in identities for alias in i.get("aliases", []))]))[:12],
                        "aliases": list(dict.fromkeys(alias for i in identities for alias in i.get("aliases", [])))[:32]}
                    for official in effective_officials:
                        pages = [c.get("upstream_page") for c in result.get("candidates", [])
                                 if c.get("upstream_page") and urlsplit(c["upstream_page"]).hostname in official["page_hosts"]]
                        enhanced = collect(official, {**supplemented, "upstream_pages": pages})
                        prior = next((r for r in results if r["source_id"] == official["id"]),
                                     {"source_id": official["id"], **previous.get(official["id"], {}).get("success", {})})
                        # An exhausted second pass cannot discard the first pass.
                        enhanced["candidates"] = module.enrich_candidates(module.merge_candidates(
                            [*prior.get("candidates", []), *enhanced.get("candidates", [])]), identities)
                        if prior in results:
                            results[results.index(prior)] = enhanced
                        else:
                            results.append(enhanced)
        else:
            for rule in fallback:
                # This game has adequate publisher evidence; no network request.
                results.append({"source_id": rule["id"], "status": "skipped", "candidates": [], "diagnostics": []})
        # Confirmed shared upstream pages can carry index provenance. Version,
        # language and DLC uncertainty never collapse two candidates together.
        official_candidates = [c for r in results for c in r.get("candidates", []) if r["source_id"] in {s["id"] for s in effective_officials}]
        indexed_candidates = [c for r in results for c in r.get("candidates", []) if r["source_id"] in {s["id"] for s in fallback}]
        for indexed in indexed_candidates:
            for official in official_candidates:
                upstream = indexed.get("upstream_page")
                if (not upstream or module.canonical_url(upstream) != module.canonical_url(official["source_page"])
                        or indexed.get("game_appid") != query["appid"]
                        or indexed.get("match_basis") not in ("appid", "verified_alias", "official_title")
                        or indexed.get("variant") not in ("base", "dlc") or indexed["variant"] != official.get("variant")):
                    continue
                if official.get("game_appid") is None:
                    official["game_appid"] = indexed["game_appid"]
                    official["parent_appid"] = indexed["game_appid"] if official["variant"] == "dlc" else None
                    official["match_basis"] = "official_title"
                    official["identity_evidence"] = [*official.get("identity_evidence", []),
                        {"kind": "indexed_upstream_page", "appid": indexed["game_appid"], "page": indexed["source_page"]}]
                if official["game_appid"] != indexed["game_appid"]:
                    continue
                relation = {"source_id": indexed["source_id"], "page": indexed["source_page"]}
                official.setdefault("related_index_pages", []).append(relation)
                same_version = (official.get("version_raw") and official.get("version_raw") == indexed.get("version_raw")
                                and official.get("languages") and set(official["languages"]) == set(indexed.get("languages", [])))
                if (same_version and indexed.get("format_reported") == "zip" and official.get("format_observed") == "exe"):
                    for item in (official, indexed):
                        item["warnings"] = list(dict.fromkeys([*item.get("warnings", []), "artifact_format_conflict"]))
                    indexed["format_observed"] = "exe"
                if (same_version
                        and all(official.get(k) is not None and official.get(k) == indexed.get(k)
                                for k in ("requires_dlc", "excludes_dlc", "patch_kind"))):
                    official["provenance"] = [*official.get("provenance", []), relation]
        return {"schema_version": 1, "results": results}

    async def _discovery_worker(self, payload):
        data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        if len(data) > MAX_DISCOVERY_INPUT:
            raise ValueError("oversized_discovery_request")
        process = await asyncio.create_subprocess_exec("/usr/bin/python3", "-I", str(Path(__file__).absolute()),
            "--discover", str(self.state_dir.parent), str(self.config_path.absolute()),
            stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
            env=_worker_environment())
        self._worker_processes.add(process)
        async def exchange():
            process.stdin.write(data)
            await process.stdin.drain()
            process.stdin.close()
            async def bounded_read(stream, limit):
                chunks = []
                size = 0
                while True:
                    chunk = await stream.read(min(65536, limit - size + 1))
                    if not chunk:
                        return b"".join(chunks)
                    size += len(chunk)
                    if size > limit:
                        raise ValueError("oversized_discovery_response")
                    chunks.append(chunk)
            stdout, stderr = await asyncio.gather(bounded_read(process.stdout, MAX_DISCOVERY_OUTPUT),
                                                  bounded_read(process.stderr, 4096))
            await process.wait()
            if process.returncode:
                raise ValueError("source_worker_failed:" + stderr.decode(errors="replace")[:200])
            return json.loads(stdout)
        try:
            return await asyncio.wait_for(exchange(), 30)
        finally:
            if process.returncode is None:
                process.kill()
            await process.wait()
            self._worker_processes.discard(process)

    async def discover_game(self, game, name_hints=None, preferred_language=None, force=False, generation=None, is_current=None):
        if (not isinstance(game, dict) or type(game.get("appid")) is not int or not 0 < game["appid"] <= 9999999999
                or not isinstance(game.get("name"), str) or not game["name"].strip() or len(game["name"]) > 1024
                or type(force) is not bool):
            raise ValueError("invalid_discovery_game")
        hints = [] if name_hints is None else name_hints
        if (not isinstance(hints, list) or len(hints) > 3 or any(not isinstance(n, str) or not n.strip() or len(n) > 256 for n in hints)
                or preferred_language is not None and (not isinstance(preferred_language, str) or not re.fullmatch(r"[a-zA-Z]{2,3}(?:-[a-zA-Z]{2,4})?", preferred_language))):
            raise ValueError("invalid_discovery_query")
        if self._closed:
            raise ValueError("source_registry_closed")
        appid = game["appid"]
        legacy_matches = self.matches(game)
        if preferred_language:
            preferred = preferred_language.casefold().replace("_", "-")
            legacy_matches = [entry for entry in legacy_matches
                              if entry["language"].casefold().replace("_", "-") == preferred
                              or "-" not in entry["language"] and preferred.startswith(entry["language"].casefold() + "-")]
        query = {"appid": appid, "name": game["name"], "name_hints": hints,
                 "preferred_language": preferred_language, "legacy_match": bool(legacy_matches), "force": force,
                 "source_context": [r["id"] for r in self._effective_discovery_rules(appid)]}
        context_key = (generation, self.settings_revision, self._query_key(query), force)
        existing = self._discovery_tasks.get(appid)
        if existing and not existing.done():
            result = await asyncio.shield(existing)
            if getattr(existing, "discovery_context", None) == context_key:
                return {**result, "generation": generation}
            # A new scan/settings/query must run after an obsolete request, not
            # inherit that request's stale result and remain cold indefinitely.
            return await self.discover_game(game, hints, preferred_language, force, generation, is_current)
        revision = self.settings_revision
        task = asyncio.create_task(self._run_discovery(game, query, force, revision, is_current))
        task.discovery_context = context_key
        self._discovery_tasks[appid] = task
        try:
            result = await asyncio.shield(task)
            return {**result, "generation": generation}
        finally:
            if task.done() and self._discovery_tasks.get(appid) is task:
                del self._discovery_tasks[appid]

    async def _run_discovery(self, game, query, force, revision, is_current):
        appid = query["appid"]
        rules = self._effective_discovery_rules(appid)
        key = self._query_key(query)
        previous = {r["id"]: self._game_previous(r, appid) for r in rules}
        pending = []
        for rule in rules:
            old = previous[rule["id"]].get("success", {})
            ttl = 24 * 3600 if rule["role"] == "fallback_indexer" and old.get("candidates") else 6 * 3600
            confirmed = any(c.get("match_basis") in ("appid", "verified_alias", "official_title") for c in old.get("candidates", []))
            if force or not self._discovery_fresh(old, ttl) or not confirmed and old.get("query_key") != key:
                pending.append(rule)
        if not pending:
            return self.cached_discovery(game)
        locks = [self._source_discovery_locks.setdefault(r["id"], asyncio.Lock()) for r in sorted(rules, key=lambda r: r["id"])]
        async with self._discovery_semaphore:
            acquired = []
            try:
                for lock in locks:
                    await lock.acquire()
                    acquired.append(lock)
                if revision != self.settings_revision or is_current is not None and not is_current():
                    return {**self.cached_discovery(game), "status": "stale", "settings_revision": revision}
                payload = {"schema_version": 1, "query": query, "source_ids": [r["id"] for r in rules],
                           "refresh_source_ids": [r["id"] for r in pending]}
                try:
                    if getattr(sys, "frozen", False):
                        response = await self._discovery_worker(payload)
                    else:
                        response = await asyncio.wait_for(asyncio.to_thread(self._discovery_sync, query, pending, previous), 30)
                    if not isinstance(response, dict) or response.get("schema_version") != 1 or not isinstance(response.get("results"), list):
                        raise ValueError("invalid_discovery_response")
                    if len(json.dumps(response).encode()) > MAX_DISCOVERY_OUTPUT:
                        raise ValueError("oversized_discovery_response")
                    results = response["results"]
                    result_ids = [r.get("source_id") for r in results if isinstance(r, dict)]
                    if (len(result_ids) != len(results) or len(set(result_ids)) != len(result_ids)
                            or not {r["id"] for r in pending}.issubset(result_ids)
                            or not set(result_ids).issubset({r["id"] for r in rules})):
                        raise ValueError("invalid_discovery_response")
                    for result in results:
                        rule = next(r for r in rules if r["id"] == result["source_id"])
                        if result.get("status") not in ("complete", "partial", "failed", "skipped"):
                            raise ValueError("invalid_discovery_response")
                        result["candidates"] = self._validate_candidates(rule, appid, result.get("candidates", []))
                        if "index" in result and (not isinstance(result["index"], dict) or result["index"].get("parser_version") != DISCOVERY_PARSER_VERSION):
                            raise ValueError("invalid_discovery_index")
                        if not isinstance(result.get("diagnostics", []), list) or any(not isinstance(d, dict) or not isinstance(d.get("code"), str) for d in result.get("diagnostics", [])):
                            raise ValueError("invalid_discovery_response")
                except Exception as error:
                    results = [{"source_id": r["id"], "status": "failed", "candidates": [],
                                "diagnostics": [{"source_id": r["id"], "code": "source_refresh_failed", "detail": str(error)[:200]}]} for r in pending]
                with self._sync_lock:
                    if self._closed or revision != self.settings_revision or is_current is not None and not is_current():
                        return {**self.cached_discovery(game), "status": "stale", "settings_revision": revision}
                    for result in results:
                        source_id = result["source_id"]
                        old = previous[source_id]
                        attempt = {"status": result["status"], "candidates": result["candidates"],
                                   "diagnostics": result.get("diagnostics", []), "checked_at": int(time.time()), "query_key": key,
                                   "parser_version": DISCOVERY_PARSER_VERSION, "query_version": DISCOVERY_QUERY_VERSION}
                        attempt["cached_at"] = attempt["checked_at"] if result["status"] == "complete" else old.get("success", {}).get("cached_at")
                        for field in ("index", "checkpoint", "queries", "query_cache"):
                            if field in result:
                                attempt[field] = result[field]
                            elif field in old.get("attempt", {}):
                                attempt[field] = old["attempt"][field]
                            elif field in old.get("success", {}):
                                attempt[field] = old["success"][field]
                        success = attempt if result["status"] == "complete" or result["status"] == "skipped" and not old.get("success") else old.get("success", {})
                        record = {"schema_version": 1, "source_id": source_id, "appid": appid,
                                  "success": {k: v for k, v in success.items() if k not in ("index", "checkpoint", "queries", "query_cache")},
                                  "attempt": {k: v for k, v in attempt.items() if k != "candidates" or result["status"] == "partial"}}
                        # Single-source/game shards remain below the worker bound.
                        if len(json.dumps(record, ensure_ascii=False).encode()) > MAX_DISCOVERY_OUTPUT:
                            continue
                        _write_json(self._discovery_path(source_id, appid), record)
                        self._discovery_cache[(source_id, appid)] = record
                        if "index" in result:
                            old_index = self._discovery_indexes.get(source_id, {})
                            index_record = {"schema_version": 1, "checked_at": result["index"].get("checked_at", old_index.get("checked_at", attempt["checked_at"])), "index": result["index"]}
                            if len(json.dumps(index_record).encode()) <= MAX_DISCOVERY_OUTPUT:
                                _write_json(self._index_path(source_id), index_record)
                                self._discovery_indexes[source_id] = index_record
                        status = {"checked_at": attempt["checked_at"], "cached_at": record["success"].get("cached_at"),
                            "entry_count": len(record["success"].get("candidates", [])),
                            "code": "source_refreshed" if result["status"] == "complete" else "source_partial" if result["status"] == "partial" else "source_offline_cached" if record["success"].get("candidates") else "source_refresh_failed"}
                        _write_json(self._status_path(source_id), status)
                        self._status[source_id] = status
                result = self.cached_discovery(game)
                result["status"] = {"cache_complete": "complete", "cache_partial": "partial"}.get(result["status"], result["status"])
                return result
            finally:
                for lock in reversed(acquired):
                    lock.release()

    async def close(self):
        self._closed = True
        tasks = list(self._discovery_tasks.values())
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        for process in list(self._worker_processes):
            if process.returncode is None:
                process.kill()
            await process.wait()
        self._discovery_tasks.clear()

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
        if not force and self._fresh(old, 24 * 3600):
            return {"source_id": rule["id"], "code": "source_cache_fresh", "entries": len(old.get("entries", [])),
                    "cached_at": old.get("cached_at"), "checked_at": old.get("checked_at")}
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
            rules = [dict(rule) for rule in self._rules if rule["enabled"] and not rule["discovery_only"]]
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
    if len(sys.argv) == 4 and sys.argv[1] == "--discover":
        data = sys.stdin.buffer.read(MAX_DISCOVERY_INPUT + 1)
        if len(data) > MAX_DISCOVERY_INPUT:
            raise SystemExit("oversized_discovery_request")
        payload = json.loads(data)
        if (not isinstance(payload, dict) or payload.get("schema_version") != 1
                or not isinstance(payload.get("query"), dict) or not isinstance(payload.get("source_ids"), list)):
            raise SystemExit("invalid_discovery_request")
        registry = SourceRegistry(Path(sys.argv[2]), Path(sys.argv[3]))
        query = payload["query"]
        if type(query.get("appid")) is not int or query["appid"] <= 0:
            raise SystemExit("invalid_discovery_request")
        rules = [r for r in registry._effective_discovery_rules(query["appid"]) if r["id"] in payload["source_ids"]]
        if len(rules) != len(payload["source_ids"]):
            raise SystemExit("invalid_discovery_request")
        previous = {r["id"]: registry._game_previous(r, query["appid"]) for r in rules}
        refresh_ids = payload.get("refresh_source_ids", payload["source_ids"])
        if not isinstance(refresh_ids, list) or any(source not in payload["source_ids"] for source in refresh_ids):
            raise SystemExit("invalid_discovery_request")
        result = registry._discovery_sync(query, [r for r in rules if r["id"] in refresh_ids], previous)
        if len(json.dumps(result).encode()) > MAX_DISCOVERY_OUTPUT:
            raise SystemExit("oversized_discovery_response")
    elif len(sys.argv) == 5 and sys.argv[1] == "--refresh" and sys.argv[4] in ("0", "1"):
        registry = SourceRegistry(Path(sys.argv[2]), Path(sys.argv[3]))
        result = asyncio.run(registry.refresh(force=sys.argv[4] == "1"))
    else:
        raise SystemExit("invalid_source_worker_arguments")
    print(json.dumps(result, ensure_ascii=True))
