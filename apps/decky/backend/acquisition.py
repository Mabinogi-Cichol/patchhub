"""Discover local ZIP candidates. The core remains the installation authority."""
import base64
import hashlib
import json
import os
from pathlib import Path
import re
from threading import Event
from urllib.parse import unquote, urlsplit
from tasks import current_task

MAX_ARCHIVE_BYTES = 8 * 1024 * 1024 * 1024


def release_metadata(envelope_path: Path, revision: int, release_id: str):
    # Read the same envelope used by the core's signed scan. These hints only
    # select a file: plan_install independently authenticates it and every file.
    if envelope_path.stat().st_size > 8 * 1024 * 1024:
        raise ValueError("catalog_too_large")
    envelope = json.loads(envelope_path.read_text(encoding="utf-8"))
    payload = json.loads(base64.b64decode(envelope["payload"], validate=True))
    catalog = payload["catalog"]
    if catalog["revision"] != revision:
        raise ValueError("plan_authorization_changed")
    release = next((r for r in catalog["releases"] if r["release_id"] == release_id), None)
    if not release or not release.get("recipe"):
        raise ValueError("source_requires_guidance")
    digest = release["recipe"]["artifact_sha256"]
    if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
        raise ValueError("invalid_artifact")
    artifact = release.get("artifact") or {}
    return digest, artifact.get("size"), unquote(Path(urlsplit(artifact.get("url", "")).path).name)


def no_links(path: Path):
    for part in (path, *path.parents):
        if part.is_symlink() or (hasattr(part, "is_junction") and part.is_junction()):
            return False
    return True


def find_archive(roots: list[Path], digest: str, size: int | None, filename: str, cancel: Event):
    candidates = []
    for root in roots:
        if not root.is_dir() or not no_links(root):
            continue
        # Only inspect the two allowed directories, never a recursive home scan.
        with os.scandir(root) as entries:
            for number, entry in enumerate(entries):
                if number >= 4096:
                    raise ValueError("local_archive_search_limit")
                if not entry.name.lower().endswith(".zip") or not entry.is_file(follow_symlinks=False):
                    continue
                path = root / entry.name
                stat = entry.stat(follow_symlinks=False)
                if stat.st_size <= 0 or stat.st_size > MAX_ARCHIVE_BYTES or (size and stat.st_size != size):
                    continue
                candidates.append((entry.name not in {filename, f"{digest}.zip"}, -stat.st_mtime_ns, path))
    for _, _, path in sorted(candidates):
        if cancel.is_set():
            raise RuntimeError("download_cancelled")
        if not no_links(path):
            continue
        try:
            fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
            with os.fdopen(fd, "rb") as stream:
                before = os.fstat(stream.fileno())
                task = current_task.get()
                if task:
                    task.stage("validating", before.st_size, 1, archive_bytes=before.st_size, current_file=path.name)
                hasher = hashlib.sha256()
                while chunk := stream.read(4 * 1024 * 1024):
                    if cancel.is_set():
                        raise RuntimeError("download_cancelled")
                    hasher.update(chunk)
                    if task:
                        task.update(bytes_done=task.value["bytes_done"] + len(chunk))
                after = os.fstat(stream.fileno())
                current = path.stat()
                identity = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns)
                if identity(before) == identity(after) == identity(current) and hasher.hexdigest() == digest:
                    return path
        except OSError:
            continue
    return None
