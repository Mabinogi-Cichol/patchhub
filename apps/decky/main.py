"""Decky entry point. Bundle bin/steam-patch and catalog files with this plugin."""
from pathlib import Path
import sys
import os
import asyncio
from threading import Event
import hashlib
import re

import decky

sys.path.insert(0, str(Path(__file__).parent / "backend"))
import caches
from bridge import CoreBridge
from acquisition import find_archive, release_metadata
from publisher import Publisher, _released, RELEASE_PATTERN
from plugin_settings import Settings
from sources import SourceRegistry
from tasks import TaskManager, current_task


class Plugin:
    async def _main(self):
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
        self.tasks = TaskManager()
        self._plan_metadata = {}
        self.settings = Settings(state_dir)
        self.sources = SourceRegistry(state_dir, root / "backend/sources.json")
        self.publisher = Publisher(self.bridge, self.sources)

    async def scan(self, refresh_sources: bool = True):
        source_diagnostics = await self.sources.refresh(force=True) if refresh_sources else []
        report = await self.bridge.scan()
        report["diagnostics"].extend({"code": issue["code"], "path": issue.get("source_id", ""),
            "message": issue.get("detail", "")} for issue in source_diagnostics
            if issue["code"] not in ("source_refreshed", "source_not_modified"))
        report["sources"] = self.sources.list_sources()
        source_by_id = {source["id"]: source for source in report["sources"]}
        existing_transactions = await self.list_transactions()
        for row in report["installs"]:
            game = row["game"]
            row["sources"] = self.sources.list_game_sources(game)
            # Source subscriptions own discovery. The bundled catalog remains
            # available to the core for existing receipts and CLI operations.
            row["patches"] = []
            row["match_status"] = "no_source_match"
            entries = self.publisher.matches(game)
            for entry in entries:
                patch = dict(release_id=entry["release_id"], name=f"{entry['source_id']} · 官方 ZIP · {entry['language']} · 默认游戏根目录",
                    source_page=entry["source_page"], support_level="publisher_zip", download_available=True,
                    compatibility="publisher_default", blockers=[], detection=dict(status="unknown", files=[]))
                source = source_by_id.get(entry["source_id"], {})
                patch.update(source_id=entry["source_id"], source_name=source.get("name"),
                    source_status=source.get("status", "unknown"), source_cached_at=source.get("cached_at"))
                if not _released(entry):
                    patch["blockers"].append("source_not_released")
                if game.get("availability") != "installed":
                    patch["blockers"].append("game_not_ready")
                if not game.get("buildid"):
                    patch["blockers"].append("build_not_verified")
                if any(t.get("instance_id") == game["instance_id"] and t.get("release_id") != entry["release_id"]
                       and t["state"] not in ("planned", "uninstalled", "rolled_back")
                       and t.get("backups") is not False for t in existing_transactions):
                    patch["blockers"].append("publisher_existing_transaction")
                local = self.publisher.bridge_for(game["instance_id"], entry["release_id"])
                if local is not None:
                    try:
                        observed = await local.scan()
                        if observed.get("catalog_error") == "catalog_expired":
                            local = await self.publisher.refresh_signature(game["instance_id"], entry["release_id"])
                            observed = await local.scan()
                        local_row = next((i for i in observed["installs"] if i["game"]["instance_id"] == game["instance_id"]), None)
                        local_patch = next((p for p in (local_row or {}).get("patches", []) if p["release_id"] == entry["release_id"]), None)
                        if observed.get("catalog_trust") != "signed":
                            patch["blockers"].append("catalog_untrusted")
                        if local_patch:
                            patch["detection"] = local_patch["detection"]
                            transactions = await local.list_transactions()
                            # A committed install without backups cannot be uninstalled, so it
                            # must not block re-installing over it. Without a known state dir
                            # (e.g. a test double) keep the conservative blocking behaviour.
                            local_state = getattr(local, "state_dir", None)
                            if local_state is None:
                                active = any(t["state"] not in ("planned", "uninstalled", "rolled_back") for t in transactions)
                            else:
                                local_jobs = Path(local_state) / "jobs"
                                active = any(t["state"] not in ("planned", "uninstalled", "rolled_back")
                                             and caches.has_backups(local_jobs, t["transaction_id"]) for t in transactions)
                            if active:
                                patch["blockers"].append("publisher_existing_transaction")
                            patch["blockers"].extend(b for b in local_patch["blockers"]
                                if active or b not in ("version_incompatible", "build_not_verified"))
                    except Exception:
                        patch["blockers"].append("publisher_state_unavailable")
                row["patches"].insert(0, patch)
                row["match_status"] = "matched"
        for issue in self.publisher.state_diagnostics:
            report["diagnostics"].append({"code": "publisher_state_unavailable", "path": issue.get("path", ""), "message": issue.get("code", "")})
        return report

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
        if re.fullmatch(RELEASE_PATTERN, release_id):
            bridge = self.publisher.bridge_for(instance_id, release_id)
            if bridge is None:
                raise ValueError("publisher_prepare_first")
            return await bridge.preview(instance_id, release_id)
        return await self.bridge.preview(instance_id, release_id)

    async def plan_install(self, instance_id: str, release_id: str, archive_path: str = ""):
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
        return await self.bridge.plan_install(instance_id, release_id, archive)

    async def fetch_artifact(self, instance_id: str, release_id: str):
        return await self.bridge.fetch_artifact(instance_id, release_id)

    async def prepare_install(self, instance_id: str, release_id: str, archive_path: str = ""):
        """Acquire a reviewed ZIP and prepare a plan; never write game files."""
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
            report = await self.scan(refresh_sources=False)
            if task:
                task.check()
            row = next((i for i in report["installs"] if i["game"]["instance_id"] == instance_id), None)
            if row is None:
                raise ValueError("instance_not_found")
            patch = next((p for p in row["patches"] if p["release_id"] == release_id), None)
            if patch is None:
                raise ValueError("release_not_found")
            if patch["blockers"]:
                raise ValueError(patch["blockers"][0])
            local_archive = self._local_archive(archive_path) if archive_path else None
            if patch["support_level"] == "publisher_zip":
                entry = self.publisher.match(row["game"], release_id)
                if entry is None or entry["release_id"] != release_id:
                    raise ValueError("publisher_match_changed")
                state_of = getattr(self.publisher, "_state", None)
                if state_of is not None:
                    await asyncio.to_thread(self._forget_without_backups, state_of(instance_id, release_id))
                bridge, archive, plan, acquisition = await self.publisher.prepare(row["game"], entry, cancel, local_archive)
                if cancel.is_set():
                    await bridge.discard_plan(plan["plan_id"])
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
            base_state = getattr(self.bridge, "state_dir", None)
            if base_state is not None:
                await asyncio.to_thread(self._forget_without_backups, base_state)
            plan = await self.plan_install(instance_id, release_id, str(archive))
            if cancel.is_set():
                await self.bridge.discard_plan(plan["plan_id"])
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
        self.sources.set_enabled(source_id, enabled)
        return self.sources.list_sources()

    async def add_source(self, rule_json: str):
        if self._prepare_cancel is not None or self.tasks.busy():
            raise RuntimeError("installer_busy")
        import json
        if not isinstance(rule_json, str) or len(rule_json) > 32768:
            raise ValueError("invalid_source_rule")
        self.sources.add_source(json.loads(rule_json))
        return self.sources.list_sources()

    async def set_game_source(self, appid: int, source_id: str, subscribed: bool):
        if self._prepare_cancel is not None or self.tasks.busy():
            raise RuntimeError("installer_busy")
        self.sources.set_game_source(appid, source_id, subscribed)
        return {"appid": appid}

    async def cancel_download(self):
        if self._prepare_cancel is not None:
            self._prepare_cancel.set()
        return await self.bridge.cancel_download()

    async def start_install(self, plan_id: str, confirmed_game_closed: bool = False):
        if self.tasks.busy() and current_task.get() is None:
            raise RuntimeError("installer_busy")
        bridge, value, owner = self._dispatch_id(plan_id)
        result = await bridge.start_install(value, confirmed_game_closed)
        if result.get("state") == "committed":
            await self._cleanup_after_install(plan_id, value, owner, self.settings.auto_cleanup)
        return self._wrap_publisher(result, *owner) if owner else result

    async def _cleanup_after_install(self, plan_id, core_plan_id, owner, enabled):
        """Best-effort post-commit cleanup; never changes the install outcome."""
        metadata = self._plan_metadata.pop(plan_id, None) or {}
        cache = metadata.get("cache")
        if enabled:
            try:
                if cache.get("kind") == "publisher":
                    await asyncio.to_thread(self.publisher.clear_cache, cache["instance_id"], cache["release_id"])
                elif cache.get("kind") == "core":
                    await asyncio.to_thread(self._clear_core_cache, cache.get("sha256"))
            except Exception:
                pass
        # Staged copies are not needed after commit: apply is done and uninstall
        # restores from backups only. Backups are removed unless the user opted
        # into keeping them (which keeps uninstall available).
        try:
            suffixes = (".stage",) if self.settings.keep_backups else (".stage", ".backup")
            await asyncio.to_thread(caches.delete_files, self._jobs_root(owner), core_plan_id, suffixes)
        except Exception:
            pass

    def _jobs_root(self, owner):
        if owner is None:
            return self.bridge.state_dir / "jobs"
        instance_id, release_id = owner
        return self.publisher._state(instance_id, release_id) / "jobs"

    def _forget_without_backups(self, state_dir):
        """Clear a core transaction store that holds no backups.

        Such an install cannot be uninstalled, so its committed record must not
        block re-installing over it. The recipe, signed catalog and keys stay.
        """
        jobs = Path(state_dir) / "jobs"
        if not jobs.is_dir() or caches.any_backups(jobs):
            return
        caches.forget_transactions(state_dir)

    def _clear_core_cache(self, sha256):
        return caches.clear_core(self.bridge.state_dir / "downloads", sha256)

    async def cache_status(self):
        status = await asyncio.to_thread(caches.status, self.publisher, self.bridge.state_dir / "downloads")
        status["jobs"] = await asyncio.to_thread(caches.jobs_status, self.publisher, self.bridge.state_dir)
        return status

    async def clear_cache(self):
        if self.tasks.busy() or self._prepare_cancel is not None:
            raise RuntimeError("installer_busy")
        removed = await asyncio.to_thread(caches.clear, self.publisher, self.bridge.state_dir / "downloads")
        status = await self.cache_status()
        status["removed_bytes"] = removed
        return status

    async def clear_transaction_backups(self):
        """Delete staged/backup files for finished transactions, freeing space."""
        if self.tasks.busy() or self._prepare_cancel is not None:
            raise RuntimeError("installer_busy")
        removable = ("committed", "uninstalled", "rolled_back")
        removed = 0
        base_ids = {t["transaction_id"] for t in await self.bridge.list_transactions() if t["state"] in removable}
        removed += await asyncio.to_thread(caches.clear_jobs, self.bridge.state_dir / "jobs", base_ids)
        for instance_id, release_id, bridge in self.publisher.bridges():
            ids = {t["transaction_id"] for t in await bridge.list_transactions() if t["state"] in removable}
            jobs_root = self.publisher._state(instance_id, release_id) / "jobs"
            removed += await asyncio.to_thread(caches.clear_jobs, jobs_root, ids)
        status = await self.cache_status()
        status["removed_bytes"] = removed
        return status

    async def get_settings(self):
        return {**self.settings.snapshot(), "cache": await self.cache_status()}

    async def set_auto_cleanup(self, enabled: bool):
        self.settings.set_auto_cleanup(enabled)
        return await self.get_settings()

    async def set_keep_backups(self, enabled: bool):
        self.settings.set_keep_backups(enabled)
        return await self.get_settings()

    async def list_transactions(self):
        result = await self.bridge.list_transactions()
        self._annotate_backups(result, self.bridge.state_dir / "jobs")
        for instance_id, release_id, bridge in self.publisher.bridges():
            transactions = await bridge.list_transactions()
            self._annotate_backups(transactions, self.publisher._state(instance_id, release_id) / "jobs")
            result.extend(self._wrap_publisher(tx, instance_id, release_id) for tx in transactions)
        return result

    @staticmethod
    def _annotate_backups(transactions, jobs_root):
        for transaction in transactions:
            transaction["backups"] = caches.has_backups(jobs_root, transaction.get("transaction_id"))

    async def uninstall(self, transaction_id: str, confirmed_game_closed: bool = False):
        if self.tasks.busy():
            raise RuntimeError("installer_busy")
        bridge, value, owner = self._dispatch_id(transaction_id)
        if not caches.has_backups(self._jobs_root(owner), value):
            raise ValueError("uninstall_backups_missing")
        result = await bridge.uninstall(value, confirmed_game_closed)
        return self._wrap_publisher(result, *owner) if owner else result

    async def recover(self, confirmed_game_closed: bool = False):
        if self.tasks.busy():
            raise RuntimeError("installer_busy")
        result = await self.bridge.recover(confirmed_game_closed)
        for instance_id, release_id, bridge in self.publisher.bridges():
            result.extend(self._wrap_publisher(tx, instance_id, release_id) for tx in await bridge.recover(confirmed_game_closed))
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

    async def start_prepare_task(self, instance_id: str, release_id: str, archive_path: str = ""):
        async def work(task):
            return await self.prepare_install(instance_id, release_id, archive_path)
        return self.tasks.start("prepare", work, instance_id=instance_id, release_id=release_id)

    async def start_install_task(self, plan_id: str, confirmed_game_closed: bool = False):
        if confirmed_game_closed is not True:
            raise ValueError("game_closed_confirmation_required")
        self._dispatch_id(plan_id)
        async def work(task):
            metadata = self._plan_metadata.get(plan_id, {})
            if metadata.get("archive_bytes") is not None:
                task.update(archive_bytes=metadata["archive_bytes"])
            return await self.start_install(plan_id, True)
        metadata = self._plan_metadata.get(plan_id, {})
        return self.tasks.start("install", work, plan_id=plan_id,
            instance_id=metadata.get("instance_id"), release_id=metadata.get("release_id"))

    async def task_status(self, task_id: str):
        return self.tasks.get(task_id).snapshot(renew=True)

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
        result = await bridge.discard_plan(value)
        return self._wrap_publisher(result, *owner) if owner else result

    async def _unload(self):
        await self.tasks.shutdown()
