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

test("a pending start is single flight, and leaving cancels its late reply", async () => {
  const start = deferred<{ task_id: string }>();
  let cancelled = 0, polled = 0;
  const controller = createTaskController(api({ install: () => start.promise,
    cancel: async () => { cancelled++; return snapshot("cancelled"); },
    status: async () => { polled++; return snapshot(); } }), 5, 5);
  const first = controller.startInstall("game", "patch", null, "install", "req-1");
  await assert.rejects(controller.startInstall("game", "patch", null, "install", "req-2"), /installer_busy/);
  controller.dismount();
  start.resolve({ task_id: "task-1" });
  await first;
  await tick();
  assert.equal(cancelled, 1);
  assert.equal(polled, 0);
  assert.equal(controller.getSnapshot(), null);
});

test("failed explicit cancel RPC retries without status polling", async () => {
  let polled = 0, cancelled = 0;
  const controller = createTaskController(api({
    status: async () => { polled++; return snapshot(); },
    cancel: async () => { cancelled++; throw new Error("offline"); },
  }), 50, 5);
  await controller.startInstall("game", "patch", null, "install", "req-1");
  await tick();
  await controller.requestCancel();
  await tick(35);
  assert.equal(polled, 1);
  assert.ok(cancelled >= 2);
  assert.equal(controller.getSnapshot()?.state, "cancelling");
  controller.dismount();
});

test("dismount cancels and ignores a late completed status reply", async () => {
  const pending = deferred<TaskSnapshot>();
  let cancelled = 0;
  const controller = createTaskController(api({ status: () => pending.promise,
    cancel: async () => { cancelled++; return snapshot("cancelled"); } }), 5, 5);
  await controller.startInstall("game", "patch", null, "install", "req-1");
  controller.dismount();
  pending.resolve(snapshot("completed"));
  await tick();
  assert.equal(cancelled, 1);
  assert.equal(controller.getSnapshot()?.state, "cancelling");
});

test("a new frontend ignores historical terminal results without renewing leases", async () => {
  let polled = 0;
  const controller = createTaskController(api({ list: async () => [snapshot("completed"), { ...snapshot("cancelled"), task_id: "old-cancel" }, { ...snapshot("failed"), task_id: "old-failure", error: "installer_busy" }],
    status: async () => { polled++; return snapshot(); } }), 5, 5);
  await controller.resume();
  assert.equal(controller.getSnapshot(), null);
  assert.equal(polled, 0);
  controller.dismount();
  await controller.resume();
  assert.equal(controller.getSnapshot(), null);
});

test("returning to a game does not replay a terminal result already handled in this session", async () => {
  const controller = createTaskController(api({ status: async () => snapshot("completed"), list: async () => [snapshot("completed")] }));
  await controller.startInstall("game", "patch", null, "install", "req-1");
  await tick();
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
  await controller.startInstall("game", "patch", null, "install", "req-1");
  await tick();
  await controller.requestCancel();
  await tick(25);
  assert.equal(controller.getSnapshot()?.state, "cancelled");
  assert.ok(polled >= 2);
  controller.dismount();
});

test("install, reinstall and overwrite preserve request ID and optional archive", async () => {
  const calls: unknown[][] = [];
  const controller = createTaskController(api({
    install: async (...args) => { calls.push(args); return { task_id: "task-1" }; },
    status: async () => snapshot("completed"),
  }));
  await controller.startInstall("game", null, "exe-selection-1", "install", "req-exe");
  await tick();
  controller.clear();
  await controller.startInstall("game", "zip-release", null, "reinstall", "req-zip", "local.zip");
  assert.deepEqual(calls[0], ["game", null, "exe-selection-1", null, "install", "req-exe", null]);
  assert.deepEqual(calls[1], ["game", "zip-release", null, "local.zip", "reinstall", "req-zip", null]);
  await tick();
  controller.clear();
  await controller.startInstall("game", null, "zip-selection-1", "install", "req-dynamic-zip");
  assert.deepEqual(calls[2], ["game", null, "zip-selection-1", null, "install", "req-dynamic-zip", null]);
  await tick();
  controller.clear();
  await controller.startInstall("game", null, "exe-selection-2", "overwrite", "req-overwrite");
  assert.deepEqual(calls[3], ["game", null, "exe-selection-2", null, "overwrite", "req-overwrite", null]);
  controller.dismount();
});

