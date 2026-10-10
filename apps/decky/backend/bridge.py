"""Fixed-method Decky to Rust JSON-lines bridge. UI cannot select a binary or catalog."""
import asyncio
import inspect
import os
import json
from pathlib import Path
import uuid
from threading import Event
from weakref import WeakValueDictionary
from tasks import current_task

MAX_RESPONSE_BYTES = 8 * 1024 * 1024
MAX_REQUEST_BYTES = 64 * 1024
PROTOCOL_VERSION = 3
RENEW_SECONDS = 3
SOURCE_METHODS = {
    "list_sources", "list_game_sources", "set_source_enabled", "set_game_source",
    "add_source", "refresh_sources", "discover_game", "scan_sources",
}
METHODS = {"get_capabilities", "discard_plan", "scan", "preview", "install_patch",
           "list_transactions", "recover", "cleanup_cache"} | SOURCE_METHODS
CONTROLLED_METHODS = {"install_patch", "refresh_sources", "scan_sources"}
SOURCE_EVENTS = {"source_progress", "source_update"}
INSTALL_STAGES = {"connecting", "downloading", "validating", "analyzing_installer",
                  "extracting", "planning", "applying", "rolling_back", "cleanup"}
PROGRESS_FIELDS = {"stage", "substage", "bytes_done", "total_bytes", "files_done",
                   "total_files", "current_file", "archive_bytes"}


class CoreError(RuntimeError):
    def __init__(self, code, detail=None):
        super().__init__(code)
        self.code = code
        self.detail = detail


# Calls sharing the same state directory share the
# reader queue only while calls are alive, and never across event loops.
_journal_read_locks = WeakValueDictionary()


