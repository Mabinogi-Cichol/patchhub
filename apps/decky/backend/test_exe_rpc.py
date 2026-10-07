"""EXE RPC fences, plan identity and post-commit recovery retention."""
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock
import uuid

from test_discovery_rpc import module


class ExeRpcTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.state = Path(self.temp.name)
        self.release = "exe-" + "a" * 24 + "-42"
        self.instance = "42|synthetic-library|synthetic-game"
        self.core_id = str(uuid.uuid4())
        self.game = {"instance_id": self.instance, "appid": 42}
        self.local = SimpleNamespace(state_dir=self.state, cleanup_transactions=AsyncMock(), preview=AsyncMock(), discard_plan=AsyncMock(return_value={"state": "rolled_back"}),
            start_install=AsyncMock(return_value={"plan_id": self.core_id, "state": "committed"}))
        self.plugin = module.Plugin()
        p = self.plugin
        p.tasks = module.TaskManager()
        p.settings = SimpleNamespace(keep_backups=True, auto_cleanup=True)
        p._prepare_cancel = None
        p._plan_metadata = {}
        p._scan_games = {self.instance: self.game}
        p.bridge = SimpleNamespace(scan=AsyncMock(return_value={"installs": [{"game": self.game}]}), state_dir=self.state)
        p.publisher = SimpleNamespace(bridges=lambda: [(self.instance, self.release, self.local)],
            bridge_for=Mock(return_value=self.local), _state=lambda *args: self.state,
            clear_cache=Mock())
        p.exe_profiles = SimpleNamespace(resolve=Mock())
        p.exe_installation = SimpleNamespace(prepare=AsyncMock(), validate_apply=AsyncMock(), release_lease=Mock())
        p._read_report = AsyncMock(return_value={"installs": [{"game": self.game, "patches": [{"release_id": self.release,
                            "support_level": "publisher_exe", "blockers": []}]}]})
        self.wrapped_id = p._publisher_id(self.instance, self.release, self.core_id)

    async def test_exe_cannot_use_import_or_core_fetch_and_plan_shortcuts(self):
        p = self.plugin
        for call, code in ((p._plan_install(self.instance, self.release, "/tmp/arbitrary.zip"), "publisher_prepare_first"),
                           (p.fetch_artifact(self.instance, self.release), "exe_prepare_first"),
                           (p._prepare_install(self.instance, self.release, "/tmp/arbitrary.exe"), "exe_manual_import_not_supported")):
            with self.assertRaisesRegex(ValueError, code):
                await call
        p.exe_installation.prepare.assert_not_awaited()
        self.assertIsNone(p._prepare_cancel)

    async def test_preview_resolves_current_profile_before_local_bridge(self):
        p = self.plugin
        p.exe_profiles.resolve.side_effect = ValueError("exe_profile_changed")
        with self.assertRaisesRegex(ValueError, "exe_profile_changed"):
            await p.preview(self.instance, self.release)
        p.publisher.bridge_for.assert_not_called()
        self.local.preview.assert_not_awaited()

    async def test_apply_rechecks_fresh_service_without_manual_confirmation(self):
        p = self.plugin
        p.exe_installation.validate_apply.side_effect = ValueError("exe_candidate_changed")
        with self.assertRaisesRegex(ValueError, "exe_candidate_changed"):
            await p._start_install(self.wrapped_id)
        p.exe_installation.validate_apply.assert_awaited_once_with(self.game, self.release, self.core_id)
        self.local.start_install.assert_not_awaited()

    async def test_committed_exe_requests_core_terminal_cleanup(self):
        p = self.plugin
        p.settings.keep_backups = False
        job = self.state / "jobs" / self.core_id
        job.mkdir(parents=True)
        (job / "0.backup").write_bytes(b"original")
        (job / "0.stage").write_bytes(b"patched")
        result = await p._start_install(self.wrapped_id)
        self.assertEqual(result["state"], "committed")
        self.assertEqual(result["plan_id"], self.wrapped_id)
        self.local.cleanup_transactions.assert_awaited_once()
        p.publisher.clear_cache.assert_called_once_with(self.instance, self.release)

    async def test_cleanup_evidence_failure_preserves_committed_result_and_cache(self):
        p = self.plugin
        p.exe_installation.release_lease.side_effect = ValueError("exe_policy_invalid")
        result = await p._start_install(self.wrapped_id)
        self.assertEqual(result["state"], "committed")
        p.publisher.clear_cache.assert_not_called()

    async def test_completed_rollback_releases_exe_lease_without_success_cleanup(self):
        self.local.start_install.return_value = {"plan_id": self.core_id, "state": "rolled_back", "error": "task_cancelled"}
        result = await self.plugin._start_install(self.wrapped_id)
        self.assertEqual(result["state"], "rolled_back")
        self.plugin.exe_installation.release_lease.assert_called_once_with(self.state, self.core_id)
        self.plugin.publisher.clear_cache.assert_not_called()

    async def test_discard_rpc_failure_still_invalidates_plan_locally(self):
        job=self.state / "jobs" / self.core_id;job.mkdir(parents=True)
        self.local.discard_plan.side_effect=RuntimeError("rpc_failed")
        with self.assertRaisesRegex(RuntimeError,"rpc_failed"):
            await self.plugin.discard_plan(self.wrapped_id)
        self.assertTrue((job / "revoked").exists())
        self.plugin.exe_installation.release_lease.assert_called_once()

    async def test_selection_preparation_wraps_the_actual_content_identity(self):
        selected = "exe-selection-" + "b"*24 + "-42"
        content = "exe-content-" + "c"*24 + "-42"
        archive = self.state / "normalized.zip"; archive.write_bytes(b"checked archive")
        self.plugin._read_report.return_value = {"installs": [{"game": self.game,
            "patches": [{"release_id": selected, "support_level": "publisher_exe", "blockers": []}]}]}
        self.plugin.exe_installation.prepare.return_value = (self.local, str(archive),
            {"plan_id": self.core_id, "release_id": content}, "download")
        prepared = await self.plugin._prepare_install(self.instance, selected)
        self.assertEqual(prepared["plan"]["release_id"], content)
        self.assertEqual(prepared["plan"]["plan_id"], self.plugin._publisher_id(self.instance, content, self.core_id))
        self.assertEqual(self.plugin._plan_metadata[prepared["plan"]["plan_id"]]["release_id"], content)

    async def test_stale_selection_routes_to_same_candidate_content_for_reinstall(self):
        selected = "exe-selection-" + "b"*24 + "-42"
        content = "exe-content-" + "c"*24 + "-42"
        archive = self.state / "normalized.zip"; archive.write_bytes(b"checked archive")
        self.plugin.exe_profiles.resolve.return_value = ({}, {"candidate_id": "official-candidate"})
        self.plugin._read_report.return_value = {"installs": [{"game": self.game,
            "patches": [{"release_id": content, "candidate_id": "official-candidate", "support_level": "publisher_exe", "blockers": []}]}]}
        self.plugin.exe_installation.prepare.return_value = (self.local, str(archive),
            {"plan_id": self.core_id, "release_id": content}, "download")
        prepared = await self.plugin._prepare_install(self.instance, selected, operation="reinstall")
        self.assertEqual(prepared["plan"]["release_id"], content)
        self.plugin.exe_installation.prepare.assert_awaited_once_with(self.game, content, unittest.mock.ANY, operation="reinstall")
