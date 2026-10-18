"""Source snapshots, local source settings writes and cancellable source tasks.

All network work belongs to the Rust core. This adapter only forwards v5 source
methods, keeps a display snapshot of `SourceSummary`, and tracks one background
task per source/match key so the frontend can poll progress without a scan.
"""
import asyncio
import copy
import re
import time
import uuid
from pathlib import Path
from threading import Event

from protocol_validation import (MAX_INSTANCE_ID_BYTES, JS_MAX_INTEGER,
                                 validate_capabilities, validate_id)

TERMINAL = frozenset({"completed", "cancelled", "failed"})
CANCELLED_CODES = frozenset({"cancelled", "task_cancelled"})
NAME_HINT_LIMIT = 256
# Finished task snapshots are retained for UI polling and may carry full
# candidate lists; keep that history bounded over long-running plugin sessions.
RETAINED_FINISHED_TASKS = 64
LANGUAGE_PATTERN = re.compile(r"[A-Za-z]{2,3}(?:-[A-Za-z]{2,4})?")


def due_source_ids(summaries, now):
    """Source ids whose automatic update is due at `now` (Unix seconds).

    `list_sources` derives `next_due_at` from the last attempt; the host only
    compares it with its own clock. Fallback indexers and disabled sources or
    interval 0 never participate (p1-contract §10).
    """
    due = []
    for source in summaries:
        if not source.get("enabled") or source.get("role") != "publisher":
            continue
        interval = source.get("interval_hours")
        next_due = source.get("next_due_at")
        if type(interval) is not int or interval <= 0 or type(next_due) is not int:
            continue
        if next_due <= now:
            due.append(source["id"])
    return due


def _name_hints(value):
    if value is None:
        return []
    if (not isinstance(value, list) or len(value) > 3
            or any(not isinstance(name, str) or not name.strip() or len(name) > NAME_HINT_LIMIT
                   for name in value)):
        raise ValueError("invalid_discovery_query")
    return list(value)


def _language(value):
    if value is None:
        return None
    if not isinstance(value, str) or not LANGUAGE_PATTERN.fullmatch(value):
        raise ValueError("invalid_discovery_query")
    return value


