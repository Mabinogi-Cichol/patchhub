import copy
import hashlib
from io import BytesIO
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import sf95_audit as sf
from sf95_worker import convert
from test_sf95_audit import file_record, sample


class ConversionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "sample.exe"
        self.source.write_bytes(sample([file_record("测试.txt", content=b"replacement"), file_record("second", content=b"new")]))
        r = sf.audit(self.source.read_bytes())
        self.descriptor = {"exe_sha256": r["exe"]["sha256"], "exe_size": r["exe"]["size"],
            "metadata_sha256": r["metadata"]["sha256"], "semantics_approved": True,
            "payload": [{"path": x["destination"], "size": x["unpacked_size"], "sha256": x["payload"]["sha256"]} for x in r["files"]]}

    def run_conversion(self, descriptor=None):
        output = self.root / "normalized.zip"
        with self.source.open("rb") as stream:
            report = convert(stream, descriptor or self.descriptor, output)
        return report, output

    def test_locked_manifest_converts_only_game_files_and_is_deterministic(self):
        report, path = self.run_conversion()
        original = path.read_bytes()
        self.assertEqual(hashlib.sha256(original).hexdigest(), report["artifact_sha256"])
        self.assertEqual(len(original), report["artifact_size"])
        with zipfile.ZipFile(path) as archive:
            self.assertEqual(set(archive.namelist()), {"www/测试.txt", "www/second"})
            self.assertEqual(archive.read("www/测试.txt"), b"replacement")
            self.assertTrue(all(i.compress_type == zipfile.ZIP_STORED for i in archive.infolist()))
        path.unlink()
        self.assertEqual(self.run_conversion()[1].read_bytes(), original)

    def test_unreviewed_or_changed_bytes_metadata_manifest_write_nothing(self):
        changes = [{"semantics_approved": False}, {"exe_sha256": "f" * 64},
                   {"metadata_sha256": "a" * 64}, {"payload": []}, {"exe_size": 1}]
        for change in changes:
            with self.subTest(change=change), self.assertRaises(sf.AuditError):
                self.run_conversion({**self.descriptor, **change})
            self.assertEqual(list(self.root.iterdir()), [self.source])

    def test_second_pass_corruption_is_rejected_and_partial_removed(self):
        original = sf.Decoder.decode
        count = 0

        def corrupted(worker, *args, **kwargs):
            nonlocal count
            count += 1
            result, payload = original(worker, *args, **kwargs)
            if kwargs.get("sink") is not None:
                result["sha256"] = "a" * 64
            return result, payload

        with patch.object(sf.Decoder, "decode", corrupted), self.assertRaisesRegex(sf.AuditError, "exe_payload_changed"):
            self.run_conversion()
        self.assertGreater(count, 7)
        self.assertEqual(list(self.root.iterdir()), [self.source])

    def test_preexisting_final_and_partial_are_never_removed_on_rejection(self):
        for name in ("normalized.zip", "normalized.zip.partial"):
            existing = self.root / name
            existing.write_bytes(b"another invocation owns this")
            with self.assertRaisesRegex(sf.AuditError, "worker_output_exists"):
                self.run_conversion()
            self.assertEqual(existing.read_bytes(), b"another invocation owns this")
            existing.unlink()


if __name__ == "__main__":
    unittest.main()
