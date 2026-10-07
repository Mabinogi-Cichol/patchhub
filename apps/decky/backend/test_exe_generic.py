import binascii
import copy
import json
from pathlib import Path
import tempfile
from threading import Event
import types
import unittest

from exe_generic import candidate, facts, placeholder, selection_id
from exe_profiles import ExeProfiles, digest
from test_exe_facts import steam_fixture
from test_exe_profiles import candidate_fixture, profile_fixture


class GenericFactsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.game, self.manifest = steam_fixture(self.root)
        self.source = candidate_fixture(profile_fixture())
        rule = {"id": "kagura-cn", "role": "publisher", "enabled": True,
                "page_hosts": ["kagurafan.com"], "download_hosts": ["kaguraserver.com"]}
        self.sources = types.SimpleNamespace(_rules=[rule], builtins=[copy.deepcopy(rule)],
            game_subscribed=lambda appid, source: True, cached_candidates=lambda game: [self.source])
        registry = self.root / "profiles.json"
        registry.write_text(json.dumps({"schema_version": 1, "profiles": []}), encoding="utf-8")
        self.registry = ExeProfiles(registry, self.sources, pipeline_ready=True)
        behavior = {"condition": ["symbol", "and", ["symbol", "exists", "Game.exe"],
                    ["symbol", "==", ["symbol", "crc32", "Script.vdf"], binascii.crc32(b"script")]]}
        self.profile = dict(placeholder(self.game, self.source), behavior=behavior, behavior_sha256=digest(behavior))

    def test_unregistered_game_is_bound_to_actual_predicates_and_snapshot(self):
        selected = selection_id(self.source)
        self.assertEqual(candidate(self.registry, self.game, selected), self.source)
        result = facts(self.game, self.profile)
        self.assertEqual([row["path"] for row in result["baselines"]], ["Game.exe", "Script.vdf"])
        self.assertEqual(result["behavior_sha256"], self.profile["behavior_sha256"])
        self.assertNotIn("review", self.profile)
        (Path(self.game["install_path"]) / "Script.vdf").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "exe_installer_condition_mismatch"):
            facts(self.game, self.profile)

    def test_manifest_build_change_is_a_snapshot_change_not_a_whitelist(self):
        self.manifest.write_text(self.manifest.read_text(encoding="utf-8").replace('"100"', '"101"'), encoding="utf-8")
        current = {**self.game, "buildid": "101"}
        fresh = dict(self.profile, verified_buildids=["101"])
        self.assertEqual(facts(current, fresh)["buildid"], "101")
        with self.assertRaisesRegex(ValueError, "build_not_verified"):
            facts(current, self.profile)

    def test_language_and_dlc_constraints_are_still_checked(self):
        with self.assertRaisesRegex(ValueError, "exe_dlc_required"):
            facts(self.game, {**self.profile, "requires_dlc": [99]})
        self.manifest.write_text(self.manifest.read_text(encoding="utf-8").replace('"schinese"', '"english"'), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "exe_language_mismatch"):
            facts(self.game, self.profile)

    def test_boolean_short_circuit_and_missing_crc_do_not_invent_values(self):
        bad_crc = ["symbol", "~=", ["symbol", "crc32", "missing.file"], 123]
        profile = {**self.profile, "behavior": {"condition": ["symbol", "or", True, bad_crc]}}
        self.assertEqual(facts(self.game, profile)["baselines"], [])
        with self.assertRaisesRegex(ValueError, "exe_installer_condition_mismatch"):
            facts(self.game, {**profile, "behavior": {"condition": bad_crc}})
        with self.assertRaisesRegex(ValueError, "exe_condition_ir_invalid"):
            facts(self.game, {**profile, "behavior": {"condition": ["symbol", "not", ["symbol", "crc32", "Script.vdf"]]}})

    def test_source_identity_and_cancellation_remain_required(self):
        selected = selection_id(self.source)
        self.source["match_basis"] = "title"
        with self.assertRaisesRegex(ValueError, "exe_game_identity_unverified"):
            candidate(self.registry, self.game, selected)
        stopped = Event(); stopped.set()
        with self.assertRaises(RuntimeError): facts(self.game, self.profile, stopped)


if __name__ == "__main__": unittest.main()
