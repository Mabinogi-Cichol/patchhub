"""Fixed-method Decky to Rust JSON-lines bridge. UI cannot select a binary or catalog."""
import asyncio
import inspect
import os
import json
import logging
from pathlib import Path
import uuid
from threading import Event
from weakref import WeakValueDictionary
from tasks import current_task

from protocol_validation import (PROTOCOL_VERSION, METHODS, SOURCE_METHODS, CONTROLLED_METHODS,
    SOURCE_EVENTS, INSTALL_STAGES, MAX_RESPONSE_BYTES, MAX_INSTANCE_ID_BYTES, MAX_REQUEST_BYTES,
    validate_id, validate_request, validate_response, validate_event, validate_capabilities, JS_MAX_INTEGER)
TERMINAL_DRAIN_SECONDS = 1.0
TERMINAL_DRAIN_BYTES = MAX_RESPONSE_BYTES
TERMINAL_DRAIN_CHUNK_BYTES = 64 * 1024
PROGRESS_FIELDS = {"stage", "substage", "bytes_done", "total_bytes", "files_done",
                   "total_files", "current_file", "archive_bytes"}


def _unique_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("duplicate_field")
        value[key] = item
    return value


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
        self._terminal_observers = set()
        self._preflight_done = False
        self._preflight_lock = asyncio.Lock()
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
        return validate_id(value)

    async def preflight(self):
        async with self._preflight_lock:
            if not self._preflight_done:
                caps = await self._call("get_capabilities", {})
                validate_capabilities(caps)
                self._preflight_done = True
        return True

    async def scan(self):
        return await self._call("scan", {})

    @classmethod
    def selection(cls, instance_id, release_id=None, selection_id=None):
        if release_id is not None and selection_id is not None:
            raise ValueError("invalid_request")
        if release_id is None and selection_id is None:
            raise ValueError("selection_or_release_required")
        params = {"instance_id": validate_id(instance_id, MAX_INSTANCE_ID_BYTES)}
        key, value = ("selection_id", selection_id) if selection_id is not None else ("release_id", release_id)
        params[key] = cls._id(value)
        return params

    async def preview(self, instance_id, release_id=None, selection_id=None):
        return await self._call("preview", self.selection(instance_id, release_id, selection_id))

    async def install_patch(self, instance_id, release_id=None, selection_id=None,
                            archive=None, operation=None, request_id=None,
                            cleanup_policy="disabled"):
        params = self.selection(instance_id, release_id, selection_id)
        if operation not in ("install", "reinstall", "overwrite"):
            raise ValueError("invalid_operation")
        if cleanup_policy not in ("disabled", "after_commit"):
            raise ValueError("invalid_cleanup_policy")
        if archive is not None:
            if selection_id is not None:
                raise ValueError("archive_not_allowed_for_selection")
            params["archive"] = str(archive)
        params.update(operation=operation, cleanup_policy=cleanup_policy)
        return await self._call("install_patch", params, request_id=request_id)

    async def discard_plan(self, plan_id):
        return await self._call("discard_plan", {"plan_id": self._id(plan_id)})

    async def list_transactions(self):
        return await self._call("list_transactions", {})

    async def recover(self):
        return await self._call("recover", {})

    async def backup_status(self):
        return await self._call("backup_status", {})

    async def cleanup_backups(self):
        return await self._call("cleanup_backups", {})

    async def cleanup_cache(self):
        return await self._call("cleanup_cache", {})

    @staticmethod
    def _source_event(row, request_id, last_sequence):
        """Validate one interleaved source event; returns its (new) sequence."""
        if row.get("protocol_version") != PROTOCOL_VERSION:
            raise RuntimeError("unsupported_protocol")
        if row.get("request_id") != request_id or type(row.get("sequence")) is not int:
            raise RuntimeError("contract_mismatch")
        sequence = row["sequence"]
        if (sequence < 0 or sequence > JS_MAX_INTEGER
                or (last_sequence == -1 and sequence != 0)
                or sequence > last_sequence + 1):
            raise RuntimeError("contract_mismatch")
        return sequence

    async def _call(self, method: str, params: dict, on_event=None, cancel=None,
                    timeout=None, controlled: bool | None = None, request_id=None):
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
                return await self._exchange_call(method, params, on_event, cancel, timeout, controlled, request_id)
        return await self._exchange_call(method, params, on_event, cancel, timeout, controlled, request_id)

    async def _exchange_call(self, method, params, on_event, cancel, timeout, controlled, request_id):
        if method not in METHODS:
            raise ValueError("unsupported_method")
        request_id = str(uuid.uuid4()) if request_id is None else validate_id(request_id, 128, "invalid_request_id")
        if set(params) & {"host", "roots", "catalog", "signed_catalog", "trust_keys", "state_dir", "platform", "request_id"}:
            raise ValueError("host_parameter_forbidden")
        host = {"roots": [], "catalog": str(self.catalog), "platform": "steamos",
                "signed_catalog": str(self.signed_catalog) if self.signed_catalog else None,
                "trust_keys": str(self.trust_keys) if self.trust_keys else None,
                "state_dir": str(self.state_dir) if self.state_dir else None}
        request = {"protocol_version": PROTOCOL_VERSION, "request_id": request_id, "method": method,
                   "params": {"host": host, **params}}
        validate_request(request, method in CONTROLLED_METHODS if controlled is None else controlled)
        payload = json.dumps(request, ensure_ascii=False).encode("utf-8") + b"\n"
        if len(payload) > MAX_REQUEST_BYTES:
            raise ValueError("request_too_large")
        if method not in {"get_capabilities", "scan", "preview", "list_transactions", "list_sources", "list_game_sources", "discover_game"}:
            await self.preflight()
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
        terminal_received = asyncio.Event()

        async def signal_cancel():
            # The control message must follow the request. EOF and an explicit
            # `cancel` line both request cancellation; no periodic renewal.
            await request_sent.wait()
            while process.returncode is None and not terminal_received.is_set():
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
            while True:
                try:
                    line = await process.stdout.readline()
                except (ValueError, asyncio.LimitOverrunError) as exc:
                    # StreamReader rejects a long line before our byte check.
                    raise RuntimeError("response_too_large") from exc
                if not line:
                    break
                if len(line) > MAX_RESPONSE_BYTES:
                    raise RuntimeError("response_too_large")
                if not line.endswith(b"\n"):
                    raise RuntimeError("contract_mismatch")
                try:
                    row = json.loads(line, object_pairs_hook=_unique_object, parse_constant=lambda value: (_ for _ in ()).throw(ValueError(value)))
                except (UnicodeDecodeError, ValueError) as exc:
                    raise RuntimeError("contract_mismatch") from exc
                if not isinstance(row, dict):
                    raise RuntimeError("contract_mismatch")
                if "event" in row:
                    if not controlled or row.get("event") not in SOURCE_EVENTS | {"install_progress"}:
                        raise RuntimeError("contract_mismatch")
                    validate_event(row, request_id)
                    sequence = self._source_event(row, request_id, last_sequence)
                    if sequence <= last_sequence:
                        continue
                    last_sequence = sequence
                    if row["event"] == "install_progress":
                        if (method != "install_patch" or row.get("instance_id") != params.get("instance_id")
                                or row.get("selection_id") != params.get("selection_id")
                                or row.get("stage") not in INSTALL_STAGES):
                            raise RuntimeError("contract_mismatch")
                        if task:
                            task.update(**{key: value for key, value in row.items() if key in PROGRESS_FIELDS})
                    elif method not in {"scan_sources", "refresh_sources"}:
                        raise RuntimeError("contract_mismatch")
                    if on_event is not None:
                        emitted = on_event(row)
                        if inspect.isawaitable(emitted):
                            await emitted
                elif response is None:
                    validate_response(row, method, request_id)
                    response = row
                    terminal_received.set()
                    # EOF cancels any remaining monitoring and seals the first
                    # verified receipt. Trailing bytes are host diagnostics,
                    # regardless of shape.
                    try:
                        process.stdin.close()
                    except Exception as exc:
                        logging.getLogger(__name__).warning("core_stdin_close_after_terminal_failed: %s", type(exc).__name__)
                    break
            if response is not None:
                state = {"bytes": 0}
                logger = logging.getLogger(__name__)

                async def observe_terminal(byte_limit=None):
                    while True:
                        size = TERMINAL_DRAIN_CHUNK_BYTES
                        if byte_limit is not None:
                            remaining = byte_limit - state["bytes"]
                            if remaining <= 0:
                                return False
                            size = min(size, remaining)
                        chunk = await process.stdout.read(size)
                        if not chunk:
                            await process.wait()
                            if process.returncode != 0:
                                logger.warning("core_exit_after_terminal: %s", process.returncode)
                            return True
                        state["bytes"] += len(chunk)
                        # Buffered or synthetic peers must not starve the timer.
                        await asyncio.sleep(0)

                reason = None
                try:
                    observed = await asyncio.wait_for(observe_terminal(TERMINAL_DRAIN_BYTES), TERMINAL_DRAIN_SECONDS)
                    if not observed:
                        reason = "byte_limit"
                except asyncio.TimeoutError:
                    observed = False
                    reason = "timeout"
                except Exception as exc:
                    observed = False
                    reason = "read_failed"
                    logger.warning("core_drain_after_terminal_failed: %s", type(exc).__name__)
                if state["bytes"]:
                    logger.warning("core_output_after_terminal: %s bytes", state["bytes"])
                if not observed:
                    logger.warning("core_observation_after_terminal_deferred: %s", reason)

                    async def background_observation():
                        previous_bytes = state["bytes"]
                        try:
                            await observe_terminal()
                        except Exception as exc:
                            logger.warning("core_observation_after_terminal_failed: %s", type(exc).__name__)
                            try:
                                await process.wait()
                            except Exception as wait_error:
                                logger.warning("core_exit_observation_after_terminal_failed: %s", type(wait_error).__name__)
                        finally:
                            if state["bytes"] != previous_bytes:
                                logger.warning("core_output_after_terminal: %s bytes", state["bytes"])
                    observer = asyncio.create_task(background_observation())
                    self._terminal_observers.add(observer)
                    observer.add_done_callback(self._terminal_observers.discard)
            else:
                await process.wait()
                if process.returncode != 0:
                    raise RuntimeError("core_process_failed")
            if response is None:
                raise RuntimeError("contract_mismatch")
            if not response["ok"]:
                error = response.get("error") or {}
                raise CoreError(error["code"], error.get("detail"))
            return response["result"]

        guard = asyncio.create_task(signal_cancel()) if controlled else None
        running = asyncio.create_task(exchange())
        effective_timeout = timeout if timeout is not None else (self.timeout if not controlled else None)
        try:
            return await asyncio.wait_for(asyncio.shield(running), effective_timeout)
        except asyncio.TimeoutError:
            if terminal_received.is_set():
                return await running
            raise
        except asyncio.CancelledError:
            # Task/UI cancellation must never kill a writer. Drain it until rollback
            # or the durable commit reports its actual outcome.
            cancel.set()
            if controlled:
                return await running
            raise
        finally:
            self.active_calls.discard(cancel)
            if process.returncode is None and not terminal_received.is_set():
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
            if process.stdin and not terminal_received.is_set():
                process.stdin.close()

    async def call_sync(self, method: str, params: dict, timeout: float | None = None):
        """One-shot `serve` call outside the task controller; returns the result.

        Source config reads/writes are local and short, so they do not need the
        cancellable `serve-task` stream.
        """
        return await self._call(method, params, timeout=timeout, controlled=False)
