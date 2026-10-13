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
        for item in clean:
            revision = item.get("settings_revision")
            if type(revision) is int:
                self.settings_revision = max(self.settings_revision, revision)

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
        self.diagnostics = diagnostics
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
                entry = self._rows.setdefault(item["appid"], {"sources": None, "discovery": None})
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
        result = await self.bridge._call("discover_game", params)
        if not isinstance(result, dict):
            raise ValueError("invalid_core_response")
        appid = result.get("appid", game.get("appid"))
        discovery = {"appid": appid, "status": result.get("status", "not_queried"),
                     "candidates": result.get("candidates", []),
                     "diagnostics": result.get("diagnostics", [])}
        if type(appid) is int:
            entry = self._rows.setdefault(appid, {"sources": None, "discovery": None})
            entry["discovery"] = discovery
        revision = result.get("settings_revision")
        if type(revision) is int:
            self.settings_revision = revision
        return {**result, **discovery}
