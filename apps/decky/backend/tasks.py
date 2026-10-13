"""Foreground tasks with cancellation and safe rollback after UI lease expiry."""
import asyncio
from collections import deque
from contextvars import ContextVar
import json
import re
from pathlib import Path
from threading import Event, RLock
import time
import uuid
from plugin_settings import _write_json, _no_links
from protocol_validation import truncate_detail

current_task = ContextVar("patch_task", default=None)
TERMINAL = {"completed", "cancelled", "failed"}
LEASE_SECONDS = 15


class Task:
    def __init__(self, kind, metadata):
        self.cancel = Event()
        self.lock = RLock()
        self.started = self.lease = time.monotonic()
        self.samples = deque()
        self.value = dict(task_id=str(uuid.uuid4()), kind=kind, state="running", stage="connecting",
                          bytes_done=0, total_bytes=None, files_done=0, total_files=None,
                          current_file=None, archive_bytes=None, result=None, error=None, **metadata)
        self.worker = None

    def check(self):
        if self.cancel.is_set():
            raise RuntimeError("task_cancelled")

    def update(self, **progress):
        with self.lock:
            previous = self.value
            if previous["state"] in TERMINAL:
                return
            if progress.get("stage", previous["stage"]) != previous["stage"] or progress.get("bytes_done", previous["bytes_done"]) < previous["bytes_done"]:
                self.samples.clear()
            previous.update(progress)
            if previous["stage"] == "rolling_back" and previous["state"] not in TERMINAL:
                previous["state"] = "rolling_back"
            now = time.monotonic()
            self.samples.append((now, previous["bytes_done"]))
            while len(self.samples) > 2 and self.samples[1][0] < now - 3:
                self.samples.popleft()

    def stage(self, name, total=None, files=None, **extra):
        self.update(**dict(dict(stage=name, bytes_done=0, total_bytes=total, files_done=0,
                    total_files=files, current_file=None), **extra))

    def request_cancel(self):
        with self.lock:
            if self.value["state"] not in TERMINAL:
                self.cancel.set()
                if self.value["state"] != "rolling_back":
                    self.value["state"] = "cancelling"

    def snapshot(self, renew=False):
        with self.lock:
            now = time.monotonic()
            if renew:
                self.lease = now
            result = dict(self.value)
            speed = 0
            if len(self.samples) > 1 and now - self.samples[-1][0] < 2:
                start, count = self.samples[0]
                speed = max(0, (result["bytes_done"] - count) / max(now - start, .001))
            if result["state"] in TERMINAL or result["stage"] == "connecting":
                speed = 0
            total = result["total_bytes"]
            result.update(elapsed_seconds=round(now - self.started, 1), speed_bps=speed,
                          eta_seconds=max(0, (total - result["bytes_done"]) / speed)
                          if total is not None and speed > 0 and len(self.samples) > 1 and now - self.samples[0][0] >= 1 else None)
            return result


