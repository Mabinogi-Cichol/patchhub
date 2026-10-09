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
    def test_default_and_round_trip(self):
        settings = Settings(self.root)
        self.assertEqual(settings.snapshot(), {"schema_version": 4, "auto_cleanup": True})
        settings.set_auto_cleanup(False)
        self.assertFalse(Settings(self.root).auto_cleanup)
    def test_old_config_migrates_preserving_download_preference(self):
        path = self.root / "settings.json"
        path.write_text(json.dumps({"schema_version":1,"auto_cleanup":False,"keep_backups":True}))
        settings = Settings(self.root)
        self.assertEqual(settings.snapshot(), {"schema_version":4,"auto_cleanup":False})
        self.assertEqual(json.loads(path.read_text()), settings.snapshot())
        self.assertEqual(Settings(self.root).snapshot(), settings.snapshot())
    def test_engine_selector_is_not_persisted(self):
        path = self.root / "settings.json"
        path.write_text(json.dumps({"schema_version": 3, "auto_cleanup": False, "source_engine": "python"}))
        settings = Settings(self.root)
        self.assertEqual(settings.snapshot(), {"schema_version": 4, "auto_cleanup": False})
        self.assertEqual(json.loads(path.read_text()), settings.snapshot())
        self.assertFalse(hasattr(settings, "set_source_engine"))

    def test_invalid_config_and_non_boolean_set(self):
        for content in ("broken",json.dumps({"auto_cleanup":"yes"})):
            (self.root / "settings.json").write_text(content)
            settings = Settings(self.root)
            self.assertTrue(settings.auto_cleanup)
            self.assertEqual(settings.diagnostics[0]["code"],"settings_unavailable")
        with self.assertRaisesRegex(ValueError,"invalid_settings"):
            Settings(self.root).set_auto_cleanup(1)
