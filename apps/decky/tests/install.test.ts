import assert from "node:assert/strict";
import test from "node:test";
import { transactionResult } from "../src/install.ts";
import { canAutoInstall, installSummary, patchInstallable, type Patch, type Transaction } from "@steam-patch/ui";

const patch = (support_level: string, blockers: string[] = []): Patch => ({
  release_id: "r", name: "p", source_page: "https://example.test", support_level,
  compatibility: "verified_metadata", blockers, detection: { status: "absent", files: [] },
});

test("auto install requires backend approval and no blockers", () => {
  assert.equal(canAutoInstall(patch("managed_zip")), false);
  assert.equal(canAutoInstall({ ...patch("managed_zip"), auto_installable: true }), true);
  assert.equal(canAutoInstall({ ...patch("managed_zip", ["platform_not_verified"]), auto_installable: true }), false);
  assert.equal(canAutoInstall({ ...patch("publisher_exe"), auto_installable: true, source_id: "hgamer" }), false);
});

test("summary prefers installed and available over source guidance", () => {
  const row = (patches: Patch[]) => ({ game: { instance_id: "g", appid: 1, name: "g", library_path: "", install_path: "", buildid: "1", availability: "installed", kind: "steam" }, match_status: "matched", patches });
  const guidance = patch("discover_only");
  const available = { ...patch("publisher_exe"), auto_installable: true };
  assert.equal(installSummary(row([guidance])), "找到补丁来源");
  assert.equal(installSummary(row([guidance, available])), "有可用补丁");
  assert.equal(installSummary(row([{ ...available, compatibility: "check_required" }])), "有可用补丁");
  assert.equal(installSummary(row([{ ...available, compatibility: "check_required", source_id: "hgamer" }])), "找到补丁来源");
  assert.equal(installSummary(row([{ ...available, compatibility: "check_required", blockers: ["build_not_verified"] }])), "找到补丁来源");
  assert.equal(installSummary(row([{ ...available, detection: { status: "managed", files: [] } }])), "已安装补丁");
  assert.equal(installSummary(row([{ ...available, reinstall_available: true, detection: { status: "partial", files: [] } }])), "补丁文件不完整，需重新安装");
  assert.equal(installSummary(row([]), true), "正在查找补丁");
  assert.equal(installSummary({ ...row([]), discovery: { status: "source_refresh_failed", candidates: [], diagnostics: [] } }), "来源查询失败");
  assert.equal(installSummary({ ...row([]), discovery: { status: "cache_partial", candidates: [], diagnostics: [] } }), "来源查询不完整");
});

test("installation RPC result follows transaction state and error", () => {
  const tx = (state: string, error: string | null = null) => ({ state, error } as Transaction);
  assert.match(transactionResult(tx("committed")), /安装完成/);
  assert.match(transactionResult(tx("rolled_back", "file_changed")), /已回滚.*游戏文件已变化/);
  assert.match(transactionResult(tx("needs_recovery")), /需要运行修复/);
  assert.doesNotMatch(transactionResult(tx("applying")), /安装完成/);
});


test("Decky install buttons follow platform ZIP and EXE link probes", () => {
  const links = { zip: { available: true }, exe: { available: false, blockers: [{ code: "exe_sandbox_unavailable" }] } };
  const signed = { ...patch("managed_zip"), download_available: true };
  const zip = { ...patch("publisher_zip"), selection_id: "zip-selection-1", download_available: true };
  const exe = { ...patch("publisher_exe"), selection_id: "exe-selection-1", download_available: true };
  assert.equal(patchInstallable(signed, links, true), true);
  assert.equal(patchInstallable(signed, { ...links, zip: { available: false } }, true), false);
  assert.equal(patchInstallable(zip, links), true);
  assert.equal(patchInstallable(exe, links), false);
  assert.equal(patchInstallable(exe, { ...links, exe: { available: true } }), true);
  assert.equal(patchInstallable({ ...exe, selection_id: null }, { ...links, exe: { available: true } }), false);
});
