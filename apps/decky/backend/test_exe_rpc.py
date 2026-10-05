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
        self.local = SimpleNamespace(preview=AsyncMock(), discard_plan=AsyncMock(return_value={"state": "rolled_back"}),
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
        p.scan = AsyncMock(return_value={"installs": [{"game": self.game, "patches": [{"release_id": self.release,
                            "support_level": "publisher_exe", "blockers": []}]}]})
        self.wrapped_id = p._publisher_id(self.instance, self.release, self.core_id)

    async def test_exe_cannot_use_import_or_core_fetch_and_plan_shortcuts(self):
        p = self.plugin
        for call, code in ((p.plan_install(self.instance, self.release, "/tmp/arbitrary.zip"), "publisher_prepare_first"),
                           (p.fetch_artifact(self.instance, self.release), "exe_prepare_first"),
                           (p.prepare_install(self.instance, self.release, "/tmp/arbitrary.exe"), "exe_manual_import_not_supported")):
            with self.assertRaisesRegex(ValueError, code):
                await call
        p.settings.keep_backups = False
        with self.assertRaisesRegex(ValueError, "exe_backup_required"):
            await p.prepare_install(self.instance, self.release)
        p.exe_installation.prepare.assert_not_awaited()
        self.assertIsNone(p._prepare_cancel)

    async def test_preview_resolves_current_profile_before_local_bridge(self):
        p = self.plugin
        p.exe_profiles.resolve.side_effect = ValueError("exe_profile_changed")
        with self.assertRaisesRegex(ValueError, "exe_profile_changed"):
            await p.preview(self.instance, self.release)
        p.publisher.bridge_for.assert_not_called()
        self.local.preview.assert_not_awaited()

    async def test_apply_requires_close_confirmation_and_fresh_service_validation(self):
        p = self.plugin
        with self.assertRaisesRegex(ValueError, "game_closed_confirmation_required"):
            await p.start_install(self.wrapped_id)
        p.exe_installation.validate_apply.side_effect = ValueError("exe_candidate_changed")
        with self.assertRaisesRegex(ValueError, "exe_candidate_changed"):
            await p.start_install(self.wrapped_id, True)
        p.exe_installation.validate_apply.assert_awaited_once_with(self.game, self.release, self.core_id)
        self.local.start_install.assert_not_awaited()

    async def test_restart_and_global_setting_change_cannot_delete_exe_backup(self):
        p = self.plugin
        p.settings.keep_backups = False
        job = self.state / "jobs" / self.core_id
        job.mkdir(parents=True)
        (job / "0.backup").write_bytes(b"original")
        (job / "0.stage").write_bytes(b"patched")
        result = await p.start_install(self.wrapped_id, True)
        self.assertEqual(result["state"], "committed")
        self.assertEqual(result["plan_id"], self.wrapped_id)
        self.assertEqual((job / "0.backup").read_bytes(), b"original")
        self.assertFalse((job / "0.stage").exists())
        p.publisher.clear_cache.assert_called_once_with(self.instance, self.release)

    async def test_cleanup_evidence_failure_preserves_committed_result_and_cache(self):
        p = self.plugin
        p.exe_installation.release_lease.side_effect = ValueError("exe_policy_invalid")
        result = await p.start_install(self.wrapped_id, True)
        self.assertEqual(result["state"], "committed")
        p.publisher.clear_cache.assert_not_called()

    async def test_discard_uses_rust_before_offline_release_and_drops_metadata(self):
        p = self.plugin
        p._plan_metadata[self.wrapped_id] = {"cache": {}}
        p.exe_installation.release_lease.side_effect = ValueError("exe_policy_invalid")
        result = await p.discard_plan(self.wrapped_id)
        self.assertEqual(result["state"], "rolled_back")
        self.local.discard_plan.assert_awaited_once_with(self.core_id)
        self.assertNotIn(self.wrapped_id, p._plan_metadata)
