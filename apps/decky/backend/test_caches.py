import os
from pathlib import Path
import tempfile
import unittest

import caches


class StubPublisher:
    def __init__(self, entries):
        self.entries = entries

    def caches(self):
        return list(self.entries)

    def states(self):
        return [state for _release, state, _cache in self.entries]


class CacheTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.downloads = self.root / "downloads"
        self.downloads.mkdir()
        self.cache = self.root / "publisher" / "patchmoe-1" / "abc" / "cache"
        self.cache.mkdir(parents=True)
        (self.cache / "archive.zip").write_bytes(b"a" * 10)
        (self.cache / "download.json").write_bytes(b"{}")
        (self.cache / "download-x.tmp").write_bytes(b"tmp")
        (self.cache / "catalog.json").write_bytes(b"keep")
        self.sha = "a" * 64
        (self.downloads / f"{self.sha}.zip").write_bytes(b"b" * 20)
        (self.downloads / "abc.partial").write_bytes(b"c" * 3)
        (self.downloads / "user-patch.zip").write_bytes(b"user")
        self.publisher = StubPublisher([("patchmoe-1", self.cache.parent, self.cache)])

    def test_status_counts_only_tool_owned_files(self):
        value = caches.status(self.publisher, self.downloads)
        self.assertEqual(value["publisher_bytes"], 10 + 2 + 3)
        self.assertEqual(value["core_bytes"], 20 + 3)
        self.assertEqual(value["file_count"], 5)

    def test_clear_removes_caches_and_keeps_user_and_state(self):
        removed = caches.clear(self.publisher, self.downloads)
        self.assertEqual(removed, 10 + 2 + 3 + 20 + 3)
        self.assertFalse((self.cache / "archive.zip").exists())
        self.assertFalse((self.cache / "download.json").exists())
        self.assertFalse((self.downloads / f"{self.sha}.zip").exists())
        self.assertTrue((self.cache / "catalog.json").exists())
        self.assertTrue((self.downloads / "user-patch.zip").exists())
        self.assertEqual(caches.status(self.publisher, self.downloads)["file_count"], 0)

    def test_clear_core_only_digest_named(self):
        self.assertEqual(caches.clear_core(self.downloads, self.sha), 1)
        self.assertEqual(caches.clear_core(self.downloads, self.sha), 0)
        self.assertFalse((self.downloads / f"{self.sha}.zip").exists())
        self.assertTrue((self.downloads / "user-patch.zip").exists())

    def test_clear_core_rejects_non_digest(self):
        self.assertEqual(caches.clear_core(self.downloads, "user-patch"), 0)
        self.assertTrue((self.downloads / "user-patch.zip").exists())

    def test_symlinked_cache_file_is_skipped(self):
        target = self.root / "outside.zip"
        target.write_bytes(b"outside")
        digest = "b" * 64
        link = self.downloads / f"{digest}.zip"
        try:
            os.symlink(target, link)
        except (OSError, NotImplementedError):
            self.skipTest("symlinks unavailable")
        self.assertEqual(caches.clear_core(self.downloads, digest), 0)
        self.assertTrue(target.exists())

    def test_jobs_status_is_read_only(self):
        job = self.root / "jobs" / "tx-1"
        job.mkdir(parents=True)
        (job / "0.stage").write_bytes(b"s" * 30)
        (job / "0.backup").write_bytes(b"b" * 40)
        (job / "note.txt").write_bytes(b"keep")
        self.assertEqual(caches.jobs_status(self.publisher, self.root),
                         {"stage_bytes": 30, "backup_bytes": 40})
        self.assertTrue((job / "0.stage").exists())
        self.assertTrue((job / "0.backup").exists())


if __name__ == "__main__":
    unittest.main()
