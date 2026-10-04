import { test } from "node:test";
import assert from "node:assert/strict";
import { taskProgressPercent } from "../src/task-progress.ts";
import type { TaskSnapshot } from "../src/task-controller.ts";

const task = { bytes_done: 50, total_bytes: 100 } as TaskSnapshot;

test("progress matches Steam's 0–100 percentage contract", () => {
  assert.equal(taskProgressPercent(task), 50);
  assert.equal(taskProgressPercent({ ...task, bytes_done: 100 }), 100);
  assert.equal(taskProgressPercent({ ...task, bytes_done: 200 }), 100);
});

test("unknown or invalid totals remain indeterminate", () => {
  assert.equal(taskProgressPercent({ ...task, total_bytes: null }), null);
  assert.equal(taskProgressPercent({ ...task, total_bytes: 0 }), null);
  assert.equal(taskProgressPercent(null), null);
});
