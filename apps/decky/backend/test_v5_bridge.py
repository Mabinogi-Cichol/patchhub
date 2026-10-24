"""Protocol-v5 transport, identity and terminal-outcome regression tests."""
import asyncio
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import AsyncMock, patch
from bridge import CoreBridge, CoreError, METHODS
from tasks import Task, TaskManager, current_task
from test_bridge import _FakeProcess
from protocol_test_support import response, outcome


class Peer:
    def __init__(self):
        self.returncode = None
        self.stdout = asyncio.StreamReader()
        self.stdin = self
        self.request = None
        self.lines = []
        self.cancelled = asyncio.Event()
        self.finished = asyncio.Event()

    def write(self, data):
        self.lines.append(data)
        if self.request is None:
            self.request = json.loads(data)
        elif data == b"cancel\n":
            self.cancelled.set()

    async def drain(self):
        await asyncio.sleep(0)

    def close(self):
        pass

    async def wait(self):
        await self.finished.wait()

    def finish(self, result):
        row = response("install_patch", self.request["request_id"], outcome(**result))
        self.stdout.feed_data(json.dumps(row).encode() + b"\n")
        self.stdout.feed_eof()
        self.returncode = 0
        self.finished.set()


class V5BridgeTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        binary = Path(self.temp.name) / "patchhub-core"
        binary.touch()
        self.bridge = CoreBridge(binary, binary)
        self.bridge._preflight_done = True

    async def test_control_cancel_continues_until_receipt_drains(self):
        peer = Peer()
        task = Task("install", {})
        token = current_task.set(task)
        try:
            with patch("bridge.asyncio.create_subprocess_exec", return_value=peer):
                pending = asyncio.create_task(self.bridge.install_patch("local", selection_id="exe-selection-aaaaaaaaaaaaaaaaaaaaaaaa-1", request_id="same", operation="overwrite"))
                while peer.request is None:
                    await asyncio.sleep(0)
                self.assertEqual(peer.request["request_id"], "same")
                self.assertNotIn("request_id", peer.request["params"])
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
                await self.bridge.install_patch("local", release_id="signed", operation="overwrite", cleanup_policy="enabled")
            spawn.assert_not_called()
        peer = Peer()
        with patch("bridge.asyncio.create_subprocess_exec", return_value=peer):
            pending = asyncio.create_task(self.bridge.install_patch(
                "local", release_id="signed", cleanup_policy="after_commit", operation="overwrite"))
            while peer.request is None:
                await asyncio.sleep(0)
            self.assertEqual(peer.request["params"]["cleanup_policy"], "after_commit")
            peer.finish({"state": "committed"})
            await pending

    async def test_task_cancel_controls_background_bridge(self):
        manager = TaskManager()
        peer = Peer()
        async def work(task):
            return await self.bridge.install_patch("local", release_id="signed", operation="overwrite")
        with patch("bridge.asyncio.create_subprocess_exec", return_value=peer):
            task = manager.get(manager.start("install", work)["task_id"])
            while peer.request is None:
                await asyncio.sleep(0)
            task.request_cancel()
            await asyncio.wait_for(peer.cancelled.wait(), 1)
            self.assertTrue(task.cancel.is_set())
            self.assertFalse(task.worker.done())
            peer.finish({"state": "needs_recovery", "error": "restore_conflict"})
            await task.worker
        self.assertEqual(task.snapshot()["state"], "failed")
        self.assertTrue(task.snapshot()["result"]["recovery"]["required"])

    async def test_second_terminal_and_late_progress_are_only_diagnostics(self):
        for extra in ({"event": "install_progress"}, {"ok": True, "result": {}}):
            def factory(request_id, request):
                receipt = response("install_patch", request_id)
                return [receipt], extra
            with patch("bridge.asyncio.create_subprocess_exec", return_value=_FakeProcess(factory)):
                with self.assertLogs("bridge", level="WARNING") as captured:
                    self.assertEqual((await self.bridge.install_patch("local", "signed", operation="overwrite"))["state"], "committed")
            self.assertIn("core_output_after_terminal:", captured.output[0])

    async def test_terminal_result_returns_before_unbounded_tail_or_exit_observation(self):
        class TailReader(asyncio.StreamReader):
            def __init__(self, mode):
                super().__init__()
                self.mode = mode
                self.producing = True
                self.read_sizes = []
            async def read(self, size):
                self.read_sizes.append(size)
                if self.mode == "continuous_tail" and self.producing:
                    await asyncio.sleep(0)
                    return b"x" * size
                return await super().read(size)

        class TailPeer(Peer):
            def __init__(self, mode):
                super().__init__()
                self.stdout = TailReader(mode)
                self.mode = mode
                self.kill_calls = 0
                self.closed = False
            def write(self, data):
                first = self.request is None
                super().write(data)
                if first:
                    row = response("install_patch", self.request["request_id"])
                    self.stdout.feed_data(json.dumps(row).encode("utf-8") + b"\n")
                    if self.mode == "exit_wait":
                        self.stdout.feed_eof()
            def close(self): self.closed = True
            def kill(self):
                self.kill_calls += 1
                raise AssertionError("valid terminal receipt cannot kill a writer")
            def release_observer(self):
                self.stdout.producing = False
                if not self.stdout.at_eof():
                    self.stdout.feed_eof()
                self.returncode = 0
                self.finished.set()

        for mode, diagnostic in (("never_eof", "timeout"), ("continuous_tail", "byte_limit"), ("exit_wait", "timeout")):
            with self.subTest(mode=mode):
                peer = TailPeer(mode)
                pending_call = None
                try:
                    with patch("bridge.asyncio.create_subprocess_exec", return_value=peer), patch("bridge.TERMINAL_DRAIN_SECONDS", .2), patch("bridge.TERMINAL_DRAIN_BYTES", 1024):
                        with self.assertLogs("bridge", level="WARNING") as captured:
                            pending_call = asyncio.create_task(self.bridge.install_patch("local", "signed", operation="overwrite"))
                            completed, _ = await asyncio.wait({pending_call}, timeout=.5)
                            self.assertIn(pending_call, completed, "valid receipt remained blocked on terminal transport")
                            result = pending_call.result()
                        self.assertEqual(result["disposition"], "completed")
                        self.assertIn("core_observation_after_terminal_deferred: " + diagnostic, "\n".join(captured.output))
                        self.assertTrue(peer.closed)
                        self.assertEqual(peer.kill_calls, 0)
                        self.assertEqual(len(peer.lines), 1)
                        self.assertFalse(peer.cancelled.is_set())
                        self.assertTrue(self.bridge._terminal_observers)
                        self.assertTrue(all(0 < size <= 65536 for size in peer.stdout.read_sizes))
                finally:
                    peer.release_observer()
                    if pending_call is not None:
                        await asyncio.gather(pending_call, return_exceptions=True)
                    # Explicitly await the independent observer: no leaked task,
                    # cancellation warning, or orphan subprocess in this fixture.
                    await asyncio.gather(*self.bridge._terminal_observers)
                    await asyncio.sleep(0)
                self.assertFalse(self.bridge._terminal_observers)

    async def test_removed_methods_and_host_path_injection_are_rejected_before_spawn(self):
        with patch("bridge.asyncio.create_subprocess_exec", new_callable=AsyncMock) as spawn:
            for method in ("migrate_state", "plan_install", "get_source_state", "fetch_artifact",
                       "refresh_sources", "scan_sources", "discover_game"):
                self.assertNotIn(method, METHODS)
                with self.assertRaisesRegex(ValueError, "unsupported_method"):
                    await self.bridge._call(method, {})
            with self.assertRaisesRegex(ValueError, "host_parameter_forbidden"):
                await self.bridge._call("scan", {"state_dir": "/tmp/attacker"})
            spawn.assert_not_called()

    async def test_core_error_detail_survives_task_failure(self):
        def factory(request_id, request):
            return [], {"protocol_version": 5, "request_id": request_id, "ok": False,
                        "error": {"code": "analysis_failed", "detail": "bounded diagnostic"}}
        manager = TaskManager()
        with patch("bridge.asyncio.create_subprocess_exec", return_value=_FakeProcess(factory)):
            async def work(task):
                return await self.bridge.install_patch("local", "signed", operation="overwrite")
            task = manager.get(manager.start("install", work)["task_id"])
            await task.worker
        self.assertEqual(task.snapshot()["error"], "analysis_failed")
        self.assertEqual(task.snapshot()["error_detail"], "bounded diagnostic")

    async def test_commit_wins_late_cancel_and_terminal_update_is_ignored(self):
        manager = TaskManager()
        async def work(task):
            task.request_cancel()
            return outcome()
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
            self.assertEqual(bridge.worker, bridge.binary.with_name("patchhub-worker"))

    async def test_large_valid_scan_result_succeeds_and_a_later_call_still_works(self):
        class Process(_FakeProcess):
            def kill(self):
                self.returncode = 0
        def oversized(request_id, request):
            result = response("scan")["result"]
            result["diagnostics"] = [{"code": "note", "message": "x" * (9 * 1024 * 1024)}]
            return [], {"protocol_version": 5, "request_id": request_id, "ok": True, "result": result}
        with patch("bridge.asyncio.create_subprocess_exec", return_value=Process(oversized)):
            result = await self.bridge.scan()
        self.assertEqual(result["diagnostics"][0]["message"], "x" * (9 * 1024 * 1024))
        def normal(request_id, request):
            return [], {"protocol_version": 5, "request_id": request_id, "ok": True,
                        "result": response("scan")["result"]}
        with patch("bridge.asyncio.create_subprocess_exec", return_value=Process(normal)):
            self.assertEqual((await self.bridge.scan())["installs"], response("scan")["result"]["installs"])

    async def test_real_streamreader_reads_a_large_response_line(self):
        line_size = 9 * 1024 * 1024
        class Process(_FakeProcess):
            def __init__(self, limit):
                super().__init__(None)
                self._limit = limit
            def kill(self):
                self.returncode = 0
            async def readline(self):
                if not hasattr(self, "reader"):
                    self.reader = asyncio.StreamReader(limit=self._limit)
                    request = json.loads(self.payload.split(b"\n", 1)[0])
                    encoded = json.dumps(response("scan", request["request_id"])).encode("utf-8")
                    self.reader.feed_data(encoded + b" " * (line_size - len(encoded)) + b"\n")
                    self.reader.feed_eof()
                return await self.reader.readline()
        captured = {}
        def spawn(*args, **kwargs):
            captured.update(kwargs)
            return Process(kwargs.get("limit", 2 ** 16))
        with patch("bridge.asyncio.create_subprocess_exec", side_effect=spawn):
            result = await self.bridge.scan()
        self.assertEqual(captured["limit"], float("inf"))
        self.assertEqual(result["installs"], response("scan")["result"]["installs"])

    def test_bridge_has_no_total_call_timeout(self):
        with self.assertRaises(TypeError):
            CoreBridge(self.bridge.binary, self.bridge.catalog, timeout=1)
        self.assertFalse(hasattr(self.bridge, "timeout"))

    async def test_progressing_call_is_not_interrupted_by_a_total_duration(self):
        peer = Peer()
        with patch("bridge.asyncio.create_subprocess_exec", return_value=peer):
            pending = asyncio.create_task(self.bridge.install_patch("local", release_id="signed", operation="overwrite"))
            while peer.request is None:
                await asyncio.sleep(0)
            done, _ = await asyncio.wait({pending}, timeout=.3)
            self.assertEqual(done, set())
            peer.finish({"state": "committed"})
            self.assertEqual((await pending)["state"], "committed")
