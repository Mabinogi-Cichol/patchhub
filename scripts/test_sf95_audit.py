"""Self-generated SF9.5 framing; no publisher/game data or executable code."""
import json
import lzma
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zlib

import sf95_audit as sf


def number(fmt, value):
    return struct.pack("<" + fmt, value)


def string(text):
    data = text.encode("utf-8")
    return number("B", len(data)) + data


def compressed(data, declared=None):
    dictionary = 1 << 20
    return bytes([16]) + number("Q", len(data) if declared is None else declared) + lzma.compress(
        data, format=lzma.FORMAT_RAW, filters=[{"id": lzma.FILTER_LZMA2, "dict_size": dictionary}])


def file_record(name="测试.txt", directory="%AppFolder%\\www", content=b"test-data", condition="", package=None):
    payload = compressed(content)
    data = b"".join(string(s) for s in ("C:\\source\\" + name, name, "C:\\source", ".txt", "Archive", ""))
    data += bytes(2) + number("q", len(content)) + bytes(5) + bytes(8) + bytes(16) + bytes(8) + bytes(25)
    data += string(directory) + bytes(11) + string("") * 5 + bytes(1) + string("") + bytes(8) + string("")
    data += bytes(3) + b"\x01\x01\0" + bytes(10) + number("H", 0) + string(condition) + bytes(2)
    data += string("") * 2 + number("H", 1 if package else 0)
    if package:
        data += string(package)
    data += string("") + number("q", len(payload)) + number("I", zlib.crc32(content)) + bytes(8)
    return data, payload


def metadata(records):
    return b"unparsed-events" + number("H", len(records)) + b"\xff\xff\x01\0" + number("H", 14) + b"CSetupFileData" + bytes(5) + b"".join(r[0] for r in records) + b"unparsed-after-table"


def overlay(records):
    result = sf.SIGNATURE + bytes(10)
    for runtime in (b"not-an-executable", b"not-a-dll"):
        result += number("q", len(runtime)) + runtime
    result += number("I", 3)
    for name, content in (("irsetup.dat", metadata(records)), ("IRIMG1.BMP", b"image1"), ("IRIMG2.BMP", b"image2")):
        packed = compressed(content)
        result += name.encode().ljust(264, b"\0") + number("q", len(packed)) + number("I", zlib.crc32(content)) + bytes(4) + packed
    return result + b"".join(r[1] for r in records)


def sample(records=None, version="9.5.1.0", certificate=False, xml=None):
    manifest = xml or (f'<assembly><assemblyIdentity version="{version}"/>'
                       '<description>Setup Factory Run-time</description>'
                       '<dependency><assemblyIdentity version="6.0.0.0"/></dependency></assembly>').encode()
    resource = bytearray(1024)
    # Three directories: RT_MANIFEST -> resource ID -> language -> data entry.
    for offset, identifier, child in ((0, 24, 0x80000020), (32, 1, 0x80000040), (64, 1033, 96)):
        struct.pack_into("<H", resource, offset + 14, 1)
        struct.pack_into("<II", resource, offset + 16, identifier, child)
    struct.pack_into("<IIII", resource, 96, 0x1000 + 128, len(manifest), 0, 0)
    resource[128:128 + len(manifest)] = manifest
    headers = bytearray(512)
    headers[:2] = b"MZ"
    struct.pack_into("<I", headers, 60, 64)
    headers[64:68] = b"PE\0\0"
    struct.pack_into("<H", headers, 70, 1)
    struct.pack_into("<H", headers, 84, 224)
    struct.pack_into("<H", headers, 88, 0x10b)
    struct.pack_into("<I", headers, 88 + 60, 512)
    struct.pack_into("<I", headers, 88 + 92, 16)
    struct.pack_into("<II", headers, 88 + 112, 0x1000, 1024)
    struct.pack_into("<IIII", headers, 88 + 224 + 8, 1024, 0x1000, 1024, 512)
    result = bytes(headers) + bytes(resource) + overlay(records or [file_record()])
    if certificate:
        result += bytes((-len(result)) % 8)
        struct.pack_into("<II", headers, 88 + 128, len(result), 16)
        result = bytes(headers) + result[512:] + number("I", 12) + number("H", 0x200) + number("H", 2) + b"cert" + bytes(4)
    return result


