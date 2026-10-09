import assert from "node:assert/strict";
import { build } from "esbuild";
import { fileURLToPath } from "node:url";
const result = await build({ entryPoints: [fileURLToPath(new URL("../src/sources.ts", import.meta.url))], bundle: true, write: false, platform: "node", format: "esm" });
const { patchChip } = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`);
const patch = (status = "unknown", available = false) => ({ blockers: [], detection: { status, files: [] }, auto_installable: available });
const row = (patches = [], status = "empty", candidates = []) => ({ patches, discovery: { status, candidates } });
const cases = [
  ["managed beats ongoing lookup", row([patch("installed_managed", true)], "queued"), true, "已安装补丁"],
  ["external installation", row([patch("installed_external")]), false, "已安装补丁"],
  ["available dynamic patch beats source clue", row([patch("unknown", true)], "complete", [{match_basis:"appid"}]), false, "有可用补丁"],
  ["completed empty lookup", row(), false, "未找到补丁"],
  ["source clue alone is not installable", row([], "complete", [{match_basis:"appid"}]), false, "补丁待核对"],
  ["blocked patch is not available", row([{...patch("unknown", true), blockers:["exe_sandbox_unavailable"]}]), false, "补丁待核对"],
  ["changed installed file", row([patch("partial_or_conflict", true)]), false, "补丁需检查"],
  ["unqueried is not empty", row([], "not_queried"), false, "补丁待查询"],
  ["running lookup", row([], "queued"), true, "正在查找补丁"],
  ["failed lookup is not empty", row([], "failed"), false, "补丁查询未完成"],
];
for (const [name, value, running, expected] of cases) assert.equal(patchChip(value, running).label, expected, name);
console.log(`${cases.length} game-status cases passed`);
