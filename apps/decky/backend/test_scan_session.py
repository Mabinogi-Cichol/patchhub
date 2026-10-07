"""Behavioral checks for the event-driven scan session."""
import asyncio
import unittest

from scan_session import ScanSession


def report(count=1):
    return {"diagnostics": [], "installs": [
        {"game": {"instance_id": str(index), "appid": index, "availability": "installed"},
         "discovery": {"status": "cache_complete",
                       "candidates": [{"candidate_id": f"cached-{index}"}], "diagnostics": []}}
        for index in range(count)]}


async def no_refresh(on_event, cancel):
    return []


class ScanSessionTests(unittest.IsolatedAsyncioTestCase):
    async def test_updates_project_rows_and_progress_finishes(self):
        projected = []

        async def project(row):
            projected.append(row["game"]["instance_id"])
            row["projected"] = True

        async def scan_sources(on_event, cancel):
            await on_event({"event": "source_update", "instance_id": "0", "appid": 0,
                            "discovery": {"status": "complete", "candidates": [{"candidate_id": "new-0"}],
                                          "diagnostics": []}})
            await on_event({"event": "source_progress", "completed": 1, "total": 1, "running": False})
            return {"results": []}

        session = ScanSession(report(1), no_refresh, scan_sources, project, lambda: True)
        try:
            await asyncio.wait_for(session.worker, 1)
            snapshot = session.snapshot()
            self.assertEqual(snapshot["scan_progress"], {"completed": 1, "total": 1, "running": False})
            self.assertEqual(projected, ["0"])
            self.assertTrue(snapshot["installs"][0]["projected"])
            self.assertEqual(snapshot["installs"][0]["discovery"]["candidates"][0]["candidate_id"], "new-0")
        finally:
            await session.close()

    async def test_stale_session_ignores_events_and_keeps_cached_candidates(self):
        entered = asyncio.Event()
        release = asyncio.Event()
        current = True
        projected = []

        async def scan_sources(on_event, cancel):
            entered.set()
            await release.wait()
            await on_event({"event": "source_update", "instance_id": "0",
                            "discovery": {"status": "complete", "candidates": [{"candidate_id": "new"}],
                                          "diagnostics": []}})

        async def project(row):
            projected.append(row["game"]["instance_id"])

        session = ScanSession(report(), no_refresh, scan_sources, project, lambda: current)
        try:
            await asyncio.wait_for(entered.wait(), 1)
            current = False
            release.set()
            await asyncio.wait_for(session.worker, 1)
            self.assertEqual(projected, [])
            self.assertEqual(session.snapshot()["installs"][0]["discovery"]["candidates"][0]["candidate_id"],
                             "cached-0")
        finally:
            await session.close()

    async def test_source_failure_marks_finished_and_keeps_cache(self):
        async def scan_sources(on_event, cancel):
            raise OSError("source offline")

        session = ScanSession(report(), no_refresh, scan_sources, lambda row: None, lambda: True)
        try:
            await asyncio.wait_for(session.worker, 1)
            snapshot = session.snapshot()
            self.assertFalse(snapshot["scan_progress"]["running"])
            self.assertEqual(snapshot["installs"][0]["discovery"]["candidates"][0]["candidate_id"], "cached-0")
            self.assertEqual(snapshot["diagnostics"][0]["code"], "source_refresh_failed")
        finally:
            await session.close()

    async def test_empty_library_remains_running_until_scan_completes(self):
        release = asyncio.Event()

        async def refresh(on_event, cancel):
            await release.wait()

        session = ScanSession(report(0), refresh, lambda on_event, cancel: None, lambda row: None, lambda: True)
        self.assertTrue(session.snapshot()["scan_progress"]["running"])
        release.set()
        await asyncio.wait_for(session.worker, 1)
        self.assertFalse(session.snapshot()["scan_progress"]["running"])

    async def test_close_cancels_worker_and_clears_running(self):
        started = asyncio.Event()

        async def refresh(on_event, cancel):
            started.set()
            while not cancel.is_set():
                await asyncio.sleep(0.02)

        session = ScanSession(report(), refresh, lambda on_event, cancel: None, lambda row: None, lambda: True)
        await asyncio.wait_for(started.wait(), 1)
        await session.close()
        self.assertFalse(session.report["scan_progress"]["running"])


if __name__ == "__main__":
    unittest.main()
