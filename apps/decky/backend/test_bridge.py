"""Generated v5 fixtures exercised through the actual Decky JSONL reader."""
import asyncio
import copy
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import AsyncMock, patch
from bridge import CoreBridge, CoreError, METHODS, _unique_object
from protocol_validation import validate_response, validate_event, validate_request, validate_id
from protocol_test_support import CASES, response, event, outcome

class _FakeProcess:
    """Minimal subprocess stand-in: builds stdout from the captured request."""

    def __init__(self, factory):
        self._factory = factory
        self.returncode = None
        self.payload = b""
        self.stdin = self
        self.stdout = self
        self._lines = None

    def write(self, data):
        self.payload += data

    async def drain(self):
        pass

    def close(self):
        pass

    def kill(self):
        self.returncode = 0

    async def wait(self):
        self.returncode = 0

    async def read(self, size=65536):
        if not self._lines:
            self.returncode = 0
            return b""
        remaining = b"".join(self._lines)
        self._lines = [remaining[size:]] if len(remaining) > size else []
        return remaining[:size]

    async def readline(self):
        if self._lines is None:
            request = json.loads(self.payload.split(b"\n", 1)[0])
            events, response = self._factory(request["request_id"], request)
            self._lines = [json.dumps(item, ensure_ascii=False).encode("utf-8") + b"\n"
                           for item in events]
            self._lines.append(json.dumps(response, ensure_ascii=False).encode("utf-8") + b"\n")
        if self._lines:
            return self._lines.pop(0)
        self.returncode = 0
        return b""


class GeneratedContractTests(unittest.TestCase):
    def test_all_generated_request_response_and_event_cases(self):
        for case in CASES["requests"]:
            with self.subTest(name=case["name"]):
                if case.get("error"):
                    with self.assertRaises((ValueError, RuntimeError)):
                        validate_request(case["request"], case.get("task_channel"))
                else:
                    validate_request(case["request"], case.get("task_channel"))
        for case in CASES["responses"]:
            with self.subTest(name=case["name"]):
                if case.get("error"):
                    with self.assertRaises((ValueError, RuntimeError)):
                        validate_response(case["response"], case["method"], "fixture")
                else:
                    validate_response(case["response"], case["method"], "fixture")
        for case in CASES["events"]:
            with self.subTest(name=case["name"]):
                if case.get("error"):
                    with self.assertRaises((ValueError, RuntimeError)):
                        validate_event(case["event"], "fixture")
                else:
                    validate_event(case["event"], "fixture")

    def test_shared_sequence_and_raw_json_cases(self):
        for case in CASES["sequences"]:
            last = -1
            decisions = []
            for index, row in enumerate(case["events"]):
                if "terminal_after" in case and index >= case["terminal_after"]:
                    decisions.append("drop")
                    continue
                try:
                    validate_event(row, "fixture")
                    sequence = CoreBridge._source_event(row, "fixture", last)
                    decisions.append("drop" if sequence <= last else "accept")
                    last = max(last, sequence)
                except RuntimeError:
                    decisions.append("error")
            self.assertEqual(decisions, case["expected"], case["name"])
        for case in CASES.get("raw_requests", []):
            payload = case["json"].encode("utf-8") if "json" in case else bytes.fromhex(case["hex"])
            with self.assertRaises((ValueError, UnicodeDecodeError), msg=case["name"]):
                json.loads(payload, object_pairs_hook=_unique_object)

    def test_utf8_id_controls(self):
        self.assertEqual(validate_id("界" * 42, 128), "界" * 42)
        for value in ("界" * 43, "a\u0085", "a\x7f", "a\x00", "\ud800"):
            with self.assertRaises(ValueError):
                validate_id(value, 128)


class BridgeStreamTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        path = Path(self.temp.name) / "binary"
        path.touch()
        self.bridge = CoreBridge(path, path, state_dir=Path(self.temp.name) / "state")

    async def test_preflight_refuses_old_versions_missing_contracts_and_schemas_before_write(self):
        variants = [("version", version, "unsupported_protocol") for version in (1, 2, 3)] + [
            ("contracts", None, "capability_mismatch"), ("methods", [], "capability_mismatch"),
            ("schema", 99, "installer_schema_unsupported")]
        for key, value, expected in variants:
            captured = []
            def factory(request_id, request):
                captured.append(request)
                row = response("get_capabilities", request_id)
                if key == "version":
                    row["protocol_version"] = value
                elif key == "schema":
                    row["result"]["installer"]["schema_version"] = value
                else:
                    row["result"][key] = value
                return [], row
            with patch("bridge.asyncio.create_subprocess_exec", side_effect=lambda *a, **kw: _FakeProcess(factory)):
                with self.assertRaisesRegex((RuntimeError, ValueError), expected):
                    await self.bridge.install_patch("instance", "release", operation="overwrite")
            self.assertEqual([row["method"] for row in captured], ["get_capabilities"])
            self.assertFalse(self.bridge.state_dir.exists())

    async def test_host_null_fields_outer_id_and_required_operation(self):
        captured = []
        def factory(request_id, request):
            captured.append(request)
            return [], response(request["method"], request_id)
        with patch("bridge.asyncio.create_subprocess_exec", side_effect=lambda *a, **kw: _FakeProcess(factory)):
            with self.assertRaisesRegex(ValueError, "invalid_operation"):
                await self.bridge.install_patch("instance", "release")
            await self.bridge.install_patch("instance", "release", operation="overwrite", request_id="same")
        request = captured[-1]
        self.assertEqual(request["request_id"], "same")
        self.assertNotIn("request_id", request["params"])
        self.assertEqual(set(request["params"]["host"]), {"roots", "catalog", "signed_catalog", "trust_keys", "state_dir", "platform"})
        self.assertIsNone(request["params"]["host"]["signed_catalog"])
        self.assertIsNone(request["params"]["host"]["trust_keys"])

    async def test_event_sequence_duplicate_backward_and_terminal_wins(self):
        self.bridge._preflight_done = True
        seen = []
        def factory(request_id, request):
            first = event("source_progress", request_id, sequence=0)
            second = event("source_progress", request_id, sequence=1)
            return [first, first, second, first, response("update_source", request_id)], {"event": "source_progress"}
        with patch("bridge.asyncio.create_subprocess_exec", return_value=_FakeProcess(factory)):
            await self.bridge._call("update_source", {"source_id": "fixture"}, on_event=seen.append)
        self.assertEqual([row["sequence"] for row in seen], [0, 1])

    async def test_terminal_receipt_seals_before_malformed_or_oversized_raw_tail(self):
        self.bridge._preflight_done = True
        class Process:
            def __init__(self, tail):
                self.tail = tail
                self.returncode = None
                self.stdout = self
                self.stdin = self
                self.reader = asyncio.StreamReader()
                self.read_sizes = []
                self.readline_calls = 0
                self.messages = []
                self.closed = False
                self.drained = 0
            def write(self, data):
                self.messages.append(data)
                request = json.loads(data)
                receipt = response("install_patch", request["request_id"])
                trailing = self.tail(request["request_id"]) if callable(self.tail) else self.tail
                self.reader.feed_data(json.dumps(receipt).encode("utf-8") + b"\n" + trailing)
                self.reader.feed_eof()
            async def drain(self): pass
            def close(self): self.closed = True
            async def readline(self):
                self.readline_calls += 1
                return await self.reader.readline()
            async def read(self, size):
                self.read_sizes.append(size)
                if not self.closed:
                    raise AssertionError("terminal did not close stdin")
                chunk = await self.reader.read(size)
                self.drained += len(chunk)
                return chunk
            async def wait(self): self.returncode = 0
        tails = [b"{malformed JSON\n", b"\xff\n", b"x" * (1 << 20) + b"\n",
                 lambda rid: json.dumps(response("install_patch", rid, outcome("rolled_back", "task_cancelled"))).encode("utf-8") + b"\n"]
        for tail in tails:
            peer = Process(tail)
            with patch("bridge.asyncio.create_subprocess_exec", return_value=peer):
                with self.assertLogs("bridge", level="WARNING") as captured:
                    result = await self.bridge.install_patch("instance", "release", operation="overwrite")
            self.assertEqual(result["state"], "committed")
            self.assertEqual(result["disposition"], "completed")
            self.assertEqual(peer.readline_calls, 1)
            self.assertTrue(peer.closed)
            self.assertTrue(peer.read_sizes and all(size == 65536 for size in peer.read_sizes))
            self.assertGreater(peer.drained, 0)
            self.assertEqual(len(peer.messages), 1)
            self.assertIn("core_output_after_terminal:", captured.output[0])

    async def test_valid_terminal_disposition_survives_later_nonzero_exit(self):
        self.bridge._preflight_done = True
        class Process(_FakeProcess):
            async def wait(self):
                self.returncode = 17
        for state, error, disposition in (("committed", None, "completed"), ("rolled_back", "task_cancelled", "cancelled")):
            def factory(request_id, request):
                return [], response("install_patch", request_id, outcome(state, error))
            with patch("bridge.asyncio.create_subprocess_exec", return_value=Process(factory)):
                with self.assertLogs("bridge", level="WARNING") as captured:
                    result = await self.bridge.install_patch("instance", "release", operation="overwrite")
            self.assertEqual(result["disposition"], disposition)
            self.assertIn("core_exit_after_terminal: 17", captured.output[0])

    async def test_unknown_missing_bool_counter_wrong_id_gap_and_v3_events_rejected(self):
        self.bridge._preflight_done = True
        mutations = [lambda row: row.update(extra=True), lambda row: row.pop("total"),
                     lambda row: row.update(completed=True), lambda row: row.update(request_id="wrong"),
                     lambda row: row.update(sequence=1), lambda row: row.update(protocol_version=3),
                     lambda row: row.update(sequence=9007199254740992)]
        for mutate in mutations:
            def factory(request_id, request):
                row = event("source_progress", request_id)
                mutate(row)
                return [row], response("update_source", request_id)
            with patch("bridge.asyncio.create_subprocess_exec", return_value=_FakeProcess(factory)):
                with self.assertRaisesRegex(RuntimeError, "contract_mismatch|unsupported_protocol"):
                    await self.bridge._call("update_source", {"source_id": "fixture"})

    async def test_actions_missing_unknown_response_or_invalid_error_are_refused(self):
        for mutate in (lambda row: row.update(extra=True), lambda row: row["result"].pop("installs"),
                       lambda row: row["result"]["installs"][0]["patches"][0].pop("actions")):
            def factory(request_id, request):
                row = response("scan", request_id)
                mutate(row)
                return [], row
            with patch("bridge.asyncio.create_subprocess_exec", return_value=_FakeProcess(factory)):
                with self.assertRaisesRegex(RuntimeError, "contract_mismatch"):
                    await self.bridge.scan()
        def factory(request_id, request):
            return [], {"protocol_version": 5, "request_id": request_id, "ok": False,
                        "error": {"code": "bad; human text", "detail": None}}
        with patch("bridge.asyncio.create_subprocess_exec", return_value=_FakeProcess(factory)):
            with self.assertRaisesRegex(RuntimeError, "contract_mismatch"):
                await self.bridge.scan()

    async def test_large_request_is_sent_without_a_fixed_line_limit(self):
        self.bridge._preflight_done = True
        rule = copy.deepcopy(next(case["request"]["params"]["rule"] for case in CASES["requests"] if case["name"] == "add_source-request"))
        rule["name"] += "x" * (128 * 1024)
        peer = _FakeProcess(lambda rid, req: ([], response("add_source", rid)))
        with patch("bridge.asyncio.create_subprocess_exec", return_value=peer):
            await self.bridge._call("add_source", {"rule": rule}, request_id="large")
        sent = json.loads(peer.payload.split(b"\n", 1)[0])
        self.assertEqual(sent["params"]["rule"]["name"], rule["name"])
        self.assertGreater(len(peer.payload), 128 * 1024)

    async def test_large_response_line_is_read_fully_then_next_call_recovers(self):
        line_size = 9 * 1024 * 1024
        class Process:
            """StreamReader built with the exact limit the bridge requested."""
            def __init__(self, limit):
                self.returncode = None
                self.stdin = self
                self.closed = False
                self.payload = b""
                self.stdout = asyncio.StreamReader(limit=limit)
            def write(self, data):
                self.payload += data
                request = json.loads(data.split(b"\n", 1)[0])
                encoded = json.dumps(response("scan", request["request_id"])).encode("utf-8")
                self.stdout.feed_data(encoded + b" " * (line_size - len(encoded)) + b"\n")
                self.stdout.feed_eof()
            async def drain(self): pass
            def close(self): self.closed = True
            def kill(self): self.returncode = 0
            async def wait(self): self.returncode = 0
            async def read(self, size=65536): return await self.stdout.read(size)
            async def readline(self): return await self.stdout.readline()
        captured = {}
        def spawn(*args, **kwargs):
            captured.update(kwargs)
            return Process(kwargs.get("limit", 2 ** 16))
        with patch("bridge.asyncio.create_subprocess_exec", side_effect=spawn):
            result = await self.bridge.scan()
        self.assertEqual(captured["limit"], float("inf"))
        self.assertEqual(result["installs"], response("scan")["result"]["installs"])
        with patch("bridge.asyncio.create_subprocess_exec", return_value=_FakeProcess(lambda rid, req: ([], response("scan", rid)))):
            self.assertIsInstance((await self.bridge.scan())["installs"], list)


class BridgeIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_real_core_script_service_import_read_replace_and_failure(self):
        from source_service import SourceService
        root = Path(__file__).resolve().parents[3]
        configured = os.environ.get("PATCHHUB_TEST_CORE")
        binary = Path(configured) if configured else root / "target/debug" / ("patchhub-core.exe" if os.name == "nt" else "patchhub-core")
        if not configured and not binary.exists():
            self.skipTest("core binary not built")
        with tempfile.TemporaryDirectory() as temporary:
            catalog = Path(temporary) / "catalog.json"
            catalog.write_text('{"schema_version":1,"revision":0,"releases":[]}')
            bridge = CoreBridge(binary, catalog, state_dir=Path(temporary) / "state")
            await bridge.preflight()
            service = await SourceService(bridge, Path(temporary) / "state").initialize()
            rule = json.loads((root / "docs/features/script-source-adapters/examples/publisher/source.json").read_text(encoding="utf-8"))
            code = "-- " + "脚本源码" * 15000 + "\nwhile true do end\nreturn {}"
            await service.import_script_source(rule, 1, code)
            self.assertEqual((await service.get_script_source(rule["id"]))["code"], code)
            await service.import_script_source(rule, 1, "error('not executed on save')", True)
            with self.assertRaises(CoreError) as failure:
                await service.import_script_source(rule, 1, "return {", True)
            self.assertEqual(failure.exception.code, "script_source_syntax_error")
            self.assertIn(rule["id"], failure.exception.detail)
            restarted = SourceService(bridge, Path(temporary) / "state")
            self.assertEqual((await restarted.get_script_source(rule["id"]))["code"], "error('not executed on save')")

    async def test_real_core_v5_scan_and_error(self):
        root = Path(__file__).resolve().parents[3]
        configured_binary = os.environ.get("PATCHHUB_TEST_CORE")
        binary = Path(configured_binary) if configured_binary else root / "target/debug" / ("patchhub-core.exe" if os.name == "nt" else "patchhub-core")
        if not configured_binary and not binary.exists(): self.skipTest("core binary not built")
        with tempfile.TemporaryDirectory() as temporary:
            catalog = Path(temporary) / "catalog.json"
            catalog.write_text(json.dumps({"schema_version": 1, "revision": 0, "releases": []}))
            bridge = CoreBridge(binary, catalog, state_dir=Path(temporary) / "state")
            await bridge.preflight()
            self.assertEqual((await bridge.scan())["catalog_trust"], "unsigned_development")
            with self.assertRaisesRegex(CoreError, "instance_not_found"):
                await bridge.preview("nonexistent-p5-instance", "nonexistent-release")


