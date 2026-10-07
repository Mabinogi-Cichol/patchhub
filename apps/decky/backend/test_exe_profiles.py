import copy
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import tempfile
import time
import types
import unittest

from exe_profiles import ExeProfiles, candidate_binding, release_id, validate_profile


def profile_fixture():
    sha = lambda data: hashlib.sha256(data).hexdigest()
    return {"profile_id": "synthetic-base", "revision": 1, "source_id": "kagura-cn",
        "source_record_id": "synthetic-official-record", "source_page": "https://kagurafan.com/synthetic/",
        "appid": 42, "language": "zh-CN", "variant": "base", "version_identity": "1.03",
        "exe_sha256": sha(b"synthetic-exe"), "exe_size": 13, "metadata_sha256": sha(b"metadata"),
        "installer_version": "9.5", "compression": "lzma2",
        "payload": [{"path": "www/data.bin", "size": 7, "sha256": sha(b"patched")}],
        "verified_buildids": ["100"], "requires_dlc": [], "excludes_dlc": [1390150],
        "baselines": [{"path": "Game.exe", "sha256": sha(b"game"), "crc32": None},
                      {"path": "Script.vdf", "sha256": sha(b"script"), "crc32": None}],
        "terms": {"url": "https://kagurafan.com/synthetic/", "text": "Synthetic terms", "sha256": sha(b"Synthetic terms")},
        "review": {"semantics": True, "steamos": True, "evidence": "Synthetic test only, never ship this profile"},
        "download_hosts": ["kaguraserver.com"]}


def candidate_fixture(profile):
    return {"candidate_id": "candidate-" + "a" * 24, "source_id": profile["source_id"],
        "source_record_id": profile["source_record_id"], "game_appid": profile["appid"],
        "languages": [profile["language"]], "variant": profile["variant"],
        "version_identity": profile["version_identity"], "source_page": profile["source_page"],
        "requires_dlc": profile["requires_dlc"], "excludes_dlc": profile["excludes_dlc"],
        "mirrors": [{"url": "https://kaguraserver.com/synthetic.exe", "format": "exe"}],
        "match_basis": "appid", "capability": "discover_only", "format_reported": "exe",
        "observed_at": time.time(), "cache_status": "fresh"}


class ExeProfilesTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / "profiles.json"
        self.profile = profile_fixture()
        self.candidate = candidate_fixture(self.profile)
        self.game = {"appid": 42, "buildid": "100", "availability": "installed"}
        rule = {"id": "kagura-cn", "role": "publisher", "enabled": True,
                "page_hosts": ["kagurafan.com"], "download_hosts": ["kaguraserver.com"]}
        self.subscribed = True
        self.sources = types.SimpleNamespace(_rules=[rule], builtins=[copy.deepcopy(rule)],
            game_subscribed=lambda appid, source: self.subscribed,
            cached_candidates=lambda game: [self.candidate])
        self.write([self.profile])
        self.registry = ExeProfiles(self.path, self.sources)

    def write(self, profiles):
        self.path.write_text(json.dumps({"schema_version": 1, "profiles": profiles}), encoding="utf-8")

    def test_reviewed_profile_is_distinct_from_candidate_and_pipeline_is_guarded(self):
        selection = release_id(self.profile)
        self.assertTrue(selection.startswith("exe-"))
        self.assertTrue(selection.endswith("-42"))
        found, candidate = self.registry.resolve(self.game, selection)
        self.assertEqual(found, self.profile)
        self.assertEqual(candidate["capability"], "discover_only")
        self.assertEqual(self.registry.reviews(self.game)[0]["blockers"], ["exe_pipeline_unavailable"])
        self.assertEqual(self.candidate["capability"], "discover_only")

    def test_candidate_identity_changes_do_not_match_by_title_or_extension(self):
        for key, value in (("source_record_id", "different"), ("source_id", "hgamer"), ("game_appid", 43),
                ("languages", ["en"]), ("variant", "dlc"), ("version_identity", "new-version"),
                ("source_page", "https://kagurafan.com/other/"), ("excludes_dlc", []), ("match_basis", "title"),
                ("capability", "installable")):
            previous = self.candidate
            self.candidate = {**previous, key: value}
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "exe_candidate_changed"):
                self.registry.resolve(self.game, release_id(self.profile))
            self.candidate = previous

    def test_settings_and_subscription_are_revalidated(self):
        self.subscribed = False
        with self.assertRaisesRegex(ValueError, "exe_source_disabled"):
            self.registry.resolve(self.game, release_id(self.profile))
        self.subscribed = True
        self.sources._rules[0]["enabled"] = False
        with self.assertRaisesRegex(ValueError, "exe_source_disabled"):
            self.registry.resolve(self.game, release_id(self.profile))

    def test_fallback_cannot_widen_mirrors_or_download_whitelist(self):
        self.candidate["mirrors"] = [{"url": "https://hgamer.tw/x.exe", "format": "exe"},
                                      {"url": "https://kaguraserver.com/x.exe?next=evil", "format": "exe"}]
        with self.assertRaisesRegex(ValueError, "exe_no_direct_download"):
            self.registry.resolve(self.game, release_id(self.profile))
        self.sources._rules[0]["download_hosts"].append("evil.example")
        with self.assertRaisesRegex(ValueError, "exe_source_changed"):
            self.registry.resolve(self.game, release_id(self.profile))

    def test_stale_future_and_missing_observation_are_rejected(self):
        for observed in (0, time.time() + 3600, None, "today", True):
            self.candidate["observed_at"] = observed
            with self.subTest(observed=observed), self.assertRaisesRegex(ValueError, "exe_candidate_stale"):
                self.registry.resolve(self.game, release_id(self.profile))

    def test_real_discovery_iso_observation_is_accepted_without_naive_time_guess(self):
        self.candidate["observed_at"] = datetime.now(timezone.utc).isoformat()
        self.registry.resolve(self.game, release_id(self.profile))
        self.candidate["observed_at"] = datetime.now().isoformat()
        with self.assertRaisesRegex(ValueError, "exe_candidate_stale"):
            self.registry.resolve(self.game, release_id(self.profile))

    def test_reviews_accept_observed_exe_but_reject_format_conflicts(self):
        self.candidate["format_reported"] = "unknown"
        self.candidate["format_observed"] = "exe"
        self.assertEqual(len(self.registry.reviews(self.game)), 1)
        self.candidate["format_observed"] = "zip"
        self.assertEqual(self.registry.reviews(self.game), [])
        self.candidate["format_observed"] = "exe"
        self.candidate["format_reported"] = "zip"
        self.assertEqual(self.registry.reviews(self.game), [])

    def test_unreviewed_or_changed_profile_invalidates_old_release(self):
        selection = release_id(self.profile)
        self.profile["review"]["semantics"] = False
        self.write([self.profile])
        with self.assertRaisesRegex(ValueError, "exe_profile_changed"):
            self.registry.resolve(self.game, selection)
        with self.assertRaisesRegex(ValueError, "exe_profile_not_reviewed"):
            self.registry.resolve(self.game, release_id(self.profile))
        self.write([])
        with self.assertRaisesRegex(ValueError, "exe_profile_changed"):
            self.registry.resolve(self.game, release_id(self.profile))

    def test_build_not_auto_verified_and_receipt_binding_changes(self):
        self.game["buildid"] = "101"
        with self.assertRaisesRegex(ValueError, "build_not_verified"):
            self.registry.resolve(self.game, release_id(self.profile))
        before = candidate_binding(self.candidate)
        self.candidate["mirrors"].append({"url": "https://kaguraserver.com/b.exe", "format": "exe"})
        self.assertNotEqual(before, candidate_binding(self.candidate))
        self.candidate["observed_at"] += 5
        self.assertEqual(candidate_binding(self.candidate), candidate_binding(dict(self.candidate)))

    def test_old_build_coverage_falls_back_to_fresh_behavior_check(self):
        from exe_generic import selection_id
        self.registry.pipeline_ready = True
        self.game.update(buildid="101", instance_id="instance-test")
        review = self.registry.reviews(self.game)[0]
        self.assertEqual(review["release_id"], selection_id(self.candidate))
        self.assertEqual(review["status"], "check_required")
        self.assertEqual(review["blockers"], [])
        self.assertIsNone(review["profile_id"])
        with self.assertRaisesRegex(ValueError, "build_not_verified"):
            self.registry.resolve(self.game, release_id(self.profile))
        self.subscribed = False
        self.assertEqual(self.registry.reviews(self.game)[0]["blockers"], ["exe_source_disabled"])

    def test_untrusted_selection_and_schema_cannot_enable_profiles(self):
        for selection in (self.candidate["candidate_id"], "https://kaguraserver.com/x.exe", "../profile", {}, None):
            with self.subTest(selection=selection), self.assertRaisesRegex(ValueError, "exe_release_invalid"):
                self.registry.resolve(self.game, selection)
        for change in ({"exe_size": True}, {"appid": True}, {"extra": "unknown"},
                {"payload": [{"path": "../file", "size": 1, "sha256": "a" * 64}]},
                {"download_hosts": ["hgamer.tw"]}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                validate_profile({**profile_fixture(), **change})

    def test_production_registry_contains_only_reviewed_profiles(self):
        shipped = json.loads(Path(__file__).with_name("exe_profiles.json").read_text(encoding="utf-8"))
        self.assertEqual(shipped["schema_version"], 1)
        for profile in shipped["profiles"]:
            validate_profile(profile)
            self.assertTrue(profile["review"]["semantics"] and profile["review"]["steamos"])

    def test_baselines_are_profile_declared_and_limits_cover_large_members(self):
        custom = {**profile_fixture(),
                  "baselines": [{"path": "VBFI.exe", "sha256": "b" * 64, "crc32": 3769231163}],
                  "exe_size": 1126694464,
                  "payload": [{"path": "extra.xp3", "size": 648789888, "sha256": "c" * 64}]}
        self.assertEqual(validate_profile(custom)["baselines"][0]["path"], "VBFI.exe")
        for change in ({"baselines": []},
                       {"exe_size": 2 * 1024 ** 3 + 1},
                       {"payload": [{"path": "extra.xp3", "size": 1024 ** 3 + 1, "sha256": "c" * 64}]}):
            with self.subTest(change=list(change)), self.assertRaises(ValueError):
                validate_profile({**custom, **change})


if __name__ == "__main__":
    unittest.main()
