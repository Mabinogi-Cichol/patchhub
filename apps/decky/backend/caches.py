"""Enumerate and delete tool-owned download caches.

Only files that live inside the tool state directory and are named by the
tool itself are considered. User ZIPs in ``~/Downloads``, non-digest files in
the shared downloads directory, and transaction state are never touched.
"""
import os
from pathlib import Path
import re
import shutil

from acquisition import no_links
from publisher import cache_files, _unlink_within

CORE_CACHE_NAME = re.compile(r"[0-9a-f]{64}\.zip\Z")


def scan(publisher, downloads_root):
    """Return (kind, root, path) for every tool-owned download cache file."""
    items = []
    for _release, _state, cache in publisher.caches():
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
        jobs = list(os.scandir(jobs_root))
    except OSError:
        return
    for job in jobs:
        if not job.is_dir(follow_symlinks=False):
            continue
        try:
            entries = list(os.scandir(job.path))
        except OSError:
            continue
        for entry in entries:
            if entry.is_file(follow_symlinks=False):
                yield Path(entry.path)


def jobs_status(publisher, state_dir):
    """Return staged/backup bytes kept for transaction recovery and uninstall."""
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


def delete_files(jobs_root, plan_id, suffixes=(".stage",)):
    """Remove selected transaction files; backups are only removed when asked."""
    if not isinstance(plan_id, str) or not plan_id:
        return 0
    job = Path(jobs_root) / plan_id
    if not no_links(job) or not job.is_dir():
        return 0
    removed = 0
    try:
        for entry in os.scandir(job):
            if not entry.is_file(follow_symlinks=False):
                continue
            if not any(entry.name.endswith(suffix) for suffix in suffixes):
                continue
            size = entry.stat(follow_symlinks=False).st_size
            if _unlink_within(Path(jobs_root), Path(entry.path)):
                removed += size
    except OSError:
        pass
    return removed


def delete_stage_files(jobs_root, plan_id):
    """Remove one committed plan's staged copies; backups stay for uninstall."""
    return delete_files(jobs_root, plan_id, (".stage",))


def has_backups(jobs_root, transaction_id):
    if not isinstance(transaction_id, str) or not transaction_id:
        return False
    job = Path(jobs_root) / transaction_id
    if not no_links(job) or not job.is_dir():
        return False
    try:
        return any(entry.is_file(follow_symlinks=False) and entry.name.endswith(".backup")
                   for entry in os.scandir(job))
    except OSError:
        return False


def any_backups(jobs_root):
    """True when any transaction under ``jobs_root`` still holds a backup file."""
    jobs_root = Path(jobs_root)
    if not jobs_root.is_dir() or not no_links(jobs_root):
        return False
    try:
        for job in os.scandir(jobs_root):
            if not job.is_dir(follow_symlinks=False):
                continue
            for entry in os.scandir(job.path):
                if entry.is_file(follow_symlinks=False) and entry.name.endswith(".backup"):
                    return True
    except OSError:
        pass
    return False


def forget_transactions(state_dir):
    """Drop a core transaction store so a backup-less install no longer blocks.

    Recipe, signed catalog and keys are kept; only the journal database and the
    job store are removed. The core recreates both on its next run.
    """
    state_dir = Path(state_dir)
    if not state_dir.is_dir() or not no_links(state_dir):
        return False
    try:
        entries = list(os.scandir(state_dir))
    except OSError:
        return False
    for entry in entries:
        if entry.name == "journal.sqlite" or entry.name.startswith("journal.sqlite-"):
            if entry.is_file(follow_symlinks=False):
                _unlink_within(state_dir, Path(entry.path))
    jobs = state_dir / "jobs"
    if jobs.is_dir() and no_links(jobs):
        shutil.rmtree(jobs, ignore_errors=True)
    return True


def clear_jobs(jobs_root, transaction_ids, suffixes=(".stage", ".backup")):
    """Delete staged/backup files for the given transactions; return bytes freed."""
    removed = 0
    root = Path(jobs_root)
    for transaction_id in transaction_ids:
        if not isinstance(transaction_id, str) or not transaction_id:
            continue
        job = root / transaction_id
        if not no_links(job) or not job.is_dir():
            continue
        try:
            entries = list(os.scandir(job))
        except OSError:
            continue
        for entry in entries:
            if not entry.is_file(follow_symlinks=False):
                continue
            if not any(entry.name.endswith(suffix) for suffix in suffixes):
                continue
            size = entry.stat(follow_symlinks=False).st_size
            if _unlink_within(root, Path(entry.path)):
                removed += size
    return removed
