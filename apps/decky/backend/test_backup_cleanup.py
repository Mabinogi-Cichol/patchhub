"""Backup RPC remains a thin bridge; settings and game files are not host-deleted."""
import unittest
from types import SimpleNamespace
from unittest.mock import AsyncMock, Mock
from test_discovery_rpc import module
from protocol_validation import validate_request, validate_response

class BackupCleanupTests(unittest.IsolatedAsyncioTestCase):
    def plugin(self):
        plugin = module.Plugin()
        plugin.tasks = SimpleNamespace(busy=Mock(return_value=False))
        plugin.bridge = SimpleNamespace(backup_status=AsyncMock(return_value={"total_bytes": 12, "files": 1, "failures": []}),
            cleanup_backups=AsyncMock(return_value={"status": "partial", "removed_bytes": 12, "removed_files": 1,
                "failures": [{"code": "cleanup_failed", "detail": "access denied"}]}))
        return plugin

    async def test_passthrough_preserves_inventory_and_partial_cleanup(self):
        plugin = self.plugin()
        self.assertEqual((await plugin.backup_status())["files"], 1)
        result = await plugin.cleanup_backups()
        self.assertEqual(result["status"], "partial")
        self.assertEqual(result["failures"][0]["detail"], "access denied")
        plugin.bridge.backup_status.assert_awaited_once_with()
        plugin.bridge.cleanup_backups.assert_awaited_once_with()

    async def test_busy_task_refuses_cleanup_but_allows_readonly_inventory(self):
        plugin = self.plugin()
        plugin.tasks.busy.return_value = True
        with self.assertRaisesRegex(RuntimeError, "installer_busy"):
            await plugin.cleanup_backups()
        plugin.bridge.cleanup_backups.assert_not_awaited()
        self.assertEqual((await plugin.backup_status())["total_bytes"], 12)

    async def test_core_bridge_uses_fixed_methods_without_ui_paths(self):
        bridge = object.__new__(module.CoreBridge)
        bridge._call = AsyncMock(return_value={})
        await bridge.backup_status()
        await bridge.cleanup_backups()
        self.assertEqual(bridge._call.await_args_list, [unittest.mock.call("backup_status", {}), unittest.mock.call("cleanup_backups", {})])
