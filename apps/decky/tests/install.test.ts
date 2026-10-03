import assert from "node:assert/strict";
import test from "node:test";
import { installBlocker, planExpired, preparedSource, transactionResult } from "../src/install.ts";
import type { Patch, Plan, Transaction } from "@steam-patch/ui";

const patch = (support_level: string, blockers: string[] = []): Patch => ({
  release_id: "r", name: "p", source_page: "https://example.test", support_level,
  compatibility: "verified_metadata", blockers, detection: { status: "absent", files: [] },
});

test("reviewed ZIP and official root ZIP can be prepared when unblocked", () => {
  assert.equal(installBlocker(patch("managed_zip")), null);
  assert.equal(installBlocker(patch("publisher_zip")), null);
  assert.match(installBlocker(patch("manual_download"))!, /可用的 ZIP/);
  assert.match(installBlocker(patch("managed_zip", ["platform_not_verified"]))!, /平台尚未审核/);
});

test("plan expires at its Unix-second deadline and missing deadline is unsafe", () => {
  const plan = { expires_at: 1_700_000_100 } as Plan;
  assert.equal(planExpired(plan, 1_700_000_099_999), false);
  assert.equal(planExpired(plan, 1_700_000_100_000), true);
  assert.equal(planExpired({} as Plan), true);
});

test("installation RPC result follows transaction state and error", () => {
  const tx = (state: string, error: string | null = null) => ({ state, error } as Transaction);
  assert.match(transactionResult(tx("committed")), /安装完成/);
  assert.match(transactionResult(tx("rolled_back", "file_changed")), /已回滚.*游戏文件已变化/);
  assert.match(transactionResult(tx("needs_recovery")), /需要运行修复/);
  assert.doesNotMatch(transactionResult(tx("applying")), /安装完成/);
});

test("prepared install distinguishes a reused local ZIP from a download", () => {
  assert.match(preparedSource("local"), /已复用.*校验/);
  assert.match(preparedSource("download"), /已下载.*校验/);
  assert.doesNotMatch(preparedSource("local"), /安装完成/);
  assert.match(preparedSource("download", "publisher_root"), /官方 ZIP.*游戏根目录|官方 ZIP.*根目录/);
  assert.doesNotMatch(preparedSource("download", "publisher_root"), /已审核/);
});
