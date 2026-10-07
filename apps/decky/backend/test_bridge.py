import asyncio
import os
from pathlib import Path
import unittest
import tempfile
import json
from unittest.mock import AsyncMock, patch

from bridge import (CoreBridge, CONTROLLED_METHODS, METHODS, SOURCE_EVENTS, SOURCE_METHODS)


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

    async def wait(self):
        self.returncode = 0

    async def read(self, size=65536):
        return b""

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


class BridgeContractTests(unittest.TestCase):
    def test_source_methods_and_events_are_registered(self):
        self.assertTrue(SOURCE_METHODS.issubset(METHODS))
        self.assertTrue(CONTROLLED_METHODS.issubset(METHODS))
        self.assertEqual(SOURCE_EVENTS, {"source_progress", "source_update"})
        for method in ("list_source_matches", "get_source_state", "get_source_settings"):
            self.assertIn(method, SOURCE_METHODS)

    def test_source_event_requires_binding_and_monotonic_sequence(self):
        self.assertEqual(CoreBridge._source_event({"request_id": "r", "sequence": 0}, "r", -1), 0)
        with self.assertRaisesRegex(RuntimeError, "invalid_core_response"):
            CoreBridge._source_event({"request_id": "other", "sequence": 0}, "r", -1)
        with self.assertRaisesRegex(RuntimeError, "invalid_core_response"):
            CoreBridge._source_event({"request_id": "r", "sequence": 1}, "r", 1)
        with self.assertRaisesRegex(RuntimeError, "invalid_core_response"):
            CoreBridge._source_event({"request_id": "r", "sequence": "1"}, "r", -1)

    def test_call_sync_is_async(self):
        self.assertTrue(asyncio.iscoroutinefunction(CoreBridge.call_sync))


class BridgeStreamTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        binary = root / "steam-patch"
        binary.write_bytes(b"stub")
        catalog = root / "catalog.json"
        catalog.write_text(json.dumps({"schema_version": 1, "revision": 0, "releases": []}))
        self.bridge = CoreBridge(binary, catalog, state_dir=root / "state")

    async def test_source_events_are_forwarded_before_final_response(self):
        seen = []

        def factory(request_id, request):
            events = [
                {"event": "source_progress", "request_id": request_id, "sequence": 0,
                 "completed": 0, "total": 1, "running": True},
                {"event": "source_update", "request_id": request_id, "sequence": 1, "appid": 7,
                 "discovery": {"status": "complete", "candidates": [], "diagnostics": []}},
            ]
            response = {"protocol_version": 2, "request_id": request_id, "ok": True,
                        "result": {"settings_revision": 3, "results": []}}
            return events, response

        process = _FakeProcess(factory)
        spawned = AsyncMock(return_value=process)
        with patch("bridge.asyncio.create_subprocess_exec", spawned):
            result = await self.bridge._call("scan_sources", {}, on_event=seen.append)
        self.assertEqual(result["settings_revision"], 3)
        self.assertEqual([event["event"] for event in seen], ["source_progress", "source_update"])
        self.assertEqual(spawned.await_args.args[1], "serve-task")

    async def test_foreign_source_event_is_rejected(self):
        def factory(request_id, request):
            events = [{"event": "source_progress", "request_id": "foreign", "sequence": 0}]
            return events, {"ok": True}

        process = _FakeProcess(factory)
        with patch("bridge.asyncio.create_subprocess_exec", AsyncMock(return_value=process)):
            with self.assertRaisesRegex(RuntimeError, "invalid_core_response"):
                await self.bridge._call("scan_sources", {}, on_event=lambda event: None)

    async def test_call_sync_uses_one_shot_serve(self):
        def factory(request_id, request):
            return [], {"protocol_version": 2, "request_id": request_id, "ok": True,
                        "result": {"settings_revision": 4}}

        process = _FakeProcess(factory)
        spawned = AsyncMock(return_value=process)
        with patch("bridge.asyncio.create_subprocess_exec", spawned):
            result = await self.bridge.call_sync("set_source_enabled", {"source_id": "s", "enabled": False})
        self.assertEqual(result["settings_revision"], 4)
        self.assertEqual(spawned.await_args.args[1], "serve")

    async def test_missing_capability_surfaces_core_error(self):
        def factory(request_id, request):
            return [], {"protocol_version": 2, "request_id": request_id, "ok": False,
                        "error": {"code": "unsupported_method"}}

        process = _FakeProcess(factory)
        with patch("bridge.asyncio.create_subprocess_exec", AsyncMock(return_value=process)):
            with self.assertRaisesRegex(RuntimeError, "unsupported_method"):
                await self.bridge.call_sync("discover_game", {})


class BridgeIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        root = Path(__file__).resolve().parents[3]
        executable = "steam-patch.exe" if os.name == "nt" else "steam-patch"
        self.binary = root / "target/debug" / executable
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        catalog = Path(self.temp.name) / "catalog.json"
        catalog.write_text(json.dumps({"schema_version": 1, "revision": 0, "releases": []}))
        self.bridge = CoreBridge(self.binary, catalog)

    async def test_scan_real_core_returns_platform_and_empty_catalog(self):
        if not self.binary.exists():
            self.skipTest("core binary not built")
        result = await self.bridge.scan()
        self.assertEqual(result["platform"], "steamos")
        self.assertEqual(result["catalog_revision"], 0)
        self.assertEqual(result["catalog_trust"], "unsigned_development")
        self.assertIsInstance(result["installs"], list)

    async def test_preview_core_error_preserves_code(self):
        if not self.binary.exists():
            self.skipTest("core binary not built")
        with self.assertRaisesRegex(RuntimeError, "instance_not_found"):
            await self.bridge.preview("nonexistent-p0-instance", "nonexistent-release")

    async def test_rejects_unbounded_request_before_spawning(self):
        with self.assertRaisesRegex(ValueError, "invalid_id"):
            await self.bridge.preview("x" * 65536, "r")


if __name__ == "__main__":
    unittest.main()
