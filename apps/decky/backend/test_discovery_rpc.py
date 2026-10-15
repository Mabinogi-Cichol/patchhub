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

    async def test_instance_identity_uses_generated_utf8_limit_and_rejects_controls(self):
        plugin = self.plugin()
        limit = module.MAX_INSTANCE_ID_BYTES
        instance = "界" * (limit // 3) + "a" * (limit % 3)
        plugin._scan_games[instance] = {"instance_id": instance, "appid": 123, "name": "Actual Game"}
        result = await plugin.discover_game(instance)
        self.assertEqual(result["instance_id"], instance)
        self.assertEqual(plugin.sources.discover_game.await_count, 1)
        for invalid in (instance + "a", "local\u0085", "local\x7f"):
            with self.assertRaisesRegex(ValueError, "invalid_id"):
                await plugin.discover_game(invalid)
        self.assertEqual(plugin.sources.discover_game.await_count, 1)

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


class ProgressiveQueryTests(DiscoveryRpcTests):
    async def test_query_returns_immediately_streams_and_cancels_without_losing_results(self):
        plugin = self.plugin()
        game = {**plugin._scan_games["local"], "availability": "installed"}
        report = {"installs": [{"game": game, "patches": []}], "settings_revision": 2, "generation": 8}
        plugin._read_report = AsyncMock(return_value=report)
        entered = asyncio.Event()
        async def query(**kwargs):
            await kwargs["on_event"]({"event": "source_update", "instance_id": "local",
                "discovery": {"status": "partial", "candidates": [{"candidate_id": "found"}], "diagnostics": []}})
            entered.set()
            await asyncio.Event().wait()
        plugin.sources.scan_sources = AsyncMock(side_effect=query)
        first = await plugin.start_game_discovery("local", ["hint"], "zh-CN", True)
        self.assertTrue(first["scan_progress"]["running"])
        await entered.wait()
        second = await plugin.start_game_discovery("local", ["hint"], "zh-CN", True)
        self.assertEqual(plugin.sources.scan_sources.await_count, 1)
        self.assertEqual(second["installs"][0]["discovery"]["candidates"][0]["candidate_id"], "found")
        self.assertTrue(await plugin.cancel_source_query())
        final = await plugin.scan_status(8)
        self.assertFalse(final["scan_progress"]["running"])
        self.assertEqual(final["installs"][0]["discovery"]["candidates"][0]["candidate_id"], "found")
        self.assertEqual(plugin.sources.scan_sources.call_args.kwargs["instance_id"], "local")

    async def test_task_entry_rejects_unknown_game_before_starting_network(self):
        plugin = self.plugin()
        with self.assertRaises(ValueError):
            await plugin.start_game_discovery("foreign")
