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
    def plugin(self, entries=None, bridges=None):
        plugin = module.Plugin()
        plugin.sources = types.SimpleNamespace(
            list_game_sources=lambda _game: [],
            list_sources=lambda: [{"id": "official", "name": "Official", "status": "fresh"}])
        plugin.publisher = types.SimpleNamespace(matches=lambda _game: entries if entries is not None else [entry()],
                                                 bridges=lambda: bridges or [])
        plugin.exe_installation = None
        return plugin

    async def project(self, plugin, installed_game=None, transactions=None, discovery=None):
        row = {"game": installed_game or game(), "patches": [],
               "discovery": discovery or {"status": "complete", "candidates": [], "diagnostics": []}}
        with patch.object(module, "_released", return_value=True):
            await plugin._project_row(row, transactions or [])
        return row

    async def test_supported_zip_is_available_but_unverified_build_is_source_only(self):
        plugin = self.plugin()
        ready = await self.project(plugin)
        self.assertTrue(ready["patches"][0]["auto_installable"])
        blocked = await self.project(plugin, installed_game=game(buildid=None))
        self.assertFalse(blocked["patches"][0]["auto_installable"])
        self.assertIn("build_not_verified", blocked["patches"][0]["blockers"])

    async def test_hgamer_fallback_remains_discovery_only(self):
        plugin = self.plugin(entries=[])
        row = await self.project(plugin, discovery={"status": "complete", "candidates": [
            {"candidate_id": "hgamer-1", "source_id": "hgamer", "match_basis": "appid"}], "diagnostics": []})
        self.assertEqual(row["patches"], [])
        self.assertEqual(row["match_status"], "matched")
        self.assertEqual(row["discovery"]["candidates"][0]["source_id"], "hgamer")

    async def test_committed_owned_partial_files_allow_same_release_reinstall(self):
        observed = {"catalog_trust": "signed", "installs": [{"game": game(), "patches": [
            {"release_id": "zip-release", "detection": {"status": "partial", "files": []}, "blockers": []}]}]}
        local = types.SimpleNamespace(scan=AsyncMock(return_value=observed))
        plugin = self.plugin(bridges=[("local", "zip-release", local)])
        row = await self.project(plugin, transactions=[
            {"instance_id": "local", "release_id": "zip-release", "state": "committed"}])
        patch_result = row["patches"][0]
        self.assertEqual(patch_result["detection"]["status"], "partial")
        self.assertTrue(patch_result["auto_installable"])
        self.assertTrue(patch_result["reinstall_available"])

    async def test_other_release_and_unfinished_recovery_block_install(self):
        plugin = self.plugin()
        other = await self.project(plugin, transactions=[
            {"instance_id": "local", "release_id": "different", "state": "committed"}])
        self.assertFalse(other["patches"][0]["auto_installable"])
        self.assertIn("publisher_existing_transaction", other["patches"][0]["blockers"])
        interrupted = await self.project(plugin, transactions=[
            {"instance_id": "local", "release_id": "zip-release", "state": "needs_recovery"}])
        self.assertFalse(interrupted["patches"][0]["auto_installable"])
        self.assertIn("managed_install_exists_or_needs_recovery", interrupted["patches"][0]["blockers"])
