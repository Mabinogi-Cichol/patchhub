import copy
from pathlib import Path
import tempfile
from threading import Event
import types
import unittest
from unittest.mock import AsyncMock, patch

from exe_worker import analyze, analyze_behavior
from exe_profiles import digest


class AnalysisProtocolTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "sf9_metadata.py").write_text("# fixed test tool")
        self.result = {"schema_version": 1, "action": "analyze", "exe_sha256": "a" * 64,
            "installable": False, "report": {"installable": False, "exe": {"sha256": "a" * 64, "size": 100}}}

    async def run_analysis(self, result):
        bridge = types.SimpleNamespace(process_identity={})
        with patch("exe_worker._run_worker", new=AsyncMock(return_value=result)) as worker:
            report = await analyze(bridge, self.root, self.root / "input.exe", self.root / "output",
                                   "a" * 64, 100, Event())
            descriptor = worker.await_args.args[4]
            self.assertEqual(descriptor, {"action": "analyze", "exe_sha256": "a" * 64, "exe_size": 100})
            self.assertNotIn("semantics_approved", descriptor)
            return report

    async def test_analysis_is_content_bound_and_cannot_approve_installation(self):
        self.assertEqual(await self.run_analysis(self.result), self.result["report"])
        for field, value in (("installable", True), ("action", "convert"), ("schema_version", 2)):
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "exe_worker_invalid_response"):
                await self.run_analysis({**self.result, field: value})
        for package in (None, "not an object", {"sha256": "b" * 64, "size": 100}, {"sha256": "a" * 64, "size": 101}):
            modified = copy.deepcopy(self.result)
            modified["report"]["exe"] = package
            with self.subTest(package=package), self.assertRaisesRegex(ValueError, "exe_worker_invalid_response"):
                await self.run_analysis(modified)
        modified = copy.deepcopy(self.result)
        modified["report"]["installable"] = True
        with self.assertRaisesRegex(ValueError, "exe_worker_invalid_response"):
            await self.run_analysis(modified)

    async def test_missing_fixed_metadata_tool_stops_before_worker_launch(self):
        (self.root / "sf9_metadata.py").unlink()
        with patch("exe_worker._run_worker", new=AsyncMock()) as worker:
            with self.assertRaisesRegex(ValueError, "exe_worker_path_invalid"):
                await analyze(types.SimpleNamespace(), self.root, self.root / "input.exe", self.root / "output",
                              "a" * 64, 100, Event())
            worker.assert_not_awaited()

    async def test_behavior_analysis_checks_coverage_digest_and_package_identity(self):
        for name in ("sf9_graph.py", "sf9_lua.py", "sf9_behavior.py"):
            (self.root / name).write_text("# fixed test tool", encoding="utf-8")
        behavior = {"schema_version": 1, "condition": True, "coverage": {"complete": True}}
        report = {"exe": {"offset": 0, "sha256": "a"*64, "size": 100}, "installable": False,
                  "capability": "behavior_supported", "behavior": behavior, "behavior_sha256": digest(behavior)}
        result = {"schema_version": 1, "action": "analyze_behavior", "installable": False, "report": report}
        async def run(value):
            with patch("exe_worker._run_worker", new=AsyncMock(return_value=value)):
                return await analyze_behavior(types.SimpleNamespace(), self.root, self.root/"input.exe",
                                              self.root/"output", "a"*64, 100, Event())
        self.assertEqual(await run(result), report)
        for field, value in (("behavior_sha256", "b"*64), ("capability", "metadata_inspector"),
                             ("exe", {"offset": 0, "sha256": "a"*64, "size": 101}),
                             ("behavior", {"coverage": {"complete": False}})):
            changed = copy.deepcopy(result); changed["report"][field] = value
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "exe_worker_invalid_response"):
                await run(changed)


if __name__ == "__main__":
    unittest.main()
