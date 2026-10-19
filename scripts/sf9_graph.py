"""Bounded, sequential SF9 CArchive object decoder.

Class references are resolved from the archive map; no marker/script scanning.
Configuration values are data; unknown framing and external objects are rejected.
"""
import re
from sf95_audit import Reader, evidence, file_table, require, MAX_METADATA

VERSION = "sf9-graph-v3"
CONTROLS = {"CSUFCtrlButton": 8, "CSUFCtrlStaticText": 8,
            "CSUFCtrlRadioButton": 10, "CSUFCtrlScrollingText": 15,
            "CSUFCtrlEditField": 17, "CSUFCtrlProgressBar": 11}
SCREENS = {"CSUFScreenStaticText", "CSUFScreenLicenseAgreement",
           "CSUFScreenInstallFolder", "CSUFScreenProgressBars"}

class Archive:
    def __init__(self, data, version):
        require(version in ("9.5", "9.6", "9.7"), "unsupported_installer_version")
        self.r = Reader(data)
        self.version = version
        self.classes = {}
        self.next_id = 1
        self.objects = []
        self.events = []
        self.includes = []
        self.packages = []
        self.files, self.table = file_table(data, installer_version=version)
        self.file_index = 0
        self.context = "project"
        self.lifecycle = "project"

    def nums(self, count):
        return [self.r.number("I") for _ in range(count)]

    def expected(self, values, code="exe_project_options_unsupported"):
        require(self.nums(len(values)) == values, code)

    def byte(self):
        self.r.number("B")

    def string(self):
        return self.r.string()

    def skip_display(self):
        r = self.r
        length = r.number("B")
        wide = False
        if length == 255:
            length = r.number("H")
            if length == 65534:
                wide = True
                length = r.number("B")
                if length == 255:
                    length = r.number("H")
            if length == 65535:
                length = r.number("I")
        require(length <= MAX_METADATA // (2 if wide else 1), "string_size_limit")
        r.read(length * (2 if wide else 1))

    def array(self, allowed, limit=1024):
        count = self.r.number("H")
        require(count <= limit, "exe_object_count_limit")
        return [self.object(allowed) for _ in range(count)]

    def empty_array(self):
        require(self.r.number("H") == 0, "exe_external_objects_unsupported")

    def object(self, allowed):
        r = self.r
        start = r.pos
        tag = r.number("H")
        if tag == 0xffff:
            schema, length = r.number("H"), r.number("H")
            require(0 < length <= 128, "exe_class_name_invalid")
            name = r.read(length).decode("ascii")
            require(schema == (0 if name == "CImageInfo" else 1), "exe_class_schema_unsupported")
            self.classes[self.next_id] = name
            self.next_id += 1
        else:
            require(tag & 0x8000 and tag != 0x8000, "exe_object_reference_unsupported")
            name = self.classes.get(tag & 0x7fff)
        require(name in allowed, "exe_object_class_unsupported")
        self.next_id += 1
        require(self.next_id < 0x7fff, "exe_object_count_limit")
        result = {"class": name, "offset": start}
        if name == "CGlobalIncludeLuaFile":
            self.expected([1], "exe_include_schema_unsupported")
            kind = r.number("I")
            result.update(name=self.string(), script=self.string(), source_kind=kind)
            n = r.number("H")
            require(n <= 64, "exe_include_selectors_unsupported")
            result["selectors"] = [self.string() for _ in range(n)]
            self.includes.append(result)
        elif name == "CSetupFileData":
            self.expected([1], "exe_file_schema_unsupported")
            self.byte()
            row = self.files[self.file_index]
            self.file_index += 1
            require(r.pos == row["serialized_record"]["offset"], "exe_file_framing_invalid")
            r.read(row["serialized_record"]["size"])
            result["tail_flag"] = r.number("B")
            result["file_index"] = row["index"]
        elif name in SCREENS:
            if name == "CSUFScreenProgressBars":
                self.expected([1, 1], "exe_screen_schema_unsupported")
            self.expected([1], "exe_screen_schema_unsupported")
            result["kind"] = r.number("I")
            result["name"] = self.string()
            self.nums(1)
            self.skip_display()
            self.nums(3)
            old = self.context
            self.context = name + ":" + result["name"]
            result["controls"] = self.array(set(CONTROLS), 128)
            result["events"] = self.array({"CIREvent"}, 32)
            result["languages"] = self.array({"CSUFLanguageFile"}, 16)
            self.appearance()
            if name == "CSUFScreenProgressBars":
                self.expected([1], "exe_screen_schema_unsupported")
            elif name == "CSUFScreenLicenseAgreement":
                self.nums(2)
            elif name == "CSUFScreenInstallFolder":
                self.expected([1], "exe_screen_schema_unsupported")
                self.string()
            self.context = old
        elif name in CONTROLS:
            fields = self.nums(CONTROLS[name])
            require(fields[0] == 1, "exe_control_schema_unsupported")
            result["fields"] = fields
            if name == "CSUFCtrlScrollingText":
                self.byte()
            elif name == "CSUFCtrlEditField":
                result["variable"] = self.string()
        elif name == "CIREvent":
            self.expected([1], "exe_event_schema_unsupported")
            parameters = self.string()
            require(len(parameters) <= 8192 and all(re.fullmatch(
                r"[A-Za-z_][A-Za-z0-9_]*\s+[A-Za-z_][A-Za-z0-9_]*", p.strip())
                for p in parameters.split(",") if p), "exe_event_parameters_unsupported")
            result.update(parameters=parameters, name=self.string(), script=self.string(), context=self.context, lifecycle=self.lifecycle)
            self.events.append(result)
        elif name == "CSUFLanguageFile":
            self.expected([1], "exe_language_schema_unsupported"); self.nums(1)
            result["name"] = self.string()
            self.byte()
            self.nums(4)
            n = r.number("H")
            require(n <= 128, "exe_language_count_limit")
            result["order"] = list(r.read(n))
            result["messages"] = self.array({"CSUFMessageID"}, 2048)
        elif name == "CSUFMessageID":
            self.expected([1], "exe_message_schema_unsupported")
            result["name"] = self.string()
            self.skip_display()
        elif name == "CSessionVar":
            self.expected([1], "exe_variable_schema_unsupported")
            self.string()
            self.skip_display()
            r.number("I")
        elif name == "CSUFPackage":
            self.expected([1], "exe_package_schema_unsupported")
            result["name"] = self.string()
            options = self.nums(3)
            result.update(install=options[0] != 0, enabled=options[1] != 0, additional_space=options[2], condition=self.string())
            self.array({"CSUFLanguageFile"}, 16)
            if not result["install"]: self.packages.append(result["name"])
        elif name == "CPasswordData":
            self.skip_display()
            n = r.number("H")
            require(n <= 64, "exe_password_list_limit")
            [self.string() for _ in range(n)]
            require(r.number("H") == 0, "exe_password_unsupported")
        elif name == "CImageInfo":
            self.skip_display()
            self.skip_display()
            self.nums(1)
        result["size"] = r.pos - start
        self.objects.append(result.copy() if name == "CSUFPackage" else {k: result[k] for k in ("class", "offset", "size")})
        return result

    def appearance(self):
        self.expected([1], "exe_appearance_schema_unsupported")
        self.nums(16)  # Colors only; no paths or scripts.
        for _ in range(5):
            self.nums(5)
            self.skip_display()
        self.expected([1], "exe_appearance_schema_unsupported")
        self.skip_display()
        self.skip_display()
        self.byte()
        self.nums(26 if self.version == "9.7" else 25)  # Window geometry.

    def labels(self):
        for _ in range(3):
            self.skip_display()
            self.nums(4)
            self.skip_display()
            self.nums(4)
            self.byte()

    def uninstall(self):
        self.lifecycle = "uninstall"
        self.expected([1], "exe_uninstall_schema_unsupported"); self.nums(1)
        for _ in range(3):
            self.skip_display()  # Disabled uninstall paths are never used.
        self.nums(5)
        self.byte()
        self.nums(5)
        self.byte()
        self.labels()
        self.nums(4)
        self.skip_display(); self.skip_display()
        self.nums(2); self.byte()
        for _ in range(5): self.skip_display()
        self.nums(1); self.byte(); self.skip_display()
        self.nums(2); self.empty_array()
        self.skip_display(); self.skip_display()
        self.nums(2); self.empty_array()
        self.skip_display(); self.empty_array()
        old = self.context
        self.context = "uninstall"
        self.array(SCREENS, 32)
        self.nums(2)
        self.array(SCREENS, 32); self.array(SCREENS, 32)
        self.array({"CIREvent"}, 32)
        self.context = old

    def timestamp(self):
        require(self.r.number("I") == 0x8000000a, "exe_timestamp_schema_unsupported")
        self.r.read(8)

    def project(self):
        self.lifecycle = "project"
        r = self.r
        self.expected([1], "exe_project_schema_unsupported"); self.byte(); self.skip_display()
        r.read(6)
        self.nums(2); self.byte()
        self.nums(4)
        r.read(2)
        self.skip_display()
        r.read(2)
        self.labels()
        self.nums(6)
        self.timestamp()
        self.nums(7)
        self.skip_display()  # Project GUID, no installation authority.
        self.nums(1); self.skip_display(); self.skip_display()
        self.byte(); self.byte(); self.nums(1)
        n = r.number("H")
        r.read(n * 2)
        self.nums(6)
        for _ in range(11): self.skip_display()  # Authoring/version metadata.
        self.nums(2); self.empty_array()
        self.context = "project"
        self.array({"CIREvent"}, 32)
        self.array({"CImageInfo"}, 16)
        self.empty_array()
        self.array({"CSUFLanguageFile"}, 16)
        self.timestamp()

    def decode(self):
        self.array({"CGlobalIncludeLuaFile"}, 64)
        self.array({"CSetupFileData"}, 65535)
        require(self.file_index == len(self.files), "exe_file_count_mismatch")
        self.empty_array()  # No external/runtime files.
        self.lifecycle = "before"
        self.array(SCREENS, 32)
        self.nums(2)
        self.lifecycle = "while"
        self.array(SCREENS, 32)
        self.lifecycle = "after"
        self.array(SCREENS, 32)
        self.array({"CSessionVar"}, 256)
        self.array({"CSUFPackage"}, 64)
        self.array({"CPasswordData"}, 64)
        self.uninstall()
        self.project()
        require(self.r.pos == self.r.end, "exe_metadata_trailing_data")
        return {"decoder": VERSION, "coverage": {"complete": True, "bytes": self.r.pos},
                "metadata": evidence(self.r.data), "events": self.events, "includes": self.includes,
                "disabled_packages": self.packages,
                "objects": self.objects, "installable": False}


def decode(data, version):
    require(0 < len(data) <= MAX_METADATA, "exe_metadata_limit")
    return Archive(data, version).decode()
