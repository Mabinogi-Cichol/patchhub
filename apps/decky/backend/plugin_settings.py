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
DEFAULT_SOURCE_ENGINE = "rust"
# The shared Rust core is the only source engine; legacy values are rejected.
SOURCE_ENGINES = ("rust",)


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
        self.source_engine = DEFAULT_SOURCE_ENGINE
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
            if not isinstance(auto_cleanup, bool):
                raise ValueError("invalid_settings")
            self.auto_cleanup = auto_cleanup
            # Schema 2 predates engine selection; every old host is on the Rust
            # default, so an absent value means "rust" rather than a migration.
            source_engine = data.get("source_engine", DEFAULT_SOURCE_ENGINE)
            if source_engine not in SOURCE_ENGINES:
                source_engine = DEFAULT_SOURCE_ENGINE
                self.diagnostics.append({"code": "invalid_source_engine"})
            self.source_engine = source_engine
            if data.get("schema_version") != 3 or "keep_backups" in data:
                self._save()
        except (OSError, ValueError, json.JSONDecodeError):
            # A missing, unreadable or malformed file falls back to the safe
            # defaults and never blocks plugin startup.
            self.auto_cleanup = DEFAULT_AUTO_CLEANUP
            self.source_engine = DEFAULT_SOURCE_ENGINE
            self.diagnostics.append({"code": "settings_unavailable", "path": str(self.path)})

    def _save(self):
        _write_json(self.path, {"schema_version": 3, "auto_cleanup": self.auto_cleanup,
                                "source_engine": self.source_engine})

    def set_auto_cleanup(self, enabled):
        if not isinstance(enabled, bool):
            raise ValueError("invalid_settings")
        self.auto_cleanup = enabled
        self._save()

    def set_source_engine(self, engine):
        if engine not in SOURCE_ENGINES:
            raise ValueError("invalid_settings")
        self.source_engine = engine
        self._save()

    def snapshot(self):
        return {"schema_version": 3, "auto_cleanup": self.auto_cleanup,
                "source_engine": self.source_engine}
