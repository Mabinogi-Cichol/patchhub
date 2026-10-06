"""Enumerate and delete tool-owned download caches.

Only files that live inside the tool state directory and are named by the
tool itself are considered. User ZIPs in ``~/Downloads``, non-digest files in
the shared downloads directory, and transaction state are never touched.
"""
import os
import json
from pathlib import Path
import re
import shutil
import time
import sqlite3
import hashlib
import uuid
from contextlib import closing

from acquisition import no_links
from publisher import cache_files, _unlink_within

CORE_CACHE_NAME = re.compile(r"[0-9a-f]{64}\.zip\Z")


def scan(publisher, downloads_root):
    """Return (kind, root, path) for every tool-owned download cache file."""
    items = []
    for _release, _state, cache in publisher.caches():
        if _release.startswith("exe-") and exe_cache_leased(_state):
            continue
        items.extend(("publisher", cache, path) for path in cache_files(cache))
    if downloads_root.is_dir() and no_links(downloads_root):
        try:
            with os.scandir(downloads_root) as entries:
                for entry in entries:
                    if entry.is_file(follow_symlinks=False) and (
                            CORE_CACHE_NAME.fullmatch(entry.name) or entry.name.endswith(".partial")):
                        items.append(("core", downloads_root, downloads_root / entry.name))
        except OSError:
            pass
    return items


def status(publisher, downloads_root):
    publisher_bytes = core_bytes = 0
    count = 0
    for kind, _root, path in scan(publisher, downloads_root):
        try:
            size = path.stat(follow_symlinks=False).st_size
        except OSError:
            continue
        count += 1
        if kind == "core":
            core_bytes += size
        else:
            publisher_bytes += size
    return {"total_bytes": publisher_bytes + core_bytes, "file_count": count,
            "publisher_bytes": publisher_bytes, "core_bytes": core_bytes}


def clear(publisher, downloads_root):
    """Delete every scanned cache file; return bytes actually freed."""
    removed = 0
    for _kind, root, path in scan(publisher, downloads_root):
        try:
            size = path.stat(follow_symlinks=False).st_size
        except OSError:
            size = 0
        if _unlink_within(root, path):
            removed += size
    return removed


def clear_core(downloads_root, sha256):
    """Delete one digest-named artifact cache, if present."""
    if not isinstance(sha256, str) or not re.fullmatch(r"[0-9a-f]{64}", sha256):
        return 0
    return 1 if _unlink_within(downloads_root, downloads_root / f"{sha256}.zip") else 0


def job_roots(publisher, state_dir):
    """Yield the transaction job stores the plugin owns."""
    yield "base", Path(state_dir) / "jobs"
    for state in publisher.states():
        yield "publisher", state / "jobs"


def _job_files(jobs_root):
    if not jobs_root.is_dir() or not no_links(jobs_root):
        return
    try:
        with os.scandir(jobs_root) as entries:
            jobs = list(entries)
    except OSError:
        return
    for job in jobs:
        if not job.is_dir(follow_symlinks=False):
            continue
        try:
            with os.scandir(job.path) as iterator:
                entries = list(iterator)
        except OSError:
            continue
        for entry in entries:
            if entry.is_file(follow_symlinks=False):
                yield Path(entry.path)


def jobs_status(publisher, state_dir):
    """Return staged/backup bytes kept for unfinished transaction recovery."""
    stage_bytes = backup_bytes = 0
    for _kind, jobs_root in job_roots(publisher, state_dir):
        for path in _job_files(jobs_root):
            try:
                size = path.stat(follow_symlinks=False).st_size
            except OSError:
                continue
            if path.name.endswith(".stage"):
                stage_bytes += size
            elif path.name.endswith(".backup"):
                backup_bytes += size
    return {"stage_bytes": stage_bytes, "backup_bytes": backup_bytes}


def exe_cache_leased(state):
    """Honor a persisted preparation lease; damaged receipts fail closed."""
    jobs = Path(state) / "jobs"
    if not jobs.exists():
        return False
    if not jobs.is_dir() or not no_links(jobs):
        return True
    try:
        with os.scandir(jobs) as entries:
            items = list(entries)
        for job in items:
            if not job.is_dir(follow_symlinks=False):
                continue
            path = Path(job.path) / "exe-policy.json"
            if not path.is_file() or not no_links(path) or path.stat().st_size > 65536:
                if _ended_exe_job(Path(state), job.name):
                    continue
                return True
            try:
                policy = json.loads(path.read_text(encoding="utf-8"))
            except (ValueError, UnicodeDecodeError):
                if _ended_exe_job(Path(state), job.name):
                    continue
                return True
            if not isinstance(policy, dict):
                return True
            if (type(policy.get("schema_version")) is not int or policy["schema_version"] not in (1, 2)
                    or policy.get("kind") != "publisher_exe" or policy.get("plan_id") != job.name
                    or policy.get("release_id") != Path(state).parent.name
                    or not isinstance(policy.get("instance_id"), str)
                    or hashlib.sha256(policy["instance_id"].encode()).hexdigest()[:16] != Path(state).name
                    or type(policy.get("lease_active")) is not bool
                    or type(policy.get("expires_at")) is not int or policy["expires_at"] < 0
                    or (policy["schema_version"] == 1 and policy.get("keep_backups") is not True)):
                if _ended_exe_job(Path(state), job.name):
                    continue
                return True
            if policy.get("lease_active") is True and policy.get("expires_at", 0) > time.time():
                return True
    except (OSError, ValueError, TypeError):
        return True
    return False


def _ended_exe_job(state, plan_id):
    """Read bounded core evidence for a lease lost during a preparation crash.

    This only permits deleting acquisition cache. Recovery files and the journal
    remain protected regardless of this result. Unknown or active jobs stay pinned.
    """
    database = state / "journal.sqlite"
    try:
        if str(uuid.UUID(plan_id)) != plan_id or not no_links(database) or not database.is_file():
            return False
        with closing(sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True, timeout=.1)) as connection:
            connection.set_progress_handler(lambda: 1, 10000)
            row = connection.execute("SELECT document FROM jobs WHERE id=? AND length(document)<8388608", (plan_id,)).fetchone()
        if not row:
            return False
        job = json.loads(row[0])
        plan, transaction = job["plan"], job["transaction"]
        if (plan["plan_id"] != plan_id or transaction["plan_id"] != plan_id
                or transaction["transaction_id"] != plan_id
                or plan["release_id"] != state.parent.name
                or transaction["release_id"] != plan["release_id"]
                or transaction["instance_id"] != plan["instance_id"]
                or hashlib.sha256(plan["instance_id"].encode()).hexdigest()[:16] != state.name):
            return False
        if transaction["state"] in ("rolled_back", "uninstalled", "committed", "superseded"):
            return True
        created, expires = plan["created_at"], plan["expires_at"]
        return (transaction["state"] == "planned" and type(created) is int and type(expires) is int
                and 0 <= created <= expires <= created + 900 and expires <= time.time())
    except (OSError, sqlite3.Error, ValueError, TypeError, KeyError):
        return False
