"""Fixed-path sandbox worker. Converts only byte/metadata/payload-locked inputs.

Backend supplies a reviewed descriptor on stdin; neither package paths nor a
frontend URL/path/recipe are executable instructions. Run under exe_worker.py.
This developer entry point does not grant a production profile approval.
"""
import hashlib
import json
import mmap
import os
from pathlib import Path
import sys
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
import sf95_audit as sf

INPUT = Path("/input/archive.exe")
OUTPUT = Path("/output/normalized.zip")


def behavior_digest(value):
    return hashlib.sha256(json.dumps(value, ensure_ascii=True, sort_keys=True,
        separators=(",", ":")).encode()).hexdigest()


def audit_behavior(data, *, decode_payloads=True):
    from sf9_graph import decode
    from sf9_behavior import lower
    _, _, manifest, _ = sf.pe_overlay(data, supported_versions=("9.5", "9.6", "9.7"))
    version = ".".join(manifest["version"].split(".")[:2])
    report = sf.audit(data, supported_versions=("9.5", "9.6", "9.7"),
        metadata_inspector=lambda metadata: decode(metadata, version), decode_payloads=decode_payloads)
    report["behavior"] = lower(report["script_inventory"], report["files"], version)
    report["behavior_sha256"] = behavior_digest(report["behavior"])
    report["capability"] = "behavior_supported"
    report["blockers"] = ["game_and_source_conditions_pending"]
    return report


def analyze(stream, descriptor):
    """Inspect a content-bound package without writing a normalized artifact.

    This protocol cannot set semantics_approved or bypass the reviewed legacy
    converter. The partial script inventory explicitly refuses installation.
    """
    sf.require(isinstance(descriptor, dict) and set(descriptor) == {
        "action", "exe_sha256", "exe_size"} and descriptor["action"] in ("analyze", "analyze_behavior"), "invalid_descriptor")
    sf.require(isinstance(descriptor["exe_sha256"], str)
        and len(descriptor["exe_sha256"]) == 64
        and all(c in "0123456789abcdef" for c in descriptor["exe_sha256"]), "invalid_descriptor")
    before = os.fstat(stream.fileno())
    sf.require(type(descriptor["exe_size"]) is int and before.st_size == descriptor["exe_size"]
        and 0 < before.st_size <= sf.MAX_EXE, "exe_size_mismatch")
    from sf9_metadata import inventory
    with mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as data:
        sf.require(hashlib.sha256(data).hexdigest() == descriptor["exe_sha256"], "exe_payload_changed")
        report = audit_behavior(data, decode_payloads=False) if descriptor["action"] == "analyze_behavior" else sf.audit(data, supported_versions=("9.5", "9.6", "9.7"), metadata_inspector=inventory)
        sf.require(report["exe"]["sha256"] == descriptor["exe_sha256"], "exe_payload_changed")
    after = os.fstat(stream.fileno())
    identity = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns)
    sf.require(identity(before) == identity(after), "exe_changed_during_extraction")
    return {"schema_version": 1, "action": descriptor["action"], "exe_sha256": descriptor["exe_sha256"],
            "report": report, "installable": False}


