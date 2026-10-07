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
        self.assertEqual(settings.snapshot(), {"schema_version": 3, "auto_cleanup": True, "source_engine": "rust"})
        settings.set_auto_cleanup(False)
        self.assertFalse(Settings(self.root).auto_cleanup)
    def test_old_config_migrates_preserving_download_preference(self):
        path = self.root / "settings.json"
        path.write_text(json.dumps({"schema_version":1,"auto_cleanup":False,"keep_backups":True}))
        settings = Settings(self.root)
        self.assertEqual(settings.snapshot(), {"schema_version":3,"auto_cleanup":False,"source_engine":"rust"})
        self.assertEqual(json.loads(path.read_text()), settings.snapshot())
        self.assertEqual(Settings(self.root).snapshot(), settings.snapshot())
    def test_schema_two_accepts_rust_default(self):
        path = self.root / "settings.json"
        path.write_text(json.dumps({"schema_version":2,"auto_cleanup":True}))
        settings = Settings(self.root)
        self.assertEqual(settings.source_engine, "rust")
        self.assertEqual(json.loads(path.read_text())["schema_version"], 3)
    def test_source_engine_accepts_rust_and_rejects_others(self):
        settings = Settings(self.root)
        settings.set_source_engine("rust")
        self.assertEqual(Settings(self.root).source_engine, "rust")
        for engine in ("python", "native", "bogus"):
            with self.assertRaisesRegex(ValueError, "invalid_settings"):
                settings.set_source_engine(engine)
        (self.root / "settings.json").write_text(json.dumps(
            {"schema_version":3,"auto_cleanup":True,"source_engine":"python"}))
        fallback = Settings(self.root)
        self.assertEqual(fallback.source_engine, "rust")
        self.assertEqual(fallback.diagnostics[0]["code"], "invalid_source_engine")
    def test_invalid_config_and_non_boolean_set(self):
        for content in ("broken",json.dumps({"auto_cleanup":"yes"})):
            (self.root / "settings.json").write_text(content)
            settings = Settings(self.root)
            self.assertTrue(settings.auto_cleanup)
            self.assertEqual(settings.diagnostics[0]["code"],"settings_unavailable")
        with self.assertRaisesRegex(ValueError,"invalid_settings"):
            Settings(self.root).set_auto_cleanup(1)
