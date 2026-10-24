import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from state_host import prepare_host_state


class HostStateTests(unittest.TestCase):
    def test_idempotent_preserves_old_state_and_subscription(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            root = home / ".local/share/patchhub"
            host = prepare_host_state(root, home)
            old = root / "publisher"
            old.mkdir()
            (old / "journal.sqlite").write_bytes(b"corrupt old cache must never be read")
            settings = root / "sources/rust-v1/settings.json"
            settings.parent.mkdir(parents=True)
            settings.write_bytes(b"current Rust subscriptions")
            self.assertEqual(prepare_host_state(root, home), host)
            self.assertEqual(settings.read_bytes(), b"current Rust subscriptions")
            self.assertEqual((old / "journal.sqlite").read_bytes(), b"corrupt old cache must never be read")
            self.assertFalse((host / "tasks.json").exists())
            self.assertFalse((root / "installer").exists())

    def test_rejects_other_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(ValueError, "invalid_state_path"):
                prepare_host_state(Path(tmp) / "other", tmp)

    @unittest.skipUnless(hasattr(os, "geteuid"), "Unix identity boundary")
    def test_rejects_wrong_uid_without_chown_existing_home(self):
        with tempfile.TemporaryDirectory() as tmp, patch("state_host.os.chown") as chown:
            root = Path(tmp) / ".local/share/patchhub"
            with self.assertRaisesRegex(ValueError, "state_owner_mismatch"):
                prepare_host_state(root, tmp, os.getuid() + 1, os.getgid())
            chown.assert_not_called()

    def test_linked_parent_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp) / "home"
            target = Path(tmp) / "other"
            home.mkdir(); target.mkdir()
            try:
                (home / ".local").symlink_to(target, target_is_directory=True)
            except OSError:
                self.skipTest("symlink permission unavailable")
            with self.assertRaisesRegex(ValueError, "linked_state_path"):
                prepare_host_state(home / ".local/share/patchhub", home)
            self.assertFalse((target / "share").exists())


if __name__ == "__main__":
    unittest.main()
