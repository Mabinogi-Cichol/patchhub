import asyncio
import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
from threading import Event
import types
import unittest
from unittest.mock import AsyncMock

from acquisition import find_archive

decky_stub = types.ModuleType("decky")
sys.modules.setdefault("decky", decky_stub)
spec = importlib.util.spec_from_file_location("acquisition_plugin", Path(__file__).parents[1] / "main.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class AcquireTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.downloads = self.root / "Downloads"
        self.downloads.mkdir()
        self.cache = self.root / "state/downloads"
        self.cache.mkdir(parents=True)

    def plugin(self, release, content):
        plugin = module.Plugin()
        plugin.downloads = self.downloads
        plugin._prepare_cancel = None
        plugin.tasks = module.TaskManager()
        plugin._plan_metadata = {}
        bridge = types.SimpleNamespace(state_dir=self.root / "state", signed_catalog=self.root / "signed.json",
            fetch_artifact=AsyncMock(), cancel_download=AsyncMock(return_value={"canceled": True}),
            discard_plan=AsyncMock())
        plugin.bridge = bridge
        metadata = dict(release_id=release, recipe=dict(artifact_sha256=hashlib.sha256(content).hexdigest()),
            artifact=dict(size=len(content), url="https://download.patch.moe/shared.zip"))
        bridge.signed_catalog.write_text(json.dumps(dict(payload=base64.b64encode(json.dumps(dict(catalog=dict(
            revision=7, releases=[metadata]))).encode()).decode())))
        plugin._read_report = AsyncMock(return_value=dict(catalog_trust="signed",catalog_revision=7,installs=[
            dict(game=dict(instance_id="instance"),patches=[dict(release_id=release,blockers=[],support_level="managed_zip",download_available=True)])]))
        plugin._plan_install = AsyncMock(return_value=dict(plan_id="id",changes=[]))
        return plugin

    async def test_two_unrelated_releases_reuse_renamed_matching_archives(self):
        for release, content in (("first", b"first archive"), ("other-title", b"different archive")):
            with self.subTest(release=release):
                archive = self.downloads / (release + ".zip")
                archive.write_bytes(content)
                plugin = self.plugin(release, content)
                result = await plugin._prepare_install("instance", release)
                self.assertEqual(result["archive"], str(archive))
                self.assertEqual(result["acquisition"], "local")
                plugin.bridge.fetch_artifact.assert_not_called()
                plugin._plan_install.assert_awaited_once_with("instance", release, str(archive), "install")

    async def test_wrong_hash_falls_back_to_fetch_and_immediately_plans(self):
        (self.downloads / "wrong.zip").write_bytes(b"bad")
        plugin = self.plugin("any-title", b"good")
        path = self.cache / "good.zip"
        async def download(*args):
            path.write_bytes(b"good")
            return {"archive": str(path)}
        plugin.bridge.fetch_artifact.side_effect = download
        result = await plugin._prepare_install("instance", "any-title")
        self.assertEqual(result["acquisition"], "download")
        plugin._plan_install.assert_awaited_once()

    async def test_cancellation_between_download_and_plan_never_plans(self):
        plugin = self.plugin("any-title", b"good")
        async def downloaded(*args):
            await plugin.cancel_download()
            return {"archive": str(self.cache / "unused.zip")}
        plugin.bridge.fetch_artifact.side_effect = downloaded
        with self.assertRaisesRegex(RuntimeError, "download_cancelled"):
            await plugin._prepare_install("instance", "any-title")
        plugin._plan_install.assert_not_called()
        self.assertIsNone(plugin._prepare_cancel)

    async def test_cancellation_after_plan_discards_staged_plan(self):
        archive = self.downloads / "local.zip"
        archive.write_bytes(b"good")
        plugin = self.plugin("any-title", b"good")
        async def planned(*args):
            plugin._prepare_cancel.set()
            plan_id = "11111111-1111-4111-8111-111111111111"
            (plugin.bridge.state_dir / "jobs" / plan_id).mkdir(parents=True)
            return {"plan_id": plan_id, "changes": []}
        plugin._plan_install.side_effect = planned
        with self.assertRaisesRegex(RuntimeError, "download_cancelled"):
            await plugin._prepare_install("instance", "any-title")
        plugin.bridge.discard_plan.assert_awaited_once_with("11111111-1111-4111-8111-111111111111")
        self.assertEqual(plugin._plan_metadata, {})
        self.assertIsNone(plugin._prepare_cancel)

    async def test_cancelled_publisher_plan_is_discarded_through_its_own_bridge(self):
        plugin = self.plugin("source-title", b"good")
        plugin._read_report.return_value["installs"][0]["patches"][0]["support_level"] = "publisher_zip"
        owner = types.SimpleNamespace(discard_plan=AsyncMock(), state_dir=self.root / "publisher")
        async def prepared(game, entry, cancel, archive, operation="install"):
            cancel.set()
            plan_id = "22222222-2222-4222-8222-222222222222"
            (owner.state_dir / "jobs" / plan_id).mkdir(parents=True)
            return owner, "unused.zip", {"plan_id": plan_id}, "download"
        plugin.publisher = types.SimpleNamespace(match=lambda *args: {"release_id": "source-title"},
            prepare=AsyncMock(side_effect=prepared))
        with self.assertRaisesRegex(RuntimeError, "download_cancelled"):
            await plugin._prepare_install("instance", "source-title")
        owner.discard_plan.assert_awaited_once_with("22222222-2222-4222-8222-222222222222")
        plugin.bridge.discard_plan.assert_not_called()

    def test_canceled_local_search_aborts(self):
        (self.downloads / "candidate.zip").write_bytes(b"data")
        event = Event()
        event.set()
        with self.assertRaisesRegex(RuntimeError, "download_cancelled"):
            find_archive([self.downloads], hashlib.sha256(b"data").hexdigest(), 4, "", event)


class PublisherScanTests(unittest.IsolatedAsyncioTestCase):
    def plugin(self, *, expired=False, transaction_state="uninstalled", old_build=False, state_dir=None):
        plugin = module.Plugin()
        game = dict(instance_id="instance", appid=9, name="Game", buildid="101", availability="installed")
        base = dict(catalog_trust="signed",catalog_revision=4,installs=[dict(game=game,patches=[])],diagnostics=[])
        plugin.bridge = types.SimpleNamespace(scan=AsyncMock(return_value=base))
        plugin.sources = types.SimpleNamespace(refresh=AsyncMock(return_value=[]), list_sources=lambda:[],
            list_game_sources=lambda game: [], settings_revision=0,
            cached_discovery=lambda game: {"status": "not_queried", "candidates": [], "diagnostics": []})
        plugin.list_transactions = AsyncMock(return_value=[])
        local_patch = dict(release_id="patchmoe-9",blockers=["version_incompatible"] if old_build else [],
            detection=dict(status="not_detected",files=[]))
        signed = dict(catalog_trust="signed",catalog_error=None,installs=[dict(game=game,patches=[local_patch])])
        local = types.SimpleNamespace(scan=AsyncMock(side_effect=[dict(signed,catalog_trust="unsigned_development",catalog_error="catalog_expired"),signed] if expired else None, return_value=signed),
            list_transactions=AsyncMock(return_value=[dict(transaction_id="j1", state=transaction_state)]),
            state_dir=state_dir)
        plugin.publisher = types.SimpleNamespace(matches=lambda game:[dict(release_id="patchmoe-9",source_id="patchmoe",source_page="https://www.patch.moe/",language="en",date="2000/1/1")],
            bridge_for=lambda *args:local,bridges=lambda:[("instance", "patchmoe-9", local)],state_diagnostics=[],refresh_signature=AsyncMock(return_value=local))
        plugin.discover_game = AsyncMock(return_value={"status":"empty", "candidates":[], "diagnostics":[]})
        return plugin

    async def test_expired_local_signature_is_renewed_and_rescanned(self):
        plugin=self.plugin(expired=True)
        result=await plugin.scan()
        plugin.publisher.refresh_signature.assert_awaited_once_with("instance","patchmoe-9")
        self.assertEqual(result["installs"][0]["patches"][0]["blockers"],[])

    async def test_uninstalled_old_build_may_prepare_but_committed_is_protected(self):
        plugin=self.plugin(old_build=True)
        result=await plugin.scan()
        self.assertEqual(result["installs"][0]["patches"][0]["blockers"],["version_incompatible"])
        plugin=self.plugin(old_build=True,transaction_state="committed")
        result=await plugin.scan()
        blockers=result["installs"][0]["patches"][0]["blockers"]
        self.assertIn("version_incompatible",blockers)
        self.assertIn("version_incompatible",blockers)

    async def test_other_source_active_transaction_blocks_install(self):
        plugin = self.plugin()
        plugin.list_transactions.return_value = [dict(instance_id="instance", release_id="source-vendor-9", state="committed")]
        result = await plugin.scan()
        self.assertIn("publisher_existing_transaction", result["installs"][0]["patches"][0]["blockers"])

    async def test_committed_without_backups_does_not_block_reinstall(self):
        with tempfile.TemporaryDirectory() as temp:
            state = Path(temp)
            (state / "jobs" / "j1").mkdir(parents=True)
            plugin = self.plugin(transaction_state="committed", state_dir=state)
            result = await plugin.scan()
            self.assertEqual(result["installs"][0]["patches"][0]["blockers"], [])

    async def test_committed_with_backups_still_blocks_reinstall(self):
        with tempfile.TemporaryDirectory() as temp:
            state = Path(temp)
            job = state / "jobs" / "j1"
            job.mkdir(parents=True)
            (job / "0.backup").write_bytes(b"b")
            plugin = self.plugin(transaction_state="committed", state_dir=state)
            result = await plugin.scan()
            self.assertEqual(result["installs"][0]["patches"][0]["blockers"], [])

    async def test_multiple_sources_get_independent_entries(self):
        plugin = self.plugin()
        plugin.publisher.matches = lambda game: [dict(release_id=f"source-{sid}-9", source_id=sid,
            source_page=f"https://{sid}.example/", language="en", date="2000/1/1") for sid in ("first", "second")]
        plugin.publisher.bridge_for = lambda *args: None
        report = await plugin.scan()
        self.assertEqual({p["release_id"] for p in report["installs"][0]["patches"]}, {"source-first-9", "source-second-9"})

    async def test_prepare_scan_preserves_source_metadata_without_refreshing_sources(self):
        plugin = self.plugin()
        source = {"id": "patchmoe", "name": "Official patch feed", "status": "cached",
            "cached_at": "2026-10-03T00:00:00Z"}
        plugin.sources.list_sources = lambda: [source]
        report = await plugin.scan(refresh_sources=False)
        plugin.sources.refresh.assert_not_awaited()
        patch = report["installs"][0]["patches"][0]
        self.assertEqual(patch["source_id"], source["id"])
        self.assertEqual(patch["source_name"], source["name"])
        self.assertEqual(patch["source_status"], source["status"])
        self.assertEqual(patch["source_cached_at"], source["cached_at"])

    async def test_scan_refreshes_zip_sources_and_force_refreshes_again(self):
        plugin = self.plugin()
        plugin.publisher.matches = lambda game: []
        async def refresh(force=False):
            plugin.publisher.matches = lambda game: [dict(release_id="patchmoe-9", source_id="patchmoe",
                source_page="https://www.patch.moe/", language="en", date="2000/1/1")]
            return []
        plugin.sources.refresh.side_effect = refresh
        first = await plugin.scan()
        self.assertEqual(first["installs"][0]["patches"], [])
        await plugin._scan_session.worker
        plugin.sources.refresh.assert_awaited_once_with(force=False)
        updated = await plugin.scan_status(first["generation"])
        self.assertTrue(updated["installs"][0]["patches"][0]["auto_installable"])
        second = await plugin.scan(refresh_sources=True)
        await plugin._scan_session.worker
        self.assertEqual(second["generation"], 2)
        self.assertEqual(plugin.sources.refresh.await_count, 2)
        plugin.sources.refresh.assert_awaited_with(force=True)

    async def test_game_source_rpc_changes_scan_without_changing_other_games(self):
        from test_sources import TABLE
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            config = root / "sources.json"
            config.write_text(json.dumps({"schema_version": 1, "sources": [TABLE]}))
            plugin = self.plugin()
            plugin._prepare_cancel = None
            plugin.tasks = module.TaskManager()
            plugin.sources = module.SourceRegistry(root / "state", config)
            plugin.sources._cache["second"] = {"entries": [dict(title="Game", aliases=[], appid=9,
                url="https://cdn.example.org/game.zip", language="en", date="2000/1/1",
                source_page=TABLE["pages"][0])]}
            # Persist the cache so a settings reload can restore it.
            cache = plugin.sources._cache_path("second")
            cache.parent.mkdir(parents=True)
            cache.write_text(json.dumps(plugin.sources._cache["second"]))
            other = dict(instance_id="other", appid=10, name="Game", buildid="101", availability="installed")
            plugin.bridge.scan.return_value["installs"].append(dict(game=other, patches=[]))
            plugin.publisher.matches = plugin.sources.matches
            plugin.publisher.bridge_for = lambda *args: None
            before = await plugin.scan(refresh_sources=False)
            self.assertEqual(len(before["installs"][0]["patches"]), 1)
            await plugin.set_game_source(9, "second", False)
            after = await plugin.scan(refresh_sources=False)
            row = after["installs"][0]
            self.assertEqual(row["patches"], [])
            self.assertTrue(row["sources"][0]["matched"])
            self.assertFalse(row["sources"][0]["subscribed"])
            self.assertTrue(after["sources"][0]["enabled"])
            self.assertTrue(after["installs"][1]["sources"][0]["subscribed"])
            with self.assertRaisesRegex(ValueError, "release_not_found"):
                await plugin._prepare_install("instance", "source-second-9")
            await plugin.set_game_source(9, "second", True)
            self.assertEqual(len((await plugin.scan(refresh_sources=False))["installs"][0]["patches"]), 1)
            plugin._prepare_cancel = Event()
            with self.assertRaisesRegex(RuntimeError, "installer_busy"):
                await plugin.set_game_source(9, "second", False)


if __name__ == "__main__":
    unittest.main()
