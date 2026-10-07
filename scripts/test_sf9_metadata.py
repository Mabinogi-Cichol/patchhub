import struct
import unittest

from sf95_audit import AuditError
from sf9_metadata import declarations, includes, event, inventory


def string(value):
    raw = value.encode()
    return bytes([len(raw)]) + raw


def declaration(name):
    return b"\xff\xff\x01\0" + struct.pack("<H", len(name)) + name.encode()


def include_array():
    return (b"\x02\0" + declaration("CGlobalIncludeLuaFile")
            + struct.pack("<II", 1, 0) + string("Constants") + string("IDOK=1;") + b"\x01\0" + string("All")
            + b"\x01\x80" + struct.pack("<II", 1, 1) + string("helpers.lua") + string("function helper() end")
            + b"\x01\0" + string("All"))


def event_bytes(name, script, parameters=()):
    return struct.pack("<I", 1) + string(", ".join(parameters)) + string(name) + string(script)


class MetadataTests(unittest.TestCase):
    def test_include_selectors_and_scripts_remain_framed(self):
        raw = include_array()
        result, end = includes(raw)
        self.assertEqual(end, len(raw))
        self.assertEqual([r["source_kind"] for r in result], [0, 1])
        self.assertEqual(result[1]["script"], "function helper() end")
        self.assertEqual(result[1]["selectors"], ["All"])
        corrupt = bytearray(raw)
        corrupt[corrupt.index(b"\x01\x80")] = 2
        with self.assertRaisesRegex(AuditError, "exe_include_reference_unsupported"):
            includes(corrupt)

    def test_parameterized_error_event_is_not_skipped(self):
        from sf95_audit import Reader
        raw = event_bytes("On Install Error", "Dialog.Message(e_ErrorMsg);", ("string e_ErrorMsg", "number e_Code"))
        result = event(Reader(raw))
        self.assertEqual(result["parameters"], ["string e_ErrorMsg", "number e_Code"])
        self.assertEqual(result["name"], "On Install Error")
        with self.assertRaises(AuditError):
            event(Reader(raw[:-1]))

    def test_inventory_never_claims_complete_or_authorizes_even_simple_scripts(self):
        raw = (include_array() + declaration("CIREvent") + event_bytes("On Next", "Screen.Next();")
               + b"\x10\x80" + event_bytes("On Install Error", "", ("string e_ErrorMsg",)))
        result = inventory(raw)
        self.assertEqual([e["name"] for e in result["events"]], ["On Next", "On Install Error"])
        self.assertFalse(result["coverage"]["complete"])
        self.assertFalse(result["installable"])
        self.assertEqual(result["coverage"]["blockers"], ["exe_serialized_object_graph_incomplete"])

    def test_class_lengths_bound_names_and_nonclass_bytes_do_not_count(self):
        raw = declaration("CIREvent") + b"PC" + b"\xff\xff\x01\0\xff\xffCUnknown"
        self.assertEqual([c["name"] for c in declarations(raw)], ["CIREvent"])


if __name__ == "__main__":
    unittest.main()
