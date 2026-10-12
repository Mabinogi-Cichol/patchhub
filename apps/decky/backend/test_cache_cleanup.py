"""Shared-core cache cleanup preserves durable install outcomes."""
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock
from test_discovery_rpc import module


class CacheCleanupTests(unittest.IsolatedAsyncioTestCase):
    def make_plugin(self, enabled=True, state="committed", cleanup=None):
        plugin = module.Plugin()
        plugin.settings = SimpleNamespace(auto_cleanup=enabled)
        plugin.tasks = module.TaskManager()
        outcome = {"state": state, "transaction_id": "t", "cleanup": cleanup or {
            "status": "not_requested", "performed": False, "removed_bytes": 0, "failures": []}}
        plugin.bridge = SimpleNamespace(selection=module.CoreBridge.selection,
            install_patch=AsyncMock(return_value=outcome),
            cleanup_cache=AsyncMock(return_value={"status": "completed", "performed": True, "removed_bytes": 12, "failures": []}))
        return plugin

    async def install(self, plugin):
        started = await plugin.start_patch_install_task("instance", "signed")
        task = plugin.tasks.get(started["task_id"])
        await task.worker
        return task.snapshot()

    async def test_commit_passes_enabled_policy_and_preserves_core_cleanup_result(self):
        cleanup = {"status": "completed", "performed": True, "removed_bytes": 12, "failures": []}
        plugin = self.make_plugin(cleanup=cleanup)
        result = await self.install(plugin)
        self.assertEqual(result["state"], "completed")
        plugin.bridge.install_patch.assert_awaited_once_with("instance", "signed", None, None, "install", unittest.mock.ANY, "after_commit")
        self.assertEqual(result["result"]["cleanup"], cleanup)
        self.assertEqual(plugin._cache_result, cleanup)
        plugin.bridge.cleanup_cache.assert_not_awaited()

    async def test_disabled_policy_is_passed_and_core_cleanup_is_unchanged(self):
        cleanup = {"status": "not_requested", "performed": False, "removed_bytes": 0, "failures": []}
        plugin = self.make_plugin(False, cleanup=cleanup)
        result = await self.install(plugin)
        plugin.bridge.install_patch.assert_awaited_once_with("instance", "signed", None, None, "install", unittest.mock.ANY, "disabled")
        self.assertEqual(result["result"]["cleanup"], cleanup)
        plugin.bridge.cleanup_cache.assert_not_awaited()

    async def test_rolled_back_and_recovery_failure_keep_core_outcome_without_second_cleanup(self):
        for state, task_state in (("rolled_back", "failed"), ("needs_recovery", "failed")):
            with self.subTest(state=state):
                plugin = self.make_plugin(state=state)
                result = await self.install(plugin)
                self.assertEqual(result["state"], task_state)
                self.assertEqual(result["result"]["state"], state)
                plugin.bridge.cleanup_cache.assert_not_awaited()

    async def test_cleanup_failures_keep_real_commit_and_structured_cleanup(self):
        for cleanup in (
            {"status": "partial", "performed": True, "removed_bytes": 12,
             "failures": [{"code": "disk_unavailable", "detail": "disk full"}]},
            {"status": "failed", "performed": False, "removed_bytes": 0,
             "failures": [{"code": "permission_denied", "detail": "access denied"}]},
        ):
            with self.subTest(status=cleanup["status"]):
                plugin = self.make_plugin(cleanup=cleanup)
                result = await self.install(plugin)
                self.assertEqual(result["state"], "completed")
                self.assertEqual(result["result"]["state"], "committed")
                self.assertEqual(result["result"]["cleanup"], cleanup)
                plugin.bridge.cleanup_cache.assert_not_awaited()

    async def test_manual_cleanup_uses_shared_rpc_and_no_fabricated_inventory(self):
        plugin = self.make_plugin()
        plugin.settings.snapshot = lambda: {"schema_version": 4, "auto_cleanup": True}
        self.assertIsNone((await plugin.get_settings())["cache"])
        result = await plugin.clear_cache()
        self.assertEqual(result["removed_bytes"], 12)
        plugin.bridge.cleanup_cache.assert_awaited_once()
