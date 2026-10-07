"""Engine selection and switch checkpoint for the shared Rust source engine.

The shared Rust source engine is the only source engine. `source_engine`
accepts only `rust`; `python` or any unknown value is ignored with an
`invalid_source_engine` diagnostic and the Rust engine is used. Every launch
records `<state>/sources/rust-v1/engine-switch.json` for observability.
"""
import hashlib
import json
import os
import time
from pathlib import Path

from plugin_settings import _write_json
from source_service import SourceService, aggregate_import_diagnostics, normalize_diagnostics  # noqa: F401 (re-export)

ENGINE_ENV = "STEAM_PATCH_SOURCE_ENGINE"
ENGINE_SETTING = "source_engine"
ENGINE = "rust"
MAX_SWITCH_BYTES = 64 * 1024


def select_engine(settings, env=None):
    """Return `rust` for a valid (or absent) selection, else `invalid`.

    The environment override wins over the persisted setting; unknown values
    never select another engine, they only raise a diagnostic.
    """
    env = os.environ if env is None else env
    value = env.get(ENGINE_ENV)
    if value is None and settings is not None:
        value = getattr(settings, ENGINE_SETTING, None)
    return ENGINE if value in (None, ENGINE) else "invalid"


def rust_state_dir(state_dir):
    return Path(state_dir).absolute() / "sources" / "rust-v1"


def switch_path(state_dir):
    return rust_state_dir(state_dir) / "engine-switch.json"


def read_engine_switch(state_dir):
    path = switch_path(state_dir)
    try:
        if path.is_file() and path.stat().st_size <= MAX_SWITCH_BYTES:
            data = json.loads(path.read_text(encoding="utf-8"))
            if isinstance(data, dict):
                return data
    except (OSError, ValueError):
        pass
    return None


def write_engine_switch(state_dir, record):
    _write_json(switch_path(state_dir), record)


def _mapping(payload):
    enabled = {str(key): bool(value) for key, value in (payload.get("enabled") or {}).items()
               if type(value) is bool}
    added = [rule for rule in (payload.get("added") or []) if isinstance(rule, dict)]
    games = {}
    for appid, choices in (payload.get("games") or {}).items():
        if isinstance(choices, dict):
            games[str(appid)] = {source_id: bool(value) for source_id, value in choices.items()
                                 if type(value) is bool}
    return {"enabled": enabled, "added": added, "games": games}


def map_rust_settings(data):
    """`get_source_settings` -> the normalized `{enabled, added, games}` shape."""
    return None if not isinstance(data, dict) else _mapping(data)


def settings_hash(mapping):
    if not mapping:
        return ""
    payload = json.dumps(mapping, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


async def _export_rust_settings(bridge):
    mapped = map_rust_settings(await bridge.call_sync("get_source_settings", {}))
    if mapped is None:
        raise ValueError("invalid_source_settings")
    return mapped


def _switch_record(previous, service, exported):
    previous = previous if isinstance(previous, dict) else {}
    sequence = previous.get("switch_seq")
    captured = int(time.time())
    record = {
        "schema_version": 1,
        "active_engine": ENGINE,
        "switch_seq": sequence + 1 if type(sequence) is int else 1,
        "rust": {"revision": getattr(service, "settings_revision", 0),
                 "settings_hash": settings_hash(exported)},
        "last_common": previous.get("last_common") or {
            "settings": {"enabled": {}, "added": [], "games": {}}, "hash": "", "captured_at": 0},
    }
    if exported is not None:
        # The switch checkpoint is best-effort; a missing settings export must
        # not stop the Rust engine from serving the host.
        record["last_common"] = {"settings": exported, "hash": settings_hash(exported),
                                 "captured_at": captured}
    return record


async def open_source_engine(state_dir, bridge, settings=None, *, env=None):
    """Open the shared Rust source engine; Python is no longer available."""
    state_dir = Path(state_dir).absolute()
    previous = read_engine_switch(state_dir)
    diagnostics = [] if select_engine(settings, env) == ENGINE else [{"code": "invalid_source_engine"}]
    service = SourceService(bridge, state_dir)
    await service.initialize()
    try:
        exported = await _export_rust_settings(bridge)
    except Exception:
        exported = None
    write_engine_switch(state_dir, _switch_record(previous, service, exported))
    service.diagnostics = [*diagnostics, *getattr(service, "diagnostics", [])]
    return service
