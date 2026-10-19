"""Scan and projection RPC boundaries: one core scan, projection without a scan."""
import importlib.util
from pathlib import Path
import sys
import types
import unittest
from unittest.mock import AsyncMock, Mock

sys.modules.setdefault("decky", types.ModuleType("decky"))
spec = importlib.util.spec_from_file_location("scan_projection_plugin", Path(__file__).parents[1] / "main.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def game(buildid="123"):
    return {"instance_id": "local", "appid": 1, "name": "Game", "availability": "installed", "buildid": buildid}


class ProjectionTests(unittest.IsolatedAsyncioTestCase):
    async def test_scan_reads_the_core_once_and_primes_the_source_snapshot(self):
        core_row = {"game": game(), "patches": [{"release_id": "zip-release"}],
                    "discovery": {"status": "publisher_match", "candidates": [], "diagnostics": []}}
        plugin = module.Plugin()
        plugin._scan_generation = 6
        plugin.bridge = types.SimpleNamespace(scan=AsyncMock(return_value={
            "installs": [core_row], "sources": [], "settings_revision": 9,
            "diagnostics": []}))
        plugin.sources = types.SimpleNamespace(prime=Mock(), settings_revision=9)
        result = await plugin.scan()
        plugin.bridge.scan.assert_awaited_once_with()
        plugin.sources.prime.assert_called_once()
        self.assertEqual(result["installs"], [core_row])
        self.assertEqual(result["generation"], 7)
        self.assertEqual(result["settings_revision"], 9)

    async def test_project_instances_never_triggers_a_scan(self):
        plugin = module.Plugin()
        plugin.bridge = types.SimpleNamespace(
            scan=AsyncMock(), call_sync=AsyncMock(return_value={
                "settings_revision": 9,
                "instances": [{"game": {"instance_id": "one", "appid": 1}, "patches": []},
                              {"game": {"instance_id": "two", "appid": 2}, "patches": []}]}))
        plugin.sources = types.SimpleNamespace(settings_revision=9)
        result = await plugin.project_instances(["one", "two"])
        plugin.bridge.scan.assert_not_awaited()
        plugin.bridge.call_sync.assert_awaited_once_with(
            "project_instances", {"instance_ids": ["one", "two"]})
        self.assertEqual([row["game"]["instance_id"] for row in result["instances"]], ["one", "two"])


if __name__ == "__main__":
    unittest.main()
