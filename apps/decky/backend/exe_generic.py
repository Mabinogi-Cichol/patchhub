"""Generic official EXE selection, static analysis and instance predicates."""
import asyncio
from datetime import datetime
import hashlib
import json
from pathlib import Path
import re
from threading import Event
import time
import uuid

from acquisition import no_links
from discovery import safe_url
from exe_facts import game_facts, read_file, MAX_BASELINE
from exe_profiles import candidate_binding, digest, _path
from exe_worker import analyze_behavior
from publisher import _download, _hash_file, _safe_dir, _json_new
from tasks import current_task

SELECTION = r"exe-selection-[0-9a-f]{24}-[1-9][0-9]*"
CONTENT = r"exe-content-[0-9a-f]{24}-[1-9][0-9]*"


def selection_id(candidate):
    return f"exe-selection-{candidate_binding(candidate)[:24]}-{candidate['game_appid']}"


def candidate(registry, game, selection):
    matches = [c for c in registry.sources.cached_candidates(game)
               if c.get("game_appid") == game.get("appid") and selection_id(c) == selection]
    if len(matches) != 1: raise ValueError("exe_candidate_changed")
    value = matches[0]
    if value.get("match_basis") not in ("appid", "verified_alias", "official_title"):
        raise ValueError("exe_game_identity_unverified")
    safe_url(value["source_page"], registry.rule(value["source_id"], game["appid"])["page_hosts"])
    if (value.get("format_reported", "unknown") not in ("exe", "unknown")
            or value.get("format_observed", "unknown") not in ("exe", "unknown")
            or "exe" not in (value.get("format_reported"), value.get("format_observed"))):
        raise ValueError("exe_format_unconfirmed")
    observed = value.get("observed_at")
    if isinstance(observed, str):
        try:
            parsed = datetime.fromisoformat(observed.replace("Z", "+00:00"))
            observed = parsed.timestamp() if parsed.tzinfo else None
        except ValueError: observed = None
    now = time.time()
    if type(observed) not in (int, float) or not now-21600 <= observed <= now+60 or value.get("cache_status") not in ("fresh", "cached"):
        raise ValueError("exe_candidate_stale")
    if len(value.get("languages", [])) != 1 or value.get("variant") not in ("base", "dlc"):
        raise ValueError("exe_source_conditions_unresolved")
    if not mirrors(registry, value): raise ValueError("exe_no_direct_download")
    return dict(value)


def mirrors(registry, value):
    hosts = registry.rule(value["source_id"], value["game_appid"])["download_hosts"]
    result = []
    for mirror in value.get("mirrors", []):
        if mirror.get("format") != "exe": continue
        try: checked = safe_url(mirror.get("url"), hosts)
        except ValueError: continue
        from urllib.parse import urlsplit
        url = urlsplit(checked)
        if url.query or url.fragment or not url.path.lower().endswith(".exe"): continue
        if checked not in result: result.append(checked)
    return result


def placeholder(game, value):
    return {"generic": True, "profile_id": "generic-selection", "revision": 1,
            "appid": game["appid"], "bound_instance": game["instance_id"],
            "source_id": value["source_id"], "source_record_id": value["source_record_id"],
            "source_page": value["source_page"], "language": value["languages"][0],
            "variant": value["variant"], "version_identity": value["version_identity"],
            "verified_buildids": [game["buildid"]], "requires_dlc": value.get("requires_dlc") or [],
            "excludes_dlc": value.get("excludes_dlc") or [], "baselines": [],
            "terms": {"url": value["source_page"], "text": "", "sha256": hashlib.sha256(b"").hexdigest()}}


