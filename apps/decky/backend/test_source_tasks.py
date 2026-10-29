"""RPC boundaries for v5 source tasks, projection and installation isolation."""
import asyncio
import importlib.util
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest.mock import AsyncMock, Mock

from bridge import CoreError
from protocol_test_support import response

sys.modules.setdefault("decky", types.ModuleType("decky"))
spec = importlib.util.spec_from_file_location("source_tasks_plugin", Path(__file__).parents[1] / "main.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

TASK = {"task_id": "task-1", "kind": "update_source", "key": "patchmoe", "state": "running",
        "instances": [], "appid": None, "progress": None, "matches": {}, "result": None,
        "error": None, "error_detail": None}
PUBLISHER = {"id": "patchmoe", "name": "patch.moe", "adapter": "patchmoe", "enabled": True,
             "role": "publisher", "complete": False, "revision": 0, "entry_count": 0,
             "interval_hours": 0, "cached_at": None, "checked_at": None,
             "status": "not_refreshed", "origin": "builtin", "pages": [], "page_hosts": [],
             "download_hosts": [], "priority": 100, "discovery_only": False, "root_layout": None}


def installed(instance, appid, status):
    return {"game": {"instance_id": instance, "appid": appid, "availability": "installed"},
            "patches": [], "discovery": {"status": status, "candidates": [], "diagnostics": []}}


def pending_report(status="pending", sources=None):
    return {"installs": [
        installed("local", 123, status),
        installed("matched", 124, "publisher_match"),
        {"game": {"instance_id": "uninstalled", "appid": 125, "availability": "not_installed"},
         "patches": [], "discovery": {"status": "pending", "candidates": [], "diagnostics": []}},
    ], "sources": [PUBLISHER] if sources is None else sources, "settings_revision": 2}


class SourceTasksRpcTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)

    def plugin(self):
        plugin = module.Plugin()
        plugin._scan_generation = 4
        plugin.sources = types.SimpleNamespace(
            settings_revision=2,
            start_update_source=Mock(return_value=dict(TASK)),
            start_match_games=Mock(return_value={**TASK, "kind": "match_games", "key": "match"}),
            start_rematch_game=Mock(return_value={**TASK, "kind": "rematch_game", "key": "local"}),
            task_status=Mock(side_effect=lambda task_id=None: [dict(TASK)]
                               if task_id is None else dict(TASK)),
            cancel_task=Mock(return_value={**TASK, "state": "cancelling"}),
            set_update_interval=AsyncMock(return_value={"settings_revision": 3, "source_id": "patchmoe",
                                                         "interval_hours": 6}),
            set_name=AsyncMock(),
            reload_sources=AsyncMock(return_value=[dict(PUBLISHER)]),
            list_sources=Mock(return_value=[dict(PUBLISHER)]),
            prime=Mock(),
        )
        plugin.bridge = types.SimpleNamespace(
            scan=AsyncMock(return_value=pending_report(status="publisher_match")),
            call_sync=AsyncMock(return_value={"settings_revision": 2, "instances": []}),
            selection=module.CoreBridge.selection,
        )
        plugin.tasks = module.TaskManager()
        plugin.settings = types.SimpleNamespace(auto_cleanup=True)
        return plugin

    async def test_update_source_and_match_tasks_delegate_immediately(self):
        plugin = self.plugin()
        self.assertEqual((await plugin.update_source("patchmoe", True))["task_id"], "task-1")
        plugin.sources.start_update_source.assert_called_once_with("patchmoe", True)
        await plugin.match_games(["local"], ["Game"], True)
        plugin.sources.start_match_games.assert_called_once_with(["local"], ["Game"], True)
        await plugin.rematch_game("local", 123, ["Game"])
        plugin.sources.start_rematch_game.assert_called_once_with("local", 123, ["Game"])
        self.assertFalse(plugin.tasks.busy())

    async def test_name_write_returns_reloaded_source_names(self):
        plugin = self.plugin()
        plugin.sources.list_sources.return_value = [{**PUBLISHER, "name": "Shiravune"}]
        result = await plugin.set_source_name("patchmoe", "Shiravune")
        plugin.sources.set_name.assert_awaited_once_with("patchmoe", "Shiravune")
        self.assertEqual(result[0]["name"], "Shiravune")
        plugin.bridge.scan.assert_not_awaited()

    async def test_task_status_and_cancel_are_keyed_by_task_id(self):
        plugin = self.plugin()
        self.assertEqual((await plugin.source_task_status("task-1"))["task_id"], "task-1")
        plugin.sources.task_status.assert_called_once_with("task-1")
        self.assertEqual(len(await plugin.source_task_status()), 1)
        plugin.sources.task_status.assert_called_with(None)
        self.assertEqual((await plugin.cancel_source_task("task-1"))["state"], "cancelling")
        plugin.sources.cancel_task.assert_called_once_with("task-1")

    async def test_permission_write_requires_idle_installation(self):
        plugin = self.plugin()
        finish = asyncio.Event()
        async def work(task):
            await finish.wait()
            task.check()
            return {"disposition": "completed", "state": "committed"}
        task = plugin.tasks.get(plugin.tasks.start("install", work)["task_id"])
        try:
            with self.assertRaisesRegex(RuntimeError, "installer_busy"):
                await plugin.set_source_update_interval("patchmoe", 6)
            plugin.sources.set_update_interval.assert_not_awaited()
            with self.assertRaisesRegex(RuntimeError, "installer_busy"):
                await plugin.set_source_name("patchmoe", "Shiravune")
            plugin.sources.set_name.assert_not_awaited()
            # Source tasks and reads stay available while the installer runs.
            await plugin.update_source("patchmoe")
            await plugin.project_instances(["local"])
            self.assertEqual((await plugin.list_sources())[0]["id"], "patchmoe")
        finally:
            task.request_cancel()
            finish.set()
            await task.worker

    async def test_install_runs_without_cancelling_source_tasks(self):
        plugin = self.plugin()
        plugin.bridge.install_patch = AsyncMock(return_value={"disposition": "completed", "state": "committed"})
        started = await plugin.start_patch_install_task("local", "signed", None, None, "install", "request-1")
        await plugin.tasks.get(started["task_id"]).worker
        self.assertEqual((await plugin.task_status(started["task_id"]))["state"], "completed")
        plugin.sources.cancel_task.assert_not_called()
        for name in ("tools",):
            self.assertFalse(hasattr(plugin.sources, name))

    async def test_scan_is_one_core_read_and_project_instances_does_not_scan(self):
        plugin = self.plugin()
        report = await plugin.scan()
        plugin.bridge.scan.assert_awaited_once_with()
        self.assertEqual(report["generation"], 5)
        self.assertEqual(report["settings_revision"], 2)
        plugin.bridge.call_sync.return_value = {"settings_revision": 2,
                                                "instances": [{"game": {"instance_id": "local"}}]}
        result = await plugin.project_instances(["local"])
        plugin.bridge.scan.assert_awaited_once_with()
        plugin.bridge.call_sync.assert_awaited_once_with("project_instances", {"instance_ids": ["local"]})
        self.assertEqual(result["instances"][0]["game"]["instance_id"], "local")
        for invalid in (None, "local", [1], ["ok", 2]):
            with self.subTest(value=invalid):
                with self.assertRaises((ValueError, RuntimeError)):
                    await plugin.project_instances(invalid)

    async def test_scan_starts_one_incremental_match_for_first_seen_installs(self):
        plugin = self.plugin()
        plugin.bridge.scan.return_value = pending_report()
        await plugin.scan()
        # Installed pending rows enter matching automatically; uninstalled rows
        # and already-matched rows stay out. scan() still returns immediately.
        plugin.sources.start_match_games.assert_called_once_with(["local"], force_details=False)
        plugin.bridge.scan.assert_awaited_once_with()

    async def test_scan_skips_incremental_match_without_pending_rows_or_enabled_publisher(self):
        matched = self.plugin()
        matched.bridge.scan.return_value = {"installs": [installed("local", 123, "publisher_match")],
                                            "sources": [PUBLISHER], "settings_revision": 2}
        await matched.scan()
        matched.sources.start_match_games.assert_not_called()

        disabled = self.plugin()
        disabled.sources.list_sources.return_value = [{**PUBLISHER, "enabled": False}]
        disabled.bridge.scan.return_value = pending_report()
        await disabled.scan()
        disabled.sources.start_match_games.assert_not_called()

        fallback_only = self.plugin()
        fallback_only.sources.list_sources.return_value = [
            {**PUBLISHER, "id": "hgamer", "role": "fallback_indexer"}]
        fallback_only.bridge.scan.return_value = pending_report()
        await fallback_only.scan()
        fallback_only.sources.start_match_games.assert_not_called()

    async def test_scan_still_matches_when_sources_have_no_index(self):
        plugin = self.plugin()
        plugin.sources.list_sources.return_value = [
            {**PUBLISHER, "complete": False, "revision": 0, "status": "not_refreshed"}]
        plugin.bridge.scan.return_value = pending_report()
        await plugin.scan()
        # Incomplete sources are not skipped: the match task reports incomplete
        # instead of hiding the unqueried state.
        plugin.sources.start_match_games.assert_called_once_with(["local"], force_details=False)

    async def test_startup_check_runs_the_same_pending_match_once(self):
        plugin = self.plugin()
        plugin.bridge.scan.return_value = pending_report()
        await plugin._startup_match_check()
        plugin.bridge.scan.assert_awaited_once_with()
        plugin.sources.start_match_games.assert_called_once_with(["local"], force_details=False)

    async def test_repeated_scan_does_not_stack_an_active_match_task(self):
        class MatchBridge:
            def __init__(self):
                self.gate = asyncio.Event()
                self.stream_calls = []

            async def call_sync(self, method, params=None):
                if method == "get_capabilities":
                    return response("get_capabilities")["result"]
                if method == "list_sources":
                    return [dict(PUBLISHER)]
                raise AssertionError(method)

            async def _call(self, method, params, on_event=None, cancel=None,
                            controlled=None, request_id=None):
                self.stream_calls.append((method, params))
                while not self.gate.is_set():
                    if cancel is not None and cancel.is_set():
                        raise CoreError("task_cancelled")
                    await asyncio.sleep(0)
                return {"settings_revision": 2, "source_revisions": {}, "results": []}

        bridge = MatchBridge()
        service = await module.SourceService(bridge, self.temp.name).initialize()
        plugin = module.Plugin()
        plugin._scan_generation = 0
        plugin.sources = service
        plugin.bridge = types.SimpleNamespace(scan=AsyncMock(return_value=pending_report()))
        try:
            await plugin.scan()
            while not bridge.stream_calls:
                await asyncio.sleep(0)
            await plugin.scan()
            self.assertEqual([call[0] for call in bridge.stream_calls], ["match_games"])
            self.assertEqual(len(service.task_status()), 1)
            bridge.gate.set()
            await service._tasks[service.task_status()[0]["task_id"]]["worker"]
            # A terminal task no longer deduplicates the next scan.
            await plugin.scan()
            while len(bridge.stream_calls) < 2:
                await asyncio.sleep(0)
            self.assertEqual(len(bridge.stream_calls), 2)
        finally:
            await service.shutdown()

    async def test_removed_v4_source_rpcs_are_absent(self):
        plugin = self.plugin()
        for name in ("_project_row", "_refresh_rows", "scan_status", "start_game_discovery",
                     "cancel_source_query", "discover_game", "_discovery_game", "_merge_diagnostics"):
            self.assertFalse(hasattr(plugin, name), name)

    async def test_instance_identity_uses_generated_utf8_limit(self):
        plugin = self.plugin()
        plugin.bridge.call_sync.return_value = {"settings_revision": 2, "instances": []}
        limit = module.MAX_INSTANCE_ID_BYTES
        instance = "界" * (limit // 3) + "a" * (limit % 3)
        await plugin.project_instances([instance])
        plugin.bridge.call_sync.assert_awaited_with("project_instances", {"instance_ids": [instance]})
        for invalid in (instance + "a", "local\u0085", "local\x7f"):
            with self.assertRaisesRegex(ValueError, "invalid_id"):
                await plugin.project_instances([invalid])


if __name__ == "__main__":
    unittest.main()
