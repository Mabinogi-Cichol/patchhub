"""Bounded SF9 serialized script inventory for diagnostics, never authorization.

The include array is decoded from its actual framing. Event-reference scanning
is deliberately marked incomplete until the surrounding object graph is decoded.
No absence-of-string or class-name heuristic can approve an installation.
"""
import re
import struct

from sf95_audit import AuditError, MAX_METADATA, Reader, evidence, require

VERSION = "sf9-metadata-v1"
MAX_CLASSES = 256
MAX_INCLUDES = 64
MAX_EVENTS = 1024
MAX_PARAMETERS = 16


def declarations(data):
    """Collect correctly length-framed class declarations as evidence only."""
    result = []
    for match in re.finditer(b"\xff\xff", data):
        offset = match.start()
        if offset + 6 > len(data):
            continue
        schema, length = struct.unpack_from("<HH", data, offset + 2)
        if not 1 <= length <= 128 or offset + 6 + length > len(data):
            continue
        raw = data[offset + 6:offset + 6 + length]
        if not re.fullmatch(rb"C[A-Za-z][A-Za-z0-9]*", raw):
            continue
        result.append({"name": raw.decode("ascii"), "schema": schema,
                       **evidence(data[offset:offset + 6 + length], offset)})
        require(len(result) <= MAX_CLASSES, "exe_class_count_limit")
    return result


def includes(data):
    """Decode the leading include array, including each installation selector."""
    reader = Reader(data)
    count = reader.number("H")
    require(count <= MAX_INCLUDES, "exe_include_count_limit")
    result = []
    if not count:
        return result, reader.pos
    require(reader.read(4) == b"\xff\xff\x01\0", "exe_include_schema_unsupported")
    length = reader.number("H")
    require(length == len(b"CGlobalIncludeLuaFile")
            and reader.read(length) == b"CGlobalIncludeLuaFile", "exe_include_schema_unsupported")
    for index in range(count):
        start = reader.pos
        if index:
            require(reader.number("H") == 0x8001, "exe_include_reference_unsupported")
        require(reader.number("I") == 1, "exe_include_schema_unsupported")
        source_kind = reader.number("I")
        require(source_kind in (0, 1), "exe_include_kind_unsupported")
        name = reader.string()
        script_start = reader.pos
        script = reader.string()
        selectors_count = reader.number("H")
        require(selectors_count <= 64, "exe_selector_count_limit")
        selectors = [reader.string() for _ in range(selectors_count)]
        result.append({"name": name, "source_kind": source_kind, "script": script,
                       "selectors": selectors, "script_offset": script_start,
                       **evidence(data[start:reader.pos], start)})
    return result, reader.pos


def event(reader):
    start = reader.pos
    require(reader.number("I") == 1, "exe_event_schema_unsupported")
    parameter_text = reader.string()
    require(len(parameter_text) <= 8192, "exe_event_parameter_limit")
    parameters = [p.strip() for p in parameter_text.split(",")] if parameter_text else []
    require(len(parameters) <= MAX_PARAMETERS, "exe_event_parameter_limit")
    require(all(re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*\s+[A-Za-z_][A-Za-z0-9_]*", p)
                for p in parameters), "exe_event_parameters_unsupported")
    name = reader.string()
    require((name.startswith("On ") or name == "Global Functions") and len(name) <= 128, "exe_event_name_invalid")
    script_start = reader.pos
    script = reader.string()
    return {"name": name, "parameters": parameters, "script": script,
            "script_offset": script_start, **evidence(reader.data[start:reader.pos], start)}


def inventory(data):
    require(0 < len(data) <= MAX_METADATA, "exe_metadata_limit")
    classes = declarations(data)
    global_includes, end = includes(data)
    events = []
    diagnostics = []
    event_classes = [c for c in classes if c["name"] == "CIREvent"]
    if len(event_classes) == 1:
        declaration = event_classes[0]
        reader = Reader(data, declaration["offset"] + declaration["size"])
        events.append(event(reader))
        # The first reference identifies the registered event class in these
        # layouts. It is evidence, not a substitute for parsing the class map.
        if reader.pos + 2 <= len(data):
            reference = data[reader.pos:reader.pos + 2]
            if struct.unpack("<H", reference)[0] & 0x8000:
                for match in re.finditer(re.escape(reference) + b"\x01\x00\x00\x00", data):
                    try:
                        current = event(Reader(data, match.start() + 2))
                    except AuditError:
                        diagnostics.append({"code": "exe_event_reference_unresolved", "offset": match.start()})
                        continue
                    events.append(current)
                    require(len(events) <= MAX_EVENTS, "exe_event_count_limit")
    elif event_classes:
        diagnostics.append({"code": "exe_event_class_ambiguous"})
    else:
        diagnostics.append({"code": "exe_event_graph_unresolved"})
    return {"schema_version": 1, "decoder": VERSION, "metadata": evidence(data),
            "classes": classes, "includes": global_includes, "include_end": end,
            "events": events, "diagnostics": diagnostics,
            "coverage": {"complete": False, "blockers": ["exe_serialized_object_graph_incomplete"]},
            "installable": False}
