"""Prepare per-install, locally signed source ZIP recipes for the Rust core.

The index is a discovery hint. Only the Rust core plans and mutates game files.
"""

import asyncio
import base64
from datetime import date
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import ssl
import sys
from threading import Event
from urllib.parse import urlsplit
from urllib.request import HTTPSHandler, HTTPRedirectHandler, Request, build_opener
from urllib.error import HTTPError
import uuid
import zipfile

from acquisition import no_links
from bridge import CoreBridge
from tasks import current_task

MAX_ARCHIVE = 8 * 1024**3
MAX_EXPANDED = 16 * 1024**3
MAX_FILE = 4 * 1024**3
MAX_ENTRIES = 4096
CHUNK = 1024 * 1024


def _url(value):
    if not isinstance(value, str) or any(ord(c) < 32 for c in value):
        raise ValueError("invalid_patchmoe_url")
    parsed = urlsplit(value)
    if (parsed.scheme != "https" or parsed.hostname != "download.patch.moe"
            or parsed.username or parsed.password or parsed.port not in (None, 443)
            or parsed.query or parsed.fragment or not parsed.path.lower().endswith(".zip")
            or not parsed.path.startswith("/") or ".." in parsed.path.split("/")):
        raise ValueError("invalid_patchmoe_url")
    return value


def _released(entry):
    value = entry.get("date")
    if value is None:
        return True
    if value == "COMING SOON":
        return False
    if not isinstance(value, str) or not re.fullmatch(r"[0-9]{4}/[0-9]{1,2}/[0-9]{1,2}", value):
        raise ValueError("invalid_publisher_date")
    year, month, day = map(int, value.split("/"))
    return date(year, month, day) <= date.today()


def _safe_dir(path):
    path = Path(path).absolute()
    if not no_links(path):
        raise ValueError("linked_state_path")
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    if not no_links(path):
        raise ValueError("linked_state_path")
    return path


def cache_files(cache):
    """Publisher download-cache files. Never game files, keys or user imports."""
    if not cache.is_dir() or not no_links(cache):
        return []
    result = []
    try:
        with os.scandir(cache) as entries:
            for entry in entries:
                if not entry.is_file(follow_symlinks=False):
                    continue
                name = entry.name
                if name in ("archive.zip", "download.json", "archive.exe", "exe-download.json", "normalized.zip") or (name.startswith("download-") and name.endswith(".tmp")):
                    result.append(cache / name)
    except OSError:
        pass
    return result


def _unlink_within(root, path):
    """Remove a regular file only when it resolves inside an owned root."""
    try:
        if not no_links(path) or not path.is_file():
            return False
        if not path.resolve().is_relative_to(root.resolve()):
            return False
        path.unlink()
        return True
    except OSError:
        return False


RELEASE_PATTERN = r"(?:patchmoe|source-[a-z][a-z0-9-]{0,47}|exe-[0-9a-f]{24})-[1-9][0-9]*"


def _valid_instance(instance_id, release_id, install_path):
    appid = release_id.rsplit("-", 1)[-1]
    return (isinstance(instance_id, str) and isinstance(install_path, str)
            and instance_id.startswith(appid + "|") and instance_id.endswith("|" + install_path)
            and Path(install_path).is_absolute() and no_links(Path(install_path)))


def _json_new(path, value):
    if not no_links(path):
        raise ValueError("linked_state_path")
    temp = path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    try:
        fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as output:
            json.dump(value, output, ensure_ascii=False, indent=2)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temp, path)
    finally:
        temp.unlink(missing_ok=True)


def _hash_file(path, limit, cancel, report=True):
    if not no_links(path):
        raise ValueError("linked_file")
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    with os.fdopen(fd, "rb") as stream:
        before = os.fstat(stream.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_size > limit:
            raise ValueError("file_size_or_type")
        task = current_task.get()
        if task and report:
            task.stage("validating", before.st_size, 1, current_file=path.name, archive_bytes=before.st_size)
        digest = hashlib.sha256()
        total = 0
        while data := stream.read(CHUNK):
            if cancel.is_set():
                raise RuntimeError("download_cancelled")
            total += len(data)
            if total > limit:
                raise ValueError("file_size_limit")
            digest.update(data)
            if task:
                task.update(bytes_done=task.value["bytes_done"] + len(data))
        after = os.fstat(stream.fileno())
        current = path.stat()
        identity = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns)
        if identity(before) != identity(after) or identity(after) != identity(current):
            raise ValueError("file_changed")
        return digest.hexdigest(), total


