"""Automatic source update schedule: due selection and the runtime loop."""
import asyncio
import importlib.util
from pathlib import Path
import sys
import types
import unittest
from unittest.mock import patch

sys.modules.setdefault("decky", types.ModuleType("decky"))
spec = importlib.util.spec_from_file_location("scheduler_plugin", Path(__file__).parents[1] / "main.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def summary(source_id, **fields):
    base = {"id": source_id, "name": source_id, "role": "publisher", "enabled": True,
            "interval_hours": 24, "next_due_at": 1000, "checked_at": 900,
            "status": "source_refreshed", "revision": 1, "complete": True}
    base.update(fields)
    return base


class FakeSources:
    def __init__(self, *batches):
        self.batches = list(batches)
        self.reloads = 0
        self.started = []
        self.active = set()

    async def reload_sources(self):
        index = min(self.reloads, len(self.batches) - 1)
        self.reloads += 1
        return self.batches[index]

    def has_active_source_task(self, source_id):
        return source_id in self.active

    def start_update_source(self, source_id):
        self.started.append(source_id)
        return {"task_id": source_id}


def plugin_with(sources):
    plugin = module.Plugin()
    plugin.sources = sources
    return plugin


class SchedulerTests(unittest.IsolatedAsyncioTestCase):
    async def test_due_sources_start_once_per_check(self):
        sources = FakeSources([summary("due"), summary("later", next_due_at=2000)],
                              [summary("due", next_due_at=1000 + 24 * 3600),
                               summary("later", next_due_at=2000)])
        plugin = plugin_with(sources)
        self.assertEqual(await plugin._check_due_sources(now=1000), ["due"])
        self.assertEqual(sources.started, ["due"])
        # The attempt moved next_due_at into the future: the same window is not
        # updated again and missed periods are never backfilled.
        self.assertEqual(await plugin._check_due_sources(now=1000), [])
        self.assertEqual(sources.started, ["due"])

    async def test_disabled_fallback_interval_zero_and_not_due_are_skipped(self):
        sources = FakeSources([
            summary("disabled", enabled=False),
            summary("fallback", role="fallback_indexer"),
            summary("off", interval_hours=0),
            summary("later", next_due_at=5000),
            summary("due"),
        ])
        plugin = plugin_with(sources)
        self.assertEqual(await plugin._check_due_sources(now=1000), ["due"])

    async def test_source_with_an_active_task_is_not_queued_again(self):
        sources = FakeSources([summary("busy"), summary("idle")])
        sources.active.add("busy")
        plugin = plugin_with(sources)
        self.assertEqual(await plugin._check_due_sources(now=1000), ["idle"])
        self.assertEqual(sources.started, ["idle"])

    async def test_failure_reschedules_from_checked_at_and_does_not_backfill_missed_periods(self):
        failed = summary("source", status="source_refresh_failed", checked_at=1800,
                         next_due_at=1800 + 24 * 3600)
        sources = FakeSources([failed], [failed])
        plugin = plugin_with(sources)
        self.assertEqual(await plugin._check_due_sources(now=1800 + 100), [])
        self.assertEqual(await plugin._check_due_sources(now=1800 + 24 * 3600), ["source"])
        self.assertEqual(await plugin._check_due_sources(now=1800 + 5 * 24 * 3600), ["source"])
        self.assertEqual(len(sources.started), 2)

    async def test_runtime_loop_checks_immediately_then_waits_sixty_seconds(self):
        sources = FakeSources([summary("due")])
        plugin = plugin_with(sources)
        waits = []
        async def fake_sleep(seconds):
            waits.append(seconds)
            raise asyncio.CancelledError()
        with patch.object(module.asyncio, "sleep", new=fake_sleep):
            loop = asyncio.create_task(plugin._scheduler_loop())
            with self.assertRaises(asyncio.CancelledError):
                await loop
        self.assertEqual(sources.reloads, 1)
        self.assertEqual(sources.started, ["due"])
        self.assertEqual(waits, [module.SCHEDULER_INTERVAL_SECONDS])
        self.assertEqual(module.SCHEDULER_INTERVAL_SECONDS, 60)

    async def test_core_failure_keeps_the_loop_alive(self):
        class FailingSources(FakeSources):
            async def reload_sources(self):
                if not self.started:
                    self.started.append("failed-once")
                    raise RuntimeError("core_process_failed")
                return [summary("due")]
        sources = FailingSources()
        plugin = plugin_with(sources)
        waits = []
        async def fake_sleep(seconds):
            waits.append(seconds)
            if len(waits) == 2:
                raise asyncio.CancelledError()
        with patch.object(module.asyncio, "sleep", new=fake_sleep), \
                self.assertLogs(level="WARNING"):
            with self.assertRaises(asyncio.CancelledError):
                await plugin._scheduler_loop()
        self.assertEqual(waits, [60, 60])
        self.assertEqual(sources.started, ["failed-once", "due"])


if __name__ == "__main__":
    unittest.main()
