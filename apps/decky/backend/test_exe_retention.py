import json
from pathlib import Path
import tempfile
import time
import unittest
import sqlite3
import hashlib
import uuid
from contextlib import closing
from types import SimpleNamespace

import caches


class ExeRetentionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.instance = "42|synthetic-library|synthetic-game"
        self.state = self.root / "publisher" / ("exe-" + "a" * 24 + "-42") / hashlib.sha256(self.instance.encode()).hexdigest()[:16]
        self.jobs = self.state / "jobs"
        self.job = self.jobs / str(uuid.uuid4())
        self.job.mkdir(parents=True)
        self.cache = self.state / "cache"
        self.cache.mkdir()
        self.backup = self.job / "original.backup"
        self.backup.write_bytes(b"original")
        (self.job / "original.stage").write_bytes(b"installed")
        (self.state / "journal.sqlite").write_bytes(b"journal")
        (self.cache / "archive.exe").write_bytes(b"download")
        self.policy = self.job / "exe-policy.json"
        self.write_policy()

    def write_policy(self, **overrides):
        value = {"schema_version": 1, "kind": "publisher_exe", "plan_id": self.job.name,
                 "instance_id": self.instance, "release_id": self.state.parent.name,
                 "keep_backups": True, "expires_at": int(time.time()) + 60, "lease_active": True}
        value.update(overrides)
        self.policy.write_text(json.dumps(value))

    def test_completed_exe_manual_and_automatic_cleanup_keep_backup_even_without_receipt(self):
        self.assertTrue(caches.protected_exe_jobs(self.jobs))
        self.policy.unlink()
        freed = caches.delete_files(self.jobs, self.job.name, (".stage", ".backup"))
        self.assertEqual(freed, len(b"installed"))
        self.assertEqual(self.backup.read_bytes(), b"original")
        self.assertEqual(caches.clear_jobs(self.jobs, [self.job.name]), 0)
        self.assertFalse(caches.forget_transactions(self.state))
        self.assertEqual((self.state / "journal.sqlite").read_bytes(), b"journal")

    def test_live_persisted_lease_prevents_cache_cleanup_after_restart(self):
        publisher = SimpleNamespace(caches=lambda: iter([(self.state.parent.name, self.state, self.cache)]))
        self.assertEqual(caches.clear(publisher, self.root / "downloads"), 0)
        self.assertTrue((self.cache / "archive.exe").exists())
        self.write_policy(lease_active=False)
        self.assertEqual(caches.clear(publisher, self.root / "downloads"), len(b"download"))
        self.assertEqual(self.backup.read_bytes(), b"original")

    def test_malformed_receipt_blocks_cache_cleanup(self):
        for content in ("broken", "[]", "{}", "{\"lease_active\":false}", "{\"lease_active\":true,\"expires_at\":\"invalid\"}"):
            self.policy.write_text(content)
            self.assertTrue(caches.exe_cache_leased(self.state))

    def test_expired_lease_is_reclaimable_and_other_zip_policies_unchanged(self):
        self.write_policy(expires_at=0)
        self.assertFalse(caches.exe_cache_leased(self.state))
        zip_jobs = self.root / "publisher/patchmoe-42/abc/jobs"
        zip_job = zip_jobs / "completed"
        zip_job.mkdir(parents=True)
        (zip_job / "0.backup").write_bytes(b"original")
        self.assertFalse(caches.protected_exe_jobs(zip_jobs))
        self.assertEqual(caches.clear_jobs(zip_jobs, ["completed"]), len(b"original"))

    def test_missing_receipt_reclaims_only_verified_terminal_or_expired_core_job(self):
        instance = self.instance
        plan_id = str(uuid.uuid4())
        old_job = self.state / "jobs" / self.job.name
        old_job.rename(old_job.parent / plan_id)
        (old_job.parent / plan_id / "exe-policy.json").unlink()
        plan = {"plan_id": plan_id, "instance_id": instance, "release_id": self.state.parent.name,
                "created_at": int(time.time()) - 1000, "expires_at": int(time.time()) - 100}
        transaction = {"transaction_id": plan_id, "plan_id": plan_id, "instance_id": instance,
                       "release_id": self.state.parent.name, "state": "planned"}
        (self.state / "journal.sqlite").unlink()
        with closing(sqlite3.connect(self.state / "journal.sqlite")) as connection:
            connection.execute("CREATE TABLE jobs(id TEXT PRIMARY KEY, document TEXT)")
            def save():
                connection.execute("INSERT OR REPLACE INTO jobs VALUES(?,?)", (plan_id, json.dumps({"plan": plan, "transaction": transaction})))
                connection.commit()
            save()
            self.assertFalse(caches.exe_cache_leased(self.state))
            plan["expires_at"] = int(time.time()) + 60
            plan["created_at"] = int(time.time())
            save()
            self.assertTrue(caches.exe_cache_leased(self.state))
            transaction["state"] = "applying"
            save()
            self.assertTrue(caches.exe_cache_leased(self.state))
            transaction["state"] = "rolled_back"
            save()
            self.assertFalse(caches.exe_cache_leased(self.state))
            transaction["release_id"] = "foreign"
            save()
            self.assertTrue(caches.exe_cache_leased(self.state))
        self.assertEqual((old_job.parent / plan_id / "original.backup").read_bytes(), b"original")


if __name__ == "__main__":
    unittest.main()
