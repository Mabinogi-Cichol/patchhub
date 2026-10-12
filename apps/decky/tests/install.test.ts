import assert from "node:assert/strict";
import test from "node:test";
import { installInput, primaryActionBlockers, primaryInstallAction } from "../src/actions.ts";
import { transactionResult } from "../src/install.ts";
import { installSummary, type ActionBlockerCode, type Patch, type PatchActions, type Transaction } from "@steam-patch/ui";

const availability = (available: boolean, blockers: { code: ActionBlockerCode; detail?: string }[] = []) => ({ available, blockers });
const actions = (values: Partial<PatchActions> = {}): PatchActions => ({
  automatic: availability(false), local_archive: availability(false), reinstall: availability(false), overwrite: availability(false),
  primary: { operation: "overwrite", label: "install", input: "unavailable" }, ...values,
});
const patch = (fields: Partial<Patch> = {}): Patch => ({
  release_id: "r", name: "p", source_page: "https://example.test", support_level: "publisher_zip", actions: actions(),
  compatibility: "verified_metadata", blockers: [], detection: { status: "absent", files: [] }, ...fields,
});

test("install summary consumes required backend actions", () => {
  const row = (patches: Patch[]) => ({ game: { instance_id: "g", appid: 1, name: "g", library_path: "", install_path: "", buildid: "1", availability: "installed", kind: "steam" }, match_status: "matched", patches });
  const guidance = patch({ support_level: "discover_only" });
  const available = patch({ actions: actions({ automatic: availability(true), primary: { operation: "overwrite", label: "install", input: "automatic" } }) });
  assert.equal(installSummary(row([guidance])), "找到补丁来源");
  assert.equal(installSummary(row([guidance, available])), "有可用补丁");
  assert.equal(installSummary(row([{ ...available, detection: { status: "managed", files: [] } }])), "已安装补丁");
  assert.equal(installSummary(row([{ ...available, actions: actions({ overwrite: availability(true) }), detection: { status: "partial_or_conflict", files: [] } }])), "补丁文件不完整，可覆盖安装");
});

test("automatic primary input is enabled only by automatic action and wins over archive", () => {
  const value = patch({ actions: actions({
    automatic: availability(true), local_archive: availability(true),
    primary: { operation: "overwrite", label: "install", input: "automatic" },
  }) });
  assert.deepEqual(primaryInstallAction(value, false), { operation: "overwrite", label: "install", input: "automatic", enabled: true });
  assert.equal(installInput(value), "");
  assert.equal(installInput(value, "C:/Downloads/patch.zip"), "C:/Downloads/patch.zip");
});

test("archive primary requires a selected path while automatic may be blocked", () => {
  const value = patch({ actions: actions({
    automatic: availability(false, [{ code: "artifact_requires_manual_download" }]), local_archive: availability(true),
    primary: { operation: "overwrite", label: "install", input: "local_archive" },
  }) });
  assert.equal(primaryInstallAction(value, false).enabled, false);
  assert.equal(primaryInstallAction(value, true).enabled, true);
  assert.equal(installInput(value), null);
  assert.equal(installInput(value, "C:/Downloads/local.zip"), "C:/Downloads/local.zip");
  assert.equal(installInput(value, "  "), null);
});

test("unavailable external overwrite keeps its label but remains disabled", () => {
  const value = patch({ detection: { status: "external", files: [] }, actions: actions({
    overwrite: availability(true), primary: { operation: "overwrite", label: "overwrite", input: "unavailable" },
  }) });
  assert.deepEqual(primaryInstallAction(value, true), { operation: "overwrite", label: "overwrite", input: "unavailable", enabled: false });
  assert.equal(installInput(value), null);
  assert.equal(installInput(null), null);
});

test("blocker display follows the selected path and omits unrelated reinstall advice", () => {
  const value = patch({ actions: actions({
    automatic: availability(true), local_archive: availability(false, [{ code: "local_archive_signed_only" }]),
    reinstall: availability(false, [{ code: "reinstall_unowned_or_incomplete" }]),
    overwrite: availability(false, [{ code: "needs_recovery" }]),
    primary: { operation: "overwrite", label: "install", input: "automatic" },
  }) });
  assert.deepEqual(primaryActionBlockers(value), []);
  const unavailable = { ...value, actions: { ...value.actions, primary: { operation: "overwrite" as const, label: "overwrite" as const, input: "unavailable" as const } } };
  assert.deepEqual(primaryActionBlockers(unavailable), [{ code: "needs_recovery" }]);
});
test("installation RPC result follows transaction state and error", () => {
  const tx = (state: string, error: string | null = null) => ({ state, error } as Transaction);
  assert.match(transactionResult(tx("committed")), /安装完成/);
  assert.match(transactionResult(tx("rolled_back", "file_changed")), /已回滚.*游戏文件已变化/);
  assert.match(transactionResult(tx("needs_recovery")), /需要运行修复/);
  assert.doesNotMatch(transactionResult(tx("applying")), /安装完成/);
});