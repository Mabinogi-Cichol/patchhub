"""Decky entry point. Bundle bin/steam-patch and catalog files with this plugin."""
from pathlib import Path
import sys
import os
import asyncio
from threading import Event
import hashlib
import inspect
import re
from urllib.parse import urlsplit

import decky

sys.path.insert(0, str(Path(__file__).parent / "backend"))
import caches
from bridge import CoreBridge
from acquisition import find_archive, release_metadata
from publisher import Publisher, _released, RELEASE_PATTERN
from plugin_settings import Settings
from source_engine import open_source_engine
from tasks import TaskManager, current_task
from exe_profiles import ExeProfiles
from exe_installation import ExeInstallation
from scan_session import ScanSession
from publisher import _json_new


def _diag_key(item):
    if not isinstance(item, dict):
        return str(item)
    return (item.get("source_id"), item.get("code"), item.get("detail"))


def _merge_diagnostics(*groups):
    """Merge diagnostic lists preserving order and dropping exact duplicates."""
    merged = []
    seen = set()
    for group in groups:
        if not isinstance(group, list):
            continue
        for item in group:
            key = _diag_key(item)
            if key in seen:
                continue
            seen.add(key)
            merged.append(item)
    return merged


async def _await_maybe(value):
    # Both engines are duck-typed; only the Rust adapter's local writes are async.
    if inspect.isawaitable(value):
        return await value
    return value


