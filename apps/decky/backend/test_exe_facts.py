import binascii
import copy
import hashlib
from pathlib import Path
import tempfile
from threading import Event
import unittest

from exe_facts import game_facts, keyvalues, MAX_MANIFEST, read_file
from test_exe_profiles import profile_fixture


def steam_fixture(root, profile=None):
    profile = profile or profile_fixture()
    library = root / "library"
    game_root = library / "steamapps" / "common" / "Synthetic"
    game_root.mkdir(parents=True)
    (game_root / "Game.exe").write_bytes(b"game")
    (game_root / "Script.vdf").write_bytes(b"script")
    manifest = library / "steamapps" / f"appmanifest_{profile['appid']}.acf"
    manifest.write_text('"AppState" { "appid" "42" "installdir" "Synthetic" '
                        '"buildid" "100" "StateFlags" "4" '
                        '"UserConfig" { "language" "schinese" } "InstalledDepots" {} }', encoding="utf-8")
    game = {"appid": profile["appid"], "name": "Synthetic", "library_path": str(library),
            "install_path": str(game_root), "instance_id": f"42|{library}|{game_root}",
            "buildid": "100", "availability": "installed"}
    return game, manifest


class ExeFactsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.profile = profile_fixture()
        self.game, self.manifest = steam_fixture(self.root, self.profile)

    def change(self, before, after):
        self.manifest.write_text(self.manifest.read_text(encoding="utf-8").replace(before, after), encoding="utf-8")

    def test_manifest_baselines_and_language_are_bound_without_purchase_claim(self):
        facts = game_facts(self.game, self.profile)
        self.assertEqual(facts["buildid"], "100")
        self.assertEqual(facts["language"], "schinese")
        self.assertEqual(facts["installed_dlc"], [])
        self.assertEqual(facts["manifest_sha256"], hashlib.sha256(self.manifest.read_bytes()).hexdigest())
        self.assertEqual(facts["baselines"][1]["crc32"], binascii.crc32(b"script"))
        self.assertNotIn("owned_dlc", facts)

    def test_sha_baseline_and_optional_crc_are_both_required(self):
        self.profile["baselines"][1]["crc32"] = 1
        with self.assertRaisesRegex(ValueError, "exe_baseline_mismatch"):
            game_facts(self.game, self.profile)
        self.profile["baselines"][1]["crc32"] = binascii.crc32(b"script")
        game_facts(self.game, self.profile)
        (Path(self.game["install_path"]) / "Game.exe").write_bytes(b"different game")
        with self.assertRaisesRegex(ValueError, "exe_baseline_mismatch"):
            game_facts(self.game, self.profile)

    def test_current_build_cannot_auto_authorize_scanner_or_profile_drift(self):
        for game, profile in (({**self.game, "buildid": "101"}, self.profile),
                              (self.game, {**self.profile, "verified_buildids": ["101"]})):
            with self.subTest(game=game), self.assertRaisesRegex(ValueError, "build_not_verified"):
                game_facts(game, profile)
        self.change('"buildid" "100"', '"buildid" "101"')
        with self.assertRaisesRegex(ValueError, "build_not_verified"):
            game_facts(self.game, self.profile)

    def test_language_identity_and_downloading_refused(self):
        cases = [('"appid" "42"', '"appid" "43"', "exe_game_identity_changed"),
                 ('"installdir" "Synthetic"', '"installdir" "Other"', "exe_game_identity_changed"),
                 ('"language" "schinese"', '"language" "english"', "exe_language_mismatch"),
                 ('"StateFlags" "4"', '"StateFlags" "6"', "game_not_ready")]
        raw = self.manifest.read_text(encoding="utf-8")
        for before, after, code in cases:
            self.manifest.write_text(raw.replace(before, after), encoding="utf-8")
            with self.subTest(code=code), self.assertRaisesRegex(ValueError, code):
                game_facts(self.game, self.profile)
        self.manifest.write_text(raw, encoding="utf-8")
        downloading = Path(self.game["library_path"]) / "steamapps" / "downloading" / "42"
        downloading.mkdir(parents=True)
        with self.assertRaisesRegex(ValueError, "game_not_ready"):
            game_facts(self.game, self.profile)

    def test_exact_instance_id_and_directory_required(self):
        with self.assertRaisesRegex(ValueError, "exe_game_identity_changed"):
            game_facts({**self.game, "instance_id": self.game["instance_id"] + "|other"}, self.profile)
        self.change('"installdir" "Synthetic"', '"installdir" "../Synthetic"')
        with self.assertRaisesRegex(ValueError, "exe_manifest_invalid"):
            game_facts(self.game, self.profile)

    def test_dlc_installed_depot_conditions_are_enforced(self):
        self.change('"InstalledDepots" {}', '"InstalledDepots" { "123" { "dlcappid" "1390150" } }')
        with self.assertRaisesRegex(ValueError, "exe_dlc_excluded"):
            game_facts(self.game, self.profile)
        profile = copy.deepcopy(self.profile)
        profile["requires_dlc"], profile["excludes_dlc"] = [1390150], []
        self.assertEqual(game_facts(self.game, profile)["installed_dlc"], [1390150])
        profile["requires_dlc"] = [1390151]
        with self.assertRaisesRegex(ValueError, "exe_dlc_required"):
            game_facts(self.game, profile)

    def test_explicit_installed_dlc_and_ambiguous_language(self):
        self.change('"InstalledDepots" {}', '"InstalledDLC" "1390150"')
        with self.assertRaisesRegex(ValueError, "exe_dlc_excluded"):
            game_facts(self.game, self.profile)
        self.change('"InstalledDLC" "1390150"', '"MountedConfig" { "language" "english" }')
        with self.assertRaisesRegex(ValueError, "exe_language_mismatch"):
            game_facts(self.game, self.profile)

    def test_kv_rejects_duplicate_casefold_keys_truncation_directives_and_limit(self):
        bad = [b'"AppState" { "appid" "42" "APPID" "43" }', b'"AppState" {',
               b'"AppState" {} }', b'"AppState" {} #include "other"',
               b'"AppState" { "name" "bad\\escape" }', b'"AppState" { "name" "x" [linux] }',
               b'"AppState" {} "Other" {}', b'"AppState" { "bad" "\xff" }', b" " * (MAX_MANIFEST + 1)]
        for raw in bad:
            with self.subTest(raw=raw[:80]), self.assertRaises(ValueError):
                keyvalues(raw)
        self.assertEqual(keyvalues(b'// comment\n "AppState" { "name" "}" }'), {"name": "}"})
        with self.assertRaisesRegex(ValueError, "exe_manifest_limit"):
            keyvalues(b'"AppState" {' + b'"x" {' * 17 + b'}' * 18)

    def test_read_limits_and_cancel(self):
        with self.assertRaisesRegex(ValueError, "exe_game_file_limit"):
            read_file(self.manifest, 1)
        cancel = Event()
        cancel.set()
        with self.assertRaisesRegex(RuntimeError, "download_cancelled"):
            game_facts(self.game, self.profile, cancel)

    def test_linked_game_baseline_rejected_if_platform_supports_links(self):
        path = Path(self.game["install_path"]) / "Game.exe"
        outside = self.root / "outside"
        outside.write_bytes(b"game")
        path.unlink()
        try:
            path.symlink_to(outside)
        except OSError:
            self.skipTest("OS does not permit symlink creation")
        with self.assertRaisesRegex(ValueError, "exe_linked_game_file"):
            game_facts(self.game, self.profile)


if __name__ == "__main__":
    unittest.main()
