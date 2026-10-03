import json
from pathlib import Path
import tempfile
import unittest

from plugin_settings import Settings


class SettingsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def test_defaults_auto_cleanup_on_and_backups_off(self):
        settings = Settings(self.root)
        self.assertTrue(settings.auto_cleanup)
        self.assertFalse(settings.keep_backups)
        self.assertEqual(settings.snapshot(),
                         {"schema_version": 1, "auto_cleanup": True, "keep_backups": False})
        self.assertEqual(settings.diagnostics, [])

    def test_round_trip_persists_and_reloads(self):
        settings = Settings(self.root)
        settings.set_auto_cleanup(False)
        settings.set_keep_backups(True)
        data = json.loads((self.root / "settings.json").read_text(encoding="utf-8"))
        self.assertEqual(data, {"schema_version": 1, "auto_cleanup": False, "keep_backups": True})
        reloaded = Settings(self.root)
        self.assertFalse(reloaded.auto_cleanup)
        self.assertTrue(reloaded.keep_backups)
        self.assertEqual(reloaded.diagnostics, [])

    def test_partial_or_empty_config_uses_per_field_defaults(self):
        (self.root / "settings.json").write_text(json.dumps({}), encoding="utf-8")
        settings = Settings(self.root)
        self.assertTrue(settings.auto_cleanup)
        self.assertFalse(settings.keep_backups)
        self.assertEqual(settings.diagnostics, [])
        (self.root / "settings.json").write_text(json.dumps({"keep_backups": True}), encoding="utf-8")
        settings = Settings(self.root)
        self.assertTrue(settings.auto_cleanup)
        self.assertTrue(settings.keep_backups)

    def test_malformed_or_wrong_types_fall_back_to_defaults(self):
        for content in ("{not json", json.dumps({"auto_cleanup": "yes"}),
                        json.dumps({"keep_backups": 1})):
            (self.root / "settings.json").write_text(content, encoding="utf-8")
            settings = Settings(self.root)
            self.assertTrue(settings.auto_cleanup)
            self.assertFalse(settings.keep_backups)
            self.assertEqual(settings.diagnostics[0]["code"], "settings_unavailable")

    def test_rejects_non_boolean_set(self):
        settings = Settings(self.root)
        with self.assertRaisesRegex(ValueError, "invalid_settings"):
            settings.set_auto_cleanup("yes")
        with self.assertRaisesRegex(ValueError, "invalid_settings"):
            settings.set_keep_backups(1)


if __name__ == "__main__":
    unittest.main()
