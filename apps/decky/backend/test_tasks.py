import asyncio
import json
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import Mock, patch

from tasks import Task, TaskManager, current_task
from bridge import CoreBridge
from protocol_test_support import outcome, event, response


class ControlledProcess:
    """A subprocess peer that delays its receipt until restoration finishes."""
    def __init__(self, restoring=False):
        self.returncode = None
        self.stdout = asyncio.StreamReader()
        self.finished = asyncio.Event()
        self.request = None
        self.messages = []
        self.restoring = restoring
        self.cancel_received = asyncio.Event()
        self.stdin = Mock(write=self.write, drain=self.drain, close=lambda: None)

    async def drain(self):
        await asyncio.sleep(0)

    def write(self, payload):
        self.messages.append(payload)
        if self.request is None:
            self.request = json.loads(payload)
            self.emit(event("install_progress", self.request["request_id"],
                instance_id="instance", selection_id=None, sequence=0, stage="applying", substage=None,
                bytes_done=12, total_bytes=24, files_done=0, total_files=1, archive_bytes=48, current_file=None))
            if not self.restoring:
                self.finish("committed")
        else:
            self.cancel_received.set()
            self.emit(event("install_progress", self.request["request_id"],
                instance_id="instance", selection_id=None, sequence=1, stage="rolling_back", substage=None,
                bytes_done=0, total_bytes=None, files_done=0, total_files=None, current_file=None, archive_bytes=48))


    def emit(self, value):
        self.stdout.feed_data(json.dumps(value).encode() + b"\n")

    def finish(self, state):
        self.emit(response("install_patch", self.request["request_id"],
            outcome(state, "task_cancelled" if state == "rolled_back" else None)))
        self.returncode = 0
        self.stdout.feed_eof()
        self.finished.set()

    async def wait(self):
        await self.finished.wait()
        return self.returncode


class ControlledBridgeTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        path = Path(self.temp.name) / "binary"
        path.touch()
        self.bridge = CoreBridge(path, path)
        self.bridge._preflight_done = True
        self.task = Task("install", {})
        self.task.update(archive_bytes=48)
        token = current_task.set(self.task)
        self.addCleanup(current_task.reset, token)

    async def test_progress_preserves_known_archive_size_and_uses_controlled_process(self):
        process = ControlledProcess()
        with patch("bridge.asyncio.create_subprocess_exec", return_value=process) as spawn:
            result = await self.bridge.install_patch("instance", "release", operation="overwrite")
        self.assertEqual(spawn.call_args.args[1], "serve-task")
        self.assertEqual(result["state"], "committed")
        self.assertEqual(self.task.snapshot()["bytes_done"], 12)
        self.assertEqual(self.task.snapshot()["archive_bytes"], 48)

    async def test_cancelled_before_spawn_writes_request_before_control_and_drains_rollback(self):
        process = ControlledProcess(restoring=True)
        self.task.request_cancel()
        with patch("bridge.asyncio.create_subprocess_exec", return_value=process):
            call = asyncio.create_task(self.bridge.install_patch("instance", "release", operation="overwrite"))
            await asyncio.wait_for(process.cancel_received.wait(), 1)
            self.assertEqual(json.loads(process.messages[0])["method"], "install_patch")
            self.assertEqual(process.messages[1], b"cancel\n")
            await asyncio.sleep(.01)
            self.assertFalse(call.done())
            self.assertEqual(self.task.snapshot()["state"], "rolling_back")
            process.finish("rolled_back")
            self.assertEqual((await call)["state"], "rolled_back")

    async def test_cancelling_rpc_waits_for_durable_rollback_receipt(self):
        process = ControlledProcess(restoring=True)
        with patch("bridge.asyncio.create_subprocess_exec", return_value=process):
            call = asyncio.create_task(self.bridge.install_patch("instance", "release", operation="overwrite"))
            while process.request is None:
                await asyncio.sleep(0)
            call.cancel()
            await asyncio.wait_for(process.cancel_received.wait(), 1)
            self.assertFalse(call.done())
            process.finish("rolled_back")
            self.assertEqual((await call)["state"], "rolled_back")


