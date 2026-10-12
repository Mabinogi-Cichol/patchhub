import assert from "node:assert/strict";
import test from "node:test";
import { autoCleanupNote, cacheClearNote, cacheSummary, formatSize } from "../src/cache.ts";

test("formatSize renders binary units and guards non-finite input", () => {
  assert.equal(formatSize(0), "0 B");
  assert.equal(formatSize(512), "512 B");
  assert.equal(formatSize(1024), "1.0 KiB");
  assert.equal(formatSize(1024 * 1024 * 1.5), "1.5 MiB");
  assert.equal(formatSize(Number.NaN), "—");
  assert.equal(formatSize(-5), "0 B");
});

test("cache summary does not invent usage statistics", () => {
  assert.equal(cacheSummary(), "当前版本不提供占用统计");
});

test("cleanup and toggle notes describe the resulting state", () => {
  const result = (status: "not_requested" | "completed" | "partial" | "failed" | "busy", removed_bytes = 0) =>
    ({ status, performed: status === "completed" || status === "partial", removed_bytes, failures: [] });
  assert.match(cacheClearNote(result("completed")), /已清理 0 B/);
  assert.match(cacheClearNote(result("completed", 1536)), /1\.5 KiB/);
  assert.match(cacheClearNote(result("completed")), /已清理/);
  assert.match(cacheClearNote(result("not_requested")), /未执行/);
  assert.match(cacheClearNote(result("busy")), /稍后重试/);
  assert.match(cacheClearNote({ ...result("partial", 1536), failures: [{ code: "disk_unavailable", detail: "磁盘不可用" }] }), /1\.5 KiB.*磁盘不可用.*重试/);
  assert.match(cacheClearNote(result("failed", 512)), /清理失败.*512 B.*重试/);
  assert.match(autoCleanupNote(true), /已开启/);
  assert.match(autoCleanupNote(false), /已关闭/);
});
