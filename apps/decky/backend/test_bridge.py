import os
from pathlib import Path
import unittest
import tempfile
import json

from bridge import CoreBridge


class BridgeIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        root = Path(__file__).resolve().parents[3]
        executable = "steam-patch.exe" if os.name == "nt" else "steam-patch"
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        catalog = Path(self.temp.name) / "catalog.json"
        catalog.write_text(json.dumps({"schema_version": 1, "revision": 0, "releases": []}))
        self.bridge = CoreBridge(root / "target/debug" / executable, catalog)

    async def test_scan_real_core_returns_platform_and_empty_catalog(self):
        result = await self.bridge.scan()
        self.assertEqual(result["platform"], "steamos")
        self.assertEqual(result["catalog_revision"], 0)
        self.assertEqual(result["catalog_trust"], "unsigned_development")
        self.assertIsInstance(result["installs"], list)

    async def test_preview_core_error_preserves_code(self):
        with self.assertRaisesRegex(RuntimeError, "instance_not_found"):
            await self.bridge.preview("nonexistent-p0-instance", "nonexistent-release")

    async def test_rejects_unbounded_request_before_spawning(self):
        with self.assertRaisesRegex(ValueError, "invalid_id"):
            await self.bridge.preview("x" * 65536, "r")


if __name__ == "__main__":
    unittest.main()
