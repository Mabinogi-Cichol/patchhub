"""Durable request identities for lost replies and backend restarts."""
import asyncio
import tempfile
import unittest
import uuid

from tasks import TaskManager


class TaskHistoryTests(unittest.IsolatedAsyncioTestCase):
    async def test_retry_after_restart_reuses_completed_task(self):
        with tempfile.TemporaryDirectory() as state_dir:
            request_id = str(uuid.uuid4())
            calls = 0

            async def install(_task):
                nonlocal calls
                calls += 1
                return {"state": "committed"}

            details = dict(instance_id="game", release_id="release", operation="install",
                           archive_path="", request_id=request_id)
            manager = TaskManager(state_dir=state_dir)
            first = manager.start("install", install, **details)
            await manager.get(first["task_id"]).worker
            self.assertEqual(manager.get(first["task_id"]).snapshot()["state"], "completed")

            restarted = TaskManager(state_dir=state_dir)
            retry = restarted.start("install", install, **details)
            self.assertEqual(retry, first)
            self.assertEqual(calls, 1)
            self.assertEqual(restarted.get(first["task_id"]).snapshot()["state"], "completed")

    async def test_request_identity_rejects_changed_archive(self):
        with tempfile.TemporaryDirectory() as state_dir:
            request_id = str(uuid.uuid4())

            async def install(_task):
                return {"state": "committed"}

            details = dict(instance_id="game", release_id="release", operation="install",
                           archive_path="first.zip", request_id=request_id)
            manager = TaskManager(state_dir=state_dir)
            first = manager.start("install", install, **details)
            await manager.get(first["task_id"]).worker
            restarted = TaskManager(state_dir=state_dir)
            with self.assertRaisesRegex(ValueError, "request_identity_changed"):
                restarted.start("install", install, **{**details, "archive_path": "other.zip"})