class AuditTests(unittest.TestCase):
    def test_crc_boundary_keeps_unknown_object_tail_and_framing_out_of_file_fields(self):
        import base64
        import hashlib
        first, second = file_record("a"), file_record("b")
        framing = b"\x01\x04\x80\x01\0\0\0\0"
        footer = b"\0\0\0\x04\0\xff\xff\x01"
        records = [(first[0][:-8] + framing, first[1]), (second[0][:-8] + footer, second[1])]
        raw = metadata(records)
        rows, table = sf.file_table(raw)
        self.assertEqual(rows[0]["serialized_record"]["size"], len(first[0]) - 8)
        self.assertEqual(rows[0]["unparsed_inter_record_framing"]["base64"], base64.b64encode(framing).decode())
        self.assertIsNone(rows[1]["unparsed_inter_record_framing"])
        self.assertEqual(raw[table["end"]:], footer + b"unparsed-after-table")
        self.assertEqual(table["unparsed_regions"][1]["offset"], table["end"])
        self.assertEqual(table["end_basis"], "last_known_crc_end_not_serialized_object_end")
        self.assertEqual(len(rows[0]["unreviewed_fields"]), 11)
        self.assertEqual(rows[1]["serialized_record"]["sha256"], hashlib.sha256(second[0][:-8]).hexdigest())

    def rejects(self, code, fn, *args):
        with self.assertRaisesRegex(sf.AuditError, "^" + code + "$"):
            fn(*args)

    def test_full_sample_crc_sha_unicode_and_unreviewed_semantics(self):
        result = sf.audit(sample([file_record(condition="return NeedsDLC();", package="optional")]))
        self.assertFalse(result["installable"])
        self.assertFalse(result["semantics_reviewed"])
        self.assertEqual(result["capability"], "discover_only")
        row = result["files"][0]
        self.assertEqual(row["destination"], "www/测试.txt")
        self.assertEqual(row["condition"], "return NeedsDLC();")
        self.assertEqual(row["packages"], ["optional"])
        self.assertEqual(row["payload"]["crc32"], row["crc32"])
        self.assertEqual(len(row["payload"]["sha256"]), 64)
        self.assertTrue(row["unreviewed_fields"])
        self.assertEqual(len(result["file_table"]["unparsed_regions"]), 2)

    def test_certificate_is_framed_and_not_trusted(self):
        result = sf.audit(sample(certificate=True))
        self.assertFalse(result["certificate"]["signature_verified"])
        bad = bytearray(sample(certificate=True))
        bad[-1] = 1
        self.rejects("invalid_certificate_padding", sf.audit, bad)

    def test_multiple_file_records(self):
        report = sf.audit(sample([file_record("a.txt"), file_record("b.txt", content=b"other")]))
        self.assertEqual(len(report["files"]), 2)

    def test_pe_and_version_rejections(self):
        for data, code in ((b"<html>error</html>", "not_pe"), (b"MZ" + bytes(100), "not_pe"),
                (sample(version="10.0.0.0"), "unsupported_installer_version"),
                (sample(version="9.3.0.0"), "unsupported_installer_version"),
                (sample()[:1000], "truncated_pe_section")):
            with self.subTest(code=code):
                self.rejects(code, sf.audit, data)

    def test_untrusted_xml_entity_is_rejected(self):
        xml = b'<!DOCTYPE assembly [<!ENTITY x "9.5.1.0">]><assembly><assemblyIdentity version="&x;"/></assembly>'
        self.rejects("unsafe_manifest_xml", sf.audit, sample(xml=xml))

    def test_overlapping_pe_headers_and_bad_resource_pointer(self):
        data = bytearray(sample())
        struct.pack_into("<I", data, 88 + 224 + 20, 256)
        self.rejects("overlapping_pe_headers", sf.audit, data)
        data = bytearray(sample())
        struct.pack_into("<I", data, 512 + 20, 0x8000ffff)
        self.rejects("invalid_range", sf.audit, data)

    def test_paths_are_rejected_before_game_payload_decoding(self):
        paths = ["%AppFolder%\\..", "%AppFolder%\\C:", "\\\\server\\share", "%Temp%\\www",
                 "%AppFolder%\\www\\NUL", "%AppFolder%\\www\\COM¹", "%AppFolder%\\www\\x ",
                 "%AppFolder%\\www\\.steam-patch-backup"]
        for path in paths:
            with self.subTest(path=path):
                data = sample([file_record(directory=path)])
                calls = []
                original = sf.Decoder.decode

                def observe(worker, *args, **kwargs):
                    calls.append(kwargs.get("compressed", True))
                    return original(worker, *args, **kwargs)

                with patch.object(sf.Decoder, "decode", observe), self.assertRaises(sf.AuditError):
                    sf.audit(data)
                self.assertEqual(len(calls), 5)  # Only engine and metadata; zero game members.
        for name in ("a:b", "../x", "..\\x", "a\0b", "C:\\x", "foo/xx", "foo\\xx", "LPT9.txt", "x."):
            with self.subTest(name=name), self.assertRaises(sf.AuditError):
                sf.safe_target("%AppFolder%\\www", name)

    def test_root_level_appfolder_target_stays_at_game_root(self):
        result = sf.audit(sample([file_record(name="extra.xp3", directory="%AppFolder%",
                                              content=b"root-payload")]))
        self.assertEqual(result["files"][0]["destination"], "extra.xp3")
        self.assertEqual(result["files"][0]["unpacked_size"], len(b"root-payload"))
        # A root destination must still reject unsafe basenames before decoding.
        self.rejects("unsafe_target_path", sf.safe_target, "%AppFolder%", "..")

    def test_case_unicode_and_file_directory_conflicts(self):
        for names, code in ((("A.txt", "a.txt"), "duplicate_or_unicode_case_collision"),
                (("é.txt", "e\u0301.txt"), "duplicate_or_unicode_case_collision"),
                (("www", "www/data"), "file_directory_collision"),
                (("A/x", "a/y"), "duplicate_or_unicode_case_collision")):
            self.rejects(code, sf.validate_targets, [{"destination": n} for n in names])

    def test_strict_utf8_and_truncated_strings(self):
        self.rejects("invalid_utf8", sf.Reader(b"\x01\xff").string)
        self.rejects("truncated_record", sf.Reader(b"\x04x").string)
        self.rejects("embedded_nul", sf.Reader(b"\x03a\0b").string)

    def test_crc_size_stream_and_tail_rejections(self):
        data = compressed(b"hello")
        self.rejects("payload_crc_mismatch", sf.Decoder().decode, sf.Reader(data), len(data), 5, 99)
        self.rejects("lzma2_size_mismatch", sf.Decoder().decode, sf.Reader(data), len(data), 4)
        for payload, code in ((data[:-1], "truncated_lzma2_stream"),
                              (data + b"x", "trailing_compressed_data"),
                              (bytes([40]) + data[1:], "dictionary_limit"),
                              (compressed(b"hello", 4), "payload_size_mismatch")):
            self.rejects(code, sf.Decoder().decode, sf.Reader(payload), len(payload))
        self.rejects("overlay_size_or_tail_mismatch", sf.audit, sample() + b"tail")
        self.rejects("overlay_size_or_tail_mismatch", sf.audit, sample()[:-1])

    def test_metadata_file_count_and_resource_budgets(self):
        table = bytearray(metadata([file_record()]))
        offset = table.find(b"CSetupFileData") - 8
        struct.pack_into("<H", table, offset, sf.MAX_ENTRIES + 1)
        self.rejects("file_count_limit", sf.file_table, table)
        payload = compressed(b"x" * 4096)
        with patch.object(sf, "MAX_FILE", 1024):
            self.rejects("expanded_size_limit", sf.Decoder().decode, sf.Reader(payload), len(payload))
        with patch.object(sf, "MAX_TOTAL", 4095):
            self.rejects("expanded_size_limit", sf.Decoder().decode, sf.Reader(payload), len(payload))
        worker = sf.Decoder()
        worker.deadline = 0
        self.rejects("audit_timeout", worker.decode, sf.Reader(payload), len(payload))

    def test_cli_isolated_no_side_effects_and_structured_failure(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            path = root / "fixture.exe"
            path.write_bytes(sample())
            script = Path(__file__).with_name("sf95_audit.py")
            result = subprocess.run([sys.executable, "-I", str(script), str(path)], capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(json.loads(result.stdout)["installable"])
            self.assertEqual(list(root.iterdir()), [path])
            report = root / "report.json"
            with report.open("wb") as output:
                result = subprocess.run([sys.executable, "-I", str(script), str(path)], stdout=output,
                                        stderr=subprocess.PIPE, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(json.loads(report.read_text())["installable"])
            report.unlink()
            path.write_bytes(b"html")
            result = subprocess.run([sys.executable, "-I", str(script), str(path)], capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(json.loads(result.stdout)["error"], "not_pe")
            self.assertEqual(list(root.iterdir()), [path])


if __name__ == "__main__":
    unittest.main()
