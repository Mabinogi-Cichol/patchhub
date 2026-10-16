"""Read-only Setup Factory 9.x sample auditor (P0, never an installer).

The SF file-table layout is derived from CybercentreCanada/sfextract commit
21f5c5595a70e9284fff83476c0e307d5b92f573; see licenses/sfextract-MIT.txt.
No upstream decompressor, PE dependency, path writer or script runner is used.
Unknown serialized fields are preserved as evidence, never treated as safe.
"""
import argparse
import base64
import hashlib
import json
import lzma
import mmap
import re
import struct
import sys
import time
import unicodedata
import zlib
from pathlib import Path
from xml.etree import ElementTree

VERSION = "sf9-audit-v3"
SIGNATURE = bytes.fromhex("e0e0e1e1e2e2e3e3e4e4e5e5e6e6e7e7")
# The original Deck samples used single members below 1 GiB. Local SF9.5/9.6
# samples require ~1.28 GiB members; full read-only audits passed in nonprivileged
# WSL bwrap under the unchanged 3 GiB address-space and 180-second worker caps.
# The new large-member budget still needs final Deck verification.
# These stay below the core's archive (8 GiB), single-file (4 GiB) and total
# (16 GiB) caps.
MAX_EXE = 2 * 1024 ** 3
MAX_FILE = 2 * 1024 ** 3
MAX_TOTAL = 2 * 1024 ** 3
MAX_METADATA = 16 * 1024 ** 2
MAX_DICTIONARY = 128 * 1024 ** 2
CHUNK = 64 * 1024


class AuditError(ValueError):
    pass


def require(condition, code):
    if not condition:
        raise AuditError(code)


class Reader:
    def __init__(self, data, start=0, end=None):
        self.data = data
        self.pos = start
        self.end = len(data) if end is None else end
        require(0 <= start <= self.end <= len(data), "invalid_range")

    def read(self, size):
        require(0 <= size <= self.end - self.pos, "truncated_record")
        result = self.data[self.pos:self.pos + size]
        self.pos += size
        return result

    def number(self, fmt):
        return struct.unpack("<" + fmt, self.read(struct.calcsize("<" + fmt)))[0]

    def string(self):
        length = self.number("B")
        encoding = "utf-8"
        if length == 0xff:
            length = self.number("H")
            if length == 0xfffe:
                encoding = "utf-16-le"
                length = self.number("B")
                if length == 0xff:
                    length = self.number("H")
            if length == 0xffff:
                length = self.number("I")
                require(length != 0xffffffff, "unsupported_string_length")
        byte_length = length * (2 if encoding == "utf-16-le" else 1)
        require(byte_length <= MAX_METADATA, "string_size_limit")
        data = self.read(byte_length)
        # Preserve exactly what was serialized, including any terminator bytes.
        try:
            text = data.decode(encoding, errors="strict").rstrip("\0")
        except UnicodeDecodeError as error:
            raise AuditError("invalid_utf8") from error
        require("\0" not in text, "embedded_nul")
        return text


