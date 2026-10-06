"""Progressive, bounded discovery owned by the backend scan session."""
import asyncio
import copy


class ScanSession:
    def __init__(self, report, discover, project, current, concurrency=3, refresh=None):
        self.report = copy.deepcopy(report)
        self.current = current
        self.discover = discover
        self.project = project
        self.refresh = refresh
        self.semaphore = asyncio.Semaphore(concurrency)
        self.rows = [row for row in self.report["installs"] if row["game"].get("availability") == "installed"]
        self.report["scan_progress"] = {"completed": 0, "total": len(self.rows), "running": bool(self.rows) or bool(refresh)}
        self.worker = asyncio.create_task(self.run())

    def snapshot(self):
        return copy.deepcopy(self.report)

    async def run(self):
        async def one(row):
            async with self.semaphore:
                if not self.current():
                    return
                try:
                    result = await asyncio.wait_for(self.discover(row["game"]), 90)
                    if not self.current() or result.get("status") == "stale":
                        return
                    row["discovery"] = {key: result.get(key, []) for key in ("status", "candidates", "diagnostics")}
                    await self.project(row)
                    failures = [d for d in self.report.get("diagnostics", [])
                                if d.get("code") == "source_refresh_failed"]
                    if failures and not row.get("patches") and not row["discovery"]["candidates"]:
                        row["discovery"]["status"] = "source_partial"
                        row["discovery"]["diagnostics"].extend(failures)
                except asyncio.CancelledError:
                    raise
                except Exception as error:
                    if self.current():
                        # Preserve last successful candidates, including on timeout.
                        row.setdefault("discovery", {}).update(status="failed", diagnostics=[{"code": "source_refresh_failed", "detail": str(error)}])
                finally:
                    if self.current():
                        self.report["scan_progress"]["completed"] += 1
        try:
            if self.refresh and self.current():
                await self.refresh(self.report)
            if not self.current():
                return
            await asyncio.gather(*(one(row) for row in self.rows))
        finally:
            self.report["scan_progress"]["running"] = False

    async def close(self):
        self.worker.cancel()
        await asyncio.gather(self.worker, return_exceptions=True)
