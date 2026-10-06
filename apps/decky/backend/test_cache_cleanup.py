from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock
from test_discovery_rpc import module
from plugin_settings import Settings

class CacheCleanupTests(unittest.IsolatedAsyncioTestCase):
    def make_plugin(self, enabled=True, state="committed"):
        temp=tempfile.TemporaryDirectory();self.addCleanup(temp.cleanup)
        root=Path(temp.name);plugin=module.Plugin();plugin.settings=Settings(root)
        plugin.settings.set_auto_cleanup(enabled)
        plugin.bridge=SimpleNamespace(state_dir=root,start_install=AsyncMock(return_value={"state":state,"transaction_id":"p1"}),cleanup_transactions=AsyncMock(),list_transactions=AsyncMock(return_value=[]))
        plugin.publisher=SimpleNamespace(clear_cache=Mock(),bridges=lambda:[])
        plugin.tasks=module.TaskManager();plugin._prepare_cancel=None
        plugin._plan_metadata={"p1":{"cache":{"kind":"core","sha256":"a"*64}}}
        downloads=root/"downloads";downloads.mkdir();archive=downloads/("a"*64+".zip");archive.write_bytes(b"cached")
        return plugin,archive
    async def test_commit_cleans_download_and_requests_terminal_cleanup(self):
        plugin,archive=self.make_plugin();result=await plugin._start_install("p1")
        self.assertEqual(result["state"],"committed");self.assertFalse(archive.exists())
        plugin.bridge.cleanup_transactions.assert_awaited_once()
    async def test_download_setting_does_not_control_temporary_recovery_cleanup(self):
        plugin,archive=self.make_plugin(False);await plugin._start_install("p1")
        self.assertTrue(archive.exists());plugin.bridge.cleanup_transactions.assert_awaited_once()
    async def test_failure_keeps_download_and_recovery_material(self):
        plugin,archive=self.make_plugin(state="needs_recovery");result=await plugin._start_install("p1")
        self.assertEqual(result["state"],"needs_recovery");self.assertTrue(archive.exists())
        plugin.bridge.cleanup_transactions.assert_not_awaited()
    async def test_cleanup_failure_preserves_real_commit(self):
        plugin,_=self.make_plugin();plugin.bridge.cleanup_transactions.side_effect=OSError("disk_unavailable")
        self.assertEqual((await plugin._start_install("p1"))["state"],"committed")
