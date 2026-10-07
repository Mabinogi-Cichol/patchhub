"""Offline checks for the Rust-backed `SourceService` adapter (no real binary)."""
import inspect
import tempfile
from pathlib import Path
import unittest

from source_service import SourceService, REQUIRED_METHODS

SUMMARIES = [
    {"id": "patchmoe", "name": "patch.moe", "adapter": "patchmoe", "role": "publisher",
     "discovery_only": False, "priority": 100, "enabled": True,
     "pages": ["https://www.patch.moe/"], "page_hosts": ["www.patch.moe"],
     "download_hosts": ["download.patch.moe"], "root_layout": "game_root",
     "status": "cached", "entry_count": 2, "cached_at": None, "checked_at": None, "origin": "builtin"},
    {"id": "kagura-cn", "name": "Kagura CN", "adapter": "kagura_wordpress", "role": "publisher",
     "discovery_only": True, "priority": 100, "enabled": True,
     "pages": ["https://kagurafan.com/"], "page_hosts": ["kagurafan.com"], "download_hosts": [],
     "root_layout": None, "status": "not_refreshed", "entry_count": 0,
     "cached_at": None, "checked_at": None, "origin": "added"},
]
GAME = {"instance_id": "i1", "appid": 42, "name": "Game"}
SCAN = {
    "settings_revision": 5,
    "sources": SUMMARIES,
    "diagnostics": [{"code": "scan_note"}],
    "installs": [{
        "game": GAME,
        "sources": [{**SUMMARIES[0], "subscribed": True, "matched": True}],
        "matches": [{"source_id": "patchmoe", "release_id": "patchmoe-42", "source_page": "https://www.patch.moe/",
                     "url": "https://download.patch.moe/a.zip", "language": "zh-CN", "title": "Game",
                     "root_layout": "game_root", "enabled": True, "subscribed": True}],
        "discovery": {"appid": 42, "status": "cache_complete",
                      "candidates": [{"candidate_id": "c1"}], "diagnostics": []},
    }],
}


class FakeBridge:
    def __init__(self):
        self.sync_calls = []
        self.stream_calls = []
        self.sync_results = {}
        self.stream_results = {}
        self.events = []

    async def call_sync(self, method, params, timeout=None):
        self.sync_calls.append((method, params))
        result = self.sync_results.get(method)
        return result(params) if callable(result) else result

    async def _call(self, method, params, on_event=None, cancel=None, timeout=None, controlled=None):
        self.stream_calls.append((method, params))
        for event in self.events:
            if on_event is not None:
                emitted = on_event(event)
                if inspect.isawaitable(emitted):
                    await emitted
        return self.stream_results.get(method)


class SourceServiceTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bridge = FakeBridge()
        self.bridge.sync_results = {
            "get_capabilities": {"methods": sorted(REQUIRED_METHODS)},
            "list_sources": list(SUMMARIES),
            "get_source_state": {"settings_revision": 5,
                                 "import_report": {"imported_settings": True,
                                                   "diagnostics": [{"code": "invalid_source_cache",
                                                                    "source_id": "old"}]}},
            "get_source_settings": {"enabled": {}, "added": [], "games": {}},
        }
        self.service = SourceService(self.bridge, self.root, self.root / "sources.json")

    async def test_initialize_reads_capabilities_import_and_rules(self):
        await self.service.initialize()
        self.assertEqual(self.service.engine, "rust")
        self.assertEqual(self.service.settings_revision, 5)
        self.assertEqual({rule["id"] for rule in self.service._rules}, {"patchmoe", "kagura-cn"})
        self.assertEqual([rule["id"] for rule in self.service.builtins], ["patchmoe"])
        codes = [entry["code"] for entry in self.service.diagnostics]
        self.assertIn("source_settings_imported", codes)
        self.assertIn("invalid_source_cache", codes)

    async def test_prime_exposes_snapshot_reads(self):
        await self.service.initialize()
        self.service.prime(SCAN)
        self.assertEqual(self.service.list_sources(), SUMMARIES)
        sources = self.service.list_game_sources(GAME)
        self.assertTrue(sources[0]["subscribed"])
        self.assertEqual(self.service.matches(GAME)[0]["release_id"], "patchmoe-42")
        self.assertEqual(self.service.matches(GAME, include_disabled=True)[0]["release_id"], "patchmoe-42")
        self.assertEqual(self.service.cached_candidates(GAME)[0]["candidate_id"], "c1")
        discovery = self.service.cached_discovery(GAME)
        self.assertEqual(discovery["status"], "cache_complete")
        self.assertEqual(discovery["settings_revision"], 5)
        self.assertTrue(self.service.game_subscribed(42, "patchmoe"))
        self.assertTrue(self.service.game_subscribed(999, "unknown"))
        self.assertEqual(self.service.source_for_release("patchmoe-42")["id"], "patchmoe")
        self.assertIsNone(self.service.source_for_release("source-kagura-cn-42"))
        self.assertEqual(self.service.validate_download("patchmoe", "https://download.patch.moe/a.zip"),
                         "https://download.patch.moe/a.zip")
        with self.assertRaisesRegex(ValueError, "source_not_enabled"):
            self.service.validate_download("kagura-cn", "https://kagurafan.com/a.zip")
        with self.assertRaisesRegex(ValueError, "invalid_source_url"):
            self.service.validate_download("patchmoe", "https://evil.example/a.zip")

    async def test_local_writes_use_one_shot_call_and_reload(self):
        await self.service.initialize()
        updated_list = lambda _params: [{**SUMMARIES[0], "enabled": False}, SUMMARIES[1]]
        self.bridge.sync_results["set_source_enabled"] = {"settings_revision": 6,
                                                          "source_id": "patchmoe", "enabled": False}
        self.bridge.sync_results["list_sources"] = updated_list
        await self.service.set_enabled("patchmoe", False)
        self.assertEqual(self.service.settings_revision, 6)
        self.assertFalse(self.service._rules[0]["enabled"])
        self.assertIn(("set_source_enabled", {"source_id": "patchmoe", "enabled": False}),
                      self.bridge.sync_calls)
        await self.service.add_source({"id": "custom"})
        self.assertIn(("add_source", {"rule": {"id": "custom"}}), self.bridge.sync_calls)
        await self.service.set_game_source(42, "patchmoe", False)
        self.assertIn(("set_game_source", {"appid": 42, "source_id": "patchmoe", "subscribed": False}),
                      self.bridge.sync_calls)

    async def test_refresh_maps_statuses_and_streams_events(self):
        await self.service.initialize()
        collected = []
        self.bridge.events = [{"event": "source_update", "source_id": "patchmoe", "status": {"code": "source_refreshed"}}]
        self.bridge.stream_results["refresh_sources"] = {
            "settings_revision": 7,
            "statuses": [{"checked_at": 10, "code": "source_refreshed", "entry_count": 3, "cached_at": 9}]}
        diagnostics = await self.service.refresh(force=True, on_event=collected.append, cancel=None)
        self.assertIn(("refresh_sources", {"force": True}), self.bridge.stream_calls)
        self.assertEqual(diagnostics[0]["source_id"], "patchmoe")
        self.assertEqual(diagnostics[0]["code"], "source_refreshed")
        self.assertEqual(diagnostics[0]["entries"], 3)
        self.assertEqual(self.service.settings_revision, 7)
        self.assertEqual(len(collected), 1)

    async def test_scan_sources_and_discover_update_rows(self):
        await self.service.initialize()
        self.service.prime(SCAN)
        self.bridge.stream_results["scan_sources"] = {
            "settings_revision": 8,
            "results": [{"instance_id": "i1", "appid": 42, "status": "complete",
                         "candidates": [{"candidate_id": "c2"}], "diagnostics": []}]}
        await self.service.scan_sources(force=False)
        self.assertEqual(self.service.cached_candidates(GAME)[0]["candidate_id"], "c2")
        self.bridge.stream_results["discover_game"] = {
            "instance_id": "i1", "appid": 42, "generation": 9, "settings_revision": 8,
            "status": "complete", "candidates": [{"candidate_id": "c3"}], "diagnostics": []}
        result = await self.service.discover_game(GAME, ["hint"], "zh-CN", True, 9, is_current=lambda: True)
        self.assertEqual(result["status"], "complete")
        self.assertEqual(result["candidates"][0]["candidate_id"], "c3")
        self.assertEqual(result["generation"], 9)
        before = len(self.bridge.stream_calls)
        stale = await self.service.discover_game(GAME, [], None, False, 10, is_current=lambda: False)
        self.assertEqual(stale["generation"], 10)
        self.assertEqual(stale["candidates"][0]["candidate_id"], "c3")
        self.assertEqual(len(self.bridge.stream_calls), before)

    async def test_close_is_idempotent(self):
        await self.service.initialize()
        await self.service.close()
        await self.service.close()
        self.assertTrue(self.service._closed)


if __name__ == "__main__":
    unittest.main()
