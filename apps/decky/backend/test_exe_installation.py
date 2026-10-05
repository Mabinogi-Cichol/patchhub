import asyncio
import copy
import hashlib
import json
from pathlib import Path
import tempfile
from threading import Event
import time
import types
import unittest
from unittest.mock import patch
import uuid
import zipfile

from exe_installation import ExeInstallation, _space
from exe_profiles import ExeProfiles, release_id
from test_exe_facts import steam_fixture
from test_exe_profiles import candidate_fixture, profile_fixture

SHA = lambda b: hashlib.sha256(b).hexdigest()
ACCEPT = {"terms_accepted": True, "keep_backups": True, "conditions_accepted": True}


class FakeBridge:
    def __init__(self, state):
        self.state_dir = state
        self.transactions = []
        self.plans = []
        self.discarded = []
        self.on_plan = None

    async def list_transactions(self):
        return self.transactions

    async def plan_install(self, instance, release, archive):
        plan_id = str(uuid.uuid4())
        (self.state_dir / "jobs" / plan_id).mkdir(parents=True)
        plan = {"plan_id": plan_id, "expires_at": int(time.time()) + 900,
                "instance_id": instance, "release_id": release}
        self.plans.append((plan, Path(archive)))
        self.transactions.append({"transaction_id": plan_id, "plan_id": plan_id, "state": "planned",
            "instance_id": instance, "release_id": release, "created_at": int(time.time())})
        if self.on_plan:
            self.on_plan()
        return plan

    async def discard_plan(self, plan_id):
        self.discarded.append(plan_id)
        for transaction in self.transactions:
            if transaction.get("plan_id") == plan_id:
                transaction["state"] = "rolled_back"
        return {"plan_id": plan_id, "state": "rolled_back"}


class FakePublisher:
    def __init__(self, root):
        self.base = root / "state" / "publisher"
        self.bridge = types.SimpleNamespace(process_identity={})
        self.bridges_by_state = {}
        self.native_calls = []

    def _state(self, instance, release):
        return self.base / release / SHA(instance.encode())[:16]

    def _bridge(self, state):
        return self.bridges_by_state.setdefault(state, FakeBridge(state))

    async def _native(self, *args):
        self.native_calls.append(args)
        if args[0] == "keygen":
            Path(args[1]).write_bytes(b"private test key")
            Path(args[2]).write_text('{"keys": []}')
        elif args[0] == "sign-catalog":
            Path(args[4]).write_text('{"synthetic_signature": true}')
        else:
            raise AssertionError(args)


class ExeInstallationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.profile = profile_fixture()
        self.candidate = candidate_fixture(self.profile)
        self.game, self.manifest = steam_fixture(self.root, self.profile)
        self.registry_path = self.root / "profiles.json"
        self.write_profiles([self.profile])
        self.subscribed = True
        rule = {"id": "kagura-cn", "role": "publisher", "enabled": True,
                "page_hosts": ["kagurafan.com"], "download_hosts": ["kaguraserver.com"]}
        sources = types.SimpleNamespace(_rules=[rule], builtins=[copy.deepcopy(rule)],
            game_subscribed=lambda appid, source: self.subscribed,
            cached_candidates=lambda game: [self.candidate])
        self.profiles = ExeProfiles(self.registry_path, sources, pipeline_ready=True)
        self.publisher = FakePublisher(self.root)
        self.service = ExeInstallation(self.publisher, self.profiles, self.root / "tools")
        self.selection = release_id(self.profile)
        self.state = self.publisher._state(self.game["instance_id"], self.selection)
        self.download_calls = []
        self.extract_calls = []
        self.download_patch = patch("exe_installation._download", self.download)
        self.extract_patch = patch("exe_installation.extract", self.extract)
        self.download_patch.start()
        self.extract_patch.start()
        self.addCleanup(self.download_patch.stop)
        self.addCleanup(self.extract_patch.stop)

    def write_profiles(self, profiles):
        self.registry_path.write_text(json.dumps({"schema_version": 1, "profiles": profiles}), encoding="utf-8")

    def download(self, url, cache, cancel, validate, **kwargs):
        validate(url)
        self.download_calls.append((url, kwargs))
        path = cache / kwargs["archive_name"]
        path.write_bytes(b"synthetic-exe")
        (cache / kwargs["manifest_name"]).write_text(json.dumps({"url": url, "final_url": url,
            "size": 13, "sha256": SHA(b"synthetic-exe")}))
        return path, SHA(b"synthetic-exe"), 13, "download"

    async def extract(self, bridge, tools, archive, output, profile, cancel):
        self.extract_calls.append((archive, output, profile))
        normalized = output / "normalized.zip"
        with zipfile.ZipFile(normalized, "w", compression=zipfile.ZIP_STORED) as z:
            z.writestr("www/data.bin", b"patched")
        return {"exe_sha256": profile["exe_sha256"], "metadata_sha256": profile["metadata_sha256"],
                "artifact_sha256": SHA(normalized.read_bytes()), "artifact_size": normalized.stat().st_size,
                "files": profile["payload"], "extractor": "synthetic-test-only"}

    async def prepare(self, confirmations=None, cancel=None):
        return await self.service.prepare(self.game, self.selection, cancel or Event(),
                                           ACCEPT if confirmations is None else confirmations)

    def unchanged_game(self):
        root = Path(self.game["install_path"])
        self.assertEqual((root / "Game.exe").read_bytes(), b"game")
        self.assertEqual((root / "Script.vdf").read_bytes(), b"script")
        self.assertFalse((root / "www").exists())

    async def test_prepare_signs_full_zip_recipe_preserves_hash_chain_and_no_game_writes(self):
        bridge, archive, plan, acquisition = await self.prepare()
        self.assertEqual(Path(archive), self.state / "cache" / "normalized.zip")
        self.assertEqual(acquisition, "download")
        self.assertEqual(self.download_calls[0][1]["expected_sha256"], self.profile["exe_sha256"])
        self.assertEqual(self.download_calls[0][1]["expected_size"], self.profile["exe_size"])
        catalog = json.loads((self.state / "catalog.json").read_text(encoding="utf-8"))
        release = catalog["releases"][0]
        self.assertEqual(release["support_level"], "managed_zip")
        self.assertEqual(release["platforms"], ["steamos"])
        self.assertEqual(release["verified_buildids"], ["100"])
        self.assertEqual(release["artifact"]["url"], self.candidate["mirrors"][0]["url"])
        self.assertEqual(release["recipe"]["files"], [{"source": "www/data.bin", "destination": "www/data.bin",
            "original_sha256": None, "installed_sha256": SHA(b"patched")}])
        receipt = self.service.plan_evidence(self.state, plan["plan_id"])
        self.assertEqual(receipt["exe_sha256"], SHA(b"synthetic-exe"))
        self.assertEqual(receipt["normalized_sha256"], SHA(Path(archive).read_bytes()))
        self.assertNotEqual(receipt["exe_sha256"], receipt["normalized_sha256"])
        self.assertTrue(receipt["keep_backups"] and receipt["lease_active"])
        self.assertEqual(receipt["terms_sha256"], self.profile["terms"]["sha256"])
        self.assertTrue(receipt["condition_declaration"])
        self.assertFalse(list((self.state / "exe-work").iterdir()))
        self.assertEqual(len(bridge.plans), 1)
        self.unchanged_game()
        await self.service.validate_apply(self.game, self.selection, plan["plan_id"])

    async def test_matches_ui_contract_and_empty_registry_fail_closed(self):
        matches = self.service.matches(self.game)
        self.assertEqual(matches[0]["support_level"], "publisher_exe")
        self.assertEqual(matches[0]["exe"]["terms"], self.profile["terms"])
        self.assertTrue(matches[0]["exe"]["backup_required"])
        self.write_profiles([])
        self.assertEqual(self.service.matches(self.game), [])
        with self.assertRaisesRegex(ValueError, "exe_profile_changed"):
            await self.prepare()
        self.assertFalse(self.download_calls)

    async def test_confirmation_booleans_and_conditions_required_before_download(self):
        for missing, code in (("terms_accepted", "exe_terms_confirmation_required"),
                              ("keep_backups", "exe_backups_required"),
                              ("conditions_accepted", "exe_conditions_confirmation_required")):
            with self.subTest(missing=missing), self.assertRaisesRegex(ValueError, code):
                await self.prepare({**ACCEPT, missing: False})
        for confirmations in ({**ACCEPT, "url": "https://example.com/x.exe"},
                              {**ACCEPT, "keep_backups": 1}, {**ACCEPT, "terms_accepted": "yes"}):
            with self.subTest(confirmations=confirmations), self.assertRaisesRegex(ValueError, "exe_confirmation_invalid"):
                await self.prepare(confirmations)
        self.assertFalse(self.download_calls)

    async def test_apply_revalidates_source_subscription_profile_candidate_and_current_facts(self):
        _, _, plan, _ = await self.prepare()
        self.subscribed = False
        with self.assertRaisesRegex(ValueError, "exe_source_disabled"):
            await self.service.validate_apply(self.game, self.selection, plan["plan_id"])
        self.subscribed = True
        self.candidate["mirrors"].append({"url": "https://kaguraserver.com/another.exe", "format": "exe"})
        with self.assertRaisesRegex(ValueError, "exe_candidate_changed"):
            await self.service.validate_apply(self.game, self.selection, plan["plan_id"])
        self.candidate["mirrors"].pop()
        self.manifest.write_text(self.manifest.read_text() + "\n// Steam manifest changed\n")
        with self.assertRaisesRegex(ValueError, "exe_game_facts_changed"):
            await self.service.validate_apply(self.game, self.selection, plan["plan_id"])
        self.write_profiles([])
        with self.assertRaisesRegex(ValueError, "exe_profile_changed"):
            await self.service.validate_apply(self.game, self.selection, plan["plan_id"])
        # Offline restore metadata survives disable and needs no remote lookup.
        self.assertTrue(self.service.plan_evidence(self.state, plan["plan_id"])["keep_backups"])
        self.service.release_lease(self.state, plan["plan_id"])
        self.assertFalse(self.service.plan_evidence(self.state, plan["plan_id"])["lease_active"])

    async def test_apply_restart_uses_persisted_receipt_and_blocks_missing_or_changed_confirmations(self):
        _, _, plan, _ = await self.prepare()
        restarted = ExeInstallation(self.publisher, self.profiles, self.root / "tools")
        await restarted.validate_apply(self.game, self.selection, plan["plan_id"])
        with self.assertRaisesRegex(ValueError, "exe_backups_required"):
            await restarted.validate_apply(self.game, self.selection, plan["plan_id"], {**ACCEPT, "keep_backups": False})
        (self.state / "jobs" / plan["plan_id"] / "exe-policy.json").unlink()
        with self.assertRaisesRegex(ValueError, "exe_policy_unavailable"):
            await restarted.validate_apply(self.game, self.selection, plan["plan_id"])

    async def test_active_transaction_prevents_catalog_key_instance_and_download_changes(self):
        self.state.mkdir(parents=True)
        (self.state / "catalog.json").write_bytes(b"existing catalog")
        (self.state / "private.key").write_bytes(b"existing private")
        (self.state / "public.json").write_bytes(b"existing public")
        for status in ("planned", "committed", "backing_up", "needs_recovery"):
            self.publisher._bridge(self.state).transactions = [{"state": status}]
            with self.subTest(status=status), self.assertRaisesRegex(RuntimeError, "publisher_existing_transaction"):
                await self.prepare()
        self.assertEqual((self.state / "catalog.json").read_bytes(), b"existing catalog")
        self.assertEqual((self.state / "private.key").read_bytes(), b"existing private")
        self.assertFalse((self.state / "instance.json").exists())
        self.assertFalse(self.download_calls)

    async def test_cancel_after_plan_discards_only_created_plan_and_normalized_cache(self):
        cancel = Event()
        self.publisher._bridge(self.state).on_plan = cancel.set
        with self.assertRaisesRegex(RuntimeError, "download_cancelled"):
            await self.prepare(cancel=cancel)
        bridge = self.publisher._bridge(self.state)
        self.assertEqual(bridge.discarded, [bridge.plans[0][0]["plan_id"]])
        self.assertFalse((self.state / "cache" / "normalized.zip").exists())
        self.assertFalse(list((self.state / "exe-work").iterdir()))
        self.unchanged_game()

    async def test_source_changed_after_plan_discards_and_releases_policy(self):
        self.publisher._bridge(self.state).on_plan = lambda: setattr(self, "subscribed", False)
        with self.assertRaisesRegex(ValueError, "exe_source_disabled"):
            await self.prepare()
        bridge = self.publisher._bridge(self.state)
        plan_id = bridge.plans[0][0]["plan_id"]
        self.assertEqual(bridge.discarded, [plan_id])
        self.assertFalse(self.service.plan_evidence(self.state, plan_id)["lease_active"])
        self.unchanged_game()

    async def test_invalid_worker_hash_or_unreviewed_zip_member_rejected_without_plan(self):
        real_extract = self.extract
        async def bad_hash(*args):
            report = await real_extract(*args)
            report["artifact_sha256"] = "0" * 64
            return report
        with patch("exe_installation.extract", bad_hash), self.assertRaisesRegex(ValueError, "exe_worker_evidence_changed"):
            await self.prepare()
        async def bad_member(*args):
            report = await real_extract(*args)
            path = args[3] / "normalized.zip"
            with zipfile.ZipFile(path, "a") as archive:
                archive.writestr("extra.bin", b"unreviewed")
            report["artifact_sha256"], report["artifact_size"] = SHA(path.read_bytes()), path.stat().st_size
            return report
        with patch("exe_installation.extract", bad_member), self.assertRaisesRegex(ValueError, "exe_manifest_changed"):
            await self.prepare()
        self.assertFalse(self.publisher._bridge(self.state).plans)
        self.unchanged_game()

    async def test_downloader_changed_bytes_rejected_before_extraction(self):
        real_download = self.download
        def bad_download(*args, **kwargs):
            result = real_download(*args, **kwargs)
            result[0].write_bytes(b"attacker-exe")
            return result
        with patch("exe_installation._download", bad_download), self.assertRaisesRegex(ValueError, "exe_payload_changed"):
            await self.prepare()
        self.assertFalse(self.extract_calls)

    async def test_state_and_target_space_fail_before_download(self):
        for location, code in (("state", "exe_state_insufficient_space"), ("target", "exe_target_insufficient_space")):
            def usage(path):
                insufficient = (Path(path) == Path(self.game["install_path"])) if location == "target" else True
                return types.SimpleNamespace(free=0 if insufficient else 100 * 1024 ** 3)
            with self.subTest(location=location), patch("exe_installation.shutil.disk_usage", usage), self.assertRaisesRegex(ValueError, code):
                await self.prepare()
        self.assertFalse(self.download_calls)

    async def test_baseline_and_dlc_invalid_never_download(self):
        (Path(self.game["install_path"]) / "Game.exe").write_bytes(b"different")
        with self.assertRaisesRegex(ValueError, "exe_baseline_mismatch"):
            await self.prepare()
        self.assertFalse(self.download_calls)

    async def test_catalog_binding_and_lease_expiry_fail_closed(self):
        _, _, plan, _ = await self.prepare()
        (self.state / "catalog.json").write_text((self.state / "catalog.json").read_text(encoding="utf-8") + " ", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "exe_local_authorization_changed"):
            await self.service.validate_apply(self.game, self.selection, plan["plan_id"])
        self.service.release_lease(self.state, plan["plan_id"])
        with self.assertRaisesRegex(ValueError, "plan_expired"):
            await self.service.validate_apply(self.game, self.selection, plan["plan_id"])

    async def test_policy_path_and_link_refused(self):
        with self.assertRaisesRegex(ValueError, "invalid_id"):
            self.service.plan_evidence(self.state, "../../other")
        _, _, plan, _ = await self.prepare()
        path = self.state / "jobs" / plan["plan_id"] / "exe-policy.json"
        outside = self.root / "outside-policy.json"
        outside.write_bytes(path.read_bytes())
        path.unlink()
        try:
            path.symlink_to(outside)
        except OSError:
            self.skipTest("OS does not permit symlink creation")
        with self.assertRaisesRegex(ValueError, "exe_policy_unavailable"):
            self.service.plan_evidence(self.state, plan["plan_id"])

    async def test_expired_planned_receipt_allows_reprepare_but_live_lease_blocks(self):
        _, _, plan, _ = await self.prepare()
        with self.assertRaisesRegex(RuntimeError, "publisher_existing_transaction"):
            await self.prepare()
        policy = self.state / "jobs" / plan["plan_id"] / "exe-policy.json"
        receipt = json.loads(policy.read_text())
        receipt["expires_at"] = int(time.time()) - 1
        policy.write_text(json.dumps(receipt))
        _, _, new, _ = await self.prepare()
        self.assertNotEqual(new["plan_id"], plan["plan_id"])
        self.assertEqual(self.publisher._bridge(self.state).discarded, [plan["plan_id"]])
        self.assertFalse(self.service.plan_evidence(self.state, plan["plan_id"])["lease_active"])

    async def test_missing_receipt_crash_blocks_until_core_max_ttl_then_sweeps(self):
        _, _, plan, _ = await self.prepare()
        policy = self.state / "jobs" / plan["plan_id"] / "exe-policy.json"
        policy.unlink()
        with self.assertRaisesRegex(RuntimeError, "publisher_existing_transaction"):
            await self.prepare()
        bridge = self.publisher._bridge(self.state)
        bridge.transactions[0]["created_at"] = int(time.time()) - 901
        _, _, new, _ = await self.prepare()
        self.assertNotEqual(new["plan_id"], plan["plan_id"])
        self.assertEqual(bridge.discarded, [plan["plan_id"]])

    async def test_released_receipt_reclaims_only_unexecuted_plan(self):
        _, _, plan, _ = await self.prepare()
        self.service.release_lease(self.state, plan["plan_id"])
        _, _, new, _ = await self.prepare()
        self.assertNotEqual(new["plan_id"], plan["plan_id"])
        bridge = self.publisher._bridge(self.state)
        self.service.release_lease(self.state, new["plan_id"])
        bridge.transactions[-1]["state"] = "backing_up"
        with self.assertRaisesRegex(RuntimeError, "publisher_existing_transaction"):
            await self.prepare()
        self.assertEqual(bridge.discarded, [plan["plan_id"]])


if __name__ == "__main__":
    unittest.main()
