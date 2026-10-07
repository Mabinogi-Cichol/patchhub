"""Bounded, sequential SF9 CArchive object decoder.

Class references are resolved from the archive map; no marker/script scanning.
Only the default file-copy project settings are supported. Nondefault options,
external files, selectors, passwords and unknown classes are rejected.
"""
import re
from sf95_audit import Reader, evidence, file_table, require, MAX_METADATA

VERSION = "sf9-graph-v1"
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
        self.variables = {}
        self.packages = []
        self.terms = []
        self.files, self.table = file_table(data, installer_version=version)
        self.file_index = 0
        self.context = "project"

    def nums(self, count):
        return [self.r.number("I") for _ in range(count)]

    def expected(self, values, code="exe_project_options_unsupported"):
        require(self.nums(len(values)) == values, code)

    def byte(self, value=0):
        require(self.r.number("B") == value, "exe_project_options_unsupported")

    def string(self, *, display=False):
        # ANSI display text uses the serialized language's code page; preserve
        # byte values until presentation. Executable names/scripts stay strict.
        r = self.r
        if not display:
            return r.string()
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
        raw = r.read(length * (2 if wide else 1))
        try:
            value = raw.decode("utf-16-le" if wide else "gb18030").rstrip("\0")
        except UnicodeError as error:
            raise ValueError("exe_display_encoding_unsupported") from error
        require("\0" not in value, "embedded_nul")
        return value

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
            self.expected([1])
            kind = r.number("I")
            require(kind in (0, 1), "exe_include_kind_unsupported")
            result.update(name=self.string(), script=self.string(), source_kind=kind)
            n = r.number("H")
            require(n <= 64, "exe_include_selectors_unsupported")
            require([self.string() for _ in range(n)] in ([], ["All"]), "exe_include_selectors_unsupported")
            self.includes.append(result)
        elif name == "CSetupFileData":
            self.expected([1], "exe_file_schema_unsupported")
            self.byte()
            row = self.files[self.file_index]
            self.file_index += 1
            require(r.pos == row["serialized_record"]["offset"], "exe_file_framing_invalid")
            r.read(row["serialized_record"]["size"])
            self.byte()
            result["file_index"] = row["index"]
        elif name in SCREENS:
            if name == "CSUFScreenProgressBars":
                self.expected([1, 1], "exe_screen_schema_unsupported")
            self.expected([1], "exe_screen_schema_unsupported")
            result["kind"] = r.number("I")
            result["name"] = self.string()
            require(r.number("I") in (1, 2), "exe_screen_schema_unsupported")
            self.string(display=True)
            self.expected([0, 0, 0], "exe_screen_conditions_unsupported")
            old = self.context
            self.context = name + ":" + result["name"]
            result["controls"] = self.array(set(CONTROLS), 128)
            result["events"] = self.array({"CIREvent"}, 32)
            expected_events = ["On Preload", "On Progress", "On Cancel"] if name == "CSUFScreenProgressBars" else ["On Preload", "On Back", "On Next", "On Cancel", "On Help", "On Ctrl Message"]
            require([e["name"] for e in result["events"]] == expected_events, "exe_event_order_unsupported")
            result["languages"] = self.array({"CSUFLanguageFile"}, 16)
            self.appearance()
            if name == "CSUFScreenProgressBars":
                self.expected([1], "exe_screen_schema_unsupported")
            elif name == "CSUFScreenLicenseAgreement":
                self.expected([1, 1], "exe_terms_selection_unsupported")
                for lang in result["languages"]:
                    for message in lang["messages"]:
                        if message["name"] == "IDS_CTRL_SCROLLTEXT_BODY":
                            self.terms.append(message["text"])
            elif name == "CSUFScreenInstallFolder":
                self.expected([1], "exe_screen_schema_unsupported")
                require(self.string() == "%AppFolder%", "unsupported_target_variable")
            self.context = old
        elif name in CONTROLS:
            fields = self.nums(CONTROLS[name])
            require(fields[0] == 1, "exe_control_schema_unsupported")
            result["fields"] = fields
            if name == "CSUFCtrlScrollingText":
                self.byte()
            elif name == "CSUFCtrlEditField":
                result["variable"] = self.string()
                require(result["variable"] == "%AppFolder%", "exe_control_variable_unsupported")
        elif name == "CIREvent":
            self.expected([1], "exe_event_schema_unsupported")
            parameters = self.string()
            require(len(parameters) <= 8192 and all(re.fullmatch(
                r"[A-Za-z_][A-Za-z0-9_]*\s+[A-Za-z_][A-Za-z0-9_]*", p.strip())
                for p in parameters.split(",") if p), "exe_event_parameters_unsupported")
            result.update(parameters=parameters, name=self.string(), script=self.string(), context=self.context)
            require(result["name"].startswith("On ") or result["name"] == "Global Functions", "exe_event_name_invalid")
            self.events.append(result)
        elif name == "CSUFLanguageFile":
            self.expected([1, 1], "exe_language_schema_unsupported")
            result["name"] = self.string()
            self.byte()
            self.expected([0, 0, 1, 4], "exe_language_schema_unsupported")
            n = r.number("H")
            require(n <= 128, "exe_language_count_limit")
            result["order"] = list(r.read(n))
            result["messages"] = self.array({"CSUFMessageID"}, 2048)
        elif name == "CSUFMessageID":
            self.expected([1], "exe_message_schema_unsupported")
            result.update(name=self.string(), text=self.string(display=True))
        elif name == "CSessionVar":
            self.expected([1], "exe_variable_schema_unsupported")
            name = self.string()
            value = self.string(display=True)
            flag = r.number("I")
            require(flag in (0, 1, 2) and name not in self.variables, "exe_variable_invalid")
            self.variables[name] = value
        elif name == "CSUFPackage":
            self.expected([1], "exe_package_schema_unsupported")
            result["name"] = self.string()
            self.expected([0, 0, 0], "exe_package_selection_unsupported")
            require(self.string() == "0", "exe_package_condition_unsupported")
            self.array({"CSUFLanguageFile"}, 16)
            self.packages.append(result["name"])
        elif name == "CPasswordData":
            self.string(display=True)
            n = r.number("H")
            require(n <= 64, "exe_password_list_limit")
            require([self.string() for _ in range(n)] == ["All"], "exe_password_components_unsupported")
            require(r.number("H") == 0, "exe_password_unsupported")
        elif name == "CImageInfo":
            self.string(display=True)
            self.string(display=True)
            require(r.number("I") in (1, 2), "exe_image_reference_unsupported")
        result["size"] = r.pos - start
        self.objects.append({k: result[k] for k in ("class", "offset", "size")})
        return result

    def appearance(self):
        self.expected([1], "exe_appearance_schema_unsupported")
        self.nums(16)  # Colors only; no paths or scripts.
        for _ in range(5):
            self.nums(5)
            self.string(display=True)
        self.expected([1], "exe_appearance_schema_unsupported")
        self.string(display=True)
        self.string(display=True)
        self.byte()
        self.nums(26 if self.version == "9.7" else 25)  # Window geometry.

    def labels(self):
        for _ in range(3):
            self.string(display=True)
            self.nums(4)
            self.string(display=True)
            self.nums(4)
            require(self.r.number("B") in (0, 1), "exe_appearance_schema_unsupported")

    def uninstall(self):
        self.expected([1, 0], "exe_create_uninstall_unsupported")
        for _ in range(3):
            self.string(display=True)  # Disabled uninstall paths are never used.
        self.expected([0, 0, 0, 1, 0])
        self.byte()
        self.nums(5)
        self.byte()
        self.labels()
        self.expected([1, 1, 1, 1])
        self.string(display=True); self.string(display=True)
        self.expected([0, 0]); self.byte()
        for _ in range(5): self.string(display=True)
        self.expected([0]); self.byte(); self.string(display=True)
        self.expected([0, 0]); self.empty_array()
        self.string(display=True); self.string(display=True)
        self.expected([0, 0]); self.empty_array()
        self.string(display=True); self.empty_array()
        old = self.context
        self.context = "uninstall"
        self.array(SCREENS, 32)
        self.expected([1, 1])
        self.array(SCREENS, 32); self.array(SCREENS, 32)
        events = self.array({"CIREvent"}, 32)
        require([e["name"] for e in events] == ["On Startup", "On Pre Uninstall", "On Uninstall Error", "On Post Uninstall", "On Shutdown"], "exe_event_order_unsupported")
        self.context = old

    def timestamp(self):
        require(self.r.number("I") == 0x8000000a, "exe_timestamp_schema_unsupported")
        self.r.read(8)

    def project(self):
        r = self.r
        self.expected([1]); self.byte(); self.string(display=True)
        require(r.read(6) == bytes.fromhex("000100000100"), "exe_project_options_unsupported")
        self.expected([1, 0]); self.byte()
        self.nums(4)
        require(r.read(2) == b"\0\1", "exe_project_options_unsupported")
        self.string(display=True)
        require(r.read(2) == b"\0\0", "exe_project_options_unsupported")
        self.labels()
        self.expected([1, 0, 0, 30, 30, 1])
        self.timestamp()
        self.expected([0, 0, 1, 1, 2, 1, 0])
        self.string(display=True)  # Project GUID, no installation authority.
        self.expected([0]); self.string(display=True); self.string(display=True)
        self.byte(); self.byte(); self.expected([1])
        n = r.number("H")
        require(n == (16 if self.version == "9.5" else 19) and r.read(n * 2) == b"\0\x80" + b"\xff\xff" * (n - 1), "exe_platform_condition_unsupported")
        self.expected([0, 0, 0, 0, 1, 0])
        for _ in range(11): self.string(display=True)  # Authoring/version metadata.
        self.expected([0, 0]); self.empty_array()
        self.context = "project"
        events = self.array({"CIREvent"}, 32)
        require([e["name"] for e in events] == ["Global Functions", "On Startup", "On Pre Install", "On Install Error", "On Post Install", "On Shutdown"], "exe_event_order_unsupported")
        self.array({"CImageInfo"}, 16)
        self.empty_array()
        self.array({"CSUFLanguageFile"}, 16)
        self.timestamp()

    def decode(self):
        self.array({"CGlobalIncludeLuaFile"}, 64)
        self.array({"CSetupFileData"}, 8192)
        require(self.file_index == len(self.files), "exe_file_count_mismatch")
        self.empty_array()  # No external/runtime files.
        self.array(SCREENS, 32)
        self.expected([1, 1])
        self.array(SCREENS, 32); self.array(SCREENS, 32)
        self.array({"CSessionVar"}, 256)
        self.array({"CSUFPackage"}, 64)
        self.array({"CPasswordData"}, 64)
        self.uninstall()
        self.project()
        require(self.r.pos == self.r.end, "exe_metadata_trailing_data")
        return {"decoder": VERSION, "coverage": {"complete": True, "bytes": self.r.pos},
                "metadata": evidence(self.r.data), "events": self.events, "includes": self.includes,
                "variables": self.variables, "disabled_packages": self.packages,
                "terms": self.terms, "objects": self.objects, "installable": False}


def decode(data, version):
    require(0 < len(data) <= MAX_METADATA, "exe_metadata_limit")
    return Archive(data, version).decode()