class TaskManager:
    def __init__(self, lease_seconds=LEASE_SECONDS, state_dir=None):
        self.lease_seconds = lease_seconds
        self.tasks = {}
        self.path = Path(state_dir) / "tasks.json" if state_dir else None
        self.requests = {}
        if self.path and self.path.exists():
            if not _no_links(self.path) or self.path.stat().st_size > 1024 * 1024:
                raise ValueError("invalid_task_history")
            data = json.loads(self.path.read_text(encoding="utf-8"))
            if data.get("schema_version") != 1 or not isinstance(data.get("tasks"), list):
                raise ValueError("invalid_task_history")
            for value in data["tasks"][-64:]:
                if not isinstance(value, dict) or not isinstance(value.get("task_id"), str):
                    raise ValueError("invalid_task_history")
                task = Task(value.get("kind", "install"), {})
                task.value.update(value)
                if task.value["state"] not in TERMINAL:
                    task.update(state="failed", error="task_interrupted", result=None)
                self.tasks[value["task_id"]] = task
                if value.get("request_id"):
                    self.requests[value["request_id"]] = value["task_id"]

    def _save(self):
        if self.path:
            _write_json(self.path, {"schema_version": 1, "tasks": [t.value for t in self.tasks.values()]})

    def busy(self):
        return any(t.value["state"] not in TERMINAL for t in self.tasks.values())

    def start(self, kind, work, **metadata):
        request_id = metadata.get("request_id")
        if request_id is not None:
            if (not isinstance(request_id, str) or not 1 <= len(request_id.encode("utf-8")) <= 128
                    or any(ord(c) < 32 or ord(c) == 127 for c in request_id)):
                raise ValueError("invalid_request_id")
            previous = self.tasks.get(self.requests.get(request_id))
            if previous:
                if any(previous.value.get(k) != metadata.get(k) for k in ("instance_id", "release_id", "selection_id", "operation", "archive_path")):
                    raise ValueError("request_identity_changed")
                return {"task_id": previous.value["task_id"]}
        if self.busy():
            raise RuntimeError("installer_busy")
        task = Task(kind, metadata)
        # Persist bounded receipts for reconnects and retries of a lost start RPC.
        for key in list(self.tasks)[:-63]:
            if self.tasks[key].value["state"] in TERMINAL:
                self.requests.pop(self.tasks[key].value.get("request_id"), None)
                del self.tasks[key]
        self.tasks[task.value["task_id"]] = task
        if request_id:
            self.requests[request_id] = task.value["task_id"]
        try:
            self._save()
        except Exception:
            del self.tasks[task.value["task_id"]]
            self.requests.pop(request_id, None)
            raise
        task.worker = asyncio.create_task(self._run(task, work))
        return {"task_id": task.value["task_id"]}

    async def _run(self, task, work):
        token = current_task.set(task)
        guard = asyncio.create_task(self._watch_lease(task))
        try:
            task.check()
            result = await work(task)
            if task.value["kind"] == "install":
                disposition = result.get("disposition")
                if disposition not in TERMINAL:
                    raise RuntimeError("contract_mismatch")
                cause = result.get("cause") or {}
                task.update(state=disposition, result=result,
                            error=cause.get("code") if disposition != "completed" else None,
                            error_detail=cause.get("detail"))
            else:
                task.check()
                task.update(state="completed", result=result)
        except Exception as error:
            domain = getattr(error, "code", None)
            raw = domain or str(error)
            code = raw if re.fullmatch(r"[a-z0-9_]{1,128}", raw) else "host_transport_failed"
            detail = getattr(error, "detail", None) or (str(error) if code != raw else None)
            if detail is not None:
                detail = truncate_detail(str(detail))
            task.update(state="cancelled" if code in ("cancelled", "task_cancelled") else "failed",
                        error=code, error_detail=detail,
                        error_origin="domain" if domain else "host")
        finally:
            guard.cancel()
            await asyncio.gather(guard, return_exceptions=True)
            current_task.reset(token)
            try:
                self._save()
            except (OSError, ValueError):
                # The durable core receipt is authoritative after a commit.
                pass

    def _expire_lease(self, task):
        with task.lock:
            if time.monotonic() - task.lease >= self.lease_seconds:
                task.request_cancel()

    async def _watch_lease(self, task):
        while task.value["state"] not in TERMINAL and not task.cancel.is_set():
            self._expire_lease(task)
            await asyncio.sleep(min(1, self.lease_seconds / 4))

    def status(self, task_id):
        task = self.get(task_id)
        # A late reconnect cannot revive an expired foreground lease.
        self._expire_lease(task)
        return task.snapshot(renew=not task.cancel.is_set())

    def get(self, task_id):
        if not isinstance(task_id, str) or task_id not in self.tasks:
            raise ValueError("task_not_found")
        return self.tasks[task_id]

    async def shutdown(self):
        for task in self.tasks.values():
            task.request_cancel()
        await asyncio.gather(*(t.worker for t in self.tasks.values() if t.worker), return_exceptions=True)
