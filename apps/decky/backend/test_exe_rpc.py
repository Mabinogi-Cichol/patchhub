"""EXE uses the same opaque selection and core outcomes as every install."""
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock
from test_source_tasks import module


class ExeRpcTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.selection = "exe-selection-" + "a" * 24 + "-42"
        self.plugin = module.Plugin()
        self.plugin.tasks = module.TaskManager()
        self.plugin.settings = SimpleNamespace(auto_cleanup=False)
        self.plugin.bridge = SimpleNamespace(selection=module.CoreBridge.selection,
            preview=AsyncMock(return_value={"executable": False, "selection_id": self.selection, "changes": []}),
            install_patch=AsyncMock(return_value={"disposition": "completed", "state": "committed", "selection_id": self.selection,
                "content_id": "exe-content-actual", "release_id": "source-official-42", "transaction_id": "core-id"}),
            list_transactions=AsyncMock(return_value=[{"transaction_id": "core-id", "state": "needs_recovery"}]),
            recover=AsyncMock(return_value=[{"transaction_id": "core-id", "state": "rolled_back"}]))

    async def test_exe_archive_import_rejected_before_core(self):
        with self.assertRaisesRegex(ValueError, "archive_not_allowed_for_selection"):
            await self.plugin.start_patch_install_task("instance", None, self.selection, "/tmp/arbitrary.exe", "overwrite")
        self.plugin.bridge.install_patch.assert_not_awaited()
        self.assertFalse(self.plugin.tasks.busy())

    async def test_preview_delegates_read_only_selection_to_core(self):
        result = await self.plugin.preview("instance", None, self.selection)
        self.assertFalse(result["executable"])
        self.plugin.bridge.preview.assert_awaited_once_with("instance", None, self.selection)

    async def test_terminal_identity_is_preserved_without_publisher_wrapper(self):
        start = await self.plugin.start_patch_install_task("instance", None, self.selection, None, "reinstall", "request")
        task = self.plugin.tasks.get(start["task_id"])
        await task.worker
        result = task.snapshot()["result"]
        self.assertEqual(result["transaction_id"], "core-id")
        self.assertEqual(result["selection_id"], self.selection)
        self.assertEqual(result["content_id"], "exe-content-actual")
        self.plugin.bridge.install_patch.assert_awaited_once_with("instance", None, self.selection, None, "reinstall", "request", "disabled")

    async def test_history_and_recovery_have_one_core_state_root(self):
        self.assertEqual((await self.plugin.list_transactions())[0]["state"], "needs_recovery")
        self.assertEqual((await self.plugin.recover())[0]["state"], "rolled_back")
        self.assertFalse(hasattr(self.plugin, "_dispatch_id"))