class _SameHostRedirect(HTTPRedirectHandler):
    def __init__(self, validate=_url):
        self.validate = validate

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        self.validate(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def _download(url, cache, cancel, validate=_url, *, archive_name="archive.zip", manifest_name="download.json",
              max_size=MAX_ARCHIVE, expected_sha256=None, expected_size=None):
    task = current_task.get()
    if cancel.is_set():
        raise RuntimeError("download_cancelled")
    validate(url)
    if (not no_links(cache) or not cache.is_dir()
            or not all(isinstance(n, str) and re.fullmatch(r"[a-z][a-z.-]{0,63}", n)
                       for n in (archive_name, manifest_name))):
        raise ValueError("unsafe_download_cache")
    manifest = cache / manifest_name
    archive = cache / archive_name
    cached = None
    headers = {"User-Agent": "SteamPatchSearcher/1"}
    if manifest.is_file() and archive.is_file() and no_links(manifest) and no_links(archive):
        try:
            data = json.loads(manifest.read_text(encoding="utf-8"))
            digest, size = _hash_file(archive, max_size, cancel)
            if (data.get("url") == url and data.get("sha256") == digest and data.get("size") == size and size
                    and (expected_sha256 is None or digest == expected_sha256)
                    and (expected_size is None or size == expected_size)):
                cached = (archive, digest, size, "local")
                for key, header in (("etag", "If-None-Match"), ("last_modified", "If-Modified-Since")):
                    value = data.get(key)
                    if isinstance(value, str) and value and not any(ord(c) < 32 for c in value):
                        headers[header] = value
        except (OSError, ValueError, json.JSONDecodeError):
            pass
    temp = cache / ("download-" + uuid.uuid4().hex + ".tmp")
    digest = hashlib.sha256()
    size = 0
    try:
        context = ssl.create_default_context()
        if getattr(sys, "frozen", False):
            # Use SteamOS's maintained CA store in Decky's frozen runtime.
            # Certificate and hostname verification remain enabled.
            for candidate in ("/etc/ssl/certs/ca-certificates.crt", "/etc/ssl/cert.pem"):
                if Path(candidate).is_file():
                    context.load_verify_locations(cafile=candidate)
                    break
        opener = build_opener(HTTPSHandler(context=context), _SameHostRedirect(validate))
        try:
            if task:
                task.stage("connecting")
            if cancel.is_set():
                raise RuntimeError("download_cancelled")
            # Bound a stalled connect/read so UI cancellation can clean up at
            # the next chunk boundary without limiting the whole download.
            source = opener.open(Request(url, headers=headers), timeout=15)
        except HTTPError as error:
            if error.code == 304 and cached and len(headers) > 1:
                error.close()
                if cancel.is_set():
                    raise RuntimeError("download_cancelled")
                return cached
            raise
        with source:
            validate(source.geturl())
            length = source.headers.get("Content-Length")
            total_bytes = int(length) if length and length.isdigit() else None
            if total_bytes is not None and total_bytes > max_size:
                raise ValueError("archive_size_limit")
            if total_bytes is not None and expected_size is not None and total_bytes != expected_size:
                raise ValueError("download_size_mismatch")
            if task:
                task.stage("downloading", total_bytes, archive_bytes=total_bytes)
            response_metadata = {"etag": source.headers.get("ETag"), "last_modified": source.headers.get("Last-Modified"),
                                 "final_url": source.geturl()}
            with os.fdopen(os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as output:
                while data := getattr(source, "read1", source.read)(CHUNK):
                    if cancel.is_set():
                        raise RuntimeError("download_cancelled")
                    size += len(data)
                    if size > max_size or expected_size is not None and size > expected_size:
                        raise ValueError("archive_size_limit")
                    digest.update(data)
                    output.write(data)
                    if task:
                        task.update(bytes_done=size)
                if cancel.is_set():
                    raise RuntimeError("download_cancelled")
                if total_bytes is not None and size != total_bytes:
                    raise ValueError("download_size_mismatch")
                output.flush()
                os.fsync(output.fileno())
        if not size:
            raise ValueError("empty_archive")
        if expected_size is not None and size != expected_size:
            raise ValueError("download_size_mismatch")
        if expected_sha256 is not None and digest.hexdigest() != expected_sha256:
            raise ValueError("exe_payload_changed")
        if cancel.is_set():
            raise RuntimeError("download_cancelled")
        if task:
            task.update(archive_bytes=size)
        os.replace(temp, archive)
        _json_new(manifest, {"url": url, "sha256": digest.hexdigest(), "size": size, **response_metadata})
        return archive, digest.hexdigest(), size, "download"
    finally:
        temp.unlink(missing_ok=True)


def _relative(name):
    if (not name or len(name) > 1024 or "\\" in name or ":" in name
            or any(ord(c) < 32 for c in name)):
        raise ValueError("unsafe_relative_path")
    for part in name.split("/"):
        device = part.split(".")[0].upper()
        if (part in ("", ".", "..") or part.endswith((".", " "))
                or any(c in '<>"|?*' for c in part)
                or device in {"CON", "PRN", "AUX", "NUL"}
                or re.fullmatch(r"(?:COM|LPT)[0-9]", device)):
            raise ValueError("unsafe_relative_path")
    return name


def _extra_safe(extra):
    i = 0
    while i + 4 <= len(extra):
        tag = int.from_bytes(extra[i:i + 2], "little")
        size = int.from_bytes(extra[i + 2:i + 4], "little")
        i += 4
        record = extra[i:i + size]
        if len(record) != size or tag in (0x000d, 0x756e):
            raise ValueError("zip_unsafe_extra_metadata")
        if tag == 0x000a and (len(record) != 32 or record[:8] != bytes([0, 0, 0, 0, 1, 0, 24, 0])):
            raise ValueError("zip_unsafe_extra_metadata")
        i += size
    if i != len(extra):
        raise ValueError("zip_invalid_extra_metadata")


def _recipe(archive, root, cancel):
    task = current_task.get()
    files = []
    names = set()
    directory_names = set()
    prefixes = {}
    expanded = 0
    with zipfile.ZipFile(archive) as z:
        if len(z.infolist()) > MAX_ENTRIES:
            raise ValueError("archive_entry_limit")
        for info in z.infolist():
            name = _relative(info.filename.removesuffix("/"))
            if any(part.lower().startswith(".steam-patch-") for part in name.split("/")):
                raise ValueError("reserved_installer_path")
            if not (info.flag_bits & 0x800) and not info.filename.isascii():
                raise ValueError("zip_name_encoding")
            key = name.lower()
            if key in names:
                raise ValueError("zip_duplicate_or_case_collision")
            names.add(key)
            if info.is_dir():
                directory_names.add(key)
            prefix = ""
            for part in name.split("/"):
                prefix = f"{prefix}/{part}" if prefix else part
                previous = prefixes.setdefault(prefix.lower(), prefix)
                if previous != prefix:
                    raise ValueError("zip_duplicate_or_case_collision")
            mode = (info.external_attr >> 16) & 0o170000
            if info.is_dir():
                if mode not in (0, stat.S_IFDIR):
                    raise ValueError("zip_link_or_special_file")
            elif mode not in (0, stat.S_IFREG):
                raise ValueError("zip_link_or_special_file")
            _extra_safe(info.extra)
            if info.file_size > MAX_FILE or (info.file_size > CHUNK and info.file_size // max(info.compress_size, 1) > 200):
                raise ValueError("zip_expansion_limit")
            expanded += info.file_size
            if expanded > MAX_EXPANDED:
                raise ValueError("zip_expansion_limit")
        if task:
            original_bytes = 0
            for info in z.infolist():
                candidate = root.joinpath(*info.filename.removesuffix("/").split("/"))
                if no_links(candidate) and candidate.is_file():
                    original_bytes += candidate.stat().st_size
            task.stage("validating", expanded + original_bytes, sum(not i.is_dir() for i in z.infolist()), archive_bytes=Path(archive).stat().st_size)
        for info in z.infolist():
            name = info.filename.removesuffix("/")
            if any(parent.lower() in names and parent.lower() not in directory_names
                   for parent in _parents(name)):
                raise ValueError("zip_file_directory_collision")
            if info.is_dir():
                continue
            if cancel.is_set():
                raise RuntimeError("download_cancelled")
            if task:
                task.update(current_file=name)
            digest = hashlib.sha256()
            size = 0
            with z.open(info) as stream:
                while data := stream.read(CHUNK):
                    if cancel.is_set():
                        raise RuntimeError("download_cancelled")
                    size += len(data)
                    if size > MAX_FILE:
                        raise ValueError("zip_expansion_limit")
                    digest.update(data)
                    if task:
                        task.update(bytes_done=task.value["bytes_done"] + len(data))
            if size != info.file_size:
                raise ValueError("zip_size_mismatch")
            target = root.joinpath(*name.split("/"))
            if not no_links(target):
                raise ValueError("linked_target")
            if target.exists():
                if not target.is_file():
                    raise ValueError("target_not_file")
                original, _ = _hash_file(target, MAX_FILE, cancel, report=False)
            else:
                original = None
            files.append({"source": name, "destination": name,
                          "original_sha256": original, "installed_sha256": digest.hexdigest()})
            if task:
                task.update(files_done=len(files))
    if not files:
        raise ValueError("empty_recipe")
    return files


def _parents(name):
    parts = name.split("/")
    return ("/".join(parts[:i]) for i in range(1, len(parts)))


class Publisher:
    def __init__(self, bridge: CoreBridge, sources):
        self.bridge = bridge
        self.sources = sources
        if not bridge.state_dir:
            raise ValueError("state_dir_required")
        self.base = Path(bridge.state_dir) / "publisher"
        home = bridge.process_env.get("HOME") if hasattr(bridge, "process_env") else None
        if home and not self.base.absolute().is_relative_to(Path(home).absolute()):
            raise ValueError("publisher_state_outside_user_home")
        if not no_links(self.base):
            raise ValueError("linked_state_path")
        self.state_diagnostics = []

    def matches(self, game: dict):
        return self.sources.matches(game)

    def match(self, game: dict, release_id=None):
        found = self.matches(game)
        if release_id is not None:
            return next((entry for entry in found if entry["release_id"] == release_id), None)
        return found[0] if len(found) == 1 else None

    def _state(self, instance_id, release_id):
        if not isinstance(instance_id, str) or not instance_id or len(instance_id) > 256:
            raise ValueError("invalid_instance_id")
        if not re.fullmatch(RELEASE_PATTERN, release_id):
            raise ValueError("invalid_release_id")
        return self.base / release_id / hashlib.sha256(instance_id.encode()).hexdigest()[:16]

    def cache_dir(self, instance_id, release_id):
        return self._state(instance_id, release_id) / "cache"

    def clear_cache(self, instance_id, release_id):
        """Delete one instance's downloaded ZIP cache; keep recipe and journal."""
        cache = self.cache_dir(instance_id, release_id)
        return sum(1 for path in cache_files(cache) if _unlink_within(cache, path))

    def states(self):
        """Yield every existing publisher state directory (no metadata checks)."""
        if not self.base.is_dir() or not no_links(self.base):
            return
        try:
            releases = list(os.scandir(self.base))
        except OSError:
            return
        for release in releases:
            release_dir = self.base / release.name
            if not release.is_dir(follow_symlinks=False) or not no_links(release_dir):
                continue
            try:
                states = list(os.scandir(release_dir))
            except OSError:
                continue
            for state in states:
                state_dir = release_dir / state.name
                if state.is_dir(follow_symlinks=False) and no_links(state_dir):
                    yield state_dir

    def caches(self):
        """Yield (release_id, state_dir, cache) for every publisher cache."""
        for state_dir in self.states():
            cache = state_dir / "cache"
            if cache.is_dir() and no_links(cache):
                yield state_dir.parent.name, state_dir, cache

    def _bridge(self, state):
        bridge = CoreBridge(self.bridge.binary, state / "catalog.json", timeout=self.bridge.timeout,
                          signed_catalog=state / "signed.json", trust_keys=state / "public.json",
                          state_dir=state, run_uid=self.bridge.process_identity.get("user"),
                          run_gid=self.bridge.process_identity.get("group"),
                          user_home=Path(self.bridge.process_env["HOME"]) if "HOME" in self.bridge.process_env else None)
        bridge.process_env = self.bridge.process_env.copy()
        bridge.process_identity = self.bridge.process_identity.copy()
        return bridge

    async def _native(self, *args):
        process = await asyncio.create_subprocess_exec(str(self.bridge.binary), *map(str, args),
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
            env=self.bridge.process_env, **self.bridge.process_identity)
        stdout, stderr = await process.communicate()
        if process.returncode:
            raise RuntimeError("core_maintenance_failed:" + stderr.decode(errors="replace")[:500])
        return stdout

    def bridges(self):
        result = []
        self.state_diagnostics = []
        if not self.base.is_dir() or not no_links(self.base):
            return result
        for release_dir in self.base.iterdir():
            if not release_dir.is_dir() or not no_links(release_dir):
                self.state_diagnostics.append({"code": "unsafe_publisher_state", "path": str(release_dir)})
                continue
            for state in release_dir.iterdir():
                if not state.is_dir() or not no_links(state):
                    self.state_diagnostics.append({"code": "unsafe_publisher_state", "path": str(state)})
                    continue
                try:
                    metadata_path = state / "instance.json"
                    if not no_links(metadata_path):
                        continue
                    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
                    instance_id = metadata["instance_id"]
                    release_id = metadata["release_id"]
                    if (self._state(instance_id, release_id) != state or release_id != release_dir.name
                            or not _valid_instance(instance_id, release_id, metadata["install_path"])
                            or not all((state / p).is_file() and no_links(state / p)
                                       for p in ("catalog.json", "signed.json", "public.json"))):
                        if (state / "journal.sqlite").exists():
                            self.state_diagnostics.append({"code": "publisher_transaction_state_invalid", "path": str(state)})
                        continue
                    result.append((instance_id, release_id, self._bridge(state)))
                except (OSError, KeyError, ValueError, TypeError, json.JSONDecodeError):
                    if (state / "journal.sqlite").exists():
                        self.state_diagnostics.append({"code": "publisher_transaction_state_invalid", "path": str(state)})
                    continue
        return result

    def bridge_for(self, instance_id, release_id):
        for found_instance, found_release, bridge in self.bridges():
            if (found_instance, found_release) == (instance_id, release_id):
                return bridge
        return None

    async def refresh_signature(self, instance_id, release_id):
        """Renew an expired local envelope without changing the installed recipe/revision."""
        bridge = self.bridge_for(instance_id, release_id)
        if bridge is None:
            return None
        state = self._state(instance_id, release_id)
        signed = state / "signed.json"
        if not no_links(signed):
            raise ValueError("linked_state_path")
        import time
        envelope = json.loads(signed.read_text(encoding="utf-8"))
        payload = json.loads(base64.b64decode(envelope["payload"], validate=True))
        if payload["expires_at"] > int(time.time()) + 86400:
            return bridge
        private = state / "private.key"
        public = state / "public.json"
        if not all(p.is_file() and no_links(p) for p in (private, public, state / "catalog.json")):
            raise ValueError("invalid_local_keys")
        key_id = "local-" + hashlib.sha256(instance_id.encode()).hexdigest()[:16]
        temporary = state / ("signed-" + uuid.uuid4().hex + ".json")
        try:
            await self._native("sign-catalog", state / "catalog.json", private, key_id, temporary, "7776000")
            os.replace(temporary, signed)
        finally:
            temporary.unlink(missing_ok=True)
        return self._bridge(state)

    async def prepare(self, game: dict, entry: dict, cancel: Event, local_archive=None):
        matched = self.match(game, entry.get("release_id"))
        if matched is None or entry.get("url") != matched["url"]:
            raise ValueError("publisher_match_changed")
        if not _released(matched):
            raise ValueError("source_not_released")
        instance_id = game["instance_id"]
        release_id = matched["release_id"]
        root = Path(game["install_path"]).absolute()
        if (game.get("availability") != "installed" or not isinstance(game.get("buildid"), str)
                or not game["buildid"] or not root.is_dir() or not no_links(root)
                or not _valid_instance(instance_id, release_id, game["install_path"])):
            raise ValueError("unsafe_game_root")
        state = self._state(instance_id, release_id)
        if state.absolute() == root or state.absolute().is_relative_to(root) or root.is_relative_to(state.absolute()):
            raise ValueError("state_overlaps_game")
        _safe_dir(state)
        metadata_path = state / "instance.json"
        if metadata_path.exists():
            if not no_links(metadata_path):
                raise ValueError("linked_state_path")
            metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
            if (metadata.get("instance_id"), metadata.get("release_id"), metadata.get("install_path")) != (
                    instance_id, release_id, str(root)):
                raise ValueError("publisher_state_identity_changed")
        if (state / "catalog.json").is_file():
            existing = self._bridge(state)
            transactions = await existing.list_transactions()
            if any(t["state"] not in ("planned", "uninstalled", "rolled_back") for t in transactions):
                raise RuntimeError("publisher_existing_transaction")
        revision = 1
        catalog_path = state / "catalog.json"
        if catalog_path.is_file():
            revision = json.loads(catalog_path.read_text(encoding="utf-8"))["revision"] + 1
        cache = _safe_dir(state / "cache")
        validate = lambda url: self.sources.validate_download(matched["source_id"], url)
        if local_archive is None:
            archive, digest, archive_size, acquisition = await asyncio.to_thread(_download, validate(matched["url"]), cache, cancel, validate)
        else:
            archive = Path(local_archive)
            digest, archive_size = await asyncio.to_thread(_hash_file, archive, MAX_ARCHIVE, cancel)
            acquisition = "local"
        files = await asyncio.to_thread(_recipe, archive, root, cancel)
        if cancel.is_set():
            raise RuntimeError("download_cancelled")
        buildid = game.get("buildid")
        if not isinstance(buildid, str) or not buildid:
            raise ValueError("missing_buildid")
        release = {"patch_id": release_id, "release_id": release_id, "appid": game["appid"],
                   "name": f"{matched['title']} — official ZIP, game-root layout", "version": str(revision),
                   "language": matched["language"], "category": "publisher_zip",
                   "source_page": matched["url"], "support_level": "managed_zip",
                   "platforms": ["windows", "steamos"], "verified_buildids": [buildid],
                   "detection": [{"path": f["destination"], "sha256": f["installed_sha256"]} for f in files],
                   "recipe": {"artifact_sha256": digest, "files": files},
                   "artifact": {"url": matched["url"], "size": archive_size,
                                "sha256": digest, "allowed_redirect_hosts": []}}
        _json_new(catalog_path, {"schema_version": 1, "revision": revision, "releases": [release]})
        private = state / "private.key"
        public = state / "public.json"
        key_id = "local-" + hashlib.sha256(instance_id.encode()).hexdigest()[:16]
        if not private.exists() and not public.exists():
            await self._native("keygen", private, public, key_id)
        elif not (private.is_file() and public.is_file() and no_links(private) and no_links(public)):
            raise ValueError("invalid_local_keys")
        signed = state / "signed.json"
        temporary = state / ("signed-" + uuid.uuid4().hex + ".json")
        try:
            await self._native("sign-catalog", catalog_path, private, key_id, temporary, "7776000")
            os.replace(temporary, signed)
        finally:
            temporary.unlink(missing_ok=True)
        _json_new(state / "instance.json", {"instance_id": instance_id, "release_id": release_id,
                                             "install_path": str(root), "url": matched["url"]})
        bridge = self._bridge(state)
        if cancel.is_set():
            raise RuntimeError("download_cancelled")
        plan = await bridge.plan_install(instance_id, release_id, archive)
        return bridge, str(archive), plan, acquisition
