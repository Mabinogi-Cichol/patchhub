import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { build } from "esbuild";
import { fileURLToPath } from "node:url";
async function load(path) {
  const result = await build({ entryPoints: [fileURLToPath(new URL(path, import.meta.url))], bundle: true, write: false, platform: "node", format: "esm" });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`);
}
const ui = await load("../packages/ui/src/index.ts");
const protocol = await load("../packages/contracts/generated/protocol.ts");
const cases = JSON.parse(await readFile(new URL("../packages/contracts/fixtures/v4/cases.json", import.meta.url), "utf8"));
const success = method => cases.responses.find(c => c.name === `${method}-success`).response.result;
assert.deepEqual(success("get_capabilities").methods, [...protocol.METHODS]);
for (const c of cases.details) assert.equal(protocol.truncateDetail(c.input), c.expected, c.name);
for (const id of ["", "a".repeat(129), "\u0001", "界".repeat(43)]) assert.equal(protocol.validId(id, protocol.MAX_REQUEST_ID_BYTES), false);
assert.equal(protocol.validId("界".repeat(42), protocol.MAX_REQUEST_ID_BYTES), true);
assert.equal(ui.reason("unknown_future_code"), "unknown_future_code");
const patch = success("scan").installs[0].patches[0];
assert.equal(ui.primaryInstallAction(patch, false).available, true);
const blocked = structuredClone(patch);
blocked.actions.automatic = { available: false, blockers: [{ code: "game_not_ready" }] };
blocked.actions.primary.input = "unavailable";
blocked.detection.status = "installed_managed";
assert.equal(ui.primaryInstallAction(blocked, true).available, false);
assert.equal(ui.installInput(blocked), null);
const committed = success("install_patch");
committed.cleanup = {status:"partial",performed:true,removed_bytes:0,failures:[{code:"cleanup_failed",detail:"disk"}]};
assert.equal(ui.installationResult(committed), "安装完成");
for (const c of cases.events.filter(c => !c.error && c.event.event === "install_progress")) {
  assert.equal(typeof ui.progressStageLabel(c.event.stage, c.event.substage), "string");
  if (c.event.total_bytes === null) assert.equal(ui.progressPercent(c.event.bytes_done, c.event.total_bytes), null);
}
for (const url of ["http://example.com", "https://user@example.com", "https://127.0.0.1", "https://foo.local", "https://example.com:444", "https://localhost./", "https://foo.localhost./", "https://foo.local./", "https://example.com./"]) assert.equal(ui.safeSourceUrl(url), false);
assert.equal(ui.safeSourceUrl("https://example.com/source"), true);
for (const code of ["fresh", "source_refreshed", "source_not_modified"]) assert.equal(ui.sourceRefreshSucceeded(code), true);
console.log("Shared v4 generated DTO, UTF-8 bounds, action refusal, terminal, progress and URL fixtures passed");
