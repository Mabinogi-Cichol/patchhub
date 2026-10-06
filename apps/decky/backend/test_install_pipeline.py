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
    async def test_single_rpc_prepares_applies_and_deduplicates_retry(self):
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "patch.zip"
            archive.write_bytes(b"archive")
            plugin = module.Plugin()
            plugin.tasks = module.TaskManager(state_dir=temporary)
            plugin._prepare_install = AsyncMock(return_value={
                "archive": str(archive), "plan": {"plan_id": "plan-1"}})
            plugin._start_install = AsyncMock(return_value={"state": "committed", "transaction_id": "tx-1"})
            request_id = str(uuid.uuid4())

            first = await plugin.start_patch_install_task("local", "zip-release", "install", request_id)
            task = plugin.tasks.get(first["task_id"])
            await task.worker
            retry = await plugin.start_patch_install_task("local", "zip-release", "install", request_id)

            self.assertEqual(first, retry)
            self.assertEqual(task.snapshot()["state"], "completed")
            self.assertEqual(task.snapshot()["result"]["transaction_id"], "tx-1")
            plugin._prepare_install.assert_awaited_once_with("local", "zip-release", "", "install")
            plugin._start_install.assert_awaited_once_with("plan-1")

    async def test_startup_revokes_unbound_planned_job_even_if_discard_rpc_fails(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary)
            plan_id = str(uuid.uuid4())
            (state / "jobs" / plan_id).mkdir(parents=True)
            bridge = types.SimpleNamespace(state_dir=state,
                discard_plan=AsyncMock(side_effect=RuntimeError("rpc_failed")))
            plugin = module.Plugin()
            plugin.bridge = bridge
            plugin._list_transactions = AsyncMock(return_value=[
                {"state": "planned", "plan_id": plan_id},
                {"state": "committed", "plan_id": "committed-untouched"}])
            await plugin._reclaim_plans()
            self.assertTrue((state / "jobs" / plan_id / "revoked").is_file())
            bridge.discard_plan.assert_awaited_once_with(plan_id)

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

    async def test_startup_waits_for_exiting_core_lock_but_not_other_errors(self):
        from unittest.mock import patch
        plugin = module.Plugin()
        bridge = types.SimpleNamespace(migrate_state=AsyncMock(side_effect=[RuntimeError("installer_busy"), []]))
        with patch.object(module.asyncio, "sleep", new_callable=AsyncMock) as pause:
            self.assertEqual(await plugin._migrate_startup(bridge), [])
            pause.assert_awaited_once_with(.25)
        bridge.migrate_state = AsyncMock(side_effect=RuntimeError("journal_invalid_identity"))
        with self.assertRaisesRegex(RuntimeError, "journal_invalid_identity"):
            await plugin._migrate_startup(bridge)