class SourceService:
    def __init__(self, bridge, state_dir):
        self.bridge = bridge
        self._state_root = Path(state_dir).absolute()
        self.state_dir = self._state_root / "sources"
        self.settings_revision = 0
        self._summaries = []
        self._tasks = {}
        self._closed = False

    # -- lifecycle ---------------------------------------------------------
    async def initialize(self):
        caps = await self.bridge.call_sync("get_capabilities", {})
        validate_capabilities(caps)
        self._apply_summaries(await self.bridge.call_sync("list_sources", {}))
        return self

    def prime(self, scan_report):
        """Refresh the display snapshot from one Rust `scan` result (no network)."""
        if not isinstance(scan_report, dict):
            raise ValueError("invalid_scan_report")
        revision = scan_report.get("settings_revision")
        if type(revision) is int:
            self.settings_revision = revision
        if scan_report.get("sources") is not None:
            self._apply_summaries(scan_report["sources"])

    async def close(self):
        await self.shutdown()

    async def shutdown(self):
        self._closed = True
        tasks = list(self._tasks.values())
        for task in tasks:
            if task["state"] not in TERMINAL:
                task["state"] = "cancelling"
                task["cancel"].set()
        await asyncio.gather(*(task["worker"] for task in tasks if task["worker"] is not None),
                             return_exceptions=True)

    # -- snapshot bookkeeping ---------------------------------------------
    def _apply_summaries(self, summaries):
        if not isinstance(summaries, list):
            raise ValueError("invalid_source_summaries")
        self._summaries = [dict(item) for item in summaries]

    def _apply_source_status(self, source_id, status, revision=None, complete=None):
        if not isinstance(status, dict):
            return
        for summary in self._summaries:
            if summary.get("id") != source_id:
                continue
            if "code" in status:
                summary["status"] = status["code"]
            for field in ("checked_at", "cached_at", "entry_count"):
                if field in status:
                    summary[field] = status[field]
            if type(revision) is int:
                summary["revision"] = revision
            if type(complete) is bool:
                summary["complete"] = complete
            return

    def list_sources(self):
        return [copy.deepcopy(item) for item in self._summaries]

    async def reload_sources(self):
        self._apply_summaries(await self.bridge.call_sync("list_sources", {}))
        return self.list_sources()

    # -- local writes (one-shot serve) ------------------------------------
    async def _call_sync(self, method, params):
        result = await self.bridge.call_sync(method, params)
        self.settings_revision = result["settings_revision"]
        await self.reload_sources()
        return result

    async def set_enabled(self, source_id, enabled):
        return await self._call_sync("set_source_enabled",
                                     {"source_id": source_id, "enabled": bool(enabled)})

    async def add_source(self, rule):
        return await self._call_sync("add_source", {"rule": rule})

    async def set_game_source(self, appid, source_id, subscribed):
        return await self._call_sync("set_game_source",
                                     {"appid": appid, "source_id": source_id,
                                      "subscribed": bool(subscribed)})

    async def set_update_interval(self, source_id, interval_hours):
        validate_id(source_id)
        if type(interval_hours) is not int or not 0 <= interval_hours <= JS_MAX_INTEGER:
            raise ValueError("invalid_interval")
        return await self._call_sync("set_source_update_interval",
                                     {"source_id": source_id, "interval_hours": interval_hours})

    # -- source task registry ---------------------------------------------
    def _snapshot(self, task):
        return {
            "task_id": task["task_id"], "kind": task["kind"], "key": task["key"],
            "state": task["state"], "instances": list(task["instances"]), "appid": task["appid"],
            "progress": copy.deepcopy(task["progress"]), "matches": copy.deepcopy(task["matches"]),
            "result": copy.deepcopy(task["result"]), "error": task["error"],
            "error_detail": task["error_detail"], "started_at": task["started_at"],
        }

    def _active(self, kind, key):
        for task in self._tasks.values():
            if task["kind"] == kind and task["key"] == key and task["state"] not in TERMINAL:
                return task
        return None

    def _prune(self):
        while len(self._tasks) > RETAINED_FINISHED_TASKS:
            oldest = next(iter(self._tasks.values()))
            if oldest["state"] not in TERMINAL:
                return
            del self._tasks[oldest["task_id"]]

    def _start(self, kind, key, method, params, instances, appid):
        existing = self._active(kind, key)
        if existing is not None:
            return self._snapshot(existing)
        if self._closed:
            raise RuntimeError("source_service_closed")
        task = {"task_id": str(uuid.uuid4()), "kind": kind, "key": key, "state": "running",
                "instances": list(instances), "appid": appid, "progress": None, "matches": {},
                "result": None, "error": None, "error_detail": None,
                "cancel": Event(), "started_at": int(time.time()), "worker": None}
        self._tasks[task["task_id"]] = task
        self._prune()
        task["worker"] = asyncio.create_task(self._run(task, method, params))
        return self._snapshot(task)

    def start_update_source(self, source_id, force=False):
        validate_id(source_id)
        if type(force) is not bool:
            raise ValueError("invalid_request")
        return self._start("update_source", source_id, "update_source",
                           {"source_id": source_id, "force": force}, [], None)

    def start_match_games(self, instances=None, name_hints=None, preferred_language=None,
                          force_details=False):
        selected = [] if instances is None else instances
        if not isinstance(selected, list):
            raise ValueError("invalid_request")
        for instance in selected:
            validate_id(instance, MAX_INSTANCE_ID_BYTES)
        hints = _name_hints(name_hints)
        language = _language(preferred_language)
        if type(force_details) is not bool:
            raise ValueError("invalid_request")
        params = {}
        if selected:
            params["instances"] = list(selected)
        if hints:
            params["name_hints"] = hints
        if language is not None:
            params["preferred_language"] = language
        if force_details:
            params["force_details"] = True
        return self._start("match_games", "match", "match_games", params, selected, None)

    def start_rematch_game(self, instance_id=None, appid=None, name_hints=None,
                           preferred_language=None):
        if (instance_id is None) == (appid is None):
            raise ValueError("instance_required")
        if instance_id is not None:
            validate_id(instance_id, MAX_INSTANCE_ID_BYTES)
        if appid is not None and (type(appid) is not int or not 0 <= appid <= 0xFFFFFFFF):
            raise ValueError("invalid_request")
        hints = _name_hints(name_hints)
        language = _language(preferred_language)
        params = {"instance_id": instance_id} if instance_id is not None else {"appid": appid}
        if hints:
            params["name_hints"] = hints
        if language is not None:
            params["preferred_language"] = language
        key = instance_id if instance_id is not None else f"appid:{appid}"
        return self._start("rematch_game", key, "rematch_game", params,
                           [instance_id] if instance_id is not None else [], appid)

    def task_status(self, task_id=None):
        if task_id is None:
            return [self._snapshot(task) for task in self._tasks.values()]
        task = self._tasks.get(task_id)
        if task is None:
            raise ValueError("task_not_found")
        return self._snapshot(task)

    def cancel_task(self, task_id):
        task = self._tasks.get(task_id)
        if task is None:
            raise ValueError("task_not_found")
        if task["state"] not in TERMINAL:
            task["state"] = "cancelling"
            task["cancel"].set()
        return self._snapshot(task)

    def has_active_source_task(self, source_id):
        return any(task["kind"] == "update_source" and task["key"] == source_id
                   and task["state"] not in TERMINAL for task in self._tasks.values())

    # -- task execution ----------------------------------------------------
    def _on_event(self, task, event):
        if not isinstance(event, dict):
            return
        revision = event.get("settings_revision")
        if type(revision) is int:
            self.settings_revision = revision
        kind = event.get("event")
        if kind in ("source_progress", "match_progress"):
            task["progress"] = {
                "phase": event.get("phase"),
                "completed": event.get("completed"),
                "total": event.get("total"),
                "running": bool(event.get("running", True)),
                "source_id": event.get("source_id"),
                "settings_revision": revision,
            }
        elif kind == "source_update":
            self._apply_source_status(event.get("source_id"), event.get("status"))
        elif kind == "match_update":
            appid = event.get("appid")
            if type(appid) is int:
                task["matches"][str(appid)] = {
                    "appid": appid,
                    "instance_id": event.get("instance_id"),
                    "status": event.get("status"),
                    "candidates": event.get("candidates", []),
                    "diagnostics": event.get("diagnostics", []),
                    "settings_revision": revision,
                }

    def _finish_result(self, task, method, result):
        if not isinstance(result, dict):
            raise ValueError("invalid_core_response")
        revision = result.get("settings_revision")
        if type(revision) is int:
            self.settings_revision = revision
        if method == "update_source":
            self._apply_source_status(result.get("source_id"), result.get("status"),
                                      result.get("revision"), result.get("complete"))
        task["result"] = result
        task["state"] = "completed"

    async def _run(self, task, method, params):
        try:
            result = await self.bridge._call(method, params,
                                             on_event=lambda event: self._on_event(task, event),
                                             cancel=task["cancel"])
            self._finish_result(task, method, result)
        except asyncio.CancelledError:
            task["state"] = "cancelled"
            task["error"] = "cancelled"
            raise
        except Exception as error:
            raw = getattr(error, "code", None) or str(error)
            code = raw if re.fullmatch(r"[a-z0-9_]{1,128}", raw) else "host_transport_failed"
            detail = getattr(error, "detail", None) or (str(error) if code != raw else None)
            task["state"] = "cancelled" if task["cancel"].is_set() or code in CANCELLED_CODES else "failed"
            task["error"] = code
            task["error_detail"] = detail
