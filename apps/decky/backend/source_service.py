"""Source display/subscription snapshots; all discovery and installation belong to Rust."""
from pathlib import Path
from bridge import METHODS, PROTOCOL_VERSION

from protocol_validation import SOURCE_METHODS, validate_capabilities

REQUIRED_METHODS = frozenset(SOURCE_METHODS)


class SourceService:
    def __init__(self, bridge, state_dir):
        self.bridge = bridge
        self._state_root = Path(state_dir).absolute()
        self.state_dir = self._state_root / "sources"
        self.settings_revision = 0
        self.diagnostics = []
        self._summaries = []
        self._rows = {}
        self._rows_by_instance = {}
        self._primed = False
        self._closed = False

    # -- lifecycle ---------------------------------------------------------
    async def initialize(self):
        caps = await self.bridge.call_sync("get_capabilities", {})
        validate_capabilities(caps)
        self._apply_summaries(await self.bridge.call_sync("list_sources", {}))
        return self

    def prime(self, scan_report):
        """Fill the read snapshot from one Rust `scan` result (no network)."""
        if not isinstance(scan_report, dict):
            raise ValueError("invalid_scan_report")
        revision = scan_report.get("settings_revision")
        if type(revision) is int:
            self.settings_revision = revision
        if scan_report.get("sources") is not None:
            self._apply_summaries(scan_report["sources"])
        rows = {}
        by_instance = {}
        for row in scan_report["installs"]:
            game = row["game"]
            entry = {"sources": row.get("sources"), "discovery": row.get("discovery")}
            rows[game["appid"]] = entry
            by_instance[game["instance_id"]] = entry
        self._rows = rows
        self._rows_by_instance = by_instance
        self._primed = True

    async def close(self):
        self._closed = True

    # -- snapshot bookkeeping ---------------------------------------------
    def _apply_summaries(self, summaries):
        if not isinstance(summaries, list):
            raise ValueError("invalid_source_summaries")
        clean = [dict(item) for item in summaries]
        self._summaries = clean
        for item in clean:
            revision = item.get("settings_revision")
            if type(revision) is int:
                self.settings_revision = max(self.settings_revision, revision)

    def _entry(self, game):
        appid = game["appid"]
        if appid in self._rows:
            return self._rows[appid]
        return self._rows_by_instance.get(game["instance_id"])

    # -- sync reads (snapshot) --------------------------------------------
    def list_sources(self):
        return [dict(item) for item in self._summaries]

    def cached_discovery(self, game):
        entry = self._entry(game)
        appid = game["appid"]
        discovery = entry["discovery"] if entry is not None else None
        if discovery is None:
            discovery = {"appid": appid, "status": "not_queried", "candidates": [], "diagnostics": []}
        result = dict(discovery)
        result.setdefault("appid", appid)
        result["settings_revision"] = self.settings_revision
        return result

    # -- local writes (one-shot serve) ------------------------------------
    async def _call_sync(self, method, params):
        result = await self.bridge.call_sync(method, params)
        self.settings_revision = result["settings_revision"]
        await self._reload_snapshot()
        return result

    async def _reload_snapshot(self):
        self._apply_summaries(await self.bridge.call_sync("list_sources", {}))

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
        ids = [rule["id"] for rule in self._summaries
               if rule.get("enabled") and not rule.get("discovery_only")]
        diagnostics = []
        for index, status in enumerate(statuses):
            entry = {"code": status["code"]}
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
        self.settings_revision = result["settings_revision"]
        diagnostics = self._status_diagnostics(result["statuses"])
        await self._reload_snapshot()
        self.diagnostics = diagnostics
        return diagnostics

    async def scan_sources(self, force=False, on_event=None, cancel=None,
                           instance_id=None, name_hints=None, preferred_language=None):
        params = {"force": bool(force)}
        if instance_id is not None:
            params.update(instance_id=instance_id, name_hints=list(name_hints or []))
            if preferred_language is not None:
                params["preferred_language"] = preferred_language
        result = await self.bridge._call("scan_sources", params,
                                         on_event=on_event, cancel=cancel)
        self.settings_revision = result["settings_revision"]
        for item in result["results"]:
            appid = item.get("appid")
            if type(appid) is not int:
                continue
            entry = self._rows.setdefault(appid, {"sources": None, "discovery": None})
            entry["discovery"] = {"appid": appid, "status": item["status"],
                                  "candidates": item["candidates"],
                                  "diagnostics": item["diagnostics"]}
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
        result = await self.bridge._call("discover_game", params)
        if not isinstance(result, dict):
            raise ValueError("invalid_core_response")
        appid = result["appid"]
        discovery = {"appid": appid, "status": result["status"],
                     "candidates": result["candidates"], "diagnostics": result["diagnostics"]}
        entry = self._rows.setdefault(appid, {"sources": None, "discovery": None})
        entry["discovery"] = discovery
        self.settings_revision = result["settings_revision"]
        return {**result, **discovery}
