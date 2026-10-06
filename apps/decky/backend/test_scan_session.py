"""Behavioral checks for progressive scan sessions."""
import asyncio
import unittest

from scan_session import ScanSession


def report(count=1):
    return {"installs": [
        {"game": {"instance_id": str(index), "availability": "installed"},
         "discovery": {"status": "cache_complete", "candidates": [{"candidate_id": f"cached-{index}"}], "diagnostics": []}}
        for index in range(count)]}


class ScanSessionTests(unittest.IsolatedAsyncioTestCase):
    async def test_concurrency_is_bounded_and_progress_finishes(self):
        started = asyncio.Queue()
        release = asyncio.Event()
        active = peak = 0

        async def discover(game):
            nonlocal active, peak
            active += 1
            peak = max(peak, active)
            await started.put(game["instance_id"])
            await release.wait()
            active -= 1
            return {"status": "complete", "candidates": [], "diagnostics": []}

        async def project(row):
            row["projected"] = True

        session = ScanSession(report(5), discover, project, lambda: True, concurrency=2)
        try:
            await asyncio.wait_for(started.get(), 1)
            await asyncio.wait_for(started.get(), 1)
            self.assertEqual(started.qsize(), 0)
            self.assertEqual(peak, 2)
            self.assertEqual(session.snapshot()["scan_progress"], {"completed": 0, "total": 5, "running": True})
            release.set()
            await asyncio.wait_for(session.worker, 1)
            result = session.snapshot()
            self.assertEqual(result["scan_progress"], {"completed": 5, "total": 5, "running": False})
            self.assertTrue(all(row["projected"] for row in result["installs"]))
        finally:
            await session.close()

    async def test_stale_result_does_not_replace_cached_candidates(self):
        entered = asyncio.Event()
        release = asyncio.Event()
        current = True
        projected = 0

        async def discover(_game):
            entered.set()
            await release.wait()
            return {"status": "complete", "candidates": [{"candidate_id": "new"}], "diagnostics": []}

        async def project(_row):
            nonlocal projected
            projected += 1

        session = ScanSession(report(), discover, project, lambda: current)
        try:
            await asyncio.wait_for(entered.wait(), 1)
            current = False
            release.set()
            await asyncio.wait_for(session.worker, 1)
            self.assertEqual(projected, 0)
            self.assertEqual(session.snapshot()["installs"][0]["discovery"]["candidates"][0]["candidate_id"], "cached-0")
        finally:
            await session.close()

    async def test_source_failure_keeps_successful_cached_candidates(self):
        async def discover(_game):
            raise OSError("source offline")

        session = ScanSession(report(), discover, lambda _row: None, lambda: True)
        try:
            await asyncio.wait_for(session.worker, 1)
            result = session.snapshot()["installs"][0]["discovery"]
            self.assertEqual(result["status"], "failed")
            self.assertEqual(result["candidates"][0]["candidate_id"], "cached-0")
            self.assertEqual(result["diagnostics"][0]["code"], "source_refresh_failed")
        finally:
            await session.close()

    async def test_empty_library_remains_running_until_source_refresh_completes(self):
        release = asyncio.Event()
        async def refresh(snapshot):
            await release.wait()
            snapshot["sources"] = [{"id": "official", "status": "source_refreshed"}]
        session = ScanSession(report(0), lambda game: None, lambda row: None, lambda: True, refresh=refresh)
        self.assertTrue(session.snapshot()["scan_progress"]["running"])
        release.set()
        await session.worker
        self.assertFalse(session.snapshot()["scan_progress"]["running"])
        self.assertEqual(session.snapshot()["sources"][0]["status"], "source_refreshed")

    async def test_zip_source_failure_does_not_claim_complete_empty_query(self):
        async def refresh(snapshot):
            snapshot["diagnostics"] = [{"source_id": "zip", "code": "source_refresh_failed"}]
        async def discover(game):
            return {"status": "cache_complete", "candidates": [], "diagnostics": []}
        async def project(row):
            row["patches"] = []
        session = ScanSession(report(), discover, project, lambda: True, refresh=refresh)
        await session.worker
        self.assertEqual(session.snapshot()["installs"][0]["discovery"]["status"], "source_partial")