def facts(game, profile, cancel=None):
    # Current build is recorded for TOCTOU detection; no registry of approved
    # builds is consulted. Source language/DLC constraints are still enforced.
    manifest_profile = dict(profile, baselines=[])
    result = game_facts(game, manifest_profile, cancel)
    snapshots = {}
    def file(path):
        _path(path)
        if path not in snapshots:
            target = Path(game["install_path"]).joinpath(*path.split("/"))
            if not no_links(target): raise ValueError("linked_target")
            if target.exists():
                if not target.is_file(): raise ValueError("exe_condition_not_file")
                snapshots[path] = {"path": path, "exists": True, **read_file(target, MAX_BASELINE, cancel)}
            else: snapshots[path] = {"path": path, "exists": False}
        return snapshots[path]
    budget = [0]
    def evaluate(node):
        budget[0] += 1
        if budget[0] > 8192: raise ValueError("exe_condition_ir_limit")
        if type(node) is bool: return node
        if not isinstance(node, (list, tuple)) or len(node) < 2 or node[0] != "symbol": raise ValueError("exe_condition_ir_invalid")
        kind = node[1]
        if kind == "exists" and len(node) == 3: return file(node[2])["exists"]
        if kind == "crc32" and len(node) == 3:
            observed = file(node[2])
            if not observed["exists"]: raise ValueError("exe_installer_condition_mismatch")
            return observed["crc32"]
        if kind in ("and", "or") and len(node) == 4:
            left = evaluate(node[2])
            if type(left) is not bool: raise ValueError("exe_condition_ir_invalid")
            if (kind == "and" and not left) or (kind == "or" and left): return left
            right = evaluate(node[3])
            if type(right) is not bool: raise ValueError("exe_condition_ir_invalid")
            return right
        if kind == "not" and len(node) == 3:
            value = evaluate(node[2])
            if type(value) is not bool: raise ValueError("exe_condition_ir_invalid")
            return not value
        if kind in ("==", "~=") and len(node) == 4:
            operands = [evaluate(x) if isinstance(x, (list, tuple)) else x for x in node[2:]]
            if not all(type(x) is int for x in operands): raise ValueError("exe_condition_ir_invalid")
            return operands[0] == operands[1] if kind == "==" else operands[0] != operands[1]
        raise ValueError("exe_condition_ir_invalid")
    condition = profile.get("behavior", {}).get("condition")
    if evaluate(condition) is not True: raise ValueError("exe_installer_condition_mismatch")
    result["baselines"] = [snapshots[key] for key in sorted(snapshots)]
    result["behavior_sha256"] = profile["behavior_sha256"]
    return result


def routing(service, game, content):
    """Persisted routing identifies a source, never authorizes installation."""
    from exe_installation import _read_json
    path = service.publisher._state(game["instance_id"], content) / "generic-source.json"
    value = _read_json(path, 8192)
    if (set(value) != {"schema_version", "instance_id", "release_id", "selection_id", "candidate_sha256"}
            or value["schema_version"] != 1 or value["instance_id"] != game["instance_id"]
            or value["release_id"] != content or not re.fullmatch(SELECTION, value["selection_id"])):
        raise ValueError("exe_policy_invalid")
    current = candidate(service.profiles, game, value["selection_id"])
    if candidate_binding(current) != value["candidate_sha256"]: raise ValueError("exe_candidate_changed")
    return dict(placeholder(game, current), selection_id=value["selection_id"],
                candidate_sha256=value["candidate_sha256"], analysis_pending=True)


