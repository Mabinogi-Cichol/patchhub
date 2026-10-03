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

test("cache summary falls back safely when settings are unavailable", () => {
  assert.equal(cacheSummary({ total_bytes: 2048, file_count: 2, publisher_bytes: 2048, core_bytes: 0, jobs: { stage_bytes: 0, backup_bytes: 0 } }), "2.0 KiB · 2 个文件");
  assert.equal(cacheSummary(undefined), "0 B · 0 个文件");
});

test("cleanup and toggle notes describe the resulting state", () => {
  assert.match(cacheClearNote(0), /已清理 0 B/);
  assert.match(cacheClearNote(1536), /1\.5 KiB/);
  assert.match(autoCleanupNote(true), /已开启/);
  assert.match(autoCleanupNote(false), /已关闭/);
});
