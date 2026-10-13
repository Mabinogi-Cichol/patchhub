import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { build } from "esbuild";
import { fileURLToPath } from "node:url";

const result = await build({ entryPoints: [fileURLToPath(new URL("../src/sources.ts", import.meta.url))], bundle: true, write: false, platform: "node", format: "esm" });
const { patchChip, primaryInstallAction, actionBlockers } = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`);
const availability = (available, blockers = []) => ({ available, blockers });
const patch = ({ status = "unknown", automatic = false, localArchive = false, reinstall = false, overwrite = false, label = "install", input = "automatic", blockers = [], localBlockers = [], overwriteBlockers = [], reinstallBlockers = [] } = {}) => ({
  blockers: ["legacy"],
  detection: { status, files: [] },
  actions: {
    automatic: availability(automatic, blockers),
    local_archive: availability(localArchive, localBlockers),
    reinstall: availability(reinstall, reinstallBlockers),
    overwrite: availability(overwrite, overwriteBlockers),
    primary: { operation: "overwrite", label, input },
  },
});
const row = (patches = [], status = "empty", candidates = []) => ({ patches, discovery: { status, candidates } });
const cases = [
  ["managed is display-only installed status", row([patch({ status: "installed_managed", automatic: true })], "queued"), true, "已安装补丁"],
  ["external installation remains visible", row([patch({ status: "installed_external" })]), false, "已安装补丁"],
  ["available automatic action beats source clue", row([patch({ automatic: true })], "complete", [{ match_basis: "appid" }]), false, "有可用补丁"],
  ["completed empty lookup", row(), false, "未找到补丁"],
  ["source clue alone is not installable", row([], "complete", [{ match_basis: "appid" }]), false, "补丁待核对"],
  ["unavailable actions stay unavailable despite installed detection", row([patch({ status: "installed_external", input: "unavailable", label: "overwrite" })]), false, "已安装补丁"],
  ["changed installed file", row([patch({ status: "partial_or_conflict", overwrite: true })]), false, "补丁需检查"],
  ["unqueried is not empty", row([], "not_queried"), false, "补丁待查询"],
  ["running lookup", row([], "queued"), true, "正在查找补丁"],
  ["failed lookup is not empty", row([], "failed"), false, "补丁查询未完成"],
];
for (const [name, value, running, expected] of cases) assert.equal(patchChip(value, running).label, expected, name);

const manual = patch({ localArchive: true, input: "local_archive", label: "install", blockers: [{ code: "zip_download_unavailable" }], localBlockers: [{ code: "local_archive_signed_only" }], reinstallBlockers: [{ code: "reinstall_unowned_or_incomplete" }] });
assert.deepEqual(primaryInstallAction(manual, false), { operation: "overwrite", label: "install", input: "local_archive", available: false }, "local ZIP action requires a selected archive");
assert.equal(primaryInstallAction(manual, true).available, true, "local ZIP action works while automatic is unavailable");
const automatic = patch({ automatic: true, localArchive: true, input: "automatic", label: "overwrite" });
assert.equal(primaryInstallAction(automatic, true).input, "automatic", "selected local ZIP does not override the core primary route");
assert.equal(primaryInstallAction(automatic, true, "local_archive").input, "local_archive", "explicit local ZIP action selects its own route");
assert.equal(primaryInstallAction(automatic, true, "local_archive").available, true, "explicit local ZIP action consumes a selected archive");
assert.equal(primaryInstallAction(automatic, false, "local_archive").available, false, "explicit local ZIP action requires an archive");
assert.equal(primaryInstallAction(patch({ input: "unavailable", label: "overwrite" }), false).available, false, "external installed label does not enable an unavailable primary action");
assert.equal(primaryInstallAction(automatic, false).label, "overwrite", "primary label is consumed directly from actions");
assert.equal("auto_installable" in automatic, false, "legacy installability alias is absent");
assert.equal("reinstall_available" in automatic, false, "legacy reinstall alias is absent");
assert.deepEqual(actionBlockers(patch({ automatic: true, input: "automatic", blockers: [{ code: "game_not_ready", detail: "build is unknown" }] })), [{ code: "game_not_ready", detail: "build is unknown" }], "primary automatic blockers preserve detail without unrelated action blockers");
assert.deepEqual(actionBlockers(manual), [{ code: "zip_download_unavailable" }, { code: "local_archive_signed_only" }], "local archive primary explains automatic and local blockers without reinstall-only blockers");
assert.deepEqual(actionBlockers(patch({ input: "unavailable", blockers: [{ code: "game_not_ready" }], overwriteBlockers: [{ code: "needs_recovery" }] })), [{ code: "needs_recovery" }], "unavailable primary shows overwrite blockers only");
console.log(`${cases.length} game-status cases and 13 Windows action assertions passed`);
const fixture = JSON.parse(await readFile(new URL("../../../packages/contracts/fixtures/v4/cases.json", import.meta.url), "utf8"));
const shared = await build({ entryPoints: [fileURLToPath(new URL("../../../packages/ui/src/index.ts", import.meta.url))], bundle: true, write: false, platform: "node", format: "esm" });
const { installationResult, progressStageLabel, progressPercent } = await import(`data:text/javascript;base64,${Buffer.from(shared.outputFiles[0].text).toString("base64")}`);
const completed = fixture.responses.find(item => item.name === "install_patch-success").response.result;
assert.equal(completed.disposition, "completed");
assert.match(installationResult(completed), /安装完成/);
assert.match(installationResult(completed, true), /覆盖安装完成/);
for (const item of fixture.events) {
  if (!item.error && item.event.event === "install_progress") {
    assert.equal(typeof progressStageLabel(item.event.stage, item.event.substage), "string");
    const percent = progressPercent(item.event.bytes_done, item.event.total_bytes);
    assert.ok(percent == null || percent >= 0 && percent <= 100);
  }
}
console.log("Shared generated v4 Windows outcome and event presentation fixtures passed");

for (const fixtureOutcome of fixture.responses.filter(item => item.name.startsWith("outcome-"))) {
  const outcome = fixtureOutcome.response.result;
  const text = installationResult(outcome);
  if (outcome.recovery?.required) assert.match(text, /修复/);
  else if (outcome.disposition === "cancelled") assert.match(text, /取消/);
  else assert.match(text, /安装未完成/);
}
console.log("Windows generated cancelled/failed/recovery dispositions passed");
