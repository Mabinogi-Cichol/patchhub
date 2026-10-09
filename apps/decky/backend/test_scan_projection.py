"""Scan projection exposes safe installation capability for real rows."""
import importlib.util
from pathlib import Path
import sys
import types
import unittest
from unittest.mock import AsyncMock, patch

sys.modules.setdefault("decky", types.ModuleType("decky"))
spec = importlib.util.spec_from_file_location("scan_projection_plugin", Path(__file__).parents[1] / "main.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def game(buildid="123"):
    return {"instance_id": "local", "appid": 1, "name": "Game", "availability": "installed", "buildid": buildid}


def entry(release="zip-release"):
    return {"release_id": release, "source_id": "official", "source_page": "https://example.test/patch",
            "language": "zh-CN"}


class ProjectionTests(unittest.IsolatedAsyncioTestCase):
    async def test_projection_uses_rust_selection_and_receipt_without_python_policy(self):
        patch_result = {"release_id": "source-official-1", "selection_id": "zip-selection-id-1",
            "support_level": "publisher_zip", "detection": {"status": "partial"},
            "blockers": ["source_disabled"], "download_available": False}
        core_row = {"game": game(), "patches": [patch_result], "discovery": {"status": "cache_complete"}}
        plugin = module.Plugin()
        plugin.bridge = types.SimpleNamespace(scan=AsyncMock(return_value={"installs": [core_row]}))
        from unittest.mock import Mock
        plugin.sources = types.SimpleNamespace(prime=Mock())
        row = {"game": game(), "patches": [{"release_id": "stale"}]}
        await plugin._project_row(row)
        self.assertEqual(row, core_row)
        plugin.sources.prime.assert_called_once()

    async def test_read_report_preserves_shared_projection(self):
        report = {"installs": [{"game": game(), "patches": [{"selection_id": "exe-selection-current-1"}]}],
                  "sources": [], "settings_revision": 4}
        plugin = module.Plugin()
        plugin.bridge = types.SimpleNamespace(scan=AsyncMock(return_value=report))
        from unittest.mock import Mock
        plugin.sources = types.SimpleNamespace(prime=Mock(), settings_revision=4)
        plugin._scan_generation = 2
        result = await plugin._read_report()
        self.assertEqual(result["installs"], report["installs"])
        self.assertEqual(result["generation"], 2)


    async def test_source_refresh_batches_all_rows_into_one_core_scan(self):
        from unittest.mock import Mock
        rows = [{"game": {**game(), "instance_id": identity}, "patches": []} for identity in ("one", "two")]
        plugin = module.Plugin()
        plugin.bridge = types.SimpleNamespace(scan=AsyncMock(return_value={"installs": rows, "sources": []}))
        plugin.sources = types.SimpleNamespace(prime=Mock())
        report = {"installs": [{"game": dict(row["game"]), "patches": [{"release_id": "old"}]} for row in rows]}
        await plugin._refresh_rows(report)
        plugin.bridge.scan.assert_awaited_once()
        self.assertEqual(report["installs"], rows)
