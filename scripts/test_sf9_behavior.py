import base64
import copy
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch
import zipfile
import zlib

from sf95_audit import AuditError, Reader, file_table
from sf9_behavior import Compiler, lower
from sf9_graph import Archive, decode
from sf9_lua import parse
from sf95_worker import audit_behavior, convert
from test_sf95_audit import compressed, sample


def num(*values): return b"".join(struct.pack("<I", v & 0xffffffff) for v in values)
def small(value): return struct.pack("<H", value)
def text(value):
    raw = value.encode()
    return bytes([len(raw)]) + raw if len(raw) < 255 else b"\xff" + small(len(raw)) + raw


class Fixture:
    def __init__(self): self.classes = {}; self.next = 1
    def obj(self, name, body):
        if name not in self.classes:
            self.classes[name] = self.next; self.next += 1
            prefix = b"\xff\xff" + small(0 if name == "CImageInfo" else 1) + small(len(name)) + name.encode()
        else: prefix = small(0x8000 | self.classes[name])
        self.next += 1
        return prefix + body()
    def array(self, name, bodies): return small(len(bodies)) + b"".join(self.obj(name, b) for b in bodies)
    def events(self, names, script=""):
        return self.array("CIREvent", [lambda name=name: num(1) + text("") + text(name) + text(script if name == "On Next" else "") for name in names])
    def appearance(self, version):
        return num(1, *([0] * 16)) + (num(0, 1, -13, 400, 0) + text("Arial")) * 5 + num(1) + text("") * 2 + b"\0" + num(*([0] * (26 if version == "9.7" else 25)))
    def file(self, version, name, content):
        packed = compressed(content)
        mask = b"\0\x80" + b"\xff\xff" * (15 if version == "9.5" else 18)
        stamp = num(0x8000000a)
        body = num(1) + b"\0" + b"".join(text(v) for v in (name, name, "C:\\source", ".bin", "Archive", ""))
        body += small(1) + struct.pack("<q", len(content)) + b" " + stamp + bytes(8) + stamp + bytes(8) + stamp + bytes(8) + bytes(25)
        body += text("%AppFolder%") + b"\2" + bytes(10) + text("") * 5 + b"\0" + text("") + bytes(8) + text("")
        body += bytes(4 if version == "9.7" else 3) + b"\1\1\0" + bytes.fromhex("00000000e80300000000") + small(len(mask) // 2) + mask
        body += text("") + small(1) + text("All") + text("None") + small(0) + text("") + struct.pack("<qI", len(packed), zlib.crc32(content)) + b"\0"
        return body, packed
    def metadata(self, version="9.5", name="payload.bin", script=None, display_blob=None, session_vars=(), includes=()):
        script = script or 'if File.DoesExist("%AppFolder%\\\\engine.exe") and File.GetCRC("%AppFolder%\\\\identity.dat") == 123 then Screen.Next(); end'
        body, self.packed = self.file(version, name, b"generic-content")
        include_bodies = [lambda kind=kind, selectors=selectors: num(1, kind) + text("synthetic") + text("") + small(len(selectors)) + b"".join(text(v) for v in selectors) for kind, selectors in includes]
        result = self.array("CGlobalIncludeLuaFile", include_bodies) + self.array("CSetupFileData", [lambda: body]) + small(0)
        def screen():
            return num(1, 110) + text("Any install folder title") + num(2) + (text("Any display text") if display_blob is None else display_blob) + num(0, 0, 0) + small(0) + self.events(["On Preload", "On Back", "On Next", "On Cancel", "On Help", "On Ctrl Message"], script) + small(0) + self.appearance(version) + num(1) + text("%AppFolder%")
        result += self.array("CSUFScreenInstallFolder", [screen]) + num(1, 1) + small(0) * 2
        variables = [lambda name=name, blob=blob, flag=flag: num(1) + text(name) + blob + num(flag) for name, blob, flag in session_vars]
        result += self.array("CSessionVar", variables) + small(0) * 2
        result += num(1, 0) + text("") * 3 + num(0, 0, 0, 1, 0) + b"\0" + num(*([0] * 5)) + b"\0"
        labels = (text("") + num(*([0] * 4)) + text("") + num(*([0] * 4)) + b"\0") * 3
        result += labels + num(1, 1, 1, 1) + text("") * 2 + num(0, 0) + b"\0" + text("") * 5 + num(0) + b"\0" + text("") + num(0, 0) + small(0) + text("") * 2 + num(0, 0) + small(0) + text("") + small(0)
        result += small(0) + num(1, 1) + small(0) * 2 + self.events(["On Startup", "On Pre Uninstall", "On Uninstall Error", "On Post Uninstall", "On Shutdown"])
        result += num(1) + b"\0" + text("") + bytes.fromhex("000100000100") + num(1, 0) + b"\0" + num(*([0] * 4)) + b"\0\1" + text("") + bytes(2) + labels
        result += num(1, 0, 0, 30, 30, 1) + num(0x8000000a, 0, 0) + num(0, 0, 1, 1, 2, 1, 0) + text("arbitrary-project-id") + num(0) + text("") * 2 + bytes(2) + num(1)
        mask = b"\0\x80" + b"\xff\xff" * (15 if version == "9.5" else 18)
        result += small(len(mask)//2) + mask + num(0, 0, 0, 0, 1, 0) + text("") * 11 + num(0, 0) + small(0)
        result += self.events(["Global Functions", "On Startup", "On Pre Install", "On Install Error", "On Post Install", "On Shutdown"]) + small(0) * 3 + num(0x8000000a, 0, 0)
        return result


class BehaviorTests(unittest.TestCase):
    def test_parameterized_conditions_and_complete_graph_across_versions(self):
        for version in ("9.5", "9.6", "9.7"):
            data = Fixture().metadata(version, "arbitrary-name.bin")
            graph = decode(data, version)
            result = lower(graph, file_table(data, installer_version=version)[0], version)
            self.assertEqual(graph["coverage"], {"complete": True, "bytes": len(data)})
            self.assertEqual(result["condition"], ("symbol", "and", ("symbol", "exists", "engine.exe"), ("symbol", "==", ("symbol", "crc32", "identity.dat"), 123)))
            self.assertEqual(result["copy_files"], ["arbitrary-name.bin"])
    def test_display_bytes_and_length_headers_do_not_change_installation(self):
        # Invalid GB18030 and unpaired UTF-16 surrogates are display data only.
        ansi = b"\xa9 "
        wide = b"\x00\xd8\x00\x00"
        variants = [bytes([2]) + ansi, b"\xff\x02\x00" + ansi,
                    b"\xff\xff\xff" + num(2) + ansi,
                    b"\xff\xfe\xff\x02" + wide,
                    b"\xff\xfe\xff\xff\x02\x00" + wide,
                    b"\xff\xfe\xff\xff\xff\xff" + num(2) + wide]
        for version in ("9.5", "9.6", "9.7"):
            reference = Fixture().metadata(version)
            expected = lower(decode(reference, version), file_table(reference, installer_version=version)[0], version)
            for blob in variants:
                with self.subTest(version=version, header=blob.hex()):
                    data = Fixture().metadata(version, display_blob=blob)
                    graph = decode(data, version)
                    observed = lower(graph, file_table(data, installer_version=version)[0], version)
                    self.assertEqual(observed, expected)
                    self.assertNotIn("variables", graph)
                    self.assertNotIn("terms", graph)
                    self.assertNotIn("terms", observed)
                    archive = Archive(reference, version)
                    archive.r = Reader(blob + num(0x12345678))
                    self.assertIsNone(archive.skip_display())
                    self.assertEqual(archive.r.number("I"), 0x12345678)
                    for cut in range(len(blob)):
                        archive.r = Reader(blob[:cut])
                        with self.assertRaisesRegex(AuditError, "truncated_record"):
                            archive.skip_display()

    def test_session_values_are_skipped_and_include_selectors_are_evidence(self):
        data = Fixture().metadata(session_vars=[("same", b"\x02\xa9 ", 255), ("same", b"\x01\x80", 0xffffffff)], includes=[(77, ["Custom", "All"])])
        graph = decode(data, "9.5")
        self.assertNotIn("variables", graph)
        self.assertEqual(graph["includes"][0]["source_kind"], 77)
        self.assertEqual(graph["includes"][0]["selectors"], ["Custom", "All"])
        lower(graph, file_table(data)[0], "9.5")

    def test_non_display_paths_and_scripts_still_reject_invalid_encoding(self):
        data = Fixture().metadata()
        for marker in (text("payload.bin"), text("On Next")):
            changed = data.replace(marker, bytes([len(marker)-1]) + b"\xff" + marker[2:], 1)
            with self.assertRaisesRegex(AuditError, "invalid_utf8"):
                decode(changed, "9.5")

    def test_reference_selection_and_lifecycle_match_v2_contract(self):
        data = Fixture().metadata(script="Screen.Next()")
        graph = decode(data, "9.5")
        self.assertEqual({e["lifecycle"] for e in graph["events"]}, {"before", "project", "uninstall"})
        graph["objects"].extend([
            {"class": "CSUFPackage", "name": "a", "install": False},
            {"class": "CSUFPackage", "name": "b", "install": True}])
        graph["events"].extend([
            {"context": "project", "name": "On Startup", "script": 'SetupData.SetPackageProperties("a", {Install=true})'},
            {"context": "uninstall", "name": "On Startup", "script": 'File.Run("never-installed")'},
            {"context": "CSUFScreenStaticText:after", "lifecycle": "after", "name": "On Next", "script": "Screen.Next()"}])
        files = file_table(data)[0]
        files[0].update(packages=["a", "b"], condition='File.DoesExist("%AppFolder%\\\\sub\\\\engine.exe")', runtime_folder="Data", attributes=999)
        result = lower(graph, files, "9.5")
        self.assertIs(result["condition"], True)
        self.assertEqual(result["file_conditions"], [{"path": "payload.bin", "condition": ("symbol", "exists", "sub/engine.exe")}])

    def test_corrupt_objects_tail_and_order_fail_closed(self):
        data = Fixture().metadata()
        for changed in (data[:-1], data + b"unexpected", data.replace(b"CSUFScreenInstallFolder", b"CSUFScreenExecuteStuff", 1)):
            with self.assertRaises((ValueError, KeyError, UnicodeError)):
                decode(changed, "9.5")

    def test_behavior_converter_rechecks_same_graph_and_conditions(self):
        fixture = Fixture(); metadata = fixture.metadata("9.7")
        record = (b"unused", fixture.packed)
        with patch("test_sf95_audit.metadata", return_value=metadata):
            package = sample([record], version="9.7.0.0")
        report = audit_behavior(package)
        descriptor = {"action": "convert_behavior", "exe_sha256": report["exe"]["sha256"], "exe_size": len(package), "metadata_sha256": report["metadata"]["sha256"], "behavior_sha256": report["behavior_sha256"], "payload": [{"path": r["destination"], "size": r["unpacked_size"], "sha256": r["payload"]["sha256"]} for r in report["files"]]}
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)/"input.exe"; source.write_bytes(package)
            output = Path(directory)/"normalized.zip"
            with source.open("rb") as stream:
                convert(stream, descriptor, output)
            with zipfile.ZipFile(output) as archive: self.assertEqual(archive.read("payload.bin"), b"generic-content")
            output.unlink()
            with source.open("rb") as stream, self.assertRaisesRegex(AuditError, "exe_behavior_changed"):
                convert(stream, {**descriptor, "behavior_sha256": "0"*64}, output)
            self.assertFalse(output.exists())
    def test_ast_rejects_reachable_side_effect_dynamic_call_and_pattern_change(self):
        for source in ('File.Delete("%AppFolder%\\\\identity.dat")', 'local x="File"; _G[x].Run("payload")', '_NeedsReboot=true; System.Reboot()', 'local f=File.Run; f("payload")', 'while true do Screen.Next() end', 'if string.find(strInstallFolderPath,"^%a:wrong$") then Screen.Next() end', 'function evil() _NeedsReboot=true end evil(); if _NeedsReboot then System.Reboot() end'):
            compiler = Compiler(100); compiler.mode="event"; compiler.event_name="On Next"
            with self.subTest(source=source), self.assertRaises(ValueError): compiler.block(parse(source), [(compiler.env, True, False, None)])
    def test_comments_do_not_hide_actions_and_dead_branches_are_proven(self):
        compiler = Compiler(100); compiler.mode="event"; compiler.event_name="On Next"
        compiler.block(parse('--[=[ File.Run("bad") ]=]\nif false then File.Run("bad") else Screen.Next() end'), [(compiler.env, True, False, None)])
        self.assertEqual(compiler.next_paths, [True])
        with self.assertRaises(ValueError): compiler.block(parse('-- safe\nFile.Run("bad")'), [(compiler.env, True, False, None)])
    def test_bounded_lexer_and_ast(self):
        for source in ('if true then', '"unterminated', '--[[ unfinished', '(' * 100 + 'true' + ')' * 100):
            with self.assertRaises(ValueError): parse(source)
        self.assertEqual(parse('local string_value="ok"; local x=string.find(strInstallFolderPath,"x")')[1][2][0][1], ('index', ('name', 'string'), ('literal', 'find')))

    def test_cross_event_runtime_guards_cannot_be_reset(self):
        fixture = Fixture(); data = fixture.metadata()
        graph = decode(data, "9.5")
        files = file_table(data, installer_version="9.5")[0]
        graph["events"].append({"context": "project", "name": "On Startup", "script": "_NeedsReboot=true"})
        with self.assertRaisesRegex(ValueError, "exe_lua_runtime_mutation_unsupported"):
            lower(graph, files, "9.5")

    def test_progress_checks_all_stages_for_effects(self):
        fixture = Fixture(); data = fixture.metadata()
        graph = decode(data, "9.5")
        graph["events"].append({"context": "CSUFScreenProgressBars:any", "name": "On Progress", "script": 'if e_Stage == 3 then File.Run("bad") end'})
        with self.assertRaisesRegex(ValueError, "exe_lua_call_unsupported:File.Run"):
            lower(graph, file_table(data, installer_version="9.5")[0], "9.5")
        graph["events"][-1]["script"] = 'if e_CurrentItemText ~= "" then File.Run("bad") end'
        with self.assertRaisesRegex(ValueError, "exe_lua_call_unsupported:File.Run"):
            lower(graph, file_table(data, installer_version="9.5")[0], "9.5")

    def test_lua_and_or_keep_literal_operand_values(self):
        compiler = Compiler(100)
        compiler.block(parse('local value=true and "original"; local fallback=false or "fallback"'), [(compiler.env, True, False, None)])
        self.assertEqual(compiler.env["value"], "original")
        self.assertEqual(compiler.env["fallback"], "fallback")

if __name__ == "__main__": unittest.main()