def evidence(data, offset=0):
    return {"offset": offset, "size": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def _at(data, offset, fmt):
    return Reader(data, offset).number(fmt)


def pe_overlay(data, *, supported_versions=("9.5",)):
    """Use the PE resource table, not arbitrary XML/signature string searches."""
    require(len(data) >= 64 and data[:2] == b"MZ", "not_pe")
    pe = _at(data, 60, "I")
    header = Reader(data, pe)
    require(header.read(4) == b"PE\0\0", "not_pe")
    header.read(2)
    count = header.number("H")
    header.read(12)
    optional_size = header.number("H")
    header.read(2)
    require(1 <= count <= 96 and optional_size >= 96, "invalid_pe_header")
    optional_start = header.pos
    optional = Reader(data, optional_start, optional_start + optional_size)
    magic = optional.number("H")
    require(magic in (0x10b, 0x20b), "invalid_pe_optional_header")
    directory_offset = 96 if magic == 0x10b else 112
    require(optional_size >= directory_offset + 40, "invalid_pe_directories")
    number_dirs = _at(data, optional_start + directory_offset - 4, "I")
    require(5 <= number_dirs <= 16, "invalid_pe_directories")
    size_headers = _at(data, optional_start + 60, "I")
    sections = []
    table = Reader(data, optional_start + optional_size)
    require(size_headers >= table.pos + count * 40 and size_headers <= len(data), "invalid_pe_headers_size")
    for _ in range(count):
        section = table.read(40)
        virtual_size, rva, raw_size, raw_offset = struct.unpack_from("<IIII", section, 8)
        require(raw_offset + raw_size <= len(data), "truncated_pe_section")
        if raw_size:
            require(raw_offset >= size_headers, "overlapping_pe_headers")
        sections.append((rva, virtual_size, raw_offset, raw_size))
    intervals = sorted((s[2], s[2] + s[3]) for s in sections if s[3])
    require(all(a[1] <= b[0] for a, b in zip(intervals, intervals[1:])), "overlapping_pe_sections")
    overlay = max([size_headers] + [s[2] + s[3] for s in sections])
    resource_rva = _at(data, optional_start + directory_offset + 16, "I")
    resource_size = _at(data, optional_start + directory_offset + 20, "I")
    certificate_offset = _at(data, optional_start + directory_offset + 32, "I")
    certificate_size = _at(data, optional_start + directory_offset + 36, "I")
    certificate = None
    overlay_end = len(data)
    if certificate_offset or certificate_size:
        require(certificate_offset >= overlay and certificate_offset % 8 == 0
                and 8 <= certificate_size <= MAX_METADATA
                and certificate_offset + certificate_size == len(data), "invalid_pe_certificate_range")
        cert = Reader(data, certificate_offset)
        length, revision, kind = cert.number("I"), cert.number("H"), cert.number("H")
        require(8 <= length <= certificate_size and (length + 7) // 8 * 8 == certificate_size
                and revision == 0x200 and kind == 2, "unsupported_pe_certificate")
        cert.read(length - 8)
        require(cert.read(certificate_size - length) == bytes(certificate_size - length), "invalid_certificate_padding")
        certificate = {**evidence(data[certificate_offset:], certificate_offset), "signature_verified": False}
        overlay_end = certificate_offset

    def resolve(rva, size):
        matches = [s[2] + rva - s[0] for s in sections
                   if s[0] <= rva and rva + size <= s[0] + s[3]]
        require(len(matches) == 1, "invalid_resource_rva")
        return matches[0]

    require(0 < resource_size <= MAX_METADATA, "invalid_resource_size")
    resource = resolve(resource_rva, resource_size)

    def directory(relative):
        cursor = Reader(data, resource + relative, resource + resource_size)
        cursor.read(12)
        n = cursor.number("H") + cursor.number("H")
        return [(cursor.number("I"), cursor.number("I")) for _ in range(n)]

    manifests = []
    for kind, node in directory(0):
        if kind != 24:
            continue
        require(node & 0x80000000, "invalid_manifest_directory")
        for _, language_node in directory(node & 0x7fffffff):
            require(language_node & 0x80000000, "invalid_manifest_directory")
            for _, leaf in directory(language_node & 0x7fffffff):
                require(not leaf & 0x80000000, "invalid_manifest_leaf")
                entry = Reader(data, resource + leaf, resource + resource_size)
                rva, size = entry.number("I"), entry.number("I")
                require(0 < size <= MAX_METADATA, "manifest_size_limit")
                offset = resolve(rva, size)
                manifests.append(data[offset:offset + size])
    require(len(manifests) == 1, "ambiguous_or_missing_manifest")
    manifest = manifests[0]
    try:
        text = manifest.decode("utf-8", errors="strict")
        require("<!DOCTYPE" not in text.upper() and "<!ENTITY" not in text.upper(), "unsafe_manifest_xml")
        xml = ElementTree.fromstring(text)
    except (ElementTree.ParseError, UnicodeDecodeError) as error:
        raise AuditError("invalid_manifest_xml") from error
    identities = [n for n in xml if n.tag.rsplit("}", 1)[-1] == "assemblyIdentity"]
    descriptions = [n.text for n in xml if n.tag.rsplit("}", 1)[-1] == "description"]
    version = identities[0].get("version", "") if len(identities) == 1 else ""
    require(re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+", version) is not None
            and ".".join(version.split(".")[:2]) in supported_versions, "unsupported_installer_version")
    require(any(d in ("Setup Factory 9 Run-time", "Setup Factory Run-time") for d in descriptions), "unsupported_installer_brand")
    require(data[overlay:overlay + 16] == SIGNATURE, "unsupported_overlay")
    return overlay, overlay_end, {**evidence(manifest), "version": version}, certificate


def safe_target(directory, basename):
    # Reviewed installers may place files directly in the game root, where the
    # serialized destination is exactly `%AppFolder%` with no trailing separator.
    require(directory == "%AppFolder%" or directory.startswith("%AppFolder%\\"),
            "unsupported_target_variable")
    relative = directory[len("%AppFolder%"):].lstrip("\\")
    name = (relative + "\\" + basename) if relative else basename
    require(0 < len(name) <= 1024 and "/" not in name and ":" not in name
            and "%" not in name and all(ord(c) >= 32 for c in name), "unsafe_target_path")
    for part in name.split("\\"):
        require(part not in ("", ".", "..") and not part.endswith((".", " "))
                and not any(c in '<>"|?*' for c in part), "unsafe_target_path")
        device = part.split(".", 1)[0].upper()
        require(device not in ("CON", "PRN", "AUX", "NUL")
                and not re.fullmatch(r"(?:COM|LPT)[0-9¹²³]", device), "unsafe_target_path")
        require(not part.casefold().startswith(".steam-patch-"), "reserved_target_path")
    require("\\" not in basename, "unsafe_basename")
    return name.replace("\\", "/")


def validate_targets(records):
    names, prefixes = {}, {}
    for record in records:
        name = record["destination"]
        key = unicodedata.normalize("NFC", name).casefold()
        require(key not in names, "duplicate_or_unicode_case_collision")
        names[key] = name
        parts = name.split("/")
        for i in range(1, len(parts) + 1):
            prefix = "/".join(parts[:i])
            prefix_key = unicodedata.normalize("NFC", prefix).casefold()
            require(prefixes.setdefault(prefix_key, prefix) == prefix, "duplicate_or_unicode_case_collision")
    for key in names:
        require(not any("/".join(key.split("/")[:i]) in names
                        for i in range(1, len(key.split("/")))), "file_directory_collision")


def file_table(metadata, *, installer_version="9.5"):
    require(installer_version in ("9.5", "9.6", "9.7"), "unsupported_installer_version")
    marker = metadata.find(b"CSetupFileData")
    require(marker >= 8 and metadata.find(b"CSetupFileData", marker + 1) == -1, "ambiguous_file_table")
    start = marker - 8
    r = Reader(metadata, start)
    count = r.number("H")
    # The count is a genuine u16 field; the serialized records bound how many
    # entries actually exist, so only an empty table is rejected here.
    require(count > 0, "file_count_limit")
    require(r.read(4) == b"\xff\xff\x01\0" and r.number("H") == 14, "unsupported_file_table")
    require(r.read(14) == b"CSetupFileData", "unsupported_file_table")
    table_header = r.read(5)
    records = []
    total = 0
    for index in range(count):
        record_start = r.pos
        source, basename, source_directory, suffix, runtime_folder, description = [r.string() for _ in range(6)]
        opaque = []

        def unknown(n):
            offset = r.pos
            data = r.read(n)
            opaque.append({**evidence(data, offset), "base64": base64.b64encode(data).decode("ascii")})

        unknown(2)
        unpacked = r.number("q")
        require(0 <= unpacked <= MAX_FILE, "file_size_limit")
        total += unpacked
        require(total <= MAX_TOTAL, "expanded_size_limit")
        attributes = r.number("B")
        unknown(4)
        creation = r.number("q")
        unknown(16)
        modified = r.number("q")
        unknown(25)
        destination = r.string()
        unknown(11)
        shortcuts = [r.string() for _ in range(5)]
        unknown(1)
        icon = r.string()
        unknown(8)
        font = r.string()
        # SF 9.7 serializes an additional byte before the compression flags.
        # Keep it as unreviewed evidence: framing support is not semantics approval.
        unknown(4 if installer_version == "9.7" else 3)
        compressed, original_attributes, forced_attributes = [r.number("B") for _ in range(3)]
        require(compressed in (0, 1) and original_attributes in (0, 1), "invalid_file_flags")
        unknown(10)
        unknown(r.number("H") * 2)
        condition = r.string()
        unknown(2)
        install_type, extra = r.string(), r.string()
        packages_count = r.number("H")
        packages = [r.string() for _ in range(packages_count)]
        notes = r.string()
        packed, crc = r.number("q"), r.number("I")
        require(0 <= packed <= MAX_EXE, "invalid_packed_size")
        # The upstream eight-byte skip crosses an object boundary. Preserve it
        # only as unknown inter-record framing; the last record needs no skip.
        record_end = r.pos
        framing = None
        if index + 1 < count:
            framing_offset = r.pos
            framing_bytes = r.read(8)
            framing = {**evidence(framing_bytes, framing_offset),
                       "base64": base64.b64encode(framing_bytes).decode("ascii")}
        records.append({"index": index, "destination": safe_target(destination, basename),
                        "source": source, "source_directory": source_directory,
                        "suffix": suffix, "runtime_folder": runtime_folder, "description": description,
                        "unpacked_size": unpacked, "packed_size": packed, "crc32": crc,
                        "compressed": bool(compressed), "attributes": attributes,
                        "use_original_attributes": original_attributes, "forced_attributes": forced_attributes,
                        "creation_time": creation, "modified_time": modified,
                        "shortcuts": shortcuts, "icon": icon, "font": font, "condition": condition,
                        "install_type": install_type, "extra": extra, "packages": packages, "notes": notes,
                        "serialized_record": evidence(metadata[record_start:record_end], record_start),
                        "unparsed_inter_record_framing": framing,
                        "unreviewed_fields": opaque})
    validate_targets(records)
    return records, {"start": start, "end": r.pos, "end_basis": "last_known_crc_end_not_serialized_object_end",
                     "header_base64": base64.b64encode(table_header).decode(),
                     "unparsed_regions": [evidence(metadata[:start]), evidence(metadata[r.pos:], r.pos)]}


class Decoder:
    def __init__(self):
        self.total = 0
        self.deadline = time.monotonic() + 180

    def decode(self, reader, packed, expected=None, crc=None, retain=False, compressed=True, sink=None):
        limit = MAX_METADATA if retain else MAX_FILE
        require(0 <= packed <= reader.end - reader.pos, "truncated_payload")
        if expected is not None:
            require(0 <= expected <= limit and self.total + expected <= MAX_TOTAL, "expanded_size_limit")
        end = reader.pos + packed
        decoder = None
        if compressed:
            require(packed >= 10, "truncated_lzma2_header")
            prop = reader.number("B")
            require(prop <= 40, "unsupported_compression")
            dictionary = (2 | (prop & 1)) << (prop // 2 + 11) if prop < 40 else 0xffffffff
            require(dictionary <= MAX_DICTIONARY, "dictionary_limit")
            header_size = reader.number("Q")
            require(header_size <= limit, "expanded_size_limit")
            if expected is not None:
                require(header_size == expected, "lzma2_size_mismatch")
            expected = header_size
            require(self.total + expected <= MAX_TOTAL, "expanded_size_limit")
            decoder = lzma.LZMADecompressor(lzma.FORMAT_RAW,
                filters=[{"id": lzma.FILTER_LZMA2, "dict_size": dictionary}])
        digest, checksum, size, parts = hashlib.sha256(), 0, 0, []
        while reader.pos < end or (decoder and not decoder.eof and not decoder.needs_input):
            require(time.monotonic() < self.deadline, "audit_timeout")
            data = reader.read(min(CHUNK, end - reader.pos)) if not decoder or decoder.needs_input else b""
            if decoder:
                try:
                    output = decoder.decompress(data, max_length=min(CHUNK, limit - size + 1))
                except lzma.LZMAError as error:
                    raise AuditError("invalid_lzma2_stream") from error
            else:
                output = data
            size += len(output)
            self.total += len(output)
            require(size <= limit and self.total <= MAX_TOTAL, "expanded_size_limit")
            require(expected is None or size <= expected, "payload_size_mismatch")
            digest.update(output)
            checksum = zlib.crc32(output, checksum)
            if retain:
                parts.append(output)
            if sink is not None:
                sink.write(output)
            if decoder and decoder.eof:
                require(not decoder.unused_data and reader.pos == end, "trailing_compressed_data")
                break
        require(decoder is None or decoder.eof, "truncated_lzma2_stream")
        require(expected is None or size == expected, "payload_size_mismatch")
        require(crc is None or checksum == crc, "payload_crc_mismatch")
        return {"size": size, "crc32": checksum, "sha256": digest.hexdigest()}, b"".join(parts)


def audit(data, *, supported_versions=("9.5",), metadata_inspector=None):
    require(0 < len(data) <= MAX_EXE, "exe_size_limit")
    overlay, overlay_end, manifest, certificate = pe_overlay(data, supported_versions=supported_versions)
    installer_version = ".".join(manifest["version"].split(".")[:2])
    r = Reader(data, overlay, overlay_end)
    require(r.read(16) == SIGNATURE, "unsupported_overlay")
    overlay_header = r.read(10)
    decoder = Decoder()
    diagnostics = []
    # No embedded engine is written or loaded. These two blobs are just hashed.
    for name in ("irsetup.exe", "lua5.1.dll"):
        size = r.number("q")
        require(0 < size <= MAX_FILE and decoder.total + size <= MAX_TOTAL, "runtime_size_limit")
        offset = r.pos
        info, _ = decoder.decode(r, size, size, compressed=False)
        diagnostics.append({"name": name, "offset": offset, "packed_size": size,
                            "hashed_representation": "stored_blob", **info})
    count = r.number("I")
    require(count == 3, "unsupported_metadata_records")
    metadata = None
    for expected_name in ("irsetup.dat", "IRIMG1.BMP", "IRIMG2.BMP"):
        raw_name = r.read(264)
        name = raw_name.split(b"\0", 1)[0]
        require(name == expected_name.encode() and raw_name[len(name):] == bytes(264 - len(name)), "unsupported_metadata_name")
        size, crc = r.number("q"), r.number("I")
        unknown = r.read(4)
        offset = r.pos
        info, content = decoder.decode(r, size, crc=crc, retain=expected_name == "irsetup.dat")
        diagnostics.append({"name": expected_name, "offset": offset, "packed_size": size,
                            "unknown_header_base64": base64.b64encode(unknown).decode(), **info})
        if expected_name == "irsetup.dat":
            metadata = content
    script_inventory = metadata_inspector(metadata) if metadata_inspector else None
    records, table = file_table(metadata, installer_version=installer_version)
    # The complete directory/paths/counts and compressed ranges are checked first.
    payload_end = r.pos + sum(row["packed_size"] for row in records)
    padding = (-payload_end) % 8 if certificate is not None else 0
    require(payload_end + padding == r.end, "overlay_size_or_tail_mismatch")
    require(decoder.total + sum(row["unpacked_size"] for row in records) <= MAX_TOTAL, "expanded_size_limit")
    for record in records:
        record["payload_offset"] = r.pos
        info, _ = decoder.decode(r, record["packed_size"], record["unpacked_size"],
                                 record["crc32"], compressed=record["compressed"])
        record["payload"] = info
    require(r.read(padding) == bytes(padding) and r.pos == overlay_end, "trailing_overlay_data")
    result = {"schema_version": 1, "auditor": VERSION, "exe": evidence(data), "installer": "Setup Factory " + installer_version,
            "overlay_offset": overlay, "overlay_header_base64": base64.b64encode(overlay_header).decode(),
            "manifest": manifest, "certificate": certificate, "certificate_alignment_bytes": padding,
            "diagnostics": diagnostics, "metadata": evidence(metadata),
            "file_table": table, "files": records, "expanded_total": decoder.total,
            "capability": "discover_only", "installable": False, "semantics_reviewed": False,
            "blockers": ["unparsed_installer_events", "unreviewed_serialized_file_fields",
                         "game_language_build_dlc_not_verified", "steamos_install_restore_not_verified"]}
    if script_inventory is not None:
        result["script_inventory"] = script_inventory
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("sample", type=Path)
    parser.add_argument("--metrics", action="store_true", help="Write worker timing/RSS to stderr (Linux).")
    parser.add_argument("--sf9", action="store_true", help="Inspect SF9.5/9.6/9.7 with incomplete script inventory; never authorizes installation.")
    args = parser.parse_args()
    started = time.monotonic()
    try:
        # Bound the development worker too; production sandbox remains a P2 gate.
        if sys.platform == "linux":
            import resource
            for key, value in ((resource.RLIMIT_AS, 3 * 1024 ** 3), (resource.RLIMIT_CPU, 180),
                               (resource.RLIMIT_FSIZE, MAX_METADATA), (resource.RLIMIT_CORE, 0)):
                _, hard = resource.getrlimit(key)
                bounded = min(value, hard) if hard != resource.RLIM_INFINITY else value
                resource.setrlimit(key, (bounded, bounded))
        with args.sample.open("rb") as stream:
            require(0 < stream.seek(0, 2) <= MAX_EXE, "exe_size_limit")
            with mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as data:
                if args.sf9:
                    from sf9_metadata import inventory
                    report = audit(data, supported_versions=("9.5", "9.6", "9.7"), metadata_inspector=inventory)
                else:
                    report = audit(data)
        encoded = json.dumps(report, ensure_ascii=True, separators=(",", ":"))
        require(len(encoded) + 1 <= MAX_METADATA, "report_size_limit")
        sys.stdout.write(encoded + "\n")
        sys.stdout.flush()
        if args.metrics and sys.platform == "linux":
            sys.stderr.write(json.dumps({"seconds": round(time.monotonic() - started, 3),
                "worker_peak_rss_kib": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss}) + "\n")
    except (AuditError, OSError, MemoryError) as error:
        json.dump({"auditor": VERSION, "installable": False, "error": str(error)}, sys.stdout)
        sys.stdout.write("\n")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
