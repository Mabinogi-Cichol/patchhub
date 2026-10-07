"""Offline checks for engine selection and switch checkpoints (Rust only)."""
import tempfile
from pathlib import Path
import types
import unittest

from source_engine import (
    aggregate_import_diagnostics, map_rust_settings, open_source_engine,
    read_engine_switch, select_engine, settings_hash, switch_path, write_engine_switch,
)
from test_source_service import FakeBridge, REQUIRED_METHODS, SUMMARIES


class EngineSelectionTests(unittest.TestCase):
    def test_only_rust_is_valid(self):
        python = types.SimpleNamespace(source_engine="python")
        self.assertEqual(select_engine(python, {}), "invalid")
        self.assertEqual(select_engine(python, {"STEAM_PATCH_SOURCE_ENGINE": "rust"}), "rust")
        self.assertEqual(select_engine(None, {}), "rust")
        self.assertEqual(select_engine(types.SimpleNamespace(source_engine="bogus"), {}), "invalid")
        self.assertEqual(select_engine(types.SimpleNamespace(source_engine="rust"), {}), "rust")
        self.assertEqual(select_engine(python, {"STEAM_PATCH_SOURCE_ENGINE": "bogus"}), "invalid")


class MappingTests(unittest.TestCase):
    def test_rust_settings_are_normalized(self):
        mapped = map_rust_settings({
            "settings_revision": 4,
            "enabled": {"patchmoe": False, "bad": "yes"},
            "added": [{"id": "custom"}],
            "games": {"42": {"patchmoe": True, "bad": 1}},
        })
        self.assertEqual(mapped, {"enabled": {"patchmoe": False}, "added": [{"id": "custom"}],
                                  "games": {"42": {"patchmoe": True}}})
        self.assertIsNone(map_rust_settings(None))

    def test_settings_hash_is_stable(self):
        mapping = {"enabled": {"patchmoe": True}, "added": [], "games": {"42": {"patchmoe": False}}}
        self.assertEqual(settings_hash(mapping), settings_hash(dict(reversed(list(mapping.items())))))
        self.assertEqual(settings_hash(None), "")

    def test_import_diagnostics_aggregate_counts_and_details(self):
        diagnostics = aggregate_import_diagnostics({
            "imported_settings": True, "imported_cache": 3, "imported_discovery": 0, "imported_index": 1,
            "diagnostics": [{"code": "invalid_source_cache", "source_id": "old"}]})
        self.assertEqual([d["code"] for d in diagnostics],
                         ["source_settings_imported", "source_cache_imported",
                          "source_index_imported", "invalid_source_cache"])
        self.assertEqual(diagnostics[1]["detail"], "3")
        self.assertEqual(aggregate_import_diagnostics(None), [])


class SwitchCheckpointTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def test_switch_file_round_trips_and_increments(self):
        self.assertIsNone(read_engine_switch(self.root))
        write_engine_switch(self.root, {"schema_version": 1, "active_engine": "rust",
                                        "switch_seq": 1, "rust": {}, "last_common": {}})
        self.assertEqual(read_engine_switch(self.root)["switch_seq"], 1)
        self.assertTrue(switch_path(self.root).is_file())


class OpenEngineTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def capabilities(self, methods):
        return {"methods": list(methods)}

    def rust_bridge(self):
        bridge = FakeBridge()
        bridge.sync_results = {
            "get_capabilities": self.capabilities(sorted(REQUIRED_METHODS)),
            "list_sources": [dict(item) for item in SUMMARIES],
            "get_source_state": {"settings_revision": 5, "import_report": None},
            "get_source_settings": {"enabled": {"patchmoe": True}, "added": [], "games": {}},
        }
        return bridge

    async def test_rust_success_writes_checkpoint(self):
        bridge = self.rust_bridge()
        settings = types.SimpleNamespace(source_engine="rust")
        service = await open_source_engine(self.root, bridge, settings)
        self.assertEqual(service.engine, "rust")
        self.assertEqual([d["code"] for d in service.diagnostics], [])
        record = read_engine_switch(self.root)
        self.assertEqual(record["active_engine"], "rust")
        self.assertEqual(record["switch_seq"], 1)
        self.assertEqual(record["rust"]["revision"], 5)
        again = await open_source_engine(self.root, bridge, settings)
        self.assertEqual(read_engine_switch(self.root)["switch_seq"], 2)

    async def test_invalid_engine_defaults_to_rust_with_diagnostic(self):
        bridge = self.rust_bridge()
        service = await open_source_engine(self.root, bridge, types.SimpleNamespace(source_engine="python"))
        self.assertEqual(service.engine, "rust")
        self.assertEqual(service.diagnostics[0]["code"], "invalid_source_engine")
        self.assertEqual(read_engine_switch(self.root)["active_engine"], "rust")

    async def test_missing_capability_has_no_python_fallback(self):
        bridge = FakeBridge()
        bridge.sync_results = {"get_capabilities": self.capabilities(["scan"])}
        with self.assertRaisesRegex(ValueError, "source_capability_unavailable"):
            await open_source_engine(self.root, bridge, types.SimpleNamespace(source_engine="rust"))


if __name__ == "__main__":
    unittest.main()
