"""Import diagnostics and Rust-format settings mapping (offline)."""
import unittest

from source_engine import aggregate_import_diagnostics, map_rust_settings
from source_service import normalize_diagnostics


class SourceImportTests(unittest.TestCase):
    def test_import_report_counts_and_diagnostics_are_flattened(self):
        diagnostics = aggregate_import_diagnostics({
            "imported_settings": False, "imported_cache": 2, "imported_discovery": 4,
            "imported_index": 0, "diagnostics": [{"code": "invalid_discovery_cache", "source_id": "kagura-cn"}]})
        self.assertEqual([entry["code"] for entry in diagnostics],
                         ["source_cache_imported", "source_discovery_imported", "invalid_discovery_cache"])
        self.assertEqual(diagnostics[0]["detail"], "2")

    def test_map_rust_settings_filters_invalid_values(self):
        mapped = map_rust_settings({"enabled": {"patchmoe": 1, "ok": True},
                                    "added": ["nope", {"id": "custom"}],
                                    "games": {"42": {"patchmoe": True}, "bad": 3}})
        self.assertEqual(mapped, {"enabled": {"ok": True}, "added": [{"id": "custom"}],
                                  "games": {"42": {"patchmoe": True}}})

    def test_normalize_diagnostics_drops_unknown_shapes(self):
        self.assertEqual(normalize_diagnostics([{"code": "a"}, "b", {"nope": 1}, 3]),
                         [{"code": "a"}, {"code": "b"}])


if __name__ == "__main__":
    unittest.main()
