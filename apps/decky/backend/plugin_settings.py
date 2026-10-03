"""Backend-owned plugin settings persisted as an atomic JSON document.

The auto-cleanup switch must live in the backend because the deletion happens
inside the install flow. The frontend only reads and writes it through fixed
methods; it never selects a path.
"""
import json
import os
from pathlib import Path
import uuid

MAX_SETTINGS_BYTES = 64 * 1024
DEFAULT_AUTO_CLEANUP = True
DEFAULT_KEEP_BACKUPS = False


def _no_links(path):
    return all(not p.is_symlink() and not (hasattr(p, "is_junction") and p.is_junction())
               for p in (path, *path.parents))


def _write_json(path, data):
    if not _no_links(path):
        raise ValueError("linked_settings_path")
    path.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
    if not _no_links(path.parent):
        raise ValueError("linked_settings_path")
    temp = path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    try:
        with os.fdopen(os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w", encoding="utf-8") as stream:
            json.dump(data, stream, ensure_ascii=False, indent=2)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp, path)
    finally:
        temp.unlink(missing_ok=True)


class Settings:
    def __init__(self, state_dir):
        self.path = Path(state_dir).absolute() / "settings.json"
        self.auto_cleanup = DEFAULT_AUTO_CLEANUP
        self.keep_backups = DEFAULT_KEEP_BACKUPS
        self.diagnostics = []
        self._load()

    def _load(self):
        try:
            if not self.path.exists():
                return
            if (not _no_links(self.path) or not self.path.is_file()
                    or self.path.stat().st_size > MAX_SETTINGS_BYTES):
                raise ValueError("invalid_settings")
            data = json.loads(self.path.read_text(encoding="utf-8"))
            auto_cleanup = data.get("auto_cleanup", DEFAULT_AUTO_CLEANUP)
            keep_backups = data.get("keep_backups", DEFAULT_KEEP_BACKUPS)
            if not isinstance(auto_cleanup, bool) or not isinstance(keep_backups, bool):
                raise ValueError("invalid_settings")
            self.auto_cleanup = auto_cleanup
            self.keep_backups = keep_backups
        except (OSError, ValueError, json.JSONDecodeError):
            # A missing, unreadable or malformed file falls back to the safe
            # defaults and never blocks plugin startup.
            self.auto_cleanup = DEFAULT_AUTO_CLEANUP
            self.keep_backups = DEFAULT_KEEP_BACKUPS
            self.diagnostics.append({"code": "settings_unavailable", "path": str(self.path)})

    def _save(self):
        _write_json(self.path, {"schema_version": 1, "auto_cleanup": self.auto_cleanup,
                                "keep_backups": self.keep_backups})

    def set_auto_cleanup(self, enabled):
        if not isinstance(enabled, bool):
            raise ValueError("invalid_settings")
        self.auto_cleanup = enabled
        self._save()

    def set_keep_backups(self, enabled):
        if not isinstance(enabled, bool):
            raise ValueError("invalid_settings")
        self.keep_backups = enabled
        self._save()

    def snapshot(self):
        return {"schema_version": 1, "auto_cleanup": self.auto_cleanup, "keep_backups": self.keep_backups}
