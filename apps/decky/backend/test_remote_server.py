"""Real loopback HTTP checks; source operations remain mocked at the host boundary."""
import asyncio
import io
import importlib.util
import json
from pathlib import Path
import socket
import sys
import tempfile
import types
import unittest
from unittest.mock import AsyncMock, Mock, patch

from aiohttp import ClientSession, ClientConnectorError, web
from bridge import CoreError
from remote_server import PREFERRED_PORT, RemoteServer, local_ipv4_addresses
from source_service import SourceService


TASK = {"task_id": "remote-task", "kind": "update_source", "key": "custom", "state": "running",
        "progress": {"phase": "fetch_index", "completed": 1, "total": 2}, "result": None,
        "error": None, "error_detail": None}
SOURCE = {"id": "custom", "role": "publisher", "enabled": True}


class RemoteServerTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        for name in ("index.html", "app.js", "tabler.min.css", "LICENSE-tabler.txt", "style.css"):
            (root / name).write_text(f"static {name}", encoding="utf-8")
        self.plugin = types.SimpleNamespace(
            sources=types.SimpleNamespace(settings_revision=9, task_status=Mock(return_value=[dict(TASK)])),
            list_sources=AsyncMock(return_value=[dict(SOURCE)]),
            import_script_source=AsyncMock(return_value={"rule": dict(SOURCE), "settings_revision": 9}),
            get_script_source=AsyncMock(return_value={"code": "return {}", "source_id": "custom"}),
            get_script_source_prompt=AsyncMock(return_value={"prompt": "完整 AI 提示词"}),
            set_source_enabled=AsyncMock(return_value=[dict(SOURCE)]),
            set_source_name=AsyncMock(return_value=[dict(SOURCE)]),
            set_source_update_interval=AsyncMock(return_value={"interval_hours": 24}),
            remove_source=AsyncMock(return_value=[]),
            update_source=AsyncMock(return_value=dict(TASK)),
            source_task_status=AsyncMock(return_value=dict(TASK)),
            cancel_source_task=AsyncMock(return_value={**TASK, "state": "cancelling"}),
        )
        self.server = RemoteServer(self.plugin, root)
        self.addresses = patch("remote_server.local_ipv4_addresses", return_value=["127.0.0.1"])
        self.addresses.start()
        self.addCleanup(self.addresses.stop)
        self.assertFalse(self.server.status()["enabled"])
        self.status = await self.server.start("127.0.0.1")
        self.client = ClientSession()

    async def asyncTearDown(self):
        await self.client.close()
        await self.server.stop()

    async def request(self, method, path, *, body=None, headers=None):
        values = {"Authorization": f"Bearer {self.status['access_key']}", "Origin": self.status["url"]}
        values.update(headers or {})
        payload = None
        if body is not None:
            payload = io.BytesIO(json.dumps(body, ensure_ascii=False).encode())
            values["Content-Type"] = "application/json"
        async with self.client.request(method, self.status["url"] + path, data=payload, headers=values) as response:
            return response.status, await response.json()

    async def test_static_resources_are_local_and_never_contain_key(self):
        for path in ("/", "/index.html", "/app.js", "/tabler.min.css", "/style.css", "/LICENSE-tabler.txt"):
            async with self.client.get(self.status["url"] + path) as response:
                self.assertEqual(response.status, 200)
                self.assertNotIn(self.status["access_key"], await response.text())
                self.assertEqual(response.headers["X-Content-Type-Options"], "nosniff")
        status, _ = await self.request("GET", "/main.py")
        self.assertEqual(status, 404)

    async def test_auth_host_origin_and_fixed_route_boundaries(self):
        for authorization in ("", "Bearer wrong"):
            status, error = await self.request("GET", "/api/sources", headers={"Authorization": authorization})
            self.assertEqual((status, error["code"]), (401, "unauthorized"))
        status, error = await self.request("GET", "/api/sources", headers={"Host": "attacker.invalid"})
        self.assertEqual((status, error["code"]), (403, "invalid_host"))
        for origin in ("", "http://attacker.invalid"):
            status, error = await self.request("POST", "/api/sources", body={"code": "return {}"}, headers={"Origin": origin})
            self.assertEqual((status, error["code"]), (403, "invalid_origin"))
        self.plugin.import_script_source.assert_not_awaited()
        status, _ = await self.request("POST", "/api/install_patch", body={})
        self.assertEqual(status, 404)
        status, _ = await self.request("POST", "/api/sources", body=["invalid"])
        self.assertEqual(status, 400)
        status, _ = await self.request("POST", "/api/sources", body={"code": False})
        self.assertEqual(status, 400)

    async def test_source_routes_reuse_host_operations_and_full_lua(self):
        status, data = await self.request("GET", "/api/sources")
        self.assertEqual((status, data), (200, {"sources": [SOURCE], "settings_revision": 9}))
        code = "-- " + "完整来源" * 100000 + "\nreturn {}"
        status, data = await self.request("POST", "/api/sources", body={"code": code})
        self.assertEqual((status, data["task_id"]), (200, "remote-task"))
        self.plugin.import_script_source.assert_awaited_once_with(code, None)
        self.plugin.update_source.assert_awaited_once_with("custom", True)
        status, data = await self.request("PUT", "/api/sources/custom/script", body={"code": "edited"})
        self.assertEqual(status, 200)
        self.assertNotIn("task_id", data)
        self.plugin.import_script_source.assert_awaited_with("edited", "custom")
        status, data = await self.request("GET", "/api/sources/custom/script")
        self.assertEqual((status, data["code"]), (200, "return {}"))
        status, data = await self.request("GET", "/api/script-prompt")
        self.assertEqual((status, data["prompt"]), (200, "完整 AI 提示词"))
        for operation, body in (("enabled", {"enabled": False}), ("name", {"name": "新名称"}),
                                ("interval", {"interval_hours": 24})):
            status, _ = await self.request("PUT", f"/api/sources/custom/{operation}", body=body)
            self.assertEqual(status, 200)
        self.plugin.set_source_enabled.assert_awaited_once_with("custom", False, refresh=False)
        self.plugin.set_source_name.assert_awaited_once_with("custom", "新名称", refresh=False)
        self.plugin.set_source_update_interval.assert_awaited_once_with("custom", 24, refresh=False)
        status, data = await self.request("DELETE", "/api/sources/custom")
        self.assertEqual((status, data), (200, []))
        self.plugin.remove_source.assert_awaited_once_with("custom", refresh=False)

    async def test_import_update_start_failure_preserves_save_success(self):
        self.plugin.update_source.side_effect = RuntimeError("plugin_update_in_progress")
        status, data = await self.request("POST", "/api/sources", body={"code": "return {}"})
        self.assertEqual((status, data["settings_revision"]), (200, 9))
        self.assertEqual(data["task_error"]["code"], "plugin_update_in_progress")

    async def test_saved_settings_and_delete_succeed_when_followup_refresh_fails(self):
        sys.modules.setdefault("decky", types.ModuleType("decky"))
        spec = importlib.util.spec_from_file_location("remote_plugin_test", Path(__file__).parents[1] / "main.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        plugin = module.Plugin()
        plugin.tasks = types.SimpleNamespace(busy=lambda: False)
        receipt = {"source_id": "custom", "settings_revision": 10}
        async def call_sync(method, params):
            if method == "list_sources":
                raise CoreError("source_read_failed", "read unavailable after save")
            return dict(receipt)
        bridge = types.SimpleNamespace(call_sync=AsyncMock(side_effect=call_sync))
        plugin.sources = SourceService(bridge, self.temp.name)
        self.server.plugin = plugin
        for method, path, body in (("PUT", "/api/sources/custom/name", {"name": "saved"}),
                                   ("PUT", "/api/sources/custom/enabled", {"enabled": False}),
                                   ("PUT", "/api/sources/custom/interval", {"interval_hours": 24}),
                                   ("DELETE", "/api/sources/custom", None)):
            status, data = await self.request(method, path, body=body)
            self.assertEqual((status, data), (200, receipt))
            self.assertEqual(plugin.sources.settings_revision, 10)
        self.assertNotIn("list_sources", [call.args[0] for call in bridge.call_sync.await_args_list])
        status, data = await self.request("GET", "/api/sources")
        self.assertEqual((status, data["code"]), (400, "source_read_failed"))

    async def test_domain_busy_and_storage_errors_keep_details(self):
        detail = "source.lua:12: " + "完整错误" * 1000
        for failure, expected_status, expected_code in (
                (CoreError("script_source_syntax_error", detail), 400, "script_source_syntax_error"),
                (RuntimeError("installer_busy"), 409, "installer_busy"),
                (OSError("disk unavailable"), 500, "storage_error")):
            self.plugin.import_script_source.side_effect = failure
            status, data = await self.request("POST", "/api/sources", body={"code": "invalid"})
            self.assertEqual((status, data["code"]), (expected_status, expected_code))
            if expected_code == "script_source_syntax_error":
                self.assertEqual(data["detail"], detail)

    async def test_remote_tasks_use_original_ids_progress_cancellation_and_result(self):
        status, _ = await self.request("GET", "/api/tasks/local-install-task")
        self.assertEqual(status, 404)
        status, task = await self.request("POST", "/api/sources/custom/updates", body={"force": True})
        self.assertEqual((status, task), (200, {"task_id": "remote-task"}))
        status, data = await self.request("GET", "/api/tasks/remote-task")
        self.assertTrue(data["running"])
        self.assertEqual(data["progress"], TASK["progress"])
        status, data = await self.request("POST", "/api/tasks/remote-task/cancel")
        self.assertEqual(status, 200)
        self.assertTrue(data["cancel_requested"])
        self.plugin.cancel_source_task.assert_awaited_once_with("remote-task")
        self.plugin.source_task_status.return_value = {**TASK, "state": "failed", "error": "network_failed", "error_detail": "upstream offline"}
        _, data = await self.request("GET", "/api/tasks/remote-task")
        self.assertFalse(data["running"])
        self.assertEqual(data["error"], {"code": "network_failed", "detail": "upstream offline"})
        self.plugin.source_task_status.return_value = {**TASK, "state": "completed", "result": {"settings_revision": 10}}
        _, data = await self.request("GET", "/api/tasks/remote-task")
        self.assertEqual(data["result"], {"settings_revision": 10})

    async def test_update_all_filters_disabled_fallback_and_already_running_sources(self):
        self.plugin.list_sources.return_value = [SOURCE, {**SOURCE, "id": "disabled", "enabled": False},
                                                 {**SOURCE, "id": "fallback", "role": "fallback_indexer"},
                                                 {**SOURCE, "id": "active"}]
        self.plugin.source_task_status.return_value = [{**TASK, "key": "active"}]
        status, data = await self.request("POST", "/api/updates")
        self.assertEqual((status, data), (200, {"task_ids": ["remote-task"]}))
        self.plugin.update_source.assert_awaited_once_with("custom", False)

    async def test_stop_releases_port_and_restart_invalidates_old_key(self):
        original = self.status
        self.assertRegex(original["access_key"], r"^[0-9]{6}$")
        _, task = await self.request("POST", "/api/sources/custom/updates")
        self.assertEqual(await self.server.start("127.0.0.1"), original)
        await self.server.stop()
        self.assertEqual(self.server.status(), {"enabled": False, "addresses": ["127.0.0.1"], "url": None, "access_key": None})
        with self.assertRaises(ClientConnectorError):
            await self.client.get(original["url"])
        port = int(original["url"].rsplit(":", 1)[1])
        released = await asyncio.start_server(lambda reader, writer: writer.close(), "127.0.0.1", port)
        released.close()
        await released.wait_closed()
        self.status = await self.server.start("127.0.0.1")
        self.assertNotEqual(self.status["access_key"], original["access_key"])
        status, _ = await self.request("GET", "/api/sources", headers={"Authorization": f"Bearer {original['access_key']}"})
        self.assertEqual(status, 401)
        status, _ = await self.request("GET", f"/api/tasks/{task['task_id']}")
        self.assertEqual(status, 404)
        self.plugin.cancel_source_task.assert_not_awaited()

    async def test_default_port_stays_fixed_and_falls_back_while_occupied(self):
        self.assertEqual(int(self.status["url"].rsplit(":", 1)[1]), PREFERRED_PORT)
        await self.server.stop()
        occupied = await asyncio.start_server(lambda reader, writer: writer.close(), "127.0.0.1", PREFERRED_PORT)
        try:
            status = await self.server.start("127.0.0.1")
            self.assertNotEqual(int(status["url"].rsplit(":", 1)[1]), PREFERRED_PORT)
            await self.server.stop()
        finally:
            occupied.close()
            await occupied.wait_closed()
        status = await self.server.start("127.0.0.1")
        self.assertEqual(int(status["url"].rsplit(":", 1)[1]), PREFERRED_PORT)

    async def test_start_failures_do_not_publish_connection_details(self):
        await self.server.stop()
        with self.assertRaisesRegex(ValueError, "invalid_remote_address"):
            await self.server.start("0.0.0.0")
        with patch.object(web.TCPSite, "start", AsyncMock(side_effect=OSError("cannot bind"))):
            with self.assertRaisesRegex(OSError, "cannot bind"):
                await self.server.start("127.0.0.1")
        self.assertFalse(self.server.status()["enabled"])
        self.assertIsNone(self.server.status()["url"])
        self.assertIsNone(self.server.status()["access_key"])


@unittest.skipUnless(sys.platform == "linux", "requires Linux network interfaces")
class AddressTests(unittest.TestCase):
    def test_linux_interfaces_are_real_bindable_ipv4_addresses(self):
        for address in local_ipv4_addresses():
            with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
                listener.bind((address, 0))
