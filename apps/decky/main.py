"""Decky host for the bundled protocol-v5 Rust installer and patchhub-worker."""
from pathlib import Path
import sys
import os
import asyncio
import json
import logging
import time
import uuid
import decky

sys.path.insert(0, str(Path(__file__).parent / "backend"))
from bridge import CoreBridge
from protocol_validation import validate_id, MAX_INSTANCE_ID_BYTES
from plugin_settings import Settings, _no_links
from source_service import SourceService, due_source_ids
from state_host import prepare_host_state
from tasks import TaskManager
from app_update import AppUpdate

# Runtime-only automatic update check. The core persists intervals and derives
# next_due_at; the host never stores its own timers (p1-contract §10).
SCHEDULER_INTERVAL_SECONDS = 60


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
        state_dir = home / ".local/share/patchhub"
        self.app_update = AppUpdate(root, state_dir)
        self._app_update_gate = (home / '.cache/patchhub-decky-install/journal.json').exists()
        self._app_update_checker = asyncio.create_task(self._app_update_loop())
        run_uid = run_gid = None
        if os.geteuid() == 0:
            import pwd
            account = pwd.getpwnam(home.name)
            if account.pw_uid == 0 or Path(account.pw_dir) != home:
                raise ValueError("unprivileged_user_required")
            run_uid, run_gid = account.pw_uid, account.pw_gid
        self.bridge = CoreBridge(root / "bin/patchhub-core", root / "bin/catalog.json",
            signed_catalog=root / "bin/signed-catalog.json", trust_keys=root / "bin/trust-keys.json",
            state_dir=state_dir, run_uid=run_uid, run_gid=run_gid, user_home=home)
        await self.bridge.preflight()
        prepare_host_state(state_dir, home, run_uid, run_gid)
        self.downloads = (home / "Downloads").resolve()
        self.tasks = TaskManager(state_dir=state_dir / "host-v3")
        self.settings = Settings(state_dir)
        self.sources = await SourceService(self.bridge, state_dir).initialize()
        self._scan_generation = 0
        self._cache_result = None
        # Old recovery is a deployment gate; this RPC reads the new schema only.
        if not self._app_update_gate:
            await self.bridge.recover()
        self._scheduler = asyncio.create_task(self._scheduler_loop())
        # First-seen local games must enter matching without a user action
        # (design §3.3). The check itself is off the startup critical path.
        self._startup_check = asyncio.create_task(self._startup_match_check())

    async def get_capabilities(self):
        return await self.bridge.call_sync("get_capabilities", {})

    async def _read_report(self):
        # A scan is one read-only whole-library projection; source work never
        # triggers extra scans or a second projection.
        report = await self.bridge.scan()
        self.sources.prime(report)
        self._scan_generation = getattr(self, "_scan_generation", 0) + 1
        report["generation"] = self._scan_generation
        report["settings_revision"] = self.sources.settings_revision
        return report

    def _pending_match_instances(self, report):
        instances = []
        for row in report.get("installs", []):
            game = row.get("game") or {}
            if game.get("availability") != "installed":
                continue
            discovery = row.get("discovery")
            if discovery is not None and discovery.get("status") != "pending":
                continue
            instance = game.get("instance_id")
            if isinstance(instance, str):
                instances.append(instance)
        return instances

    def _start_pending_match(self, report):
        """Queue one incremental match for first-seen local games (design §3.3).

        The task runs in the background and is deduplicated by the source
        service key, so repeated scans while it is active do not stack. Sources
        without an index still go through matching and report `incomplete`.
        """
        instances = self._pending_match_instances(report)
        if not instances:
            return None
        if not any(source.get("enabled") is True and source.get("role") == "publisher"
                   for source in self.sources.list_sources()):
            return None
        return self.sources.start_match_games(instances, force_details=False)

    async def _startup_match_check(self):
        if getattr(self, '_app_update_gate', False):
            return
        try:
            report = await self._read_report()
        except asyncio.CancelledError:
            raise
        except Exception as error:
            logging.getLogger(__name__).warning("startup_match_check_failed: %s",
                                                type(error).__name__)
            return
        self._start_pending_match(report)

    async def scan(self):
        report = await self._read_report()
        self._start_pending_match(report)
        return report

    async def project_instances(self, instance_ids):
        if not isinstance(instance_ids, list):
            raise ValueError("invalid_request")
        for instance_id in instance_ids:
            validate_id(instance_id, MAX_INSTANCE_ID_BYTES)
        result = await self.bridge.call_sync("project_instances",
                                             {"instance_ids": list(instance_ids)})
        revision = result.get("settings_revision")
        if type(revision) is int:
            self.sources.settings_revision = revision
        return result

    async def preview(self, instance_id, release_id=None, selection_id=None):
        return await self.bridge.preview(instance_id, release_id, selection_id)

    async def list_sources(self):
        return await self.sources.reload_sources()

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

    async def set_source_update_interval(self, source_id, interval_hours):
        self._require_idle()
        return await self.sources.set_update_interval(source_id, interval_hours)

    # -- source tasks (parallel to installation, keyed by source/match) ----
    async def update_source(self, source_id, force=False):
        return self.sources.start_update_source(source_id, force)

    async def match_games(self, instances=None, name_hints=None, preferred_language=None,
                          force_details=False):
        return self.sources.start_match_games(instances, name_hints, preferred_language,
                                              force_details)

    async def rematch_game(self, instance_id=None, appid=None, name_hints=None,
                           preferred_language=None):
        return self.sources.start_rematch_game(instance_id, appid, name_hints, preferred_language)

    async def source_task_status(self, task_id=None):
        return self.sources.task_status(task_id)

    async def cancel_source_task(self, task_id):
        return self.sources.cancel_task(task_id)

    # -- automatic update schedule ----------------------------------------
    async def _scheduler_loop(self):
        while True:
            try:
                await self._check_due_sources()
            except asyncio.CancelledError:
                raise
            except Exception as error:
                # A transient core failure must not stop future due checks.
                logging.getLogger(__name__).warning("source_schedule_check_failed: %s",
                                                    type(error).__name__)
            await asyncio.sleep(SCHEDULER_INTERVAL_SECONDS)

    async def _check_due_sources(self, now=None):
        if getattr(self, '_app_update_gate', False):
            return []
        summaries = await self.sources.reload_sources()
        if getattr(self, '_app_update_gate', False):
            return []
        moment = int(time.time()) if now is None else now
        started = []
        for source_id in due_source_ids(summaries, moment):
            if self.sources.has_active_source_task(source_id):
                continue
            self.sources.start_update_source(source_id)
            started.append(source_id)
        return started

    async def _app_update_loop(self):
        while True:
            try:
                await self.get_app_update_status()
                await self.app_update.check()
            except Exception as error:
                logging.getLogger(__name__).warning('app_update_check_failed: %s', error)
            await asyncio.sleep(3600)

    async def check_app_update(self):
        return await self.app_update.check(manual=True)

    async def claim_app_update_notice(self):
        return self.app_update.claim_notice()

    async def get_app_update_status(self):
        status = self.app_update.snapshot()
        execution = status.get('execution') or {}
        if execution.get('state') in ('completed', 'failed') and not (Path(decky.DECKY_USER_HOME) / '.cache/patchhub-decky-install/journal.json').exists():
            self._app_update_gate = False
        return status

    async def skip_app_update(self, tag):
        return self.app_update.skip(tag)

    async def cancel_app_update(self):
        return self.app_update.cancel()

    async def recover_app_update(self):
        return self.app_update.start(recover=True)

    def _prepare_app_update(self):
        patch_busy = hasattr(self, 'tasks') and self.tasks.busy()
        source_busy = hasattr(self, 'sources') and any(t['state'] not in ('completed', 'failed', 'cancelled') for t in self.sources.task_status())
        if getattr(self, '_active_host_calls', 0) > 1 or patch_busy or source_busy:
            raise RuntimeError('请先完成或取消补丁、来源与扫描任务，再更新插件')
        self._app_update_gate = True
        return {'ready': True}

    async def prepare_app_update(self):
        return self._prepare_app_update()

    async def finish_app_update(self):
        if (Path(decky.DECKY_USER_HOME) / '.cache/patchhub-decky-install/journal.json').exists():
            raise RuntimeError('更新事务尚未完成')
        await self.bridge.recover()
        self._app_update_gate = False
        return {'ready': True}

    async def start_app_update(self, tag):
        self._prepare_app_update()
        try:
            return self.app_update.start(tag)
        except BaseException:
            self._app_update_gate = False
            raise

    async def get_settings(self):
        return {**self.settings.snapshot(), "cache": getattr(self, "_cache_result", None)}

    async def set_auto_cleanup(self, enabled):
        self.settings.set_auto_cleanup(enabled)
        return await self.get_settings()

    async def cache_status(self):
        return await self.bridge.cache_status()

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
                                       archive=None, operation=None, request_id=None, manual_download=None):
        self.bridge.selection(instance_id, release_id, selection_id, manual_download)
        manual_download = dict(manual_download) if manual_download is not None else None
        if manual_download is not None and archive is not None:
            raise ValueError("invalid_request")
        if operation not in ("install", "reinstall", "overwrite"):
            raise ValueError("invalid_operation")
        if selection_id is not None and archive is not None:
            raise ValueError("archive_not_allowed_for_selection")
        local = self._local_archive(archive) if archive is not None else None
        request_id = str(uuid.uuid4()) if request_id is None else validate_id(request_id, 128, "invalid_request_id")
        cleanup_policy = "after_commit" if self.settings.auto_cleanup else "disabled"

        async def work(task):
            task.check()
            args = (instance_id, release_id, selection_id, local, operation, request_id, cleanup_policy)
            result = await self.bridge.install_patch(*args, manual_download=manual_download) if manual_download is not None else await self.bridge.install_patch(*args)
            if isinstance(result.get("cleanup"), dict):
                self._cache_result = result["cleanup"]
            return result

        return self.tasks.start("install", work, instance_id=instance_id, release_id=release_id,
            selection_id=selection_id, operation=operation, request_id=request_id,
            archive_path=str(local) if local is not None else None, manual_download=manual_download)

    async def start_install(self, instance_id, release_id=None, selection_id=None,
                            archive=None, operation=None, request_id=None, manual_download=None):
        return await self.start_patch_install_task(instance_id, release_id, selection_id,
            archive, operation, request_id, manual_download)

    async def task_status(self, task_id):
        return self.tasks.status(task_id)

    async def cancel_task(self, task_id):
        task = self.tasks.get(task_id)
        task.request_cancel()
        return task.snapshot()

    async def list_tasks(self):
        return [task.snapshot() for task in self.tasks.tasks.values()]

    async def _unload(self):
        for name in ("_startup_check", "_scheduler", "_app_update_checker"):
            background = getattr(self, name, None)
            if background is not None:
                background.cancel()
                await asyncio.gather(background, return_exceptions=True)
        if hasattr(self, "sources"):
            await self.sources.close()
        if hasattr(self, "tasks"):
            await self.tasks.shutdown()