test("a manual URL uses the same foreground task with no release, selection or archive target", async () => {
  const calls: unknown[][] = [];
  const controller = createTaskController(api({
    install: async (...args) => { calls.push(args); return { task_id: "manual-task" }; },
    status: async () => ({ ...snapshot("failed"), task_id: "manual-task", error: "download_failed" }),
  }));
  const input = { source_id: "publisher", candidate_id: "candidate-1", url: "https://example.test/download?id=1" };
  await controller.startInstall("game", null, null, "overwrite", "manual-request", "", input);
  await tick();
  assert.deepEqual(calls, [["game", null, null, null, "overwrite", "manual-request", input]]);
  assert.equal(controller.getSnapshot()?.state, "failed");
  assert.equal(controller.getSnapshot()?.error, "download_failed");
  assert.equal(controller.claimTerminal(controller.getSnapshot()!), true);
  controller.dismount();
});


test("closing during startup cancels before any status renewal", async () => {
  const start = deferred<{ task_id: string }>();
  let cancelled = 0, polled = 0;
  const controller = createTaskController(api({ install: () => start.promise,
    cancel: async () => { cancelled++; return snapshot("cancelled"); },
    status: async () => { polled++; return snapshot(); } }), 5, 5);
  const pending = controller.startInstall("game", "patch", null, "install", "req-1");
  assert.equal(controller.isBusy(), true);
  await controller.requestCancel();
  start.resolve({ task_id: "task-1" });
  await pending;
  await tick();
  assert.equal(cancelled, 1);
  assert.equal(polled, 0);
  assert.equal(controller.getSnapshot()?.state, "cancelled");
  assert.equal(controller.isBusy(), false);
});

test("reconnecting cancels orphan work and observes only its restoration", async () => {
  let cancelled = 0;
  const controller = createTaskController(api({ list: async () => [snapshot()],
    cancel: async () => { cancelled++; return snapshot("rolling_back"); },
    status: async () => snapshot("cancelled") }), 5, 5);
  await controller.resume();
  await tick();
  assert.equal(cancelled, 1);
  assert.equal(controller.getSnapshot()?.state, "cancelled");
  controller.dismount();
});

test("returning after dismount reads the retained result without resuming installation", async () => {
  let cancelled = 0, polled = 0;
  const controller = createTaskController(api({
    cancel: async () => { cancelled++; return snapshot("cancelled"); },
    status: async () => { polled++; return snapshot(); },
    list: async () => [snapshot("cancelled")],
  }), 100, 5);
  await controller.startInstall("game", "patch", null, "install", "req-1");
  controller.dismount();
  await controller.resume();
  await tick();
  assert.equal(cancelled, 1);
  assert.equal(polled, 1);
  assert.equal(controller.getSnapshot()?.state, "cancelled");
  controller.clear();
  await controller.resume();
  assert.equal(controller.getSnapshot(), null);
});

test("closing a terminal result never sends cancellation", async () => {
  let cancelled = 0;
  const controller = createTaskController(api({ status: async () => snapshot("completed"), list: async () => [snapshot("completed")],
    cancel: async () => { cancelled++; return snapshot("cancelled"); } }));
  await controller.startInstall("game", "patch", null, "install", "req-1");
  await tick();
  await controller.resume();
  assert.equal(controller.getSnapshot()?.state, "completed");
  await controller.requestCancel();
  controller.clear();
  controller.dismount();
  assert.equal(cancelled, 0);
  assert.equal(controller.getSnapshot(), null);
});

test("a terminal outcome refreshes once across detail and modal remounts", async () => {
  let scans = 0;
  const controller = createTaskController(api({ status: async () => snapshot("cancelled"), list: async () => [snapshot("cancelled")] }));
  await controller.startInstall("game", "patch", null, "install", "req-1");
  await tick();
  function mountedModal() {
    const task = controller.getSnapshot();
    if (task && controller.claimTerminal(task)) scans++;
  }
  mountedModal();
  controller.dismount();
  await controller.resume();
  mountedModal();
  assert.equal(scans, 1);
  assert.equal(controller.getSnapshot()?.state, "cancelled");
  assert.equal(controller.claimTerminal({ ...snapshot("completed"), task_id: "foreign" }), false);
  assert.equal(controller.claimTerminal(snapshot("running")), false);
  controller.clear();
  await controller.resume();
  assert.equal(controller.getSnapshot(), null);
});
