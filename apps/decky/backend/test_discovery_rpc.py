"""RPC boundaries: local identity, stale fences and discovery/install isolation."""
import asyncio
import importlib.util
from pathlib import Path
import sys
import types
import unittest
from unittest.mock import AsyncMock

sys.modules.setdefault("decky", types.ModuleType("decky"))
spec = importlib.util.spec_from_file_location("discovery_rpc_plugin", Path(__file__).parents[1] / "main.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class DiscoveryRpcTests(unittest.IsolatedAsyncioTestCase):
    def plugin(self):
        plugin = module.Plugin()
        plugin._scan_generation = 7
        plugin._scan_games = {"local": {"instance_id": "local", "appid": 123, "name": "Actual Game"}}
        plugin.sources = types.SimpleNamespace(settings_revision=2, discover_game=AsyncMock(
            return_value={"appid": 123, "candidates": [], "diagnostics": [], "status": "empty"}))
        plugin.bridge = types.SimpleNamespace(preview=AsyncMock(), plan_install=AsyncMock(), fetch_artifact=AsyncMock())
        plugin.tasks = module.TaskManager()
        plugin._project_row = AsyncMock()
        return plugin

    async def test_uses_local_identity_and_does_not_occupy_install_lock(self):
        plugin = self.plugin()
        result = await plugin.discover_game("local", ["Display Name"], "zh-CN")
        args = plugin.sources.discover_game.call_args
        self.assertEqual(args.args[0]["appid"], 123)
        self.assertEqual(result["instance_id"], "local")
        self.assertEqual(result["generation"], 7)
        self.assertEqual(result["settings_revision"], 2)
        self.assertTrue(args.kwargs["is_current"]())
        self.assertFalse(plugin.tasks.busy())

    async def test_rejects_untrusted_instance_and_invalid_query(self):
        plugin = self.plugin()
        for instance, hints, language, force in (("foreign", [], None, False),
                ("local", ["a"] * 4, None, False), ("local", ["x" * 257], None, False),
                ("local", [], "../../path", False), ("local", [], None, 1)):
            with self.subTest(instance=instance, hints=hints):
                with self.assertRaises(ValueError):
                    await plugin.discover_game(instance, hints, language, force)
        plugin.sources.discover_game.assert_not_called()

    async def test_scan_and_settings_changes_invalidate_result_and_commit_callback(self):
        for field in ("generation", "revision"):
            plugin = self.plugin()
            async def query(*args, **kwargs):
                if field == "generation":
                    plugin._scan_generation += 1
                else:
                    plugin.sources.settings_revision += 1
                self.assertFalse(kwargs["is_current"]())
                return {"status": "complete", "candidates": [{"candidate_id": "candidate-123"}]}
            plugin.sources.discover_game.side_effect = query
            result = await plugin.discover_game("local")
            self.assertEqual(result["status"], "stale")
            self.assertEqual(result["candidates"], [])

    async def test_obsolete_prepare_rpcs_are_absent(self):
        plugin = self.plugin()
        for name in ("_plan_install", "fetch_artifact", "_prepare_install", "_start_install", "exe_installation_reviews"):
            self.assertFalse(hasattr(plugin, name))