def _await_startup(method):
    from functools import wraps
    @wraps(method)
    async def ready(self, *args, **kwargs):
        update_methods = {'get_app_update_status', 'check_app_update', 'skip_app_update',
                          'claim_app_update_notice', 'start_app_update', 'cancel_app_update',
                          'recover_app_update', 'prepare_app_update'}
        event = getattr(self, "_startup_ready", None)
        if event is not None:
            await event.wait()
            if self._startup_error is not None and method.__name__ not in update_methods:
                raise RuntimeError("plugin_startup_failed") from self._startup_error
        allowed = {'get_capabilities', 'list_tasks', 'source_task_status', 'get_app_update_status',
                   'prepare_app_update', 'finish_app_update', 'check_app_update', 'skip_app_update', 'cancel_app_update', 'recover_app_update', 'claim_app_update_notice'}
        if getattr(self, '_app_update_gate', False) and method.__name__ not in allowed:
            raise RuntimeError('plugin_update_in_progress')
        self._active_host_calls = getattr(self, '_active_host_calls', 0) + 1
        try:
            return await method(self, *args, **kwargs)
        finally:
            self._active_host_calls -= 1
    return ready


# Loader schedules _main asynchronously; frontend RPCs must not race migration.
for _name, _method in list(vars(Plugin).items()):
    if not _name.startswith("_") and asyncio.iscoroutinefunction(_method):
        setattr(Plugin, _name, _await_startup(_method))
