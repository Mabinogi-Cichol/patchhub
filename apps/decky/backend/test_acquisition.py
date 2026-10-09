"""Local signed ZIP inputs are host-validated; acquisition runs in Rust."""
import hashlib
from pathlib import Path
import tempfile
from threading import Event
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from acquisition import find_archive
from test_discovery_rpc import module


class AcquisitionBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.downloads = self.home / "Downloads"
        self.downloads.mkdir()
        self.plugin = module.Plugin()
        self.plugin.downloads = self.downloads
        self.plugin.bridge = SimpleNamespace(state_dir=self.home / "state")

    def test_signed_local_zip_is_passed_as_fixed_download_input(self):
        archive = self.downloads / "中文补丁.zip"
        archive.write_bytes(b"ZIP")
        self.assertEqual(self.plugin._local_archive(str(archive)), archive.resolve())
        with patch.object(module.decky, 'DECKY_USER_HOME', str(self.home), create=True):
            self.assertEqual(self.plugin._local_archive("~/Downloads/中文补丁.zip"), archive.resolve())

    def test_outside_download_or_exe_is_rejected(self):
        outside = self.home / "outside.zip"
        outside.write_bytes(b"ZIP")
        with self.assertRaisesRegex(ValueError, 'archive_outside_downloads'):
            self.plugin._local_archive(str(outside))
        exe = self.downloads / "unsafe.exe"
        exe.write_bytes(b"MZ")
        with self.assertRaisesRegex(ValueError, 'invalid_archive'):
            self.plugin._local_archive(str(exe))

    def test_retired_python_prepare_route_is_absent(self):
        for method in ('_prepare_install', 'prepare_install', '_plan_install', 'cancel_download'):
            self.assertFalse(hasattr(self.plugin, method))

    def test_developer_local_search_cancel_is_retained_as_oracle(self):
        event = Event(); event.set()
        (self.downloads / 'candidate.zip').write_bytes(b'data')
        with self.assertRaisesRegex(RuntimeError, 'download_cancelled'):
            find_archive([self.downloads], hashlib.sha256(b'data').hexdigest(), 4, '', event)


if __name__ == '__main__':
    unittest.main()
