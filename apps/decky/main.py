"""Decky host for the bundled protocol-v4 Rust installer and sf-worker."""
from pathlib import Path
import sys
import os
import asyncio
import json
import re
import uuid
import decky

sys.path.insert(0, str(Path(__file__).parent / "backend"))
from bridge import CoreBridge
from protocol_validation import validate_id, MAX_INSTANCE_ID_BYTES
from plugin_settings import Settings, _no_links
from source_service import SourceService
from state_host import prepare_host_state
from tasks import TaskManager
from scan_session import ScanSession


def _merge_diagnostics(*groups):
    result = []
    for group in groups:
        for item in group if isinstance(group, list) else []:
            if item not in result:
                result.append(item)
    return result


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
        home = Path(decky.DECKY_USER_HOME)
        state_dir = home / ".local/share/steam-patch-manager"
        run_uid = run_gid = None
        if os.geteuid() == 0:
            import pwd
            account = pwd.getpwnam(home.name)
            if account.pw_uid == 0 or Path(account.pw_dir) != home:
                raise ValueError("unprivileged_user_required")
            run_uid, run_gid = account.pw_uid, account.pw_gid
        self.bridge = CoreBridge(root / "bin/steam-patch", root / "bin/catalog.json",
            signed_catalog=root / "bin/signed-catalog.json", trust_keys=root / "bin/trust-keys.json",
            state_dir=state_dir, run_uid=run_uid, run_gid=run_gid, user_home=home)
        await self.bridge.preflight()
        prepare_host_state(state_dir, home, run_uid, run_gid)
        self.downloads = (home / "Downloads").resolve()
        self.tasks = TaskManager(state_dir=state_dir / "host-v3")
        self.settings = Settings(state_dir)
        self.sources = await SourceService(self.bridge, state_dir).initialize()
        self._scan_generation = 0
        self._scan_games = {}
        self._scan_session = None
        self._cache_result = None
        # Old recovery is a deployment gate; this RPC reads the new schema only.
        await self.bridge.recover()

    async def get_capabilities(self):
        return await self.bridge.call_sync("get_capabilities", {})

    async def _project_row(self, row, transactions=None):
        # Events carry discoveries only. Rust projects selections and receipts.
        report = await self.bridge.scan()
        self.sources.prime(report)
        found = next((item for item in report.get("installs", [])
                      if item["game"]["instance_id"] == row["game"]["instance_id"]), None)
        if found is not None:
            row.clear()
            row.update(found)

    async def _refresh_rows(self, report):
        # Reproject once for the complete snapshot. Per-row scans would rehash
        # every installed payload for each source event.
        fresh = await self.bridge.scan()
        self.sources.prime(fresh)
        by_instance = {item["game"]["instance_id"]: item for item in fresh.get("installs", [])}
        for row in report["installs"]:
            found = by_instance.get(row["game"]["instance_id"])
            if found is not None:
                row.clear()
                row.update(found)
        report["sources"] = fresh.get("sources", [])
        report["diagnostics"] = _merge_diagnostics(report.get("diagnostics"), fresh.get("diagnostics"))

    async def _read_report(self):
        report = await self.bridge.scan()
        self.sources.prime(report)
        report["generation"] = getattr(self, "_scan_generation", 0)
        report["settings_revision"] = self.sources.settings_revision
        return report

    async def scan(self, refresh_sources: bool = False):
        self._require_idle()
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
                session.report["sources"] = self.sources.list_sources()
                session.report["diagnostics"] = _merge_diagnostics(session.report.get("diagnostics"), diagnostics)
                await self._refresh_rows(session.report)
            return diagnostics

        async def scan_sources(on_event, cancel):
            result = await self.sources.scan_sources(force=refresh_sources, on_event=on_event, cancel=cancel)
            if current() and not cancel.is_set():
                await self._refresh_rows(session.report)
            return result

        session = ScanSession(report, refresh, scan_sources, self._project_row, current, project_on_update=False)
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
        validate_id(instance_id, MAX_INSTANCE_ID_BYTES)
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
        row = {"game": game, "discovery": result}
        if result.get("status") != "stale":
            await self._project_row(row)
        result["patches"] = row.get("patches", [])
        return result

    async def preview(self, instance_id, release_id=None, selection_id=None):
        return await self.bridge.preview(instance_id, release_id, selection_id)

    async def list_sources(self):
        return self.sources.list_sources()

    def _require_idle(self):
        if self.tasks.busy():
            raise RuntimeError("installer_busy")

    async def set_source_enabled(self, source_id, enabled):
        self._require_idle()
        if type(enabled) is not bool:
            raise ValueError("invalid_request")
        await self.sources.set_enabled(source_id, enabled)
        return self.sources.list_sources()

    async def add_source(self, rule_json):
        self._require_idle()
        if not isinstance(rule_json, str) or len(rule_json) > 32768:
            raise ValueError("invalid_source_rule")
        await self.sources.add_source(json.loads(rule_json))
        return self.sources.list_sources()

    async def set_game_source(self, appid, source_id, subscribed):
        self._require_idle()
        if type(subscribed) is not bool:
            raise ValueError("invalid_request")
        await self.sources.set_game_source(appid, source_id, subscribed)
        return {"appid": appid}

    async def get_settings(self):
        return {**self.settings.snapshot(), "cache": getattr(self, "_cache_result", None)}

    async def set_auto_cleanup(self, enabled):
        self.settings.set_auto_cleanup(enabled)
        return await self.get_settings()

    async def backup_status(self):
        return await self.bridge.backup_status()

    async def cleanup_backups(self):
        self._require_idle()
        return await self.bridge.cleanup_backups()

    async def clear_cache(self):
        self._require_idle()
        self._cache_result = await self.bridge.cleanup_cache()
        return self._cache_result

    async def list_transactions(self):
        return await self.bridge.list_transactions()

    async def recover(self):
        self._require_idle()
        return await self.bridge.recover()

    async def discard_plan(self, plan_id):
        self._require_idle()
        return await self.bridge.discard_plan(plan_id)

    def _local_archive(self, archive):
        if not isinstance(archive, str) or not archive:
            raise ValueError("invalid_archive")
        if archive.startswith("~/"):
            path = Path(decky.DECKY_USER_HOME) / archive[2:]
        elif archive.startswith("~"):
            raise ValueError("invalid_archive")
        else:
            path = Path(archive)
        if not path.is_absolute() or not _no_links(path):
            raise ValueError("invalid_archive")
        path = path.resolve(strict=True)
        if path.suffix.lower() != ".zip" or not path.is_file() or path.stat().st_nlink != 1:
            raise ValueError("invalid_archive")
        if not (path.is_relative_to(self.downloads) or path.is_relative_to(self.bridge.state_dir / "downloads")):
            raise ValueError("archive_outside_downloads")
        return path

    async def start_patch_install_task(self, instance_id, release_id=None, selection_id=None,
                                       archive=None, operation=None, request_id=None):
        self.bridge.selection(instance_id, release_id, selection_id)
        if operation not in ("install", "reinstall", "overwrite"):
            raise ValueError("invalid_operation")
        if selection_id is not None and archive is not None:
            raise ValueError("archive_not_allowed_for_selection")
        local = self._local_archive(archive) if archive is not None else None
        request_id = str(uuid.uuid4()) if request_id is None else validate_id(request_id, 128, "invalid_request_id")
        cleanup_policy = "after_commit" if self.settings.auto_cleanup else "disabled"

        async def work(task):
            session = getattr(self, "_scan_session", None)
            if session:
                await session.close()
            task.check()
            result = await self.bridge.install_patch(instance_id, release_id, selection_id,
                local, operation, request_id, cleanup_policy)
            if isinstance(result.get("cleanup"), dict):
                self._cache_result = result["cleanup"]
            return result

        return self.tasks.start("install", work, instance_id=instance_id, release_id=release_id,
            selection_id=selection_id, operation=operation, request_id=request_id,
            archive_path=str(local) if local is not None else None)

    async def start_install(self, instance_id, release_id=None, selection_id=None,
                            archive=None, operation=None, request_id=None):
        return await self.start_patch_install_task(instance_id, release_id, selection_id,
            archive, operation, request_id)

    async def task_status(self, task_id):
        return self.tasks.status(task_id)

    async def cancel_task(self, task_id):
        task = self.tasks.get(task_id)
        task.request_cancel()
        return task.snapshot()

    async def list_tasks(self):
        return [task.snapshot() for task in self.tasks.tasks.values()]

    async def _unload(self):
        session = getattr(self, "_scan_session", None)
        if session:
            await session.close()
        if hasattr(self, "tasks"):
            await self.tasks.shutdown()
        if hasattr(self, "sources"):
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
