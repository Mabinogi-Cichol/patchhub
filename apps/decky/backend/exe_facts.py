"""Bounded Steam instance facts and reviewed installer preconditions (read only)."""
import binascii
import hashlib
import os
from pathlib import Path
import re
import stat
import sys
from threading import Event

from acquisition import no_links

MAX_MANIFEST = 1024 * 1024
MAX_BASELINE = 512 * 1024 * 1024
LANGUAGES = {"en": "english", "zh-CN": "schinese", "zh-TW": "tchinese",
             "ja": "japanese", "ko": "koreana", "de": "german", "fr": "french",
             "es": "spanish", "ru": "russian"}


def _need(condition, code):
    if not condition:
        raise ValueError(code)


def read_file(path, limit, cancel=None, *, contents=False):
    """Hash a regular unlinked file, checking identity before/after the read."""
    path = Path(path)
    cancel = cancel or Event()
    _need(no_links(path), "exe_linked_game_file")
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    with os.fdopen(fd, "rb") as source:
        before = os.fstat(source.fileno())
        _need(stat.S_ISREG(before.st_mode) and 0 <= before.st_size <= limit, "exe_game_file_limit")
        sha, crc, size = hashlib.sha256(), 0, 0
        pieces = [] if contents else None
        while chunk := source.read(1024 * 1024):
            if cancel.is_set():
                raise RuntimeError("download_cancelled")
            size += len(chunk)
            _need(size <= limit, "exe_game_file_limit")
            sha.update(chunk)
            crc = binascii.crc32(chunk, crc)
            if pieces is not None:
                pieces.append(chunk)
        after = os.fstat(source.fileno())
        # Windows Python fstat's legacy ctime differs from path.stat's creation
        # timestamp. Linux ctime reliably detects metadata changes mid-read.
        identity = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns)
        descriptor = lambda s: (*identity(s), s.st_ctime_ns)
        current = path.stat()
        _need(no_links(path) and descriptor(before) == descriptor(after)
              and identity(after) == identity(current)
              and (sys.platform == "win32" or after.st_ctime_ns == current.st_ctime_ns), "exe_game_file_changed")
        result = {"sha256": sha.hexdigest(), "crc32": crc & 0xffffffff, "size": size}
        return (result, b"".join(pieces)) if contents else result


def keyvalues(raw):
    """Strict quoted Valve KV subset: reject duplicate keys, escapes and suffixes.

    Steam app manifests contain quoted keys/values and objects. Unknown scalar
    fields are retained; executable directives, conditionals and malformed input
    are refused instead of being interpreted as a different instance.
    """
    _need(isinstance(raw, bytes) and len(raw) <= MAX_MANIFEST, "exe_manifest_limit")
    try:
        text = raw.decode("utf-8-sig", errors="strict")
    except UnicodeDecodeError as error:
        raise ValueError("exe_manifest_invalid") from error
    _need("\x00" not in text, "exe_manifest_invalid")
    tokens, i = [], 0
    while i < len(text):
        char = text[i]
        if char.isspace():
            i += 1
        elif text.startswith("//", i):
            end = text.find("\n", i)
            i = len(text) if end < 0 else end + 1
        elif char in "{}":
            tokens.append(char)
            i += 1
        elif char == '"':
            end = text.find('"', i + 1)
            _need(end >= 0, "exe_manifest_invalid")
            value = text[i + 1:end]
            _need(len(value) <= 8192 and "\\" not in value and all(ord(c) >= 32 for c in value), "exe_manifest_invalid")
            tokens.append((value,))
            i = end + 1
        else:
            raise ValueError("exe_manifest_invalid")
        _need(len(tokens) <= 65536, "exe_manifest_limit")
    position = 0

    def parse(depth, nested=False):
        nonlocal position
        _need(depth <= 16, "exe_manifest_limit")
        result = {}
        while position < len(tokens) and tokens[position] != "}":
            token = tokens[position]
            _need(isinstance(token, tuple), "exe_manifest_invalid")
            key = token[0].casefold()
            _need(key and key not in result, "exe_manifest_duplicate")
            position += 1
            _need(position < len(tokens), "exe_manifest_invalid")
            value = tokens[position]
            position += 1
            if value == "{":
                result[key] = parse(depth + 1, True)
            else:
                _need(isinstance(value, tuple), "exe_manifest_invalid")
                result[key] = value[0]
        if nested:
            _need(position < len(tokens) and tokens[position] == "}", "exe_manifest_invalid")
            position += 1
        return result

    result = parse(0)
    _need(position == len(tokens) and set(result) == {"appstate"}
          and isinstance(result["appstate"], dict), "exe_manifest_invalid")
    return result["appstate"]


