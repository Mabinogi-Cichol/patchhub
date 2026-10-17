import assert from "node:assert/strict";
import test from "node:test";
import { installInput, primaryActionBlockers, primaryInstallAction } from "../src/actions.ts";
import { installationResult, installSummary, type InstallerOutcome, type ActionBlockerCode, type Patch, type PatchActions, type Transaction } from "@steam-patch/ui";

const availability = (available: boolean, blockers: { code: ActionBlockerCode; detail?: string }[] = []) => ({ available, blockers });
const actions = (values: Partial<PatchActions> = {}): PatchActions => ({
  automatic: availability(false), local_archive: availability(false), reinstall: availability(false), overwrite: availability(false),
  primary: { operation: "overwrite", label: "install", input: "unavailable" }, ...values,
});
const patch = (fields: Partial<Patch> = {}): Patch => ({
  release_id: "r", name: "p", source_page: "https://example.test", support_level: "publisher_zip", actions: actions(),
  compatibility: "verified_metadata", blockers: [], detection: { status: "not_detected", files: [] }, ...fields,
});

test("install summary consumes required backend actions", () => {
  const row = (patches: Patch[]) => ({ game: { instance_id: "g", appid: 1, name: "g", library_path: "", install_path: "", buildid: "1", availability: "installed", kind: "steam" }, match_status: "matched", patches });
  const guidance = patch({ support_level: "discover_only" });
  const available = patch({ actions: actions({ automatic: availability(true), primary: { operation: "overwrite", label: "install", input: "automatic" } }) });
  assert.equal(installSummary(row([guidance])), "找到补丁来源");
  assert.equal(installSummary(row([guidance, available])), "有可用补丁");
  assert.equal(installSummary(row([{ ...available, detection: { status: "installed_managed", files: [] } }])), "已安装补丁");
  assert.equal(installSummary(row([{ ...available, actions: actions({ overwrite: availability(true) }), detection: { status: "partial_or_conflict", files: [] } }])), "补丁文件不完整，可覆盖安装");
});

test("automatic primary input is enabled only by automatic action and wins over archive", () => {
  const value = patch({ actions: actions({
    automatic: availability(true), local_archive: availability(true),
    primary: { operation: "overwrite", label: "install", input: "automatic" },
  }) });
  assert.deepEqual(primaryInstallAction(value, false), { operation: "overwrite", label: "install", input: "automatic", available: true });
  assert.equal(installInput(value), "");
  assert.equal(installInput(value, "C:/Downloads/patch.zip"), "C:/Downloads/patch.zip");
});

test("archive primary requires a selected path while automatic may be blocked", () => {
  const value = patch({ actions: actions({
    automatic: availability(false, [{ code: "artifact_requires_manual_download" }]), local_archive: availability(true),
    primary: { operation: "overwrite", label: "install", input: "local_archive" },
  }) });
  assert.equal(primaryInstallAction(value, false).available, false);
  assert.equal(primaryInstallAction(value, true).available, true);
  assert.equal(installInput(value), null);
  assert.equal(installInput(value, "C:/Downloads/local.zip"), "C:/Downloads/local.zip");
  assert.equal(installInput(value, "  "), null);
});

test("unavailable external overwrite keeps its label but remains disabled", () => {
  const value = patch({ detection: { status: "installed_external", files: [] }, actions: actions({
    overwrite: availability(true), primary: { operation: "overwrite", label: "overwrite", input: "unavailable" },
  }) });
  assert.deepEqual(primaryInstallAction(value, true), { operation: "overwrite", label: "overwrite", input: "unavailable", available: false });
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
test("installation RPC result consumes core disposition and recovery", () => {
  const outcome = (disposition: "completed" | "cancelled" | "failed", recovery = false) => ({
    disposition, cause: { code: "file_changed" }, recovery: recovery ? { required: true } : null,
  } as InstallerOutcome);
  assert.match(installationResult(outcome("completed")), /安装完成/);
  assert.match(installationResult(outcome("cancelled")), /安全取消/);
  assert.match(installationResult(outcome("failed")), /安装未完成.*游戏文件已变化/);
  assert.match(installationResult(outcome("failed", true)), /需要修复/);
  assert.match(
    installationResult({ disposition: "failed", cause: { code: "install_incomplete" } } as InstallerOutcome),
    /安装未完成，已写入的文件未恢复/,
  );
  assert.match(
    installationResult({ disposition: "failed", cause: { code: "cancelled_incomplete" } } as InstallerOutcome),
    /安装已取消，但已写入的文件未恢复/,
  );
});
