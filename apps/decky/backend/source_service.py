"""Thin snapshot adapter over the shared Rust source engine.

`SourceService` is duck-type equal to the retired `sources.SourceRegistry` so the
existing publisher/EXE consumers stay unchanged: sync reads come from the last
`scan` snapshot, local writes go through a one-shot `serve` call, and network
work (`refresh`/`scan_sources`/`discover_game`) is streamed by Rust under
`serve-task`. Matching, TTL, candidate validation and concurrency are Rust-owned
and are not re-implemented here.
"""
import re
from pathlib import Path

from source_primitives import allowed_url

# `list_sources` summary keys that are not part of the rule dict.
SOURCE_SUMMARY_KEYS = {"status", "entry_count", "cached_at", "checked_at", "origin"}
# Methods the adapter depends on to serve scans/subscriptions; a core missing
# any of them is demoted to Python. `get_source_settings`/`list_source_matches`/
# `get_source_policy` are used only for optional export or Windows-host reads.
REQUIRED_METHODS = frozenset({
    "get_capabilities", "list_sources", "list_game_sources", "set_source_enabled",
    "add_source", "set_game_source", "refresh_sources", "scan_sources",
    "discover_game", "get_source_state",
})
ENGINE = "rust"


def normalize_diagnostics(values):
    """Coerce arbitrary diagnostic payloads into `{"code": ...}` dicts."""
    if not isinstance(values, list):
        return []
    clean = []
    for value in values:
        if isinstance(value, dict) and isinstance(value.get("code"), str):
            clean.append(dict(value))
        elif isinstance(value, str):
            clean.append({"code": value[:200]})
    return clean


def aggregate_import_diagnostics(import_report):
    """Flatten an `ImportReport` into diagnostics plus first-import count markers."""
    if not isinstance(import_report, dict):
        return []
    result = []
    if import_report.get("imported_settings"):
        result.append({"code": "source_settings_imported"})
    for key, code in (("imported_cache", "source_cache_imported"),
                      ("imported_discovery", "source_discovery_imported"),
                      ("imported_index", "source_index_imported")):
        count = import_report.get(key)
        if type(count) is int and count > 0:
            result.append({"code": code, "detail": str(count)})
    result.extend(normalize_diagnostics(import_report.get("diagnostics")))
    return result


