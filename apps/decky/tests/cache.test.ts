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
  assert.match(cacheClearNote(0), /已清理 0 B/);
  assert.match(cacheClearNote(1536), /1\.5 KiB/);
  assert.match(cacheClearNote(0, false), /没有可清理/);
  assert.match(autoCleanupNote(true), /已开启/);
  assert.match(autoCleanupNote(false), /已关闭/);
});
