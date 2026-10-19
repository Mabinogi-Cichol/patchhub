import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { build } from "esbuild";
import { fileURLToPath } from "node:url";

const result = await build({ entryPoints: [fileURLToPath(new URL("../src/sources.ts", import.meta.url))], bundle: true, write: false, platform: "node", format: "esm" });
const {
  actionBlockers, chipLabel, gameMatchStatusHint, gameMatchStatusLabel, gameMatchStatusTone,
  patchChip, pendingMatchInstances, primaryInstallAction, sourceCoverageText, sourceChip,
  sourceIntervalLabel, sourceIntervalOptions, statusLabel, statusTone,
} = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`);
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
const row = (patches = [], status = "pending", candidates = [], instanceId = `inst-${status}`) => ({
  game: { availability: "installed", instance_id: instanceId, appid: 42 },
  patches,
  discovery: status === null ? undefined : { status, candidates },
});
const cases = [
  ["managed is display-only installed status", row([patch({ status: "installed_managed", automatic: true })], "queued"), true, "已安装补丁"],
  ["external installation remains visible", row([patch({ status: "installed_external" })]), false, "已安装补丁"],
  ["available automatic action beats source clue", row([patch({ automatic: true })], "publisher_match", [{ match_basis: "appid" }]), false, "有可用补丁"],
  ["publisher match without patch is a clue", row([], "publisher_match", [{ match_basis: "appid" }]), false, "补丁待核对"],
  ["publisher match alone", row([], "publisher_match"), false, "发行商来源命中"],
  ["fallback match is a fallback clue", row([], "fallback_match"), false, "补丁索引（HGamer）命中"],
  ["completed lookup without candidates", row([], "no_match"), false, "已启用来源中未找到补丁"],
  ["incomplete coverage", row([], "incomplete"), false, "来源覆盖不完整"],
  ["failed lookup is not empty", row([], "failed"), false, "来源查询失败"],
  ["unqueried is not empty", row([], "pending"), false, "尚未匹配"],
  ["running match", row([], "pending"), true, "匹配中"],
  ["missing discovery is unqueried", row([], null), false, "尚未匹配"],
];
for (const [name, value, running, expected] of cases) assert.equal(patchChip(value, running).label, expected, name);

// The six-state copy must be the shared map, not a Windows-local table.
const sixStates = ["pending", "publisher_match", "fallback_match", "no_match", "incomplete", "failed"];
for (const state of sixStates) {
  assert.equal(chipLabel(state), gameMatchStatusLabel(state), `${state} chip uses the shared label`);
  assert.equal(statusLabel(state), gameMatchStatusLabel(state), `${state} sentence uses the shared label`);
  const hint = gameMatchStatusHint(state);
  assert.ok(hint === null || typeof hint === "string", `${state} hint is shared`);
}
assert.equal(gameMatchStatusTone("fallback_match"), "fallback", "tone vocabulary is the shared source");
assert.equal(statusTone("publisher_match"), "ok");
assert.equal(statusTone("fallback_match"), "warn");
assert.equal(statusTone("no_match"), "warn");
assert.equal(statusTone("incomplete"), "warn");
assert.equal(statusTone("failed"), "fail");
assert.equal(statusTone(undefined), "idle");
assert.equal(chipLabel("no_match"), "已启用来源中未找到补丁");
assert.equal(statusLabel("no_match"), "已启用来源中未找到补丁");
assert.match(statusLabel(undefined), /尚未匹配/);
assert.equal(gameMatchStatusHint("incomplete"), "普通来源覆盖不完整，尚需更新来源；HGamer 不会在来源不完整时补查。");

assert.equal(sourceIntervalLabel(0), "关闭");
assert.equal(sourceIntervalLabel(6), "6 小时");
assert.equal(sourceIntervalLabel(12), "12 小时");
assert.equal(sourceIntervalLabel(24), "24 小时");
assert.equal(sourceIntervalLabel(72), "3 天");
assert.equal(sourceIntervalLabel(168), "7 天");
assert.equal(sourceIntervalLabel(336), "14 天");
assert.equal(sourceIntervalLabel(48), "48 小时");
assert.equal(sourceIntervalLabel(null), "关闭");
assert.deepEqual(sourceIntervalOptions(), [
  { data: 0, label: "关闭" },
  { data: 6, label: "6 小时" },
  { data: 12, label: "12 小时" },
  { data: 24, label: "24 小时" },
  { data: 72, label: "3 天" },
  { data: 168, label: "7 天" },
  { data: 336, label: "14 天" },
], "interval presets come from the shared preset list");

assert.equal(sourceCoverageText({ complete: false, status: "source_refreshed" }), "索引覆盖不完整，尚需更新来源");
assert.equal(sourceCoverageText({ complete: true, status: "source_refreshed" }), "索引覆盖完整");
assert.equal(sourceCoverageText(null), "来源状态未知");

const publisher = (overrides = {}) => ({ id: "kagura-cn", role: "publisher", enabled: true, complete: true, ...overrides });
const matchTask = (running) => [{ task_id: "t", kind: "match_games", key: "match", running, cancelled: false, progress: null, last_event: null, result: null, error: null }];
assert.deepEqual(pendingMatchInstances([row([], "pending", [], "inst-a"), row([], "publisher_match", [], "inst-b")], [publisher()], []), ["inst-a"], "pending games enter an incremental match");
assert.deepEqual(pendingMatchInstances([row([], "publisher_match", [], "inst-b")], [publisher()], []), [], "matched games do not match again");
assert.deepEqual(pendingMatchInstances([row([], "pending", [], "inst-a")], [publisher()], matchTask(true)), [], "a running library match is not stacked");
assert.deepEqual(pendingMatchInstances([row([], "pending", [], "inst-a")], [publisher({ enabled: false })], []), [], "disabled publisher sources do not trigger matching");
assert.deepEqual(pendingMatchInstances([row([], "pending", [], "inst-a")], [publisher({ role: "fallback_indexer" })], []), [], "the fallback indexer alone does not trigger matching");
assert.deepEqual(pendingMatchInstances([row([], "pending", [], "inst-a")], null, []), [], "no source list means no automatic match");

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
console.log(`${cases.length} game-status cases, shared six-state/interval/coverage assertions and 8 auto-match assertions passed`);

const fixture = JSON.parse(await readFile(new URL("../../../packages/contracts/fixtures/v5/cases.json", import.meta.url), "utf8"));
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
const matchStatuses = fixture.responses.filter(item => item.name.startsWith("match-open-status-")).map(item => item.response.result.results[0].status);
assert.deepEqual([...new Set(matchStatuses)], ["pending", "publisher_match", "fallback_match", "no_match", "incomplete", "failed"], "v5 open match status fixture covers the six states");
for (const status of matchStatuses) {
  assert.equal(typeof chipLabel(status), "string");
  assert.equal(typeof statusLabel(status), "string");
}
console.log("Shared generated v5 Windows outcome, event and six-state match fixtures passed");

for (const fixtureOutcome of fixture.responses.filter(item => item.name.startsWith("outcome-"))) {
  const outcome = fixtureOutcome.response.result;
  const text = installationResult(outcome);
  if (outcome.recovery?.required) assert.match(text, /修复/);
  else if (outcome.disposition === "cancelled") assert.match(text, /取消/);
  else assert.match(text, /安装未完成/);
}
console.log("Windows generated cancelled/failed/recovery dispositions passed");