def _number(value):
    _need(isinstance(value, str) and re.fullmatch(r"[1-9][0-9]{0,9}", value), "exe_manifest_invalid")
    return int(value)


def game_facts(game, profile, cancel=None):
    """Trust the scanned library only after matching its current manifest identity.

    Installed depot DLC IDs are installation evidence, never purchase evidence.
    Absence cannot prove non-ownership; the service requires a user declaration.
    """
    _need(type(game.get("appid")) is int and game["appid"] == profile["appid"], "exe_game_identity_changed")
    _need(isinstance(game.get("library_path"), str) and isinstance(game.get("install_path"), str), "unsafe_game_root")
    library, root = Path(game["library_path"]), Path(game["install_path"])
    _need(library.is_absolute() and root.is_absolute() and no_links(library) and no_links(root)
          and library.is_dir() and root.is_dir(), "unsafe_game_root")
    expected_id = f"{game['appid']}|{library}|{root}"
    _need(game.get("instance_id") == expected_id and game.get("availability") == "installed", "exe_game_identity_changed")
    manifest = library / "steamapps" / f"appmanifest_{game['appid']}.acf"
    try:
        raw_facts, raw = read_file(manifest, MAX_MANIFEST, cancel, contents=True)
    except OSError as error:
        raise ValueError("exe_manifest_unavailable") from error
    state = keyvalues(raw)
    _need(_number(state.get("appid")) == game["appid"], "exe_game_identity_changed")
    directory = state.get("installdir")
    _need(isinstance(directory, str) and directory and directory not in (".", "..")
          and not any(c in '/\\:*?"<>|' or ord(c) < 32 for c in directory)
          and not directory.endswith((".", " ")), "exe_manifest_invalid")
    _need(root == library / "steamapps" / "common" / directory, "exe_game_identity_changed")
    _need(state.get("stateflags") == "4" and not (library / "steamapps" / "downloading" / str(game["appid"])).exists(), "game_not_ready")
    build = state.get("buildid")
    _need(isinstance(build, str) and re.fullmatch(r"[0-9]{1,20}", build)
          and build == game.get("buildid") and build in profile["verified_buildids"], "build_not_verified")
    user = state.get("userconfig", {})
    mounted = state.get("mountedconfig", {})
    _need(isinstance(user, dict) and isinstance(mounted, dict), "exe_manifest_invalid")
    language = user.get("language") or mounted.get("language")
    _need(isinstance(language, str) and language == LANGUAGES.get(profile["language"], profile["language"]), "exe_language_mismatch")
    _need(not (user.get("language") and mounted.get("language") and user["language"] != mounted["language"]), "exe_language_mismatch")
    dlc = set()
    depots = state.get("installeddepots", {})
    _need(isinstance(depots, dict) and len(depots) <= 4096, "exe_manifest_invalid")
    for depot, detail in depots.items():
        _number(depot)
        _need(isinstance(detail, dict), "exe_manifest_invalid")
        if "dlcappid" in detail:
            dlc.add(_number(detail["dlcappid"]))
    # Some clients persist installed DLC explicitly as a comma-separated list.
    installed_dlc = state.get("installeddlc", state.get("installdlc", ""))
    _need(isinstance(installed_dlc, str), "exe_manifest_invalid")
    if installed_dlc:
        _need(len(installed_dlc.split(",")) <= 128, "exe_manifest_limit")
        dlc.update(_number(value.strip()) for value in installed_dlc.split(","))
    _need(not dlc.intersection(profile["excludes_dlc"]), "exe_dlc_excluded")
    _need(set(profile["requires_dlc"]) <= dlc, "exe_dlc_required")
    baselines = []
    for condition in profile["baselines"]:
        path = condition["path"]
        _need(isinstance(path, str) and all(p not in ("", ".", "..") for p in path.split("/"))
              and not any(c in path for c in ("\\", ":", "\x00")), "exe_baseline_invalid")
        try:
            observed = read_file(root.joinpath(*path.split("/")), MAX_BASELINE, cancel)
        except OSError as error:
            raise ValueError("exe_baseline_mismatch") from error
        _need(observed["sha256"] == condition["sha256"] and
              (condition["crc32"] is None or observed["crc32"] == condition["crc32"]), "exe_baseline_mismatch")
        baselines.append({"path": path, **observed})
    return {"instance_id": expected_id, "appid": game["appid"], "library_path": str(library),
            "install_path": str(root), "buildid": build, "language": language,
            "installed_dlc": sorted(dlc), "manifest_sha256": raw_facts["sha256"],
            "baselines": baselines}