class CoreBridge:
    def __init__(self, binary: Path, catalog: Path, timeout: float = 300,
                 signed_catalog: Path | None = None, trust_keys: Path | None = None,
                 state_dir: Path | None = None, run_uid: int | None = None, run_gid: int | None = None, user_home: Path | None = None):
        self.binary = binary.resolve(strict=True)
        self.catalog = catalog.resolve(strict=True)
        self.signed_catalog = signed_catalog.resolve(strict=True) if signed_catalog and signed_catalog.is_file() else None
        self.trust_keys = trust_keys.resolve(strict=True) if trust_keys and trust_keys.is_file() else None
        self.state_dir = state_dir.resolve() if state_dir else None
        self.timeout = timeout
        self.process_identity = {"user": run_uid, "group": run_gid, "extra_groups": []} if run_uid is not None else {}
        self.active_calls = set()
        self.process_env = os.environ.copy()
        if (run_uid == 0 or (run_uid is not None and (type(run_uid) is not int or run_uid < 1
                or type(run_gid) is not int or run_gid < 1))
                or (hasattr(os, "geteuid") and os.geteuid() == 0 and run_uid is None)):
            raise ValueError("unprivileged_user_required")
        # Rust locates sf-worker beside this fixed executable; no RPC runtime path.
        self.worker = self.binary.with_name("sf-worker")
        if user_home is not None:
            home = str(user_home)
            self.process_env.update(HOME=home, USER=user_home.name, LOGNAME=user_home.name,
                                    XDG_CONFIG_HOME=str(user_home / ".config"),
                                    XDG_DATA_HOME=str(user_home / ".local/share"),
                                    XDG_CACHE_HOME=str(user_home / ".cache"))
            if run_uid is not None:
                runtime = Path("/run/user") / str(run_uid)
                if runtime.is_dir() and runtime.stat().st_uid == run_uid:
                    self.process_env["XDG_RUNTIME_DIR"] = str(runtime)
                else:
                    self.process_env.pop("XDG_RUNTIME_DIR", None)

    @staticmethod
    def _id(value: str) -> str:
        if not isinstance(value, str) or not value or len(value) > 256 or any(ord(c) < 32 for c in value):
            raise ValueError("invalid_id")
        return value

    async def scan(self):
        return await self._call("scan", {})

    @classmethod
    def selection(cls, instance_id, release_id=None, selection_id=None):
        if release_id is not None and selection_id is not None:
            raise ValueError("invalid_request")
        if release_id is None and selection_id is None:
            raise ValueError("selection_or_release_required")
        params = {"instance_id": cls._id(instance_id)}
        key, value = ("selection_id", selection_id) if selection_id is not None else ("release_id", release_id)
        params[key] = cls._id(value)
        return params

    async def preview(self, instance_id, release_id=None, selection_id=None):
        return await self._call("preview", self.selection(instance_id, release_id, selection_id))

    async def install_patch(self, instance_id, release_id=None, selection_id=None,
                            archive=None, operation="install", request_id=None):
        params = self.selection(instance_id, release_id, selection_id)
        if operation not in ("install", "reinstall", "overwrite"):
            raise ValueError("invalid_operation")
        if archive is not None:
            if selection_id is not None:
                raise ValueError("archive_not_allowed_for_selection")
            params["archive"] = str(archive)
        params.update(operation=operation, request_id=request_id or str(uuid.uuid4()))
        return await self._call("install_patch", params)

    async def discard_plan(self, plan_id):
        return await self._call("discard_plan", {"plan_id": self._id(plan_id)})

    async def list_transactions(self):
        return await self._call("list_transactions", {})

    async def recover(self):
        return await self._call("recover", {})

    async def cleanup_cache(self):
        return await self._call("cleanup_cache", {})

    @staticmethod
    def _source_event(row, request_id, last_sequence):
        """Validate one interleaved source event; returns its (new) sequence."""
        if (not isinstance(row, dict) or row.get("request_id") != request_id
                or row.get("protocol_version") != PROTOCOL_VERSION
                or type(row.get("sequence")) is not int or row["sequence"] <= last_sequence
                or (last_sequence == -1 and row["sequence"] != 0)):
            raise RuntimeError("invalid_core_response")
        return row["sequence"]

    async def _call(self, method: str, params: dict, on_event=None, cancel=None,
                    timeout=None, controlled: bool | None = None):
        # Both reads briefly take the core journal's exclusive lock. Source
        # event projection and UI requests must not race each other for it.
        # Writers still use the core lock and report genuine installer_busy.
        if method in {"scan", "list_transactions"} and self.state_dir is not None:
            key = (asyncio.get_running_loop(), self.state_dir)
            lock = _journal_read_locks.get(key)
            if lock is None:
                lock = asyncio.Lock()
                _journal_read_locks[key] = lock
            async with lock:
                return await self._exchange_call(method, params, on_event, cancel, timeout, controlled)
        return await self._exchange_call(method, params, on_event, cancel, timeout, controlled)

    async def _exchange_call(self, method, params, on_event, cancel, timeout, controlled):
        if method not in METHODS:
            raise ValueError("unsupported_method")
        request_id = params.get("request_id") or str(uuid.uuid4())
        if not isinstance(request_id, str) or not 1 <= len(request_id.encode("utf-8")) <= 128 or any(ord(c) < 32 or ord(c) == 127 for c in request_id):
            raise ValueError("invalid_request_id")
        if set(params) & {"roots", "catalog", "signed_catalog", "trust_keys", "state_dir", "platform"}:
            raise ValueError("host_parameter_forbidden")
        host_params = {"roots": [], "catalog": str(self.catalog), "platform": "steamos"}
        if self.signed_catalog:
            host_params["signed_catalog"] = str(self.signed_catalog)
        if self.trust_keys:
            host_params["trust_keys"] = str(self.trust_keys)
        if self.state_dir:
            host_params["state_dir"] = str(self.state_dir)
        request = {"protocol_version": PROTOCOL_VERSION, "request_id": request_id, "method": method,
                   "params": {**host_params, **params}}
        payload = json.dumps(request, ensure_ascii=False).encode("utf-8") + b"\n"
        if len(payload) > MAX_REQUEST_BYTES:
            raise ValueError("request_too_large")
        task = current_task.get()
        if controlled is None:
            controlled = method in CONTROLLED_METHODS
        if cancel is None:
            cancel = task.cancel if task else Event()
        process = await asyncio.create_subprocess_exec(
            str(self.binary), "serve-task" if controlled else "serve", stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.DEVNULL,
            limit=MAX_RESPONSE_BYTES + 1, env=self.process_env, **self.process_identity,
        )
        if controlled:
            self.active_calls.add(cancel)
        request_sent = asyncio.Event()

        async def signal_cancel():
            # The control message must follow the request, including when a
            # lease expires while the subprocess is starting.
            await request_sent.wait()
            next_renew = asyncio.get_running_loop().time() + RENEW_SECONDS
            while process.returncode is None:
                if cancel.is_set():
                    try:
                        process.stdin.write(b"cancel\n")
                        await process.stdin.drain()
                    except (BrokenPipeError, ConnectionResetError):
                        pass
                    return
                if asyncio.get_running_loop().time() >= next_renew:
                    try:
                        process.stdin.write(b"renew\n")
                        await process.stdin.drain()
                    except (BrokenPipeError, ConnectionResetError):
                        return
                    next_renew = asyncio.get_running_loop().time() + RENEW_SECONDS
                await asyncio.sleep(.1)

        async def exchange():
            process.stdin.write(payload)
            await process.stdin.drain()
            request_sent.set()
            if not controlled:
                process.stdin.close()
            response = None
            last_sequence = -1
            while line := await process.stdout.readline():
                if len(line) > MAX_RESPONSE_BYTES:
                    raise RuntimeError("response_too_large")
                row = json.loads(line)
                if not isinstance(row, dict):
                    raise RuntimeError("invalid_core_response")
                if response is not None:
                    if row.get("event") in SOURCE_EVENTS | {"install_progress"}:
                        # The terminal receipt wins; stale telemetry cannot turn
                        # a real commit into a transport failure.
                        continue
                    raise RuntimeError("invalid_core_response")
                if "event" in row:
                    if not controlled or row.get("event") not in SOURCE_EVENTS | {"install_progress"}:
                        raise RuntimeError("invalid_core_response")
                    last_sequence = self._source_event(row, request_id, last_sequence)
                    if row["event"] == "install_progress":
                        if (method != "install_patch" or row.get("instance_id") != params.get("instance_id")
                                or row.get("selection_id") != params.get("selection_id")
                                or row.get("stage") not in INSTALL_STAGES):
                            raise RuntimeError("invalid_core_response")
                        if task:
                            task.update(**{key: value for key, value in row.items() if key in PROGRESS_FIELDS})
                    elif method not in {"scan_sources", "refresh_sources"}:
                        raise RuntimeError("invalid_core_response")
                    if on_event is not None:
                        emitted = on_event(row)
                        if inspect.isawaitable(emitted):
                            await emitted
                elif response is None:
                    response = row
                else:
                    raise RuntimeError("invalid_core_response")
            await process.wait()
            if process.returncode != 0:
                raise RuntimeError("core_process_failed")
            if (not isinstance(response, dict) or response.get("protocol_version") != PROTOCOL_VERSION
                    or response.get("request_id") != request_id or not isinstance(response.get("ok"), bool)):
                raise RuntimeError("invalid_core_response")
            if not response["ok"]:
                error = response.get("error") or {}
                raise CoreError(error.get("code", "core_error"), error.get("detail"))
            return response["result"]

        guard = asyncio.create_task(signal_cancel()) if controlled else None
        running = asyncio.create_task(exchange())
        effective_timeout = timeout if timeout is not None else (self.timeout if not controlled else None)
        try:
            return await asyncio.wait_for(asyncio.shield(running), effective_timeout)
        except asyncio.CancelledError:
            # Task/UI cancellation must never kill a writer. Drain it until rollback
            # or the durable commit reports its actual outcome.
            cancel.set()
            if controlled:
                return await running
            raise
        finally:
            self.active_calls.discard(cancel)
            if process.returncode is None:
                if controlled:
                    cancel.set()
                    if not running.done():
                        await asyncio.gather(running, return_exceptions=True)
                    else:
                        # Malformed telemetry is a transport error, not permission
                        # to kill an in-flight transaction. EOF also requests cancel.
                        process.stdin.close()
                        while await process.stdout.read(65536):
                            pass
                    await process.wait()
                else:
                    process.kill()
                    await process.wait()
                    running.cancel()
            if guard:
                guard.cancel()
                await asyncio.gather(guard, return_exceptions=True)
            if process.stdin:
                process.stdin.close()

    async def call_sync(self, method: str, params: dict, timeout: float | None = None):
        """One-shot `serve` call outside the task controller; returns the result.

        Source config reads/writes are local and short, so they do not need the
        cancellable `serve-task` stream.
        """
        return await self._call(method, params, timeout=timeout, controlled=False)
