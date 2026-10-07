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
SOURCE_METHODS = {
    "get_capabilities", "list_sources", "list_game_sources", "set_source_enabled",
    "set_game_source", "add_source", "refresh_sources", "discover_game", "scan_sources",
    "get_source_policy", "resolve_source_record", "list_source_matches",
    "get_source_state", "get_source_settings",
}
METHODS = {"discard_plan", "scan", "preview", "plan_install", "fetch_artifact", "start_install", "list_transactions", "recover", "migrate_state"} | SOURCE_METHODS
# Methods served by `serve-task` (streaming/cancellable), not one-shot `serve`.
CONTROLLED_METHODS = {"fetch_artifact", "plan_install", "start_install",
                      "discover_game", "refresh_sources", "scan_sources"}
# Events a source task may interleave before its single final response.
SOURCE_EVENTS = {"source_progress", "source_update"}
# Publisher bridges are recreated for the same state directory. Share the
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
        self.downloads = set()
        self.process_env = os.environ.copy()
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

    async def preview(self, instance_id: str, release_id: str):
        return await self._call("preview", {"instance_id": self._id(instance_id), "release_id": self._id(release_id)})

    async def plan_install(self, instance_id: str, release_id: str, archive: Path | None = None, operation="install"):
        params = {"instance_id": self._id(instance_id), "release_id": self._id(release_id), "operation": operation}
        if archive is not None:
            params["archive"] = str(archive.resolve(strict=True))
        return await self._call("plan_install", params)

    async def fetch_artifact(self, instance_id: str, release_id: str):
        return await self._call("fetch_artifact", {"instance_id": self._id(instance_id), "release_id": self._id(release_id)})

    async def cancel_download(self):
        for cancel in tuple(self.downloads):
            cancel.set()
        return {"canceled": bool(self.downloads)}

    async def discard_plan(self, plan_id):
        return await self._call("discard_plan", {"plan_id": self._id(plan_id)})

    async def start_install(self, plan_id: str):
        return await self._call("start_install", {"plan_id": self._id(plan_id)})

    async def list_transactions(self):
        return await self._call("list_transactions", {})

    async def recover(self):
        return await self._call("recover", {})

    async def migrate_state(self):
        return await self._call("migrate_state", {})

    async def cleanup_transactions(self):
        return await self._call("migrate_state", {})

    @staticmethod
    def _source_event(row, request_id, last_sequence):
        """Validate one interleaved source event; returns its (new) sequence."""
        if (not isinstance(row, dict) or row.get("request_id") != request_id
                or type(row.get("sequence")) is not int or row["sequence"] <= last_sequence):
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
        request_id = str(uuid.uuid4())
        host_params = {"catalog": str(self.catalog), "platform": "steamos"}
        if self.signed_catalog:
            host_params["signed_catalog"] = str(self.signed_catalog)
        if self.trust_keys:
            host_params["trust_keys"] = str(self.trust_keys)
        if self.state_dir:
            host_params["state_dir"] = str(self.state_dir)
        request = {"protocol_version": 2, "request_id": request_id, "method": method,
                   "params": {**host_params, **params}}
        payload = json.dumps(request, ensure_ascii=False).encode("utf-8") + b"\n"
        if len(payload) > MAX_REQUEST_BYTES:
            raise ValueError("request_too_large")
        task = current_task.get()
        if controlled is None:
            controlled = task is not None or method in CONTROLLED_METHODS
        if cancel is None:
            cancel = task.cancel if task else Event()
        process = await asyncio.create_subprocess_exec(
            str(self.binary), "serve-task" if controlled else "serve", stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.DEVNULL,
            limit=MAX_RESPONSE_BYTES + 1, env=self.process_env, **self.process_identity,
        )
        if method == "fetch_artifact":
            self.downloads.add(cancel)
        request_sent = asyncio.Event()

        async def signal_cancel():
            # The control message must follow the request, including when a
            # lease expires while the subprocess is starting.
            await request_sent.wait()
            while process.returncode is None:
                if cancel.is_set():
                    try:
                        process.stdin.write(b"cancel\n")
                        await process.stdin.drain()
                    except (BrokenPipeError, ConnectionResetError):
                        pass
                    return
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
                if controlled and row.get("event") == "progress":
                    if task:
                        progress = row["progress"]
                        # Core subcommands may not know the already acquired package size.
                        if progress.get("archive_bytes") is None:
                            progress.pop("archive_bytes", None)
                        task.update(**progress)
                elif controlled and row.get("event") in SOURCE_EVENTS:
                    # Interleaved telemetry is never the final response; enforce the
                    # request binding and monotonic sequence before forwarding it.
                    last_sequence = self._source_event(row, request_id, last_sequence)
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
            if (not isinstance(response, dict) or response.get("protocol_version") != 2
                    or response.get("request_id") != request_id or not isinstance(response.get("ok"), bool)):
                raise RuntimeError("invalid_core_response")
            if not response["ok"]:
                raise RuntimeError(response.get("error", {}).get("code", "core_error"))
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
            self.downloads.discard(cancel)
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
            if process.stdin:
                process.stdin.close()

    async def call_sync(self, method: str, params: dict, timeout: float | None = None):
        """One-shot `serve` call outside the task controller; returns the result.

        Source config reads/writes are local and short, so they do not need the
        cancellable `serve-task` stream.
        """
        return await self._call(method, params, timeout=timeout, controlled=False)