async def prepare_analysis(service, game, selection, cancel, expected_content=None):
    registry = service.profiles
    value = candidate(registry, game, selection)
    basic = placeholder(game, value)
    await asyncio.to_thread(game_facts, game, basic, cancel)
    state = _safe_dir(service.publisher._state(game["instance_id"], selection))
    from exe_installation import _overlap
    _overlap(state, Path(game["install_path"]))
    import shutil
    if shutil.disk_usage(state).free < 2*1024**3 + 16*1024**2:
        raise ValueError("exe_state_insufficient_space")
    cache = _safe_dir(state / "cache")
    output = _safe_dir(state / "exe-work" / str(uuid.uuid4()))
    _json_new(output / "owner.json", {"schema_version": 1, "job_id": output.name})
    task = current_task.get()
    try:
        hosts = registry.rule(value["source_id"], game["appid"])["download_hosts"]
        def validate(url):
            checked = safe_url(url, hosts)
            from urllib.parse import urlsplit
            parsed = urlsplit(checked)
            if parsed.query or parsed.fragment or not parsed.path.lower().endswith(".exe"): raise ValueError("exe_download_url_invalid")
            return checked
        error = None
        for url in mirrors(registry, value):
            try:
                archive, sha, size, acquisition = await asyncio.to_thread(_download, url, cache, cancel, validate,
                    archive_name="archive.exe", manifest_name="exe-download.json", max_size=2*1024**3)
                break
            except (ValueError, OSError) as failed:
                error = failed
                if cancel.is_set(): raise RuntimeError("download_cancelled")
        else: raise error or ValueError("exe_no_direct_download")
        if task: task.stage("analyzing_installer")
        report = await analyze_behavior(service.publisher.bridge, service.tool_dir, archive, output, sha, size, cancel)
        if cancel.is_set(): raise RuntimeError("download_cancelled")
        current = candidate(registry, game, selection)
        if candidate_binding(current) != candidate_binding(value): raise ValueError("exe_candidate_changed")
        profile = dict(basic, profile_id="generic-behavior", exe_sha256=sha, exe_size=size,
            metadata_sha256=report["metadata"]["sha256"], installer_version=report["installer"].split()[-1],
            compression="lzma2", behavior=report["behavior"], behavior_sha256=report["behavior_sha256"],
            candidate_sha256=candidate_binding(value), selection_id=selection,
            payload=[{"path": r["destination"], "size": r["unpacked_size"], "sha256": r["payload"]["sha256"]} for r in report["files"]],
            download_hosts=hosts, analysis_acquisition={"url": url, "mode": acquisition})
        terms = "\n\n".join(report["behavior"]["terms"])
        profile["terms"] = {"url": value["source_page"], "text": terms, "sha256": hashlib.sha256(terms.encode()).hexdigest()}
        snapshot = await asyncio.to_thread(facts, game, profile, cancel)
        # User-selected one-step installation maps license/navigation UI away.
        # Parsed display text stays in internal evidence, never in the UI DTO.
        identity = {key: profile[key] for key in ("appid", "source_id", "source_record_id", "language", "variant", "exe_sha256", "metadata_sha256", "payload")}
        content = f"exe-content-{digest(identity)[:24]}-{game['appid']}"
        if expected_content is not None and content != expected_content:
            raise ValueError("exe_payload_changed")
        if await asyncio.to_thread(facts, game, profile, cancel) != snapshot: raise ValueError("exe_game_facts_changed")
        final = _safe_dir(service.publisher._state(game["instance_id"], content) / "cache")
        if await asyncio.to_thread(_hash_file, archive, 2*1024**3, cancel, False) != (sha, size): raise ValueError("exe_payload_changed")
        import os
        for name in ("archive.exe", "exe-download.json"):
            destination = final/name
            if not no_links(destination): raise ValueError("linked_state_path")
            os.replace(cache/name, destination)
        registry._generic_profiles[(game["instance_id"], content)] = profile
        # This small source association survives successful cache cleanup and
        # process restarts. Reinstall still performs the full worker analysis.
        association = {"schema_version": 1, "instance_id": game["instance_id"], "release_id": content,
                       "selection_id": selection, "candidate_sha256": candidate_binding(value)}
        association_path = final.parent / "generic-source.json"
        if association_path.exists():
            from exe_installation import _read_json
            if _read_json(association_path, 8192) != association: raise ValueError("exe_candidate_changed")
        else: _json_new(association_path, association)
        if task: task.update(content_release_id=content)
        return content
    finally:
        from exe_installation import _remove_work
        await asyncio.to_thread(_remove_work, state, output)
