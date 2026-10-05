"""Profile byte locks hold before the shared downloader publishes EXE caches."""
import hashlib
from io import BytesIO
import json
from pathlib import Path
import tempfile
from threading import Event
import unittest
from unittest.mock import Mock, patch
from urllib.error import HTTPError

from publisher import _download, _SameHostRedirect

URL = "https://kaguraserver.com/test.exe"


class Response(BytesIO):
    headers = {}
    def geturl(self):
        return URL


class ExeDownloadTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.cache = Path(self.temp.name)
        self.content = b"synthetic installer bytes"
        self.sha = hashlib.sha256(self.content).hexdigest()
        self.cancel = Event()

    def download(self, response, **overrides):
        options = dict(archive_name="archive.exe", manifest_name="exe-download.json",
                       max_size=1024, expected_size=len(self.content), expected_sha256=self.sha)
        options.update(overrides)
        opener = Mock()
        opener.open.return_value = response
        with patch("publisher.build_opener", return_value=opener):
            return _download(URL, self.cache, self.cancel, lambda url: self.assertEqual(url, URL), **options)

    def test_expected_hash_and_size_are_checked_before_cache_publication(self):
        original = self.cache / "archive.exe"
        original.write_bytes(b"previous cache")
        with self.assertRaisesRegex(ValueError, "exe_payload_changed"):
            self.download(Response(self.content), expected_sha256="0" * 64)
        self.assertEqual(original.read_bytes(), b"previous cache")
        self.assertFalse((self.cache / "exe-download.json").exists())
        self.assertEqual(list(self.cache.glob("download-*.tmp")), [])
        response = Response(self.content)
        response.headers = {"Content-Length": str(len(self.content) + 1)}
        with self.assertRaisesRegex(ValueError, "download_size_mismatch"):
            self.download(response)
        self.assertEqual(original.read_bytes(), b"previous cache")

    def test_locked_download_records_final_url_and_exe_cache_names(self):
        archive, sha, size, mode = self.download(Response(self.content))
        self.assertEqual((archive.name, sha, size, mode), ("archive.exe", self.sha, len(self.content), "download"))
        metadata = json.loads((self.cache / "exe-download.json").read_text())
        self.assertEqual(metadata["final_url"], URL)
        self.assertFalse((self.cache / "archive.zip").exists())

    def test_limits_and_cancellation_remove_partial_output(self):
        with self.assertRaisesRegex(ValueError, "archive_size_limit"):
            self.download(Response(self.content), max_size=1)
        event = self.cancel
        class CancelResponse(Response):
            def read1(self, size):
                data = super().read1(size)
                event.set()
                return data
        with self.assertRaisesRegex(RuntimeError, "download_cancelled"):
            self.download(CancelResponse(self.content))
        self.assertEqual(list(self.cache.iterdir()), [])

    def test_reuse_requires_profile_lock_and_every_redirect_is_validated(self):
        (self.cache / "archive.exe").write_bytes(self.content)
        (self.cache / "exe-download.json").write_text(json.dumps({"url": URL, "sha256": self.sha,
              "size": len(self.content), "etag": '"v1"'}))
        opener = Mock()
        opener.open.side_effect = HTTPError(URL, 304, "unchanged", {}, None)
        with patch("publisher.build_opener", return_value=opener):
            value = _download(URL, self.cache, self.cancel, lambda u: None, archive_name="archive.exe",
                manifest_name="exe-download.json", expected_sha256=self.sha, expected_size=len(self.content))
            self.assertEqual(value[3], "local")
            with self.assertRaises(HTTPError):
                _download(URL, self.cache, self.cancel, lambda u: None, archive_name="archive.exe",
                    manifest_name="exe-download.json", expected_sha256="0" * 64, expected_size=len(self.content))
        self.assertIsNone(opener.open.call_args.args[0].get_header("If-none-match"))
        def reject(url):
            raise ValueError("host_not_allowed")
        with self.assertRaisesRegex(ValueError, "host_not_allowed"):
            _SameHostRedirect(reject).redirect_request(None, None, 302, "found", {}, "https://foreign.invalid/file.exe")