class TaskTests(unittest.IsolatedAsyncioTestCase):
    async def test_ui_lease_expiry_cancels_backend_work(self):
        manager = TaskManager(lease_seconds=.01)
        finish = asyncio.Event()
        async def work(task):
            await finish.wait()
            task.check()
            return {"disposition": "completed", "state": "committed"}
        task = manager.get(manager.start("install", work)["task_id"])
        await asyncio.sleep(.04)
        self.assertTrue(task.cancel.is_set())
        self.assertTrue(manager.busy())
        finish.set()
        await task.worker
        self.assertEqual(task.snapshot()["state"], "cancelled")

    async def test_renewal_keeps_work_alive_and_cancellation_waits_for_rollback(self):
        manager = TaskManager(lease_seconds=.2)
        restoring = asyncio.Event()
        finish = asyncio.Event()
        async def work(task):
            while not task.cancel.is_set():
                await asyncio.sleep(.005)
            task.stage("rolling_back")
            restoring.set()
            await finish.wait()
            return {"disposition": "cancelled", "state": "rolled_back", "error": "task_cancelled"}
        task = manager.get(manager.start("install", work)["task_id"])
        for _ in range(8):
            await asyncio.sleep(.01)
            self.assertEqual(manager.status(task.value["task_id"])["state"], "running")
        task.request_cancel()
        await restoring.wait()
        self.assertEqual(task.snapshot()["state"], "rolling_back")
        with self.assertRaisesRegex(RuntimeError, "installer_busy"):
            manager.start("prepare", work)
        task.request_cancel()
        self.assertEqual(task.snapshot()["state"], "rolling_back")
        finish.set()
        await task.worker
        self.assertEqual(task.snapshot()["state"], "cancelled")

    async def test_expired_status_cannot_renew_or_revive_work(self):
        manager = TaskManager(lease_seconds=1)
        finish = asyncio.Event()
        async def work(task):
            await finish.wait()
            task.check()
            return {"disposition": "completed", "state": "committed"}
        task = manager.get(manager.start("install", work)["task_id"])
        task.lease -= 2
        expired = task.lease
        self.assertEqual(manager.status(task.value["task_id"])["state"], "cancelling")
        self.assertEqual(task.lease, expired)
        finish.set()
        await task.worker
        self.assertEqual(task.snapshot()["state"], "cancelled")

    async def test_listing_or_reading_receipts_does_not_keep_work_alive(self):
        manager = TaskManager(lease_seconds=.01)
        finish = asyncio.Event()
        async def work(task):
            await finish.wait()
            task.check()
            return {"disposition": "completed", "state": "committed"}
        task = manager.get(manager.start("install", work)["task_id"])
        for _ in range(4):
            await asyncio.sleep(.01)
            [t.snapshot() for t in manager.tasks.values()]
        self.assertTrue(task.cancel.is_set())
        finish.set()
        await task.worker
        self.assertEqual(task.snapshot()["state"], "cancelled")

    async def test_lease_expiry_waits_for_safe_rollback(self):
        manager = TaskManager(lease_seconds=.01)
        restoring, finish = asyncio.Event(), asyncio.Event()
        async def work(task):
            while not task.cancel.is_set():
                await asyncio.sleep(.001)
            task.stage("rolling_back")
            restoring.set()
            await finish.wait()
            return {"disposition": "cancelled", "state": "rolled_back", "error": "task_cancelled"}
        task = manager.get(manager.start("install", work)["task_id"])
        await asyncio.wait_for(restoring.wait(), 1)
        self.assertEqual(task.snapshot()["state"], "rolling_back")
        self.assertTrue(manager.busy())
        self.assertFalse(task.worker.done())
        finish.set()
        await task.worker
        self.assertEqual(task.snapshot()["state"], "cancelled")

    async def test_commit_wins_cancel_race_but_recovery_failure_is_not_cancelled(self):
        for receipt, expected in (({"disposition": "completed", "state": "committed"}, "completed"),
                                  ({"disposition": "failed", "state": "needs_recovery", "error": "task_cancelled; recovery_conflict"}, "failed")):
            manager = TaskManager()
            async def work(task):
                task.request_cancel()
                return receipt
            task = manager.get(manager.start("install", work)["task_id"])
            await task.worker
            self.assertEqual(task.snapshot()["state"], expected)
            self.assertEqual(task.snapshot()["result"], receipt)

    def test_speed_stalls_and_unknown_total_never_produces_eta(self):
        task = Task("prepare", {})
        task.stage("downloading")
        now = time.monotonic()
        task.samples.clear()
        task.samples.extend([(now - 2, 0), (now, 100)])
        task.value["bytes_done"] = 100
        self.assertGreater(task.snapshot()["speed_bps"], 0)
        self.assertIsNone(task.snapshot()["eta_seconds"])
        with patch("tasks.time.monotonic", return_value=now + 3):
            self.assertEqual(task.snapshot()["speed_bps"], 0)
        task.stage("connecting")
        self.assertEqual(task.snapshot()["speed_bps"], 0)


if __name__ == "__main__":
    unittest.main()
