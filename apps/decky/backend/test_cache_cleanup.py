"""Shared-core cache cleanup preserves durable install outcomes."""
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock
from test_discovery_rpc import module


class CacheCleanupTests(unittest.IsolatedAsyncioTestCase):
    def make_plugin(self, enabled=True, state="committed"):
        plugin = module.Plugin()
        plugin.settings = SimpleNamespace(auto_cleanup=enabled)
        plugin.tasks = module.TaskManager()
        plugin.bridge = SimpleNamespace(selection=module.CoreBridge.selection,
            install_patch=AsyncMock(return_value={"state": state, "transaction_id": "t", "cleanup": {"performed": state == "committed"}}),
            cleanup_cache=AsyncMock(return_value={"performed": True, "removed_bytes": 12}))
        return plugin

    async def install(self, plugin):
        started = await plugin.start_patch_install_task("instance", "signed")
        task = plugin.tasks.get(started["task_id"])
        await task.worker
        return task.snapshot()

    async def test_commit_requests_shared_cache_cleanup(self):
        plugin = self.make_plugin()
        result = await self.install(plugin)
        self.assertEqual(result["state"], "completed")
        plugin.bridge.cleanup_cache.assert_awaited_once()
        self.assertEqual(result["result"]["cleanup"]["removed_bytes"], 12)

    async def test_disabled_download_cleanup_preserves_core_transaction_cleanup(self):
        plugin = self.make_plugin(False)
        result = await self.install(plugin)
        self.assertTrue(result["result"]["cleanup"]["performed"])
        plugin.bridge.cleanup_cache.assert_not_awaited()

    async def test_recovery_failure_never_requests_cache_cleanup(self):
        plugin = self.make_plugin(state="needs_recovery")
        result = await self.install(plugin)
        self.assertEqual(result["state"], "failed")
        self.assertEqual(result["result"]["state"], "needs_recovery")
        plugin.bridge.cleanup_cache.assert_not_awaited()

    async def test_cleanup_failure_preserves_real_commit_and_diagnostic(self):
        plugin = self.make_plugin()
        plugin.bridge.cleanup_cache.side_effect = OSError("disk_unavailable")
        result = await self.install(plugin)
        self.assertEqual(result["state"], "completed")
        self.assertEqual(result["result"]["cleanup"]["code"], "disk_unavailable")

    async def test_manual_cleanup_uses_shared_rpc_and_no_fabricated_inventory(self):
        plugin = self.make_plugin()
        plugin.settings.snapshot = lambda: {"schema_version": 4, "auto_cleanup": True}
        self.assertIsNone((await plugin.get_settings())["cache"])
        result = await plugin.clear_cache()
        self.assertEqual(result, {"performed": True, "removed_bytes": 12})
