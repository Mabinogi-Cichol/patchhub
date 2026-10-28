"""Offline checks for the Rust-backed `SourceService` adapter (no real binary)."""
import asyncio
import copy
import inspect
import tempfile
from pathlib import Path
import unittest

from bridge import CoreError
from source_service import SourceService, due_source_ids
from protocol_test_support import response

SUMMARIES = [
    {"id": "patchmoe", "name": "patch.moe", "adapter": "patchmoe", "role": "publisher",
     "discovery_only": False, "priority": 100, "enabled": True,
     "pages": ["https://www.patch.moe/"], "page_hosts": ["www.patch.moe"],
     "download_hosts": ["download.patch.moe"], "root_layout": "game_root",
     "status": "cached", "entry_count": 2, "revision": 1, "complete": True,
     "interval_hours": 24, "next_due_at": 1000, "cached_at": None, "checked_at": None,
     "origin": "builtin"},
    {"id": "hgamer", "name": "HGamer", "adapter": "hgamer", "role": "fallback_indexer",
     "discovery_only": True, "priority": 100, "enabled": True,
     "pages": ["https://hgamer.tw/"], "page_hosts": ["hgamer.tw"], "download_hosts": [],
     "root_layout": None, "status": "not_refreshed", "entry_count": 0, "revision": 0,
     "complete": False, "interval_hours": 0, "next_due_at": None,
     "cached_at": None, "checked_at": None, "origin": "builtin"},
]
SCAN = {
    "settings_revision": 5,
    "sources": SUMMARIES,
    "diagnostics": [{"code": "scan_note"}],
    "installs": [{"game": {"instance_id": "i1", "appid": 42, "name": "Game"}, "patches": []}],
}


class FakeBridge:
    def __init__(self):
        self.sync_calls = []
        self.stream_calls = []
        self.sync_results = {}
        self.stream_results = {}
        self.stream_events = {}
        self.gate = None

    async def call_sync(self, method, params, timeout=None):
        self.sync_calls.append((method, params))
        result = self.sync_results.get(method)
        return copy.deepcopy(result(params) if callable(result) else result)

    async def _call(self, method, params, on_event=None, cancel=None, timeout=None, controlled=None):
        self.stream_calls.append((method, params))
        while self.gate is not None and not self.gate.is_set():
            if cancel is not None and cancel.is_set():
                raise CoreError("task_cancelled")
            await asyncio.sleep(0)
        if cancel is not None and cancel.is_set():
            raise CoreError("task_cancelled")
        for event in self.stream_events.get(method, []):
            emitted = on_event(event) if on_event is not None else None
            if inspect.isawaitable(emitted):
                await emitted
        result = self.stream_results.get(method)
        return copy.deepcopy(result(params) if callable(result) else result)


def source_update_event(**updates):
    from protocol_test_support import event
    return event("source_update", **updates)


def match_update_event(**updates):
    from protocol_test_support import event
    return event("match_update", **updates)


class SourceServiceTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bridge = FakeBridge()
        self.bridge.sync_results = {
            "get_capabilities": response("get_capabilities")["result"],
            "list_sources": list(SUMMARIES),
        }
        self.service = SourceService(self.bridge, self.root)

    async def settle(self, task_id):
        task = self.service._tasks[task_id]
        await asyncio.wait_for(task["worker"], 1)
        return self.service.task_status(task_id)

    async def test_initialize_reads_current_summaries_without_import_adapter(self):
        await self.service.initialize()
        self.assertEqual(self.service.list_sources(), SUMMARIES)
        self.assertEqual([call[0] for call in self.bridge.sync_calls], ["get_capabilities", "list_sources"])
        for name in ("matches", "cached_candidates", "_rules", "builtins", "validate_download", "source_for_release"):
            self.assertFalse(hasattr(self.service, name))

    async def test_prime_exposes_summaries_from_one_scan(self):
        await self.service.initialize()
        self.service.prime(SCAN)
        self.assertEqual(self.service.settings_revision, 5)
        self.assertEqual(self.service.list_sources(), SUMMARIES)
        self.assertEqual(len(self.bridge.sync_calls), 2)

    async def test_prime_rejects_malformed_scan_results(self):
        with self.assertRaises(ValueError):
            self.service.prime(["not", "a", "report"])
        with self.assertRaises(ValueError):
            self.service.prime({"installs": [], "sources": "not a list"})
        self.bridge.sync_results["list_sources"] = {"not": "a list"}
        with self.assertRaises(ValueError):
            await self.service.initialize()

    async def test_local_writes_use_one_shot_call_and_reload(self):
        await self.service.initialize()
        updated_list = lambda _params: [{**SUMMARIES[0], "enabled": False}, SUMMARIES[1]]
        self.bridge.sync_results["set_source_enabled"] = {"settings_revision": 6,
                                                          "source_id": "patchmoe", "enabled": False}
        self.bridge.sync_results["add_source"] = {"settings_revision": 6, "rule": {"id": "custom"}}
        self.bridge.sync_results["set_game_source"] = {"settings_revision": 6,
                                                       "source_id": "patchmoe", "subscribed": False}
        self.bridge.sync_results["list_sources"] = updated_list
        await self.service.set_enabled("patchmoe", False)
        self.assertEqual(self.service.settings_revision, 6)
        self.assertFalse(self.service.list_sources()[0]["enabled"])
        self.assertIn(("set_source_enabled", {"source_id": "patchmoe", "enabled": False}),
                      self.bridge.sync_calls)
        await self.service.add_source({"id": "custom"})
        self.assertIn(("add_source", {"rule": {"id": "custom"}}), self.bridge.sync_calls)
        await self.service.set_game_source(42, "patchmoe", False)
        self.assertIn(("set_game_source", {"appid": 42, "source_id": "patchmoe", "subscribed": False}),
                      self.bridge.sync_calls)

    async def test_update_interval_validates_generated_bounds_and_reloads(self):
        await self.service.initialize()
        self.bridge.sync_results["set_source_update_interval"] = {
            "settings_revision": 7, "source_id": "patchmoe", "interval_hours": 6}
        self.bridge.sync_results["list_sources"] = lambda _params: [
            {**SUMMARIES[0], "interval_hours": 6, "next_due_at": 5000}, SUMMARIES[1]]
        result = await self.service.set_update_interval("patchmoe", 6)
        self.assertEqual(result["interval_hours"], 6)
        self.assertEqual(self.service.settings_revision, 7)
        self.assertEqual(self.service.list_sources()[0]["interval_hours"], 6)
        self.assertIn(("set_source_update_interval",
                       {"source_id": "patchmoe", "interval_hours": 6}), self.bridge.sync_calls)
        for invalid in (-1, 1.5, "24", None, True):
            with self.subTest(value=invalid):
                with self.assertRaisesRegex(ValueError, "invalid_interval"):
                    await self.service.set_update_interval("patchmoe", invalid)
        with self.assertRaises(ValueError):
            await self.service.set_update_interval("", 6)

    async def test_update_source_maps_progress_events_and_patches_status(self):
        await self.service.initialize()
        self.bridge.stream_events["update_source"] = [
            {"event": "source_progress", "source_id": "patchmoe", "phase": "index",
             "completed": 2, "total": 5, "running": True, "settings_revision": 5},
            source_update_event(source_id="patchmoe", settings_revision=5,
                                status={"checked_at": 9, "cached_at": 8, "entry_count": 4,
                                        "code": "source_refreshed"}),
        ]
        self.bridge.stream_results["update_source"] = {
            "source_id": "patchmoe", "settings_revision": 6,
            "status": {"checked_at": 10, "cached_at": 10, "entry_count": 5, "code": "source_refreshed"},
            "revision": 3, "complete": True, "affected_appids": [42]}
        task = self.service.start_update_source("patchmoe", True)
        self.assertEqual((task["kind"], task["key"], task["state"]), ("update_source", "patchmoe", "running"))
        final = await self.settle(task["task_id"])
        self.assertEqual(final["state"], "completed")
        self.assertEqual(final["result"]["affected_appids"], [42])
        self.assertEqual(final["progress"]["phase"], "index")
        self.assertEqual(final["progress"]["completed"], 2)
        self.assertEqual(self.service.settings_revision, 6)
        summary = self.service.list_sources()[0]
        self.assertEqual((summary["status"], summary["entry_count"], summary["revision"], summary["complete"]),
                         ("source_refreshed", 5, 3, True))
        self.assertEqual(summary["checked_at"], 10)
        self.assertIn(("update_source", {"source_id": "patchmoe", "force": True}), self.bridge.stream_calls)

    async def test_same_source_is_deduplicated_while_different_sources_run_in_parallel(self):
        await self.service.initialize()
        self.bridge.gate = asyncio.Event()
        self.bridge.stream_results["update_source"] = {
            "source_id": "patchmoe", "settings_revision": 5,
            "status": {"checked_at": 1, "cached_at": None, "entry_count": 0, "code": "source_refreshed"},
            "revision": 1, "complete": True, "affected_appids": []}
        first = self.service.start_update_source("patchmoe")
        same = self.service.start_update_source("patchmoe")
        other = self.service.start_update_source("hgamer")
        self.assertEqual(first["task_id"], same["task_id"])
        self.assertNotEqual(first["task_id"], other["task_id"])
        self.assertEqual(len(self.service._tasks), 2)
        self.bridge.gate.set()
        await self.settle(first["task_id"])
        await self.settle(other["task_id"])
        self.assertEqual(len(self.bridge.stream_calls), 2)

    async def test_match_games_records_progress_and_latest_match_updates(self):
        await self.service.initialize()
        self.bridge.stream_events["match_games"] = [
            {"event": "match_progress", "phase": "inventory", "completed": 0, "total": 1,
             "running": True, "settings_revision": 5},
            match_update_event(appid=42, instance_id="i1", status="publisher_match",
                               candidates=[{"candidate_id": "c1"}], diagnostics=[],
                               settings_revision=5),
        ]
        self.bridge.stream_results["match_games"] = {
            "settings_revision": 5, "source_revisions": {"patchmoe": 3},
            "results": [{"appid": 42, "instance_id": "i1", "status": "publisher_match",
                         "candidates": [{"candidate_id": "c1"}], "diagnostics": [],
                         "source_revisions": {"patchmoe": 3}}]}
        task = self.service.start_match_games(["i1"], ["Game"], True)
        self.assertEqual((task["kind"], task["key"]), ("match_games", "match"))
        final = await self.settle(task["task_id"])
        self.assertEqual(final["state"], "completed")
        self.assertEqual(final["matches"]["42"]["status"], "publisher_match")
        self.assertEqual(final["progress"]["phase"], "inventory")
        self.assertEqual(final["result"]["results"][0]["appid"], 42)
        self.assertEqual(self.bridge.stream_calls,
                         [("match_games", {"instances": ["i1"], "name_hints": ["Game"],
                                           "force_details": True})])

    async def test_match_games_and_rematch_deduplicate_by_kind_and_key(self):
        await self.service.initialize()
        self.bridge.gate = asyncio.Event()
        for method in ("match_games", "rematch_game"):
            self.bridge.stream_results[method] = {
                "settings_revision": 5, "source_revisions": {}, "results": []}
        first = self.service.start_match_games()
        second = self.service.start_match_games()
        rematch = self.service.start_rematch_game("i1")
        rematch_again = self.service.start_rematch_game("i1")
        by_appid = self.service.start_rematch_game(appid=42)
        by_appid_again = self.service.start_rematch_game(appid=42)
        self.assertEqual(first["task_id"], second["task_id"])
        self.assertEqual(rematch["task_id"], rematch_again["task_id"])
        self.assertEqual(by_appid["task_id"], by_appid_again["task_id"])
        self.assertNotEqual(first["task_id"], rematch["task_id"])
        self.assertEqual(rematch["instances"], ["i1"])
        self.assertEqual(by_appid["appid"], 42)
        self.bridge.gate.set()
        for task in (first, rematch, by_appid):
            await self.settle(task["task_id"])

    async def test_rematch_validates_target_and_query_fields(self):
        await self.service.initialize()
        for kwargs in ({}, {"instance_id": "i1", "appid": 42}):
            with self.subTest(kwargs=kwargs):
                with self.assertRaises(ValueError):
                    self.service.start_rematch_game(**kwargs)
        for kwargs in ({"instance_id": "i1", "name_hints": ["a"] * 4},
                       {"appid": -1}, {"appid": 2 ** 32}):
            with self.subTest(kwargs=kwargs):
                with self.assertRaises(ValueError):
                    self.service.start_rematch_game(**kwargs)
        with self.assertRaises(ValueError):
            self.service.start_match_games(["i1"], None, 1)
        self.assertEqual(self.bridge.stream_calls, [])

    async def test_cancel_sets_cancelling_and_core_cancellation_is_terminal(self):
        await self.service.initialize()
        self.bridge.gate = asyncio.Event()
        self.bridge.stream_results["update_source"] = {"source_id": "patchmoe", "settings_revision": 5}
        task = self.service.start_update_source("patchmoe")
        snapshot = self.service.cancel_task(task["task_id"])
        self.assertEqual(snapshot["state"], "cancelling")
        self.bridge.gate.set()
        final = await self.settle(task["task_id"])
        self.assertEqual(final["state"], "cancelled")
        self.assertEqual(final["error"], "task_cancelled")
        with self.assertRaisesRegex(ValueError, "task_not_found"):
            self.service.task_status("missing")
        with self.assertRaisesRegex(ValueError, "task_not_found"):
            self.service.cancel_task("missing")

    async def test_task_failure_keeps_domain_code_and_detail(self):
        await self.service.initialize()
        async def failing(method, params, on_event=None, cancel=None, timeout=None, controlled=None):
            raise CoreError("source_refresh_failed", "offline")
        self.bridge._call = failing
        task = self.service.start_update_source("patchmoe")
        final = await self.settle(task["task_id"])
        self.assertEqual(final["state"], "failed")
        self.assertEqual(final["error"], "source_refresh_failed")
        self.assertEqual(final["error_detail"], "offline")

    async def test_task_status_without_id_lists_retained_tasks_and_prunes_old_terminal_ones(self):
        await self.service.initialize()
        self.bridge.stream_results["update_source"] = {
            "source_id": "patchmoe", "settings_revision": 5,
            "status": {"checked_at": 1, "code": "source_refreshed", "entry_count": 0},
            "revision": 1, "complete": True, "affected_appids": []}
        task_ids = []
        for index in range(70):
            task = self.service.start_update_source(f"source-{index}")
            await self.settle(task["task_id"])
            task_ids.append(task["task_id"])
        retained = self.service.task_status()
        self.assertLessEqual(len(retained), 64)
        self.assertEqual(retained[-1]["task_id"], task_ids[-1])
        self.assertNotEqual(retained[0]["task_id"], task_ids[0])

    async def test_close_cancels_in_flight_tasks_and_is_idempotent(self):
        await self.service.initialize()
        self.bridge.gate = asyncio.Event()
        self.bridge.stream_results["update_source"] = {"source_id": "patchmoe", "settings_revision": 5}
        task = self.service.start_update_source("patchmoe")
        await self.service.close()
        await self.service.close()
        final = self.service.task_status(task["task_id"])
        self.assertEqual(final["state"], "cancelled")
        self.assertTrue(self.service._closed)


class DueSourceTests(unittest.TestCase):
    def summary(self, **fields):
        base = {"id": "source", "enabled": True, "role": "publisher", "interval_hours": 24,
                "next_due_at": 1000, "checked_at": 100, "status": "source_refreshed"}
        base.update(fields)
        return base

    def test_due_selection_uses_next_due_and_excludes_ineligible_sources(self):
        summaries = [
            self.summary(id="due", next_due_at=1000),
            self.summary(id="not-yet", next_due_at=2000),
            self.summary(id="disabled", enabled=False),
            self.summary(id="fallback", role="fallback_indexer"),
            self.summary(id="off", interval_hours=0),
            self.summary(id="missing-interval", interval_hours=None),
            self.summary(id="missing-due", next_due_at=None),
        ]
        self.assertEqual(due_source_ids(summaries, 1000), ["due"])

    def test_failure_recomputes_next_due_from_checked_at(self):
        failed = self.summary(status="source_refresh_failed", checked_at=1800, next_due_at=1800 + 24 * 3600)
        self.assertEqual(due_source_ids([failed], 1800 + 24 * 3600 - 1), [])
        self.assertEqual(due_source_ids([failed], 1800 + 24 * 3600), ["source"])


if __name__ == "__main__":
    unittest.main()
