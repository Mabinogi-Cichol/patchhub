import assert from "node:assert/strict";
import test from "node:test";
import { sourceCardSummary, type SourceTaskSnapshot } from "../src/source-tasks.ts";

const updating: SourceTaskSnapshot = {
  task_id: "update-1", kind: "update_source", key: "patchmoe", state: "running", instances: [], appid: null,
  progress: { phase: "fetching", completed: 2, total: 4, running: true }, matches: {}, result: null, error: null,
};

test("a collapsed card shows running progress ahead of its previous failure", () => {
  const summary = sourceCardSummary({ role: "publisher", status: "source_refresh_failed" }, updating)!;
  assert.equal(summary.warning, false);
  assert.match(summary.text, /2\/4.*50%/);
  assert.deepEqual(sourceCardSummary({ role: "publisher", status: null }, { ...updating, state: "cancelling" }),
    { text: "正在取消更新…", warning: false });
});

test("finished tasks use the source's latest attempt and keep fallback wording distinct", () => {
  assert.deepEqual(sourceCardSummary({ role: "publisher", status: "source_offline_cached" }, { ...updating, state: "failed" }),
    { text: "最近更新失败", warning: true });
  assert.deepEqual(sourceCardSummary({ role: "fallback_indexer", status: "source_refresh_failed" }),
    { text: "最近补查失败", warning: true });
  assert.equal(sourceCardSummary({ role: "publisher", status: "source_refreshed" }, { ...updating, state: "completed" }), null);
  assert.equal(sourceCardSummary({ role: "publisher", status: null }), null);
});
