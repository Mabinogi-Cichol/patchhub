"""One RPC owns preparation and installation, including a lost reply retry."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import types
import unittest
import uuid
from unittest.mock import AsyncMock

sys.modules.setdefault("decky", types.ModuleType("decky"))
spec = importlib.util.spec_from_file_location("install_pipeline_plugin", Path(__file__).parents[1] / "main.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class InstallPipelineTests(unittest.IsolatedAsyncioTestCase):
    async def test_single_rpc_installs_selection_and_deduplicates_retry(self):
        with tempfile.TemporaryDirectory() as temporary:
            plugin = module.Plugin()
            plugin.tasks = module.TaskManager(state_dir=temporary)
            plugin.settings = types.SimpleNamespace(auto_cleanup=True)
            plugin.bridge = types.SimpleNamespace(selection=module.CoreBridge.selection,
                install_patch=AsyncMock(return_value={"state": "committed", "transaction_id": "tx-1"}),
                cleanup_cache=AsyncMock(return_value={"performed": True, "removed_bytes": 12}))
            request_id = str(uuid.uuid4())
            args = ("local", None, "zip-selection-value-1", None, "install", request_id)
            first = await plugin.start_patch_install_task(*args)
            task = plugin.tasks.get(first["task_id"])
            await task.worker
            retry = await plugin.start_patch_install_task(*args)
            self.assertEqual(first, retry)
            self.assertEqual(task.snapshot()["state"], "completed")
            self.assertEqual(task.snapshot()["result"]["transaction_id"], "tx-1")
            plugin.bridge.install_patch.assert_awaited_once_with("local", None, "zip-selection-value-1", None, "install", request_id)
            plugin.bridge.cleanup_cache.assert_awaited_once()
            with self.assertRaisesRegex(ValueError, "request_identity_changed"):
                await plugin.start_patch_install_task("local", None, "zip-selection-changed-1", None, "install", request_id)

    async def test_selection_archive_and_conflicting_ids_are_rejected_before_task(self):
        plugin = module.Plugin()
        plugin.tasks = module.TaskManager()
        plugin.bridge = types.SimpleNamespace(selection=module.CoreBridge.selection)
        with self.assertRaisesRegex(ValueError, "archive_not_allowed_for_selection"):
            await plugin.start_patch_install_task("local", None, "exe-selection-a-1", "a.zip")
        with self.assertRaisesRegex(ValueError, "invalid_request"):
            await plugin.start_patch_install_task("local", "release", "exe-selection-a-1")
        self.assertFalse(plugin.tasks.busy())

    async def test_public_rpc_waits_for_startup_migration_and_reports_failure(self):
        import asyncio
        plugin = module.Plugin()
        plugin._startup_ready = asyncio.Event()
        plugin._startup_error = None
        plugin.tasks = module.TaskManager()
        pending = asyncio.create_task(plugin.list_tasks())
        await asyncio.sleep(0)
        self.assertFalse(pending.done())
        plugin._startup_ready.set()
        self.assertEqual(await pending, [])
        plugin._startup_error = RuntimeError("installer_busy")
        with self.assertRaisesRegex(RuntimeError, "plugin_startup_failed"):
            await plugin.list_tasks()

    async def test_public_status_renews_foreground_but_task_list_does_not(self):
        import asyncio
        plugin = module.Plugin()
        plugin.tasks = module.TaskManager(lease_seconds=60)
        finish = asyncio.Event()
        async def work(task):
            await finish.wait()
            task.check()
            return {"state": "committed"}
        task = plugin.tasks.get(plugin.tasks.start("install", work)["task_id"])
        try:
            task.lease -= 5
            previous = task.lease
            await plugin.list_tasks()
            self.assertEqual(task.lease, previous)
            self.assertEqual((await plugin.task_status(task.value["task_id"]))["state"], "running")
            self.assertGreater(task.lease, previous)
        finally:
            task.request_cancel()
            finish.set()
            await task.worker
