"""Event-driven progressive scan session.

Concurrency, budgets and dedup are owned by the shared Rust source engine. The
session only renders cached rows up front, then feeds `source_progress` /
`source_update` into `scan_progress` and each row's `discovery`, projecting the
affected rows. It keeps the old `snapshot()` / `close()` API.
"""
import asyncio
import copy
from threading import Event


class ScanSession:
    def __init__(self, report, refresh, scan_sources, project, current):
        self.report = copy.deepcopy(report)
        self.refresh = refresh
        self.scan_sources = scan_sources
        self.project = project
        self.current = current
        self._cancel = Event()
        self.rows = [row for row in self.report["installs"] if row["game"].get("availability") == "installed"]
        self._by_instance = {}
        self._by_appid = {}
        for row in self.rows:
            instance = row["game"].get("instance_id")
            if isinstance(instance, str):
                self._by_instance[instance] = row
            appid = row["game"].get("appid")
            if type(appid) is int:
                self._by_appid.setdefault(appid, row)
        progress = self.report.setdefault("scan_progress", {})
        progress["total"] = len(self.rows)
        progress.setdefault("completed", 0)
        progress["running"] = True
        self.worker = asyncio.create_task(self.run())

    def snapshot(self):
        return copy.deepcopy(self.report)

    async def on_event(self, event):
        if not isinstance(event, dict) or not self.current():
            return
        kind = event.get("event")
        if kind == "source_progress":
            completed, total = event.get("completed"), event.get("total")
            if type(completed) is int and type(total) is int:
                self.report["scan_progress"] = {
                    "completed": completed, "total": total,
                    "running": bool(event.get("running", True))}
        elif kind == "source_update":
            discovery = event.get("discovery")
            if not isinstance(discovery, dict):
                return
            row = None
            instance = event.get("instance_id")
            if isinstance(instance, str):
                row = self._by_instance.get(instance)
            if row is None and type(event.get("appid")) is int:
                row = self._by_appid.get(event["appid"])
            if row is None:
                return
            row["discovery"] = {key: discovery.get(key, [])
                                for key in ("status", "candidates", "diagnostics")}
            try:
                await self.project(row)
            except asyncio.CancelledError:
                raise
            except Exception as error:
                row["discovery"].setdefault("diagnostics", []).append(
                    {"code": "scan_projection_failed", "detail": str(error)[:200]})

    async def run(self):
        try:
            if self.current():
                try:
                    await self.refresh(self.on_event, self._cancel)
                except asyncio.CancelledError:
                    raise
                except Exception as error:
                    self._fail(error)
            if not self.current():
                return
            try:
                await self.scan_sources(self.on_event, self._cancel)
            except asyncio.CancelledError:
                raise
            except Exception as error:
                self._fail(error)
        finally:
            self.report["scan_progress"]["running"] = False

    def _fail(self, error):
        if not self.current():
            return
        diagnostics = self.report.setdefault("diagnostics", [])
        if isinstance(diagnostics, list):
            diagnostics.append({"code": "source_refresh_failed", "detail": str(error)[:200]})

    async def close(self):
        self._cancel.set()
        self.worker.cancel()
        await asyncio.gather(self.worker, return_exceptions=True)
