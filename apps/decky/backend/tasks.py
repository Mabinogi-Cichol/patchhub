"""Backend-owned tasks with a renewable UI lease and cooperative cancellation."""
import asyncio
from collections import deque
from contextvars import ContextVar
from threading import Event, RLock
import time
import uuid

current_task = ContextVar("patch_task", default=None)
TERMINAL = {"completed", "cancelled", "failed"}
LEASE_SECONDS = 15


class Task:
    def __init__(self, kind, metadata):
        self.cancel = Event()
        self.lock = RLock()
        self.started = self.lease = time.monotonic()
        self.samples = deque()
        self.value = dict(task_id=str(uuid.uuid4()), kind=kind, state="running", stage="scanning",
                          bytes_done=0, total_bytes=None, files_done=0, total_files=None,
                          current_file=None, archive_bytes=None, result=None, error=None, **metadata)
        self.worker = None

    def check(self):
        if self.cancel.is_set():
            raise RuntimeError("task_cancelled")

    def update(self, **progress):
        with self.lock:
            previous = self.value
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
    def __init__(self, lease_seconds=LEASE_SECONDS):
        self.tasks = {}
        self.lease_seconds = lease_seconds

    def busy(self):
        return any(t.value["state"] not in TERMINAL for t in self.tasks.values())

    def start(self, kind, work, **metadata):
        if self.busy():
            raise RuntimeError("installer_busy")
        task = Task(kind, metadata)
        # Retain bounded terminal history for reconnects, without renewing leases.
        for key in list(self.tasks)[:-31]:
            if self.tasks[key].value["state"] in TERMINAL:
                del self.tasks[key]
        self.tasks[task.value["task_id"]] = task
        task.worker = asyncio.create_task(self._run(task, work))
        return {"task_id": task.value["task_id"]}

    async def _run(self, task, work):
        async def watchdog():
            while task.value["state"] not in TERMINAL:
                await asyncio.sleep(min(1, self.lease_seconds / 2))
                if time.monotonic() - task.lease > self.lease_seconds:
                    task.request_cancel()
        guard = asyncio.create_task(watchdog())
        token = current_task.set(task)
        try:
            task.check()
            result = await work(task)
            if task.value["kind"] == "install":
                state = result.get("state")
                if state == "committed":
                    # Commit wins a late cancellation race.
                    task.update(state="completed", result=result)
                elif state == "rolled_back" and "task_cancelled" in (result.get("error") or ""):
                    task.update(state="cancelled", result=result)
                else:
                    task.update(state="failed", result=result, error=result.get("error") or state or "install_failed")
            else:
                task.check()
                task.update(state="completed", result=result)
        except Exception as error:
            code = str(error)
            task.update(state="cancelled" if code in ("task_cancelled", "download_cancelled") else "failed", error=code)
        finally:
            current_task.reset(token)
            guard.cancel()

    def get(self, task_id):
        if not isinstance(task_id, str) or task_id not in self.tasks:
            raise ValueError("task_not_found")
        return self.tasks[task_id]

    async def shutdown(self):
        for task in self.tasks.values():
            task.request_cancel()
        await asyncio.gather(*(t.worker for t in self.tasks.values()), return_exceptions=True)