def convert(stream, descriptor, output):
    generic = isinstance(descriptor, dict) and descriptor.get("action") == "convert_behavior"
    expected_keys = {"exe_sha256", "exe_size", "metadata_sha256", "payload"}
    expected_keys |= {"action", "behavior_sha256"} if generic else {"semantics_approved"}
    sf.require(isinstance(descriptor, dict) and set(descriptor) == expected_keys, "invalid_descriptor")
    if not generic:
        sf.require(descriptor["semantics_approved"] is True, "exe_profile_not_reviewed")
    before = os.fstat(stream.fileno())
    sf.require(before.st_size == descriptor["exe_size"] and 0 < before.st_size <= sf.MAX_EXE, "exe_size_mismatch")
    partial = output.with_name(output.name + ".partial")
    sf.require(not output.exists() and not partial.exists(), "worker_output_exists")
    owned_partial = False
    try:
        with mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as data:
            report = audit_behavior(data) if generic else sf.audit(data)
            if generic:
                sf.require(report["behavior_sha256"] == descriptor["behavior_sha256"], "exe_behavior_changed")
            sf.require(report["exe"]["sha256"] == descriptor["exe_sha256"], "exe_payload_changed")
            sf.require(report["metadata"]["sha256"] == descriptor["metadata_sha256"], "exe_metadata_changed")
            manifest = [{"path": r["destination"], "size": r["unpacked_size"],
                         "sha256": r["payload"]["sha256"]} for r in report["files"]]
            sf.require(isinstance(descriptor["payload"], list)
                       and sorted(manifest, key=lambda r: r["path"]) == sorted(descriptor["payload"], key=lambda r: r["path"]),
                       "exe_manifest_changed")
            # All paths, framing, CRCs and the complete reviewed manifest have
            # passed before the first write into the owned output directory.
            decoder = sf.Decoder()
            with partial.open("xb") as target:
                owned_partial = True
                with zipfile.ZipFile(target, "w", compression=zipfile.ZIP_STORED, allowZip64=True) as archive:
                    for row in sorted(report["files"], key=lambda r: r["destination"]):
                        entry = zipfile.ZipInfo(row["destination"], date_time=(1980, 1, 1, 0, 0, 0))
                        entry.compress_type = zipfile.ZIP_STORED
                        entry.create_system = 3
                        entry.external_attr = 0o100644 << 16
                        entry.file_size = row["unpacked_size"]
                        with archive.open(entry, "w", force_zip64=True) as sink:
                            observed, _ = decoder.decode(sf.Reader(data, row["payload_offset"]), row["packed_size"],
                                row["unpacked_size"], row["crc32"], compressed=row["compressed"], sink=sink)
                        sf.require(observed == row["payload"], "exe_payload_changed")
                target.flush()
                os.fsync(target.fileno())
            after = os.fstat(stream.fileno())
            identity = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns)
            sf.require(identity(before) == identity(after), "exe_changed_during_extraction")
        digest = hashlib.sha256()
        size = 0
        with partial.open("rb") as archive:
            while data := archive.read(sf.CHUNK):
                digest.update(data)
                size += len(data)
                sf.require(size <= sf.MAX_TOTAL + sf.MAX_METADATA, "normalized_zip_limit")
        os.replace(partial, output)
        return {"extractor": sf.VERSION, "exe_sha256": descriptor["exe_sha256"],
            "metadata_sha256": descriptor["metadata_sha256"], "artifact_sha256": digest.hexdigest(),
            "artifact_size": size, "files": manifest}
    finally:
        if owned_partial:
            partial.unlink(missing_ok=True)


def main():
    published = False
    try:
        sf.require(sys.platform == "linux" and os.geteuid() != 0, "worker_requires_linux_user")
        import resource
        for key, value in ((resource.RLIMIT_AS, 3 * 1024 ** 3), (resource.RLIMIT_CPU, 300),
                           (resource.RLIMIT_FSIZE, sf.MAX_TOTAL + sf.MAX_METADATA), (resource.RLIMIT_CORE, 0)):
            _, hard = resource.getrlimit(key)
            limit = min(hard, value) if hard != resource.RLIM_INFINITY else value
            resource.setrlimit(key, (limit, limit))
        raw = sys.stdin.buffer.read(sf.MAX_METADATA + 1)
        sf.require(len(raw) <= sf.MAX_METADATA, "descriptor_size_limit")
        descriptor = json.loads(raw)
        with INPUT.open("rb") as source:
            if isinstance(descriptor, dict) and descriptor.get("action") in ("analyze", "analyze_behavior"):
                result = analyze(source, descriptor)
            else:
                result = convert(source, descriptor, OUTPUT)
                published = True
        encoded = json.dumps(result, ensure_ascii=True, sort_keys=True, separators=(",", ":"))
        sf.require(len(encoded) <= sf.MAX_METADATA, "worker_response_limit")
        print(encoded)
        return 0
    except (sf.AuditError, OSError, ValueError, MemoryError, KeyError, TypeError) as error:
        print(json.dumps({"error": str(error)}, ensure_ascii=True))
        if published:
            OUTPUT.unlink(missing_ok=True)
        return 1


if __name__ == "__main__":
    sys.exit(main())
