import sys
from pathlib import Path
import tempfile
from types import ModuleType
import unittest

sys.modules.setdefault("decky", ModuleType("decky"))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import main  # noqa: E402  (needs the decky stub and plugin root on path)
from plugin_settings import Settings
from tasks import TaskManager


class StubBridge:
    def __init__(self, state_dir, state="committed"):
        self.state_dir = state_dir
        self.state = state
        self.transactions = []

    async def start_install(self, plan_id, confirmed_game_closed=False):
        return {"state": self.state, "transaction_id": plan_id}

    async def list_transactions(self):
        return list(self.transactions)


class StubPublisher:
    def __init__(self):
        self.cleared = []

    def clear_cache(self, instance_id, release_id):
        self.cleared.append((instance_id, release_id))
        return 1

    def caches(self):
        return []

    def states(self):
        return []

    def bridges(self):
        return []


class CacheCleanupTests(unittest.IsolatedAsyncioTestCase):
    def make_plugin(self, auto_cleanup=True, keep_backups=False, state="committed"):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        plugin = main.Plugin()
        plugin.settings = Settings(root)
        plugin.settings.set_auto_cleanup(auto_cleanup)
        plugin.settings.set_keep_backups(keep_backups)
        plugin.bridge = StubBridge(root / "state", state)
        plugin.publisher = StubPublisher()
        plugin.tasks = TaskManager()
        plugin._plan_metadata = {}
        plugin._prepare_cancel = None
        return plugin, root

    async def test_committed_install_deletes_publisher_cache(self):
        plugin, _ = self.make_plugin()
        plugin._plan_metadata["p1"] = {"cache": {"kind": "publisher", "instance_id": "9|root|game", "release_id": "patchmoe-9"}}
        result = await plugin.start_install("p1")
        self.assertEqual(result["state"], "committed")
        self.assertEqual(plugin.publisher.cleared, [("9|root|game", "patchmoe-9")])
        self.assertNotIn("p1", plugin._plan_metadata)

    async def test_disabled_switch_keeps_cache(self):
        plugin, _ = self.make_plugin(auto_cleanup=False)
        plugin._plan_metadata["p1"] = {"cache": {"kind": "publisher", "instance_id": "i", "release_id": "patchmoe-9"}}
        await plugin.start_install("p1")
        self.assertEqual(plugin.publisher.cleared, [])
        self.assertNotIn("p1", plugin._plan_metadata)

    async def test_rolled_back_install_keeps_cache(self):
        plugin, _ = self.make_plugin(state="rolled_back")
        plugin._plan_metadata["p1"] = {"cache": {"kind": "publisher", "instance_id": "i", "release_id": "patchmoe-9"}}
        await plugin.start_install("p1")
        self.assertEqual(plugin.publisher.cleared, [])
        self.assertIn("p1", plugin._plan_metadata)

    async def test_core_cache_deleted_on_commit(self):
        plugin, _ = self.make_plugin()
        downloads = plugin.bridge.state_dir / "downloads"
        downloads.mkdir(parents=True)
        sha = "c" * 64
        (downloads / f"{sha}.zip").write_bytes(b"z")
        plugin._plan_metadata["p1"] = {"cache": {"kind": "core", "sha256": sha}}
        await plugin.start_install("p1")
        self.assertFalse((downloads / f"{sha}.zip").exists())

    async def test_committed_install_deletes_stage_and_backup_by_default(self):
        plugin, _ = self.make_plugin()
        job = plugin.bridge.state_dir / "jobs" / "p1"
        job.mkdir(parents=True)
        (job / "0.stage").write_bytes(b"stage")
        (job / "0.backup").write_bytes(b"backup")
        result = await plugin.start_install("p1")
        self.assertEqual(result["state"], "committed")
        self.assertFalse((job / "0.stage").exists())
        self.assertFalse((job / "0.backup").exists())

    async def test_committed_install_keeps_backup_when_enabled(self):
        plugin, _ = self.make_plugin(keep_backups=True)
        job = plugin.bridge.state_dir / "jobs" / "p1"
        job.mkdir(parents=True)
        (job / "0.stage").write_bytes(b"stage")
        (job / "0.backup").write_bytes(b"backup")
        await plugin.start_install("p1")
        self.assertFalse((job / "0.stage").exists())
        self.assertTrue((job / "0.backup").exists())

    async def test_list_transactions_reports_backup_presence(self):
        plugin, _ = self.make_plugin()
        job = plugin.bridge.state_dir / "jobs" / "tx-1"
        job.mkdir(parents=True)
        (job / "0.backup").write_bytes(b"b")
        plugin.bridge.transactions = [
            {"transaction_id": "tx-1", "instance_id": "i", "release_id": "r", "state": "committed", "error": None},
            {"transaction_id": "tx-2", "instance_id": "i", "release_id": "r", "state": "committed", "error": None},
        ]
        result = {t["transaction_id"]: t for t in await plugin.list_transactions()}
        self.assertTrue(result["tx-1"]["backups"])
        self.assertFalse(result["tx-2"]["backups"])

    async def test_uninstall_refuses_when_backups_are_gone(self):
        plugin, _ = self.make_plugin()
        with self.assertRaisesRegex(ValueError, "uninstall_backups_missing"):
            await plugin.uninstall("p1")

    async def test_forget_without_backups_clears_store_but_keeps_recipe(self):
        plugin, _ = self.make_plugin()
        state = plugin.bridge.state_dir
        job = state / "jobs" / "tx"
        job.mkdir(parents=True)
        (job / "0.stage").write_bytes(b"s")
        (state / "journal.sqlite").write_bytes(b"db")
        (state / "catalog.json").write_bytes(b"cat")
        plugin._forget_without_backups(state)
        self.assertFalse((state / "journal.sqlite").exists())
        self.assertFalse((state / "jobs").exists())
        self.assertTrue((state / "catalog.json").exists())

    async def test_forget_without_backups_keeps_store_when_backup_present(self):
        plugin, _ = self.make_plugin()
        state = plugin.bridge.state_dir
        job = state / "jobs" / "tx"
        job.mkdir(parents=True)
        (job / "0.backup").write_bytes(b"b")
        (state / "journal.sqlite").write_bytes(b"db")
        plugin._forget_without_backups(state)
        self.assertTrue((state / "journal.sqlite").exists())
        self.assertTrue((state / "jobs").exists())

    async def test_clear_transaction_backups_removes_finished_only(self):
        plugin, _ = self.make_plugin()
        finished = plugin.bridge.state_dir / "jobs" / "tx-done"
        finished.mkdir(parents=True)
        (finished / "0.backup").write_bytes(b"b" * 40)
        pending = plugin.bridge.state_dir / "jobs" / "tx-pending"
        pending.mkdir(parents=True)
        (pending / "0.stage").write_bytes(b"s" * 30)
        plugin.bridge.transactions = [
            {"transaction_id": "tx-done", "state": "committed"},
            {"transaction_id": "tx-pending", "state": "planned"},
        ]
        value = await plugin.clear_transaction_backups()
        self.assertGreaterEqual(value["removed_bytes"], 40)
        self.assertFalse((finished / "0.backup").exists())
        self.assertTrue((pending / "0.stage").exists())

    async def test_settings_round_trip_and_clear_cache(self):
        plugin, _ = self.make_plugin()
        settings = await plugin.set_auto_cleanup(False)
        self.assertFalse(settings["auto_cleanup"])
        self.assertIn("cache", settings)
        status = await plugin.clear_cache()
        self.assertEqual(status["file_count"], 0)
        self.assertEqual(status["removed_bytes"], 0)

    async def test_clear_cache_refuses_while_busy(self):
        plugin, _ = self.make_plugin()
        plugin.tasks.busy = lambda: True
        with self.assertRaisesRegex(RuntimeError, "installer_busy"):
            await plugin.clear_cache()


if __name__ == "__main__":
    unittest.main()
