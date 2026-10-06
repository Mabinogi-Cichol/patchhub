import assert from "node:assert/strict";
import test from "node:test";
import { createTaskController, type TaskApi, type TaskSnapshot } from "../src/task-controller.ts";

function snapshot(state: TaskSnapshot["state"] = "running"): TaskSnapshot {
  return { task_id: "task-1", kind: "install", state, stage: "downloading", bytes_done: 5,
    total_bytes: 10, files_done: 0, total_files: null, current_file: null, archive_bytes: null,
    elapsed_seconds: 1, speed_bps: 5, eta_seconds: 1, result: null, error: null,
    instance_id: "game", release_id: "patch" };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(r => { resolve = r; });
  return { promise, resolve };
}
function api(overrides: Partial<TaskApi> = {}): TaskApi {
  return { install: async () => ({ task_id: "task-1" }),
    status: async () => snapshot(), cancel: async () => snapshot("cancelled"), list: async () => [], ...overrides };
}
const tick = (ms = 15) => new Promise(resolve => setTimeout(resolve, ms));

test("a pending start is single flight, and leaving before its reply does not cancel", async () => {
  const start = deferred<{ task_id: string }>();
  let cancelled = 0, polled = 0;
  const controller = createTaskController(api({ install: () => start.promise,
    cancel: async () => { cancelled++; return snapshot("cancelled"); },
    status: async () => { polled++; return snapshot(); } }), 5, 5);
  const first = controller.startInstall("game", "patch", "install", "req-1");
  await assert.rejects(controller.startInstall("game", "patch", "install", "req-2"), /installer_busy/);
  controller.dismount();
  start.resolve({ task_id: "task-1" });
  await first;
  await tick();
  assert.equal(cancelled, 0);
  assert.equal(polled, 0);
  assert.equal(controller.getSnapshot(), null);
});

test("failed explicit cancel RPC retries without status polling", async () => {
  let polled = 0, cancelled = 0;
  const controller = createTaskController(api({
    status: async () => { polled++; return snapshot(); },
    cancel: async () => { cancelled++; throw new Error("offline"); },
  }), 50, 5);
  await controller.startInstall("game", "patch", "install", "req-1");
  await tick();
  await controller.requestCancel();
  await tick(35);
  assert.equal(polled, 1);
  assert.ok(cancelled >= 2);
  assert.equal(controller.getSnapshot()?.state, "cancelling");
  controller.dismount();
});

test("dismount ignores an in-flight status reply without cancelling", async () => {
  const pending = deferred<TaskSnapshot>();
  let cancelled = 0;
  const controller = createTaskController(api({ status: () => pending.promise,
    cancel: async () => { cancelled++; return snapshot("cancelled"); } }), 5, 5);
  await controller.startInstall("game", "patch", "install", "req-1");
  controller.dismount();
  pending.resolve(snapshot("completed"));
  await tick();
  assert.equal(cancelled, 0);
  assert.equal(controller.getSnapshot()?.state, "running");
});

test("reconnect surfaces retained cancellation result without renewing its lease", async () => {
  let polled = 0;
  const controller = createTaskController(api({ list: async () => [snapshot("cancelled")],
    status: async () => { polled++; return snapshot(); } }), 5, 5);
  await controller.resume();
  assert.equal(controller.getSnapshot()?.state, "cancelled");
  assert.equal(polled, 0);
});

test("returning to a game does not replay a terminal result already handled in this session", async () => {
  const controller = createTaskController(api({ list: async () => [snapshot("completed")] }));
  await controller.resume();
  assert.equal(controller.getSnapshot()?.state, "completed");
  controller.clear();
  await controller.resume();
  assert.equal(controller.getSnapshot(), null);
});

test("successful cancel keeps observing rollback until terminal state", async () => {
  let polled = 0;
  const controller = createTaskController(api({
    status: async () => { polled++; return snapshot(polled >= 2 ? "cancelled" : "running"); },
    cancel: async () => snapshot("rolling_back"),
  }), 5, 5);
  await controller.startInstall("game", "patch", "install", "req-1");
  await tick();
  await controller.requestCancel();
  await tick(25);
  assert.equal(controller.getSnapshot()?.state, "cancelled");
  assert.ok(polled >= 2);
  controller.dismount();
});

test("install and reinstall preserve request ID and optional archive", async () => {
  const calls: unknown[][] = [];
  const controller = createTaskController(api({
    install: async (...args) => { calls.push(args); return { task_id: "task-1" }; },
    status: async () => snapshot("completed"),
  }));
  await controller.startInstall("game", "exe-release", "install", "req-exe");
  await tick();
  controller.clear();
  await controller.startInstall("game", "zip-release", "reinstall", "req-zip", "local.zip");
  assert.deepEqual(calls[0], ["game", "exe-release", "install", "req-exe", ""]);
  assert.deepEqual(calls[1], ["game", "zip-release", "reinstall", "req-zip", "local.zip"]);
  controller.dismount();
});