class Plugin:
    async def _main(self):
        self._startup_ready = asyncio.Event()
        self._startup_error = None
        try:
            await self._initialize()
        except BaseException as error:
            self._startup_error = error
            raise
        finally:
            self._startup_ready.set()

    async def _initialize(self):
        root = Path(decky.DECKY_PLUGIN_DIR)
        catalog = root / "bin" / "catalog.json"
        state_dir = Path(decky.DECKY_USER_HOME) / ".local/share/steam-patch-manager"
        run_uid = run_gid = None
        if os.geteuid() == 0:
            import pwd
            account = pwd.getpwnam(Path(decky.DECKY_USER_HOME).name)
            if account.pw_uid == 0 or Path(account.pw_dir) != Path(decky.DECKY_USER_HOME):
                raise ValueError("unprivileged_user_required")
            run_uid, run_gid = account.pw_uid, account.pw_gid
        self.bridge = CoreBridge(
            root / "bin" / "steam-patch", catalog,
            signed_catalog=root / "bin" / "signed-catalog.json",
            trust_keys=root / "bin" / "trust-keys.json",
            state_dir=state_dir, run_uid=run_uid, run_gid=run_gid, user_home=Path(decky.DECKY_USER_HOME),
        )
        self.downloads = (Path(decky.DECKY_USER_HOME) / "Downloads").resolve()
        self._prepare_cancel = None
        self.tasks = TaskManager(state_dir=state_dir)
        self._plan_metadata = {}
        self.settings = Settings(state_dir)
        self.sources = await open_source_engine(state_dir, self.bridge, self.settings)
        self.publisher = Publisher(self.bridge, self.sources)
        self.exe_profiles = ExeProfiles(root / "backend/exe_profiles.json", self.sources, pipeline_ready=True)
        self.exe_installation = ExeInstallation(self.publisher, self.exe_profiles, root / "backend/exe_worker")
        self._scan_generation = 0
        self._scan_games = {}
        self._scan_session = None
        await self._migrate_startup(self.bridge)
        for _, _, bridge in self.publisher.bridges():
            await self._migrate_startup(bridge)
        await self._reclaim_plans()

    async def _migrate_startup(self, bridge):
        # Loader can reload immediately after installation while a child of
        # the preceding backend is still exiting. Wait for that journal lock;
        # never delete the lock file or ignore a persistent busy state.
        for attempt in range(41):
            try:
                return await bridge.migrate_state()
            except RuntimeError as error:
                if str(error) != "installer_busy" or attempt == 40:
                    raise
                await asyncio.sleep(.25)

    async def _reclaim_plans(self):
        # Startup runs before accepting tasks. A crash may happen after core
        # prepare persisted a job but before its ID reached tasks.json, so the
        # journal itself is authoritative for outstanding authorization.
        for transaction in await self._list_transactions():
            if transaction.get("state") != "planned":
                continue
            bridge, plan_id, owner = self._dispatch_id(transaction["plan_id"])
            try:
                await self._revoke_plan(bridge, plan_id, owner)
            except Exception:
                # A failed RPC is tolerable only after durable local revocation.
                from acquisition import no_links
                marker = bridge.state_dir / "jobs" / plan_id / "revoked"
                if not marker.is_file() or not no_links(marker):
                    raise

    async def _project_row(self, row, transactions=None):
        game = row["game"]
        transactions = transactions if transactions is not None else await self.list_transactions()
        row["sources"] = self.sources.list_game_sources(game)
        row["patches"] = []
        row["exe_reviews"] = []
        entries = self.publisher.matches(game)
        source_by_id = {source["id"]: source for source in self.sources.list_sources()}
        for entry in entries:
            source = source_by_id.get(entry["source_id"], {})
            patch = dict(release_id=entry["release_id"], name=f"{source.get('name') or entry['source_id']} · ZIP · {entry['language']}",
                source_page=entry["source_page"], source_id=entry["source_id"], source_name=source.get("name"),
                source_status=source.get("status", "unknown"), source_cached_at=source.get("cached_at"),
                support_level="publisher_zip", download_available=True, compatibility="publisher_default",
                blockers=[], detection=dict(status="unknown", files=[]))
            if not _released(entry):
                patch["blockers"].append("source_not_released")
            if game.get("availability") != "installed":
                patch["blockers"].append("game_not_ready")
            if not game.get("buildid"):
                patch["blockers"].append("build_not_verified")
            row["patches"].append(patch)
        exe = getattr(self, "exe_installation", None)
        if exe:
            for patch in exe.matches(game):
                try:
                    await asyncio.to_thread(exe.check_game, game, patch["release_id"])
                except (ValueError, OSError) as error:
                    patch["blockers"].append(str(error))
                row["patches"].append(patch)
            row["exe_reviews"] = self.exe_profiles.reviews(game)
        # An installed receipt remains visible when its online source is disabled.
        known = {p["release_id"]: p for p in row["patches"]}
        for instance, release, local in self.publisher.bridges():
            if instance != game["instance_id"]:
                continue
            try:
                observed = await local.scan()
                if observed.get("catalog_error") == "catalog_expired":
                    local = await self.publisher.refresh_signature(instance, release)
                    observed = await local.scan()
                if observed.get("catalog_trust") != "signed":
                    if release in known:
                        known[release]["blockers"].append("catalog_untrusted")
                    continue
                local_row = next((i for i in observed["installs"] if i["game"]["instance_id"] == instance), None)
                local_patch = next((p for p in (local_row or {}).get("patches", []) if p["release_id"] == release), None)
                if not local_patch:
                    continue
                if release not in known:
                    local_transactions = await local.list_transactions()
                    if not any(t.get("state") == "committed" for t in local_transactions):
                        continue
                    restored = dict(local_patch, support_level="publisher_exe" if release.startswith("exe-") else "publisher_zip",
                                    download_available=False, blockers=["source_not_enabled"])
                    source = source_by_id.get(local_patch.get("source_id")) or next((s for s in source_by_id.values()
                        if urlsplit(local_patch.get("source_page") or "").hostname in s.get("page_hosts", [])), {})
                    source_name = source.get("name") or local_patch.get("source_id") or urlsplit(local_patch.get("source_page") or "").hostname or "来源未知"
                    restored.update(name=f"{game.get('name', '')} · {source_name} · {'EXE' if release.startswith('exe-') else 'ZIP'} · {local_patch.get('language') or '语言待确认'}",
                                    source_id=source.get("id") or local_patch.get("source_id"), source_name=source_name)
                    row["patches"].append(restored)
                    known[release] = restored
                known[release]["detection"] = local_patch["detection"]
                known[release]["blockers"].extend(local_patch.get("blockers", []))
            except Exception:
                if release in known:
                    known[release]["blockers"].append("publisher_state_unavailable")
        for patch in row["patches"]:
            own = [t for t in transactions if t.get("instance_id") == game["instance_id"]]
            if any(t["state"] not in ("planned", "committed", "uninstalled", "rolled_back", "superseded") for t in own):
                patch["blockers"].append("managed_install_exists_or_needs_recovery")
            if any(t["state"] == "committed" and t["release_id"] != patch["release_id"] for t in own):
                patch["blockers"].append("publisher_existing_transaction")
            patch["blockers"] = list(dict.fromkeys(patch["blockers"]))
            patch["auto_installable"] = bool(not patch["blockers"] and patch.get("download_available")
                and patch["support_level"] in ("managed_zip", "publisher_zip", "publisher_exe"))
            patch["reinstall_available"] = patch["auto_installable"] and (
                any(t["state"] == "committed" and t["release_id"] == patch["release_id"] for t in own)
                or patch["detection"]["status"] in ("managed", "installed_managed", "installed_owned", "external", "installed_external"))
        row["match_status"] = "matched" if row["patches"] or any(
            c.get("match_basis") in ("appid", "verified_alias", "official_title")
            for c in row.get("discovery", {}).get("candidates", [])) else "no_source_match"

    async def _read_report(self):
        report = await self.bridge.scan()
        prime = getattr(self.sources, "prime", None)
        if prime is not None:
            prime(report)
        report["sources"] = self.sources.list_sources()
        report["generation"] = getattr(self, "_scan_generation", 0)
        report["settings_revision"] = self.sources.settings_revision
        report["diagnostics"] = _merge_diagnostics(
            report.get("diagnostics"), getattr(self.sources, "diagnostics", []))
        transactions = await self.list_transactions()
        for row in report["installs"]:
            row["discovery"] = self.sources.cached_discovery(row["game"])
            await self._project_row(row, transactions)
        return report

    async def scan(self, refresh_sources: bool = False):
        self._scan_generation = getattr(self, "_scan_generation", 0) + 1
        generation = self._scan_generation
        previous = getattr(self, "_scan_session", None)
        if previous:
            await previous.close()
        report = await self._read_report()
        self._scan_games = {row["game"]["instance_id"]: dict(row["game"]) for row in report["installs"]}
        revision = self.sources.settings_revision

        def current():
            return self._scan_generation == generation and self.sources.settings_revision == revision

        async def refresh(on_event, cancel):
            diagnostics = await self.sources.refresh(force=refresh_sources, on_event=on_event, cancel=cancel)
            if current():
                report["sources"] = self.sources.list_sources()
                report["diagnostics"] = _merge_diagnostics(report.get("diagnostics"), diagnostics)
            return diagnostics

        async def scan_sources(on_event, cancel):
            return await self.sources.scan_sources(force=refresh_sources, on_event=on_event, cancel=cancel)

        session = ScanSession(report, refresh, scan_sources, self._project_row, current)
        self._scan_session = session
        return session.snapshot()

    async def scan_status(self, generation: int):
        session = getattr(self, "_scan_session", None)
        if type(generation) is not int or not session or generation != self._scan_generation:
            raise ValueError("scan_stale")
        if session.report["settings_revision"] != self.sources.settings_revision:
            raise ValueError("scan_stale")
        return session.snapshot()

    async def discover_game(self, instance_id: str, name_hints=None, preferred_language=None, force: bool = False):
        """Discover metadata for a trusted local instance without occupying the installer."""
        if not isinstance(instance_id, str) or not instance_id or len(instance_id) > 1024:
            raise ValueError("invalid_id")
        if name_hints is None:
            name_hints = []
        if (not isinstance(name_hints, list) or len(name_hints) > 3
                or any(not isinstance(name, str) or not name.strip() or len(name) > 256 for name in name_hints)):
            raise ValueError("invalid_discovery_query")
        if (preferred_language is not None and (not isinstance(preferred_language, str)
                or not re.fullmatch(r"[A-Za-z]{2,3}(?:-[A-Za-z]{2,4})?", preferred_language))):
            raise ValueError("invalid_discovery_query")
        if type(force) is not bool:
            raise ValueError("invalid_discovery_query")
        games = getattr(self, "_scan_games", {})
        game = games.get(instance_id)
        if game is None:
            raise ValueError("instance_not_found")
        generation = self._scan_generation
        revision = self.sources.settings_revision
        result = await self.sources.discover_game(game, name_hints, preferred_language, force, generation,
            is_current=lambda: self._scan_generation == generation and self.sources.settings_revision == revision)
        result.update(instance_id=instance_id, generation=generation, settings_revision=revision)
        if generation != self._scan_generation or revision != self.sources.settings_revision:
            result.update(status="stale", candidates=[])
        profiles = getattr(self, "exe_profiles", None)
        result["exe_reviews"] = profiles.reviews(game) if profiles and result.get("status") != "stale" else []
        exe = getattr(self, "exe_installation", None)
        row = {"game": game, "discovery": result}
        if result.get("status") != "stale":
            await self._project_row(row)
        result["exe_patches"] = [p for p in row.get("patches", []) if p["support_level"] == "publisher_exe"]
        result["patches"] = row.get("patches", [])
        return result

    async def exe_installation_reviews(self, instance_id: str):
        """Read capability evidence for a scanned local instance; no URL/profile imports."""
        game = getattr(self, "_scan_games", {}).get(instance_id) if isinstance(instance_id, str) else None
        if game is None:
            raise ValueError("instance_not_found")
        return self.exe_profiles.reviews(game)

    @staticmethod
    def _installable_release(release_id):
        # Opaque discovery identities never enter catalog, download or plan APIs.
        if not isinstance(release_id, str) or release_id.startswith("candidate-") or release_id.startswith("candidate:"):
            raise ValueError("source_requires_guidance")
        if release_id.startswith("exe-") and not re.fullmatch(r"exe-(?:(?:selection|content)-)?[0-9a-f]{24}-[1-9][0-9]*", release_id):
            raise ValueError("exe_release_invalid")

    @staticmethod
    def _publisher_id(instance_id, release_id, core_id):
        return f"publisher:{release_id}:{hashlib.sha256(instance_id.encode()).hexdigest()[:16]}:{core_id}"

    def _dispatch_id(self, value):
        if not value.startswith("publisher:"):
            return self.bridge, value, None
        match = re.fullmatch(rf"publisher:({RELEASE_PATTERN}):([0-9a-f]{{16}}):([0-9a-f-]{{36}})", value)
        if not match:
            raise ValueError("invalid_id")
        release_id, instance_hash, core_id = match.groups()
        for instance_id, found_release, bridge in self.publisher.bridges():
            if found_release == release_id and hashlib.sha256(instance_id.encode()).hexdigest()[:16] == instance_hash:
                return bridge, core_id, (instance_id, release_id)
        raise ValueError("publisher_state_unavailable")

    def _wrap_publisher(self, value, instance_id, release_id):
        value = dict(value)
        for field in ("plan_id", "transaction_id"):
            if value.get(field):
                value[field] = self._publisher_id(instance_id, release_id, value[field])
        return value

    async def preview(self, instance_id: str, release_id: str):
        self._installable_release(release_id)
        if release_id.startswith("exe-"):
            game = getattr(self, "_scan_games", {}).get(instance_id)
            if game is None:
                raise ValueError("instance_not_found")
            self.exe_profiles.resolve(game, release_id)
        if re.fullmatch(RELEASE_PATTERN, release_id):
            bridge = self.publisher.bridge_for(instance_id, release_id)
            if bridge is None:
                raise ValueError("publisher_prepare_first")
            return await bridge.preview(instance_id, release_id)
        return await self.bridge.preview(instance_id, release_id)

    async def _plan_install(self, instance_id: str, release_id: str, archive_path: str = "", operation="install"):
        self._installable_release(release_id)
        if re.fullmatch(RELEASE_PATTERN, release_id):
            raise ValueError("publisher_prepare_first")
        archive = None
        if archive_path:
            if archive_path == "~" or archive_path.startswith("~/"):
                archive = Path(decky.DECKY_USER_HOME) / archive_path[2:]
            elif archive_path.startswith("~"):
                raise ValueError("invalid_archive")
            else:
                archive = Path(archive_path)
            archive = archive.resolve(strict=True)
            if archive.suffix.lower() != ".zip" or not archive.is_file():
                raise ValueError("invalid_archive")
            if not (archive.is_relative_to(self.downloads) or archive.is_relative_to(self.bridge.state_dir / "downloads")):
                raise ValueError("archive_outside_downloads")
        return await self.bridge.plan_install(instance_id, release_id, archive, operation)

    async def fetch_artifact(self, instance_id: str, release_id: str):
        self._installable_release(release_id)
        if release_id.startswith("exe-"):
            raise ValueError("exe_prepare_first")
        return await self.bridge.fetch_artifact(instance_id, release_id)

    async def _prepare_install(self, instance_id: str, release_id: str, archive_path: str = "", operation="install"):
        """Acquire a reviewed ZIP and prepare a plan; never write game files."""
        self._installable_release(release_id)
        if self._prepare_cancel is not None:
            raise RuntimeError("installer_busy")
        task = current_task.get()
        if self.tasks.busy() and task is None:
            raise RuntimeError("installer_busy")
        cancel = task.cancel if task else Event()
        self._prepare_cancel = cancel
        try:
            if task:
                task.stage("scanning")
                task.check()
            report = await self._read_report()
            if task:
                task.check()
            row = next((i for i in report["installs"] if i["game"]["instance_id"] == instance_id), None)
            if row is None:
                raise ValueError("instance_not_found")
            patch = next((p for p in row["patches"] if p["release_id"] == release_id), None)
            if patch is None and re.fullmatch(r"exe-selection-[0-9a-f]{24}-[1-9][0-9]*", release_id):
                # The same source selection may now project as its persisted
                # content identity after a previous preparation or install.
                _, candidate = self.exe_profiles.resolve(row["game"], release_id)
                patch = next((p for p in row["patches"] if p.get("candidate_id") == candidate["candidate_id"]
                              and p["support_level"] == "publisher_exe"), None)
                if patch is not None:
                    release_id = patch["release_id"]
            if patch is None:
                raise ValueError("release_not_found")
            if patch["blockers"]:
                raise ValueError(patch["blockers"][0])
            if patch["support_level"] == "publisher_exe":
                if archive_path:
                    raise ValueError("exe_manual_import_not_supported")
                bridge, archive, plan, acquisition = await self.exe_installation.prepare(row["game"], release_id, cancel, operation=operation)
                release_id = plan.get("release_id", release_id)
                if cancel.is_set():
                    await self._revoke_plan(bridge, plan["plan_id"], (instance_id, release_id))
                    raise RuntimeError("download_cancelled")
                wrapped = self._wrap_publisher(plan, instance_id, release_id)
                self._plan_metadata[wrapped["plan_id"]] = dict(instance_id=instance_id, release_id=release_id,
                    archive_bytes=Path(archive).stat().st_size,
                    cache={"kind": "publisher", "instance_id": instance_id, "release_id": release_id})
                return {"archive": archive, "acquisition": acquisition, "mode": "publisher_exe", "plan": wrapped}
            local_archive = self._local_archive(archive_path) if archive_path else None
            if patch["support_level"] == "publisher_zip":
                entry = self.publisher.match(row["game"], release_id)
                if entry is None or entry["release_id"] != release_id:
                    raise ValueError("publisher_match_changed")
                bridge, archive, plan, acquisition = await self.publisher.prepare(row["game"], entry, cancel, local_archive, operation=operation)
                if cancel.is_set():
                    await self._revoke_plan(bridge, plan["plan_id"], (instance_id, release_id))
                    raise RuntimeError("download_cancelled")
                wrapped = self._wrap_publisher(plan, instance_id, release_id)
                self._plan_metadata[wrapped["plan_id"]] = dict(
                    instance_id=instance_id, release_id=release_id, archive_bytes=Path(archive).stat().st_size,
                    cache={"kind": "publisher", "instance_id": instance_id, "release_id": release_id})
                return {"archive": archive, "acquisition": acquisition, "mode": "publisher_root",
                    "plan": wrapped}
            if report.get("catalog_trust") != "signed":
                raise ValueError("catalog_untrusted")
            if patch["support_level"] != "managed_zip":
                raise ValueError("source_requires_guidance")
            digest, size, filename = release_metadata(self.bridge.signed_catalog, report["catalog_revision"], release_id)
            if task:
                task.stage("searching")
            archive = local_archive or await asyncio.to_thread(find_archive,
                [self.bridge.state_dir / "downloads", self.downloads], digest, size, filename, cancel)
            acquisition = "local"
            if cancel.is_set():
                raise RuntimeError("download_cancelled")
            if archive is None:
                if not patch.get("download_available"):
                    raise ValueError("artifact_requires_manual_download")
                result = await self.bridge.fetch_artifact(instance_id, release_id)
                archive = result["archive"]
                acquisition = "download"
            if cancel.is_set():
                raise RuntimeError("download_cancelled")
            plan = await self._plan_install(instance_id, release_id, str(archive), operation)
            if cancel.is_set():
                await self._revoke_plan(self.bridge, plan["plan_id"])
                raise RuntimeError("download_cancelled")
            metadata = dict(instance_id=instance_id, release_id=release_id, archive_bytes=Path(archive).stat().st_size)
            try:
                core_cache = self.bridge.state_dir / "downloads" / f"{digest}.zip"
                if Path(archive).resolve() == core_cache.resolve():
                    metadata["cache"] = {"kind": "core", "sha256": digest}
            except OSError:
                pass
            self._plan_metadata[plan["plan_id"]] = metadata
            return {"archive": str(archive), "acquisition": acquisition, "mode": "reviewed", "plan": plan}
        finally:
            self._prepare_cancel = None

    async def list_sources(self):
        return self.sources.list_sources()

    async def set_source_enabled(self, source_id: str, enabled: bool):
        if self._prepare_cancel is not None or self.tasks.busy():
            raise RuntimeError("installer_busy")
        await _await_maybe(self.sources.set_enabled(source_id, enabled))
        return self.sources.list_sources()

    async def add_source(self, rule_json: str):
        if self._prepare_cancel is not None or self.tasks.busy():
            raise RuntimeError("installer_busy")
        import json
        if not isinstance(rule_json, str) or len(rule_json) > 32768:
            raise ValueError("invalid_source_rule")
        await _await_maybe(self.sources.add_source(json.loads(rule_json)))
        return self.sources.list_sources()

    async def set_game_source(self, appid: int, source_id: str, subscribed: bool):
        if self._prepare_cancel is not None or self.tasks.busy():
            raise RuntimeError("installer_busy")
        await _await_maybe(self.sources.set_game_source(appid, source_id, subscribed))
        return {"appid": appid}

    async def cancel_download(self):
        if self._prepare_cancel is not None:
            self._prepare_cancel.set()
        return await self.bridge.cancel_download()

    async def _start_install(self, plan_id: str):
        if self.tasks.busy() and current_task.get() is None:
            raise RuntimeError("installer_busy")
        bridge, value, owner = self._dispatch_id(plan_id)
        if owner and owner[1].startswith("exe-"):
            if current_task.get():
                current_task.get().stage("checking_game")
            report = await self.bridge.scan()
            game = next((row["game"] for row in report["installs"] if row["game"]["instance_id"] == owner[0]), None)
            if game is None:
                raise ValueError("instance_not_found")
            await self.exe_installation.validate_apply(game, owner[1], value)
        result = await bridge.start_install(value)
        if result.get("state") == "committed":
            await self._cleanup_after_install(plan_id, value, owner, self.settings.auto_cleanup)
        elif owner and owner[1].startswith("exe-") and result.get("state") == "rolled_back":
            try:
                self.exe_installation.release_lease(self.publisher._state(*owner), value)
            except (OSError, ValueError):
                pass  # Keep malformed evidence pinned rather than hide rollback.
        return self._wrap_publisher(result, *owner) if owner else result

    async def _cleanup_after_install(self, plan_id, core_plan_id, owner, enabled):
        """Best-effort post-commit cleanup; never changes the install outcome."""
        metadata = self._plan_metadata.pop(plan_id, None) or {}
        cache = metadata.get("cache") or {}
        if owner and owner[1].startswith("exe-"):
            cache = {"kind": "publisher", "instance_id": owner[0], "release_id": owner[1]}
            try:
                self.exe_installation.release_lease(self.publisher._state(*owner), core_plan_id)
            except Exception:
                # Commit already succeeded. Damaged evidence pins the cache;
                # housekeeping must not turn a committed result into a failure.
                enabled = False
        if enabled:
            try:
                if cache.get("kind") == "publisher":
                    await asyncio.to_thread(self.publisher.clear_cache, cache["instance_id"], cache["release_id"])
                elif cache.get("kind") == "core":
                    await asyncio.to_thread(self._clear_core_cache, cache.get("sha256"))
            except Exception:
                pass
        # The core keeps the durable receipt and clears only terminal blobs.
        try:
            await self._dispatch_id(plan_id)[0].cleanup_transactions()
        except Exception:
            pass

    def _jobs_root(self, owner):
        if owner is None:
            return self.bridge.state_dir / "jobs"
        instance_id, release_id = owner
        return self.publisher._state(instance_id, release_id) / "jobs"

    async def cache_status(self):
        status = await asyncio.to_thread(caches.status, self.publisher, self.bridge.state_dir / "downloads")
        status["jobs"] = await asyncio.to_thread(caches.jobs_status, self.publisher, self.bridge.state_dir)
        return status

    def _clear_core_cache(self, sha256):
        return caches.clear_core(self.bridge.state_dir / "downloads", sha256)

    async def clear_cache(self):
        if self.tasks.busy() or self._prepare_cancel is not None:
            raise RuntimeError("installer_busy")
        removed = await asyncio.to_thread(caches.clear, self.publisher, self.bridge.state_dir / "downloads")
        status = await self.cache_status()
        status["removed_bytes"] = removed
        return status

    async def get_settings(self):
        return {**self.settings.snapshot(), "cache": await self.cache_status()}

    async def set_auto_cleanup(self, enabled: bool):
        self.settings.set_auto_cleanup(enabled)
        return await self.get_settings()

    async def list_transactions(self):
        return await self._list_transactions()

    async def _list_transactions(self):
        result = await self.bridge.list_transactions()
        for instance_id, release_id, bridge in self.publisher.bridges():
            transactions = await bridge.list_transactions()
            result.extend(self._wrap_publisher(tx, instance_id, release_id) for tx in transactions)
        return result

    async def recover(self):
        if self.tasks.busy():
            raise RuntimeError("installer_busy")
        result = await self.bridge.recover()
        for instance_id, release_id, bridge in self.publisher.bridges():
            result.extend(self._wrap_publisher(tx, instance_id, release_id) for tx in await bridge.recover())
        return result

    def _local_archive(self, archive_path):
        if archive_path.startswith("~/"):
            path = Path(decky.DECKY_USER_HOME) / archive_path[2:]
        elif archive_path.startswith("~"):
            raise ValueError("invalid_archive")
        else:
            path = Path(archive_path)
        path = path.resolve(strict=True)
        if path.suffix.lower() != ".zip" or not path.is_file():
            raise ValueError("invalid_archive")
        if not (path.is_relative_to(self.downloads) or path.is_relative_to(self.bridge.state_dir / "downloads")):
            raise ValueError("archive_outside_downloads")
        return path

    async def _revoke_plan(self, bridge, plan_id, owner=None):
        # Local durable revocation precedes the possibly failing RPC.
        from exe_installation import revoke_plan
        revoke_plan(bridge.state_dir, plan_id)
        if owner and owner[1].startswith("exe-"):
            self.exe_installation.release_lease(self.publisher._state(*owner), plan_id)
        return await bridge.discard_plan(plan_id)

    async def start_patch_install_task(self, instance_id: str, release_id: str, operation: str, request_id: str, archive_path: str = ""):
        self._installable_release(release_id)
        if operation not in ("install", "reinstall"):
            raise ValueError("invalid_install_operation")
        if not isinstance(instance_id, str) or not instance_id or len(instance_id) > 1024:
            raise ValueError("invalid_id")
        async def work(task):
            plan_id = None
            try:
                value = await self._prepare_install(instance_id, release_id, archive_path, operation)
                plan_id = value["plan"]["plan_id"]
                task.update(plan_id=plan_id, archive_bytes=Path(value["archive"]).stat().st_size)
                self.tasks._save()
                task.check()
                return await self._start_install(plan_id)
            except BaseException:
                if plan_id:
                    bridge, core_id, owner = self._dispatch_id(plan_id)
                    try:
                        await self._revoke_plan(bridge, core_id, owner)
                    except Exception:
                        pass  # Local marker already invalidated the plan.
                raise
        return self.tasks.start("install", work, instance_id=instance_id, release_id=release_id,
                                operation=operation, request_id=request_id, archive_path=archive_path)

    async def task_status(self, task_id: str):
        return self.tasks.status(task_id)

    async def cancel_task(self, task_id: str):
        task = self.tasks.get(task_id)
        task.request_cancel()
        return task.snapshot()

    async def list_tasks(self):
        return [task.snapshot() for task in self.tasks.tasks.values()]

    async def discard_plan(self, plan_id: str):
        if self.tasks.busy():
            raise RuntimeError("installer_busy")
        bridge, value, owner = self._dispatch_id(plan_id)
        result = await self._revoke_plan(bridge, value, owner)
        if owner and owner[1].startswith("exe-"):
            try:
                self.exe_installation.release_lease(self.publisher._state(*owner), value)
            except Exception:
                pass  # Rust already discarded safely; keep an unreadable cache pinned.
        self._plan_metadata.pop(plan_id, None)
        return self._wrap_publisher(result, *owner) if owner else result

    async def _unload(self):
        session = getattr(self, "_scan_session", None)
        if session:
            await session.close()
        await self.tasks.shutdown()
        await self.sources.close()


def _await_startup(method):
    from functools import wraps
    @wraps(method)
    async def ready(self, *args, **kwargs):
        event = getattr(self, "_startup_ready", None)
        if event is not None:
            await event.wait()
            if self._startup_error is not None:
                raise RuntimeError("plugin_startup_failed") from self._startup_error
        return await method(self, *args, **kwargs)
    return ready


# Loader schedules _main asynchronously; frontend RPCs must not race migration.
for _name, _method in list(vars(Plugin).items()):
    if not _name.startswith("_") and asyncio.iscoroutinefunction(_method):
        setattr(Plugin, _name, _await_startup(_method))