class BridgeJournalReadTests(unittest.IsolatedAsyncioTestCase):
    async def test_same_state_reads_across_bridges_queue_and_sources_remain_independent(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary, catalog = root / "binary", root / "catalog.json"
            binary.write_bytes(b"stub")
            catalog.write_text("{}")
            first = CoreBridge(binary, catalog, state_dir=root / "state")
            second = CoreBridge(binary, catalog, state_dir=root / "state" / ".." / "state")
            other = CoreBridge(binary, catalog, state_dir=root / "other")
            entered, release = asyncio.Event(), asyncio.Event()

            async def hold(*args):
                entered.set()
                await release.wait()
                return []

            first._exchange_call = hold
            second._exchange_call = AsyncMock(return_value=[])
            other._exchange_call = AsyncMock(return_value=[])
            running = asyncio.create_task(first.scan())
            await entered.wait()
            queued = asyncio.create_task(second.list_transactions())
            try:
                await asyncio.sleep(0)
                second._exchange_call.assert_not_awaited()
                # Source events can project rows by calling scan/list_transactions;
                # source network work must never own this read queue.
                await second.call_sync("list_sources", {})
                await second._call("update_source", {"source_id": "fixture"})
                self.assertEqual(second._exchange_call.await_count, 2)
                await other.list_transactions()
                other._exchange_call.assert_awaited_once()
                # Cancelling a queued read starts no process and releases its slot.
                queued.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await queued
                queued = asyncio.create_task(second.list_transactions())
                await asyncio.sleep(0)
                self.assertEqual(second._exchange_call.await_count, 2)
                release.set()
                await asyncio.wait_for(asyncio.gather(running, queued), 1)
                self.assertEqual(second._exchange_call.await_count, 3)
            finally:
                release.set()
                await asyncio.gather(running, queued, return_exceptions=True)

    async def test_real_busy_errors_are_not_hidden_and_queue_is_released(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary, catalog = root / "binary", root / "catalog.json"
            binary.write_bytes(b"stub")
            catalog.write_text("{}")
            bridge = CoreBridge(binary, catalog, state_dir=root / "state")
            bridge._exchange_call = AsyncMock(side_effect=[RuntimeError("installer_busy"), []])
            with self.assertRaisesRegex(RuntimeError, "installer_busy"):
                await bridge.scan()
            self.assertEqual(await bridge.list_transactions(), [])


class ManualDownloadBridgeTests(unittest.IsolatedAsyncioTestCase):
    async def test_explicit_input_uses_the_existing_install_request(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "patchhub-core"
            binary.touch()
            bridge = CoreBridge(binary, binary)
            bridge._call = AsyncMock(return_value={"state": "committed"})
            link = {"source_id": "fixture", "candidate_id": "candidate-a", "url": "https://user.example/download"}
            await bridge.install_patch("instance", operation="overwrite", request_id="manual", manual_download=link)
            bridge._call.assert_awaited_once_with("install_patch", {"instance_id": "instance", "manual_download": link, "operation": "overwrite", "cleanup_policy": "disabled"}, request_id="manual")
            with self.assertRaisesRegex(ValueError, "invalid_request"):
                await bridge.install_patch("instance", release_id="signed", operation="overwrite", manual_download=link)
            with self.assertRaisesRegex(ValueError, "invalid_request"):
                await bridge.install_patch("instance", archive="local.zip", operation="overwrite", manual_download=link)
