"""Protocol-v3 lease, transport, identity and terminal-outcome regression tests."""
import asyncio
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import AsyncMock, patch
from bridge import CoreBridge, CoreError, METHODS
from tasks import Task, TaskManager, current_task
from test_bridge import _FakeProcess


class Peer:
    def __init__(self):
        self.returncode = None
        self.stdout = asyncio.StreamReader()
        self.stdin = self
        self.request = None
        self.lines = []
        self.cancelled = asyncio.Event()
        self.renewed = asyncio.Event()
        self.finished = asyncio.Event()

    def write(self, data):
        self.lines.append(data)
        if self.request is None:
            self.request = json.loads(data)
        elif data == b"renew\n":
            self.renewed.set()
        elif data == b"cancel\n":
            self.cancelled.set()

    async def drain(self):
        await asyncio.sleep(0)

    def close(self):
        pass

    async def wait(self):
        await self.finished.wait()

    def finish(self, result):
        row = {"protocol_version": 3, "request_id": self.request["request_id"], "ok": True, "result": result}
        self.stdout.feed_data(json.dumps(row).encode() + b"\n")
        self.stdout.feed_eof()
        self.returncode = 0
        self.finished.set()


class V3BridgeTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        binary = Path(self.temp.name) / "steam-patch"
        binary.touch()
        self.bridge = CoreBridge(binary, binary)

    async def test_renew_control_continues_until_cancel_and_receipt_drains(self):
        peer = Peer()
        task = Task("install", {})
        token = current_task.set(task)
        try:
            with patch("bridge.asyncio.create_subprocess_exec", return_value=peer), patch("bridge.RENEW_SECONDS", .01):
                pending = asyncio.create_task(self.bridge.install_patch("local", selection_id="exe-selection-a-1", request_id="same"))
                await asyncio.wait_for(peer.renewed.wait(), 1)
                self.assertEqual(peer.request["request_id"], "same")
                self.assertEqual(peer.request["params"]["request_id"], "same")
                self.assertEqual(peer.request["params"]["cleanup_policy"], "disabled")
                self.assertEqual(peer.request["method"], "install_patch")
                task.request_cancel()
                await asyncio.wait_for(peer.cancelled.wait(), 1)
                self.assertFalse(pending.done())
                count = len(peer.lines)
                await asyncio.sleep(.15)
                self.assertEqual(len(peer.lines), count)
                peer.finish({"state": "rolled_back", "error": "task_cancelled"})
                self.assertEqual((await pending)["state"], "rolled_back")
        finally:
            current_task.reset(token)

    async def test_host_cleanup_policy_is_sent_and_invalid_values_rejected(self):
        with patch("bridge.asyncio.create_subprocess_exec", new_callable=AsyncMock) as spawn:
            with self.assertRaisesRegex(ValueError, "invalid_cleanup_policy"):
                await self.bridge.install_patch("local", release_id="signed", cleanup_policy="enabled")
            spawn.assert_not_called()
        peer = Peer()
        with patch("bridge.asyncio.create_subprocess_exec", return_value=peer):
            pending = asyncio.create_task(self.bridge.install_patch(
                "local", release_id="signed", cleanup_policy="after_commit"))
            while peer.request is None:
                await asyncio.sleep(0)
            self.assertEqual(peer.request["params"]["cleanup_policy"], "after_commit")
            peer.finish({"state": "committed"})
            await pending

    async def test_ui_lease_expiry_controls_renewal_not_background_bridge(self):
        manager = TaskManager(lease_seconds=.03)
        peer = Peer()
        async def work(task):
            return await self.bridge.install_patch("local", release_id="signed")
        with patch("bridge.asyncio.create_subprocess_exec", return_value=peer), patch("bridge.RENEW_SECONDS", .01):
            task = manager.get(manager.start("install", work)["task_id"])
            await asyncio.wait_for(peer.cancelled.wait(), 1)
            self.assertTrue(task.cancel.is_set())
            self.assertFalse(task.worker.done())
            peer.finish({"state": "needs_recovery", "error": "restore_conflict", "recovery": {"required": True}})
            await task.worker
        self.assertEqual(task.snapshot()["state"], "failed")
        self.assertTrue(task.snapshot()["result"]["recovery"]["required"])

    async def test_duplicate_terminal_rejected_and_late_progress_ignored(self):
        for extra in ({"event": "install_progress"}, {"ok": True, "result": {}}):
            def factory(request_id, request):
                response = {"protocol_version": 3, "request_id": request_id, "ok": True, "result": {"state": "committed"}}
                return [response], extra
            with patch("bridge.asyncio.create_subprocess_exec", return_value=_FakeProcess(factory)):
                if "event" in extra:
                    self.assertEqual((await self.bridge.install_patch("local", "signed"))["state"], "committed")
                else:
                    with self.assertRaisesRegex(RuntimeError, "invalid_core_response"):
                        await self.bridge.install_patch("local", "signed")

    async def test_removed_methods_and_host_path_injection_are_rejected_before_spawn(self):
        with patch("bridge.asyncio.create_subprocess_exec", new_callable=AsyncMock) as spawn:
            for method in ("migrate_state", "plan_install", "start_install", "get_source_state", "fetch_artifact"):
                self.assertNotIn(method, METHODS)
                with self.assertRaisesRegex(ValueError, "unsupported_method"):
                    await self.bridge._call(method, {})
            with self.assertRaisesRegex(ValueError, "host_parameter_forbidden"):
                await self.bridge._call("scan", {"state_dir": "/tmp/attacker"})
            spawn.assert_not_called()

    async def test_core_error_detail_survives_task_failure(self):
        def factory(request_id, request):
            return [], {"protocol_version": 3, "request_id": request_id, "ok": False,
                        "error": {"code": "analysis_failed", "detail": "bounded diagnostic"}}
        manager = TaskManager()
        with patch("bridge.asyncio.create_subprocess_exec", return_value=_FakeProcess(factory)):
            async def work(task):
                return await self.bridge.install_patch("local", "signed")
            task = manager.get(manager.start("install", work)["task_id"])
            await task.worker
        self.assertEqual(task.snapshot()["error"], "analysis_failed")
        self.assertEqual(task.snapshot()["error_detail"], "bounded diagnostic")

    async def test_commit_wins_late_cancel_and_terminal_update_is_ignored(self):
        manager = TaskManager()
        async def work(task):
            task.request_cancel()
            return {"state": "committed"}
        task = manager.get(manager.start("install", work)["task_id"])
        await task.worker
        task.update(state="failed", stage="applying")
        self.assertEqual(task.snapshot()["state"], "completed")

    def test_root_requires_an_explicit_unprivileged_identity(self):
        with patch("bridge.os.geteuid", return_value=0, create=True):
            with self.assertRaisesRegex(ValueError, "unprivileged_user_required"):
                CoreBridge(self.bridge.binary, self.bridge.catalog)
            with self.assertRaisesRegex(ValueError, "unprivileged_user_required"):
                CoreBridge(self.bridge.binary, self.bridge.catalog, run_uid=0, run_gid=0)
            bridge = CoreBridge(self.bridge.binary, self.bridge.catalog, run_uid=1000, run_gid=1000)
            self.assertEqual(bridge.process_identity, {"user": 1000, "group": 1000, "extra_groups": []})
            self.assertEqual(bridge.worker, bridge.binary.with_name("sf-worker"))

    async def test_oversized_scan_is_explicit_and_a_later_call_still_succeeds(self):
        from bridge import MAX_RESPONSE_BYTES
        class Process(_FakeProcess):
            def kill(self):
                self.returncode = 0
        def oversized(request_id, request):
            return [], {"protocol_version": 3, "request_id": request_id, "ok": True,
                        "result": {"installs": [], "detail": "x" * MAX_RESPONSE_BYTES}}
        with patch("bridge.asyncio.create_subprocess_exec", return_value=Process(oversized)):
            with self.assertRaisesRegex(RuntimeError, "response_too_large"):
                await self.bridge.scan()
        def normal(request_id, request):
            return [], {"protocol_version": 3, "request_id": request_id, "ok": True,
                        "result": {"installs": [], "catalog_trust": "unavailable"}}
        with patch("bridge.asyncio.create_subprocess_exec", return_value=Process(normal)):
            self.assertEqual((await self.bridge.scan())["installs"], [])

    async def test_real_streamreader_limit_uses_response_too_large_code(self):
        from bridge import MAX_RESPONSE_BYTES
        class Process(_FakeProcess):
            def kill(self):
                self.returncode = 0
            async def readline(self):
                if not hasattr(self, "reader"):
                    self.reader = asyncio.StreamReader(limit=MAX_RESPONSE_BYTES + 1)
                    self.reader.feed_data(b"x" * (MAX_RESPONSE_BYTES + 2) + b"\n")
                    self.reader.feed_eof()
                return await self.reader.readline()
        with patch("bridge.asyncio.create_subprocess_exec", return_value=Process(None)):
            with self.assertRaisesRegex(RuntimeError, "response_too_large"):
                await self.bridge.scan()