class SourceService:
    engine = ENGINE

    def __init__(self, bridge, state_dir, config_path=None):
        self.bridge = bridge
        self._state_root = Path(state_dir).absolute()
        self.state_dir = self._state_root / "sources"
        # `config_path` is accepted for callers that still pass the legacy
        # sources.json path; the Rust engine owns rules and needs no local file.
        self.config_path = Path(config_path) if config_path is not None else None
        self.settings_revision = 0
        self.diagnostics = []
        self._import_diagnostics = []
        self._rules = []
        self.builtins = []
        self._summaries = []
        self._rows = {}
        self._rows_by_instance = {}
        self._primed = False
        self._closed = False

    # -- lifecycle ---------------------------------------------------------
    async def initialize(self):
        caps = await self.bridge.call_sync("get_capabilities", {})
        methods = caps.get("methods") if isinstance(caps, dict) else None
        if not isinstance(methods, list) or not REQUIRED_METHODS.issubset(set(methods)):
            raise ValueError("source_capability_unavailable")
        self._apply_summaries(await self.bridge.call_sync("list_sources", {}))
        state = await self.bridge.call_sync("get_source_state", {})
        if isinstance(state, dict):
            revision = state.get("settings_revision")
            if type(revision) is int:
                self.settings_revision = revision
            self._import_diagnostics = aggregate_import_diagnostics(state.get("import_report"))
        self.diagnostics = list(self._import_diagnostics)
        return self

    def prime(self, scan_report):
        """Fill the read snapshot from one Rust `scan` result (no network)."""
        if not isinstance(scan_report, dict):
            return
        revision = scan_report.get("settings_revision")
        if type(revision) is int:
            self.settings_revision = revision
        if isinstance(scan_report.get("sources"), list):
            self._apply_summaries(scan_report["sources"])
        rows = {}
        by_instance = {}
        for row in scan_report.get("installs") or []:
            if not isinstance(row, dict):
                continue
            game = row.get("game") if isinstance(row.get("game"), dict) else {}
            entry = {
                "sources": row.get("sources") if isinstance(row.get("sources"), list) else None,
                "discovery": row.get("discovery") if isinstance(row.get("discovery"), dict) else None,
                "matches": row.get("matches") if isinstance(row.get("matches"), list) else None,
            }
            if type(game.get("appid")) is int:
                rows[game["appid"]] = entry
            if isinstance(game.get("instance_id"), str):
                by_instance[game["instance_id"]] = entry
        self._rows = rows
        self._rows_by_instance = by_instance
        self._primed = True

    async def close(self):
        self._closed = True

    # -- snapshot bookkeeping ---------------------------------------------
    def _apply_summaries(self, summaries):
        if not isinstance(summaries, list):
            return
        clean = [dict(item) for item in summaries if isinstance(item, dict)]
        self._summaries = clean
        self._rules = [{key: value for key, value in item.items() if key not in SOURCE_SUMMARY_KEYS}
                       for item in clean]
        self.builtins = [{key: value for key, value in item.items() if key not in SOURCE_SUMMARY_KEYS}
                         for item in clean if item.get("origin") == "builtin"]

    def _entry(self, game):
        if not isinstance(game, dict):
            return None
        appid = game.get("appid")
        if type(appid) is int and appid in self._rows:
            return self._rows[appid]
        instance = game.get("instance_id")
        if isinstance(instance, str):
            return self._rows_by_instance.get(instance)
        return None

    # -- sync reads (snapshot) --------------------------------------------
    def list_sources(self):
        return [dict(item) for item in self._summaries]

    def list_game_sources(self, game):
        entry = self._entry(game)
        if entry is not None and entry["sources"] is not None:
            return [dict(item) for item in entry["sources"]]
        appid = game.get("appid") if isinstance(game, dict) else None
        return [{**item, "subscribed": self.game_subscribed(appid, item.get("id")),
                 "matched": bool(item.get("matched", False))} for item in self._summaries]

    def matches(self, game, *, include_disabled=False):
        entry = self._entry(game)
        stored = entry["matches"] if entry is not None else None
        if stored is None:
            return []
        result = []
        for match in stored:
            if not isinstance(match, dict):
                continue
            if not include_disabled and (match.get("enabled") is False or match.get("subscribed") is False):
                continue
            result.append(dict(match))
        return result

    def cached_candidates(self, game):
        entry = self._entry(game)
        discovery = entry["discovery"] if entry is not None else None
        if not isinstance(discovery, dict):
            return []
        return [dict(candidate) for candidate in discovery.get("candidates", [])]

    def cached_discovery(self, game):
        entry = self._entry(game)
        discovery = entry["discovery"] if entry is not None else None
        appid = game.get("appid") if isinstance(game, dict) else None
        if not isinstance(discovery, dict):
            discovery = {"appid": appid, "status": "not_queried", "candidates": [], "diagnostics": []}
        result = dict(discovery)
        result.setdefault("appid", appid)
        result.setdefault("candidates", [])
        result.setdefault("diagnostics", [])
        result["settings_revision"] = self.settings_revision
        return result

    def game_subscribed(self, appid, source_id):
        if type(appid) is int and appid in self._rows:
            sources = self._rows[appid]["sources"]
            if isinstance(sources, list):
                for source in sources:
                    if source.get("id") == source_id:
                        return bool(source.get("subscribed", True))
        return True

    def source_for_release(self, release_id):
        if not isinstance(release_id, str):
            return None
        for rule in self._rules:
            if rule.get("discovery_only"):
                continue
            prefix = "patchmoe-" if rule["id"] == "patchmoe" else f"source-{rule['id']}-"
            if release_id.startswith(prefix) and re.fullmatch(r"[1-9][0-9]*", release_id[len(prefix):]):
                return dict(rule)
        return None

    def validate_download(self, source_id, url):
        rule = next((r for r in self._rules if r["id"] == source_id and r.get("enabled")), None)
        if rule is None or rule.get("discovery_only"):
            raise ValueError("source_not_enabled")
        return allowed_url(url, rule["download_hosts"], download=True)

    # -- local writes (one-shot serve) ------------------------------------
    async def _call_sync(self, method, params):
        result = await self.bridge.call_sync(method, params)
        if isinstance(result, dict):
            revision = result.get("settings_revision")
            if type(revision) is int:
                self.settings_revision = revision
        await self._reload_snapshot()
        return result

    async def _reload_snapshot(self):
        try:
            self._apply_summaries(await self.bridge.call_sync("list_sources", {}))
        except Exception:
            pass

    async def set_enabled(self, source_id, enabled):
        return await self._call_sync("set_source_enabled",
                                     {"source_id": source_id, "enabled": bool(enabled)})

    async def add_source(self, rule):
        return await self._call_sync("add_source", {"rule": rule})

    async def set_game_source(self, appid, source_id, subscribed):
        return await self._call_sync("set_game_source",
                                     {"appid": appid, "source_id": source_id,
                                      "subscribed": bool(subscribed)})

    # -- async network work (serve-task stream) ---------------------------
    def _status_diagnostics(self, statuses):
        ids = [rule["id"] for rule in self._rules
               if rule.get("enabled") and not rule.get("discovery_only")]
        diagnostics = []
        for index, status in enumerate(statuses if isinstance(statuses, list) else []):
            if not isinstance(status, dict):
                continue
            entry = {"code": str(status.get("code", "source_refresh_failed"))}
            if index < len(ids):
                entry["source_id"] = ids[index]
            if status.get("detail"):
                entry["detail"] = str(status["detail"])[:200]
            for field, key in (("entry_count", "entries"), ("cached_at", "cached_at"), ("checked_at", "checked_at")):
                if status.get(field) is not None:
                    entry[key] = status[field]
            diagnostics.append(entry)
        return diagnostics

    async def refresh(self, force=False, on_event=None, cancel=None):
        result = await self.bridge._call("refresh_sources", {"force": bool(force)},
                                         on_event=on_event, cancel=cancel)
        if isinstance(result, dict):
            revision = result.get("settings_revision")
            if type(revision) is int:
                self.settings_revision = revision
        diagnostics = self._status_diagnostics(result.get("statuses") if isinstance(result, dict) else [])
        await self._reload_snapshot()
        self.diagnostics = [*self._import_diagnostics, *diagnostics]
        return diagnostics

    async def scan_sources(self, force=False, on_event=None, cancel=None):
        result = await self.bridge._call("scan_sources", {"force": bool(force)},
                                         on_event=on_event, cancel=cancel)
        if isinstance(result, dict):
            revision = result.get("settings_revision")
            if type(revision) is int:
                self.settings_revision = revision
            for item in result.get("results") or []:
                if not isinstance(item, dict) or type(item.get("appid")) is not int:
                    continue
                entry = self._rows.setdefault(item["appid"], {"sources": None, "discovery": None, "matches": None})
                entry["discovery"] = {"appid": item["appid"], "status": item.get("status", "not_queried"),
                                      "candidates": item.get("candidates", []),
                                      "diagnostics": item.get("diagnostics", [])}
        return result

    async def discover_game(self, game, name_hints=None, preferred_language=None, force=False,
                            generation=None, is_current=None):
        if not isinstance(game, dict):
            raise ValueError("invalid_discovery_game")
        if is_current is not None and not is_current():
            return {**self.cached_discovery(game), "generation": generation}
        params = {"instance_id": game.get("instance_id"), "name_hints": list(name_hints or []),
                  "force": bool(force)}
        if preferred_language is not None:
            params["preferred_language"] = preferred_language
        if generation is not None:
            params["generation"] = generation
        result = await self.bridge._call("discover_game", params)
        if not isinstance(result, dict):
            raise ValueError("invalid_core_response")
        appid = result.get("appid", game.get("appid"))
        discovery = {"appid": appid, "status": result.get("status", "not_queried"),
                     "candidates": result.get("candidates", []),
                     "diagnostics": result.get("diagnostics", [])}
        if type(appid) is int:
            entry = self._rows.setdefault(appid, {"sources": None, "discovery": None, "matches": None})
            entry["discovery"] = discovery
        revision = result.get("settings_revision")
        if type(revision) is int:
            self.settings_revision = revision
        return {**result, **discovery}
