import { scriptSourceError } from "../src/script-source-error.ts";
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import * as ui from "@patchhub/ui";
import * as actions from "../src/actions.ts";
import * as discovery from "../src/discovery.ts";
import * as sourceTasks from "../src/source-tasks.ts";
import * as route from "../src/route.ts";
import { feedbackFor, needsRecovery } from "../src/session.ts";
import type { Install, Patch, SourceCandidate } from "@patchhub/ui";

const require = createRequire(import.meta.url);
const source = readFileSync(new URL("../src/index.tsx", import.meta.url), "utf8");
const output = ts.transpileModule(`${source}\nexport { GameManager, QuickAccess, GlobalSources };`, { compilerOptions: {
  jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022,
} }).outputText;
const game = (): Install => ({ game: { instance_id: "game", appid: 1, name: "测试游戏", library_path: "/library", install_path: "/library/game", buildid: "123", availability: "installed", kind: "game" }, patches: [], match_status: "no_catalog_match", discovery: { status: "failed", candidates: [], diagnostics: [{ code: "source_refresh_failed", detail: "query-secret", source_id: null }] } });
const candidate = { source_id: "publisher", candidate_id: "candidate-1", patch_title: "来源补丁", source_page: "https://example.test/source", match_basis: "appid", warnings: ["language_unknown"], format_reported: "unknown", format_observed: "unknown" } as SourceCandidate;
const available = { available: true, blockers: [] };
const unavailable = { available: false, blockers: [] };
const patch = (fields: Partial<Patch> = {}): Patch => ({ release_id: "release", name: "补丁", source_page: "https://example.test/source", support_level: "managed_zip", compatibility: "verified_metadata", blockers: [], detection: { status: "not_detected", files: [] }, actions: { automatic: available, local_archive: available, overwrite: available, reinstall: unavailable, primary: { input: "automatic", operation: "overwrite", label: "install" } }, ...fields });

// Render the actual page JSX with native Decky controls represented by HTML.
// Hook state is retained so native control callbacks can be exercised without Steam.
function mount(item: Install, task?: sourceTasks.SourceTaskSnapshot) {
  const values: unknown[] = [];
  let hook = 0;
  const controls: Array<{ name: string; props: any }> = [];
  const installs: unknown[][] = [];
  const sourceWrites: unknown[][] = [];
  const scriptWrites: unknown[][] = [];
  const projections: string[][] = [];
  const copies: string[] = [];
  let pending = Promise.resolve();
  const state: any = { report: { installs: [item], sources: [], diagnostics: [] }, transactions: [], busy: null, sourceTasks: task ? { [task.task_id]: task } : {}, error: null, note: null, errorInstance: null, noteInstance: null };
  const native: Record<string, any> = {};
  for (const name of ["Button", "ButtonItem", "Field", "Focusable", "PanelSection", "PanelSectionRow", "DropdownItem", "TextField", "ToggleField"]) {
    native[name] = (props: any) => {
      controls.push({ name, props });
      if (name === "Button" || name === "ButtonItem") return React.createElement("button", { disabled: props.disabled }, props.children);
      if (name === "TextField") return React.createElement("input", { "aria-label": props.label, value: props.value, readOnly: true });
      return React.createElement("div", { "data-control": name, "data-label": props.label }, props.label, props.children);
    };
  }
  native.SidebarNavigation = (props: any) => {
    controls.push({ name: "SidebarNavigation", props });
    return props.pages.find((page: any) => page.route === props.page)?.content;
  };
  native.showModal = () => ({ Close() {} });
  native.Navigation = {};
  const exports: Record<string, any> = {};
  const mockSession = {
    getSnapshot: () => state, subscribe() {}, setFeedback(value: any) { Object.assign(state, value); },
    update(value: any) { Object.assign(state, value); },
    async projectRows(ids: string[]) { projections.push(ids); },
    commitSources(sources: unknown[]) { state.report.sources = sources; },
    commitScriptSource(sources: unknown[], revision: number) { state.report.sources = sources; state.report.settings_revision = revision; },
    invalidateDiscovery() { state.report.installs.forEach((row: any) => { row.discovery = { status: "pending", candidates: [] }; }); },
    work(_label: string, action: () => Promise<any>, success: (value: any) => void) {
      pending = (async () => { try { success(await action()); } catch (error) { state.error = String(error); } })();
      return pending;
    },
  };
  const modules: Record<string, any> = {
    "./script-source-error": { scriptSourceError },
    "@decky/api": { callable: (method: string) => async (...args: unknown[]) => {
      if (method === "list_sources") {
        if (state.rejectRefresh) throw Error("列表读取失败");
        return state.savedSources ?? [];
      }
      if (method === "get_script_source") {
        if (state.rejectRead) throw Error("读取被拒绝");
        return state.script;
      }
      if (method === "import_script_source") {
        scriptWrites.push(args);
        if (state.rejectScript) throw Error("script_source_syntax_error: 保留诊断");
        state.script = { rule: JSON.parse(args[0] as string), api_version: args[1], code: args[2], sha256: "a".repeat(64) };
        state.savedSources = [{ ...state.script.rule, script: { api_version: 1, sha256: state.script.sha256 } }];
        return { rule: state.script.rule, settings_revision: 3 };
      }
      if (method !== "set_source_name") return;
      sourceWrites.push(args);
      if (state.rejectName) throw Error("保存被拒绝");
      return state.report.sources.map((source: any) => source.id === args[0] ? { ...source, name: (args[1] as string).trim() } : source);
    }, definePlugin: () => {}, routerHook: {} },
    "@decky/ui": native,
    "@patchhub/ui": ui,
    "react-icons/fa": { FaGamepad: () => null },
    "react": { ...React, useState(initial: unknown) { const index = hook++; if (!(index in values)) values[index] = initial; return [values[index], (next: unknown) => { values[index] = typeof next === "function" ? next(values[index]) : next; }]; }, useRef: (current: unknown) => ({ current }), useEffect() {}, useLayoutEffect() {}, useSyncExternalStore: (_subscribe: unknown, snapshot: () => unknown) => snapshot() },
    "./session": { createManagerSession: () => mockSession, feedbackFor, needsRecovery },
    "./actions": actions, "./discovery": discovery, "./source-tasks": sourceTasks, "./route": route,
    "./steam": { gamePresentation: (_appid: number, name: string) => ({ name }), transactionGameName: () => "游戏", matchesGameQuery: () => true },
    "./tasks": { tasks: { isBusy: () => false, startInstall: async (...args: unknown[]) => { installs.push(args); } } },
    "./preferences": { getListPreferences: () => ({ filter: "all", query: "" }) },
    "./app-update": {}, "./cache": {}, "./task-progress-view": {},
    "./script-source-editor": { ScriptSourceEditor: (props: any) => {
      controls.push({ name: "ScriptSourceEditor", props }); return null;
    } },
  };
  runInNewContext(output, { exports, crypto: { randomUUID: () => "request-1" }, URL, console,
    navigator: { clipboard: { writeText: async (text: string) => { copies.push(text); } } },
    require: (id: string) => id.endsWith(".png") ? "logo" : modules[id] ?? require(id) });
  const render = (panel?: string) => { hook = 0; controls.length = 0; return renderToStaticMarkup(React.createElement(exports.GameManager, { item, seg: "game", panel })); };
  const renderList = () => { hook = 0; controls.length = 0; return renderToStaticMarkup(React.createElement(exports.QuickAccess)); };
  const renderSources = () => { hook = 0; controls.length = 0; return renderToStaticMarkup(React.createElement(exports.GlobalSources)); };
  const button = (label: string) => controls.find(control => control.name === "ButtonItem" && React.Children.toArray(control.props.children).join("") === label)!.props;
  return { render, renderList, renderSources, controls, button, installs, state, sourceWrites, scriptWrites, projections, copies, settle: () => pending };
}

test("source page imports and reads script documents through RPC, refreshes and preserves failed edits", async () => {
  const page = mount(game()); page.renderSources();
  page.button("导入脚本来源").onClick(); page.renderSources();
  const editor = () => page.controls.find(control => control.name === "ScriptSourceEditor")!.props;
  const rule = JSON.stringify({ id: "custom", name: "脚本来源", adapter: "lua", pages: [] });
  await editor().onSave(rule, 1, "return {}", false);
  editor().onClose(); page.state.expandedSources = { custom: true }; page.renderSources();
  assert.match(page.state.note, /脚本已保存，尚未执行/);
  assert.deepEqual(page.projections[0], ["game"]);
  page.button("编辑脚本").onClick(); await new Promise<void>(resolve => setImmediate(resolve)); page.renderSources();
  assert.equal(editor().document.code, "return {}");
  page.state.rejectScript = true;
  await assert.rejects(editor().onSave(rule, 1, "return {", true), /script_source_syntax_error/);
  page.renderSources();
  assert.equal(editor().document.code, "return {}");
  assert.equal(page.state.busy, null);
  page.state.rejectScript = false;
  await editor().onSave(rule, 1, "return { updated = true }", true); editor().onClose(); page.renderSources();
  assert.equal(page.scriptWrites.at(-1)?.[3], true);
  page.button("编辑脚本").onClick(); await new Promise<void>(resolve => setImmediate(resolve)); page.renderSources();
  assert.equal(editor().document.code, "return { updated = true }");
  page.state.rejectRefresh = true;
  await editor().onSave(rule, 1, "return { saved = true }", true);
  assert.match(page.state.error, /脚本已保存，但刷新来源或游戏状态失败/);
  assert.equal(page.state.script.code, "return { saved = true }");
  editor().onClose(); page.renderSources(); page.state.rejectRead = true;
  page.button("编辑脚本").onClick(); await new Promise<void>(resolve => setImmediate(resolve)); page.renderSources();
  assert.match(page.state.error, /读取被拒绝/);
  assert.equal(page.controls.some(control => control.name === "ScriptSourceEditor"), false);
});

test("rule examples stay optional and copy JSON without replacing the draft", async () => {
  const page = mount(game());
  page.renderSources();
  const input = () => page.controls.find(control => control.name === "TextField" && control.props.label === "源规则 JSON（高级）")!.props;
  input().onChange({ target: { value: "保留未完成规则" } });
  assert.doesNotMatch(page.renderSources(), /patches\.vendor\.example/);
  page.button("查看规则示例").onClick();
  assert.match(page.renderSources(), /table_html/);
  await page.button("复制示例").onClick();
  const example = JSON.parse(page.copies[0]);
  assert.equal(example.adapter, "table_html");
  assert.deepEqual(example.columns, { title: 0, appid: 1, download: 2, date: 3, language: 4 });
  assert.equal(input().value, "保留未完成规则");
  page.button("收起规则示例").onClick();
  assert.doesNotMatch(page.renderSources(), /patches\.vendor\.example/);
});

test("source names save through the RPC without discarding matches and failed edits stay visible", async () => {
  const page = mount(game());
  const previous = page.state.report.installs[0].discovery;
  page.state.report.sources = [{ id: "patchmoe", name: "Shiravune", enabled: true,
    role: "publisher", pages: [], interval_hours: 0, entry_count: 0, revision: 0 }];
  page.state.expandedSources = { patchmoe: true };
  page.renderSources();
  const input = () => page.controls.find(control => control.name === "TextField" && control.props.label === "来源名称")!.props;
  assert.equal(input().value, "Shiravune");
  assert.equal(page.button("保存名称").disabled, true);
  input().onChange({ target: { value: "  " } });
  page.renderSources();
  assert.equal(page.button("保存名称").disabled, true);
  input().onChange({ target: { value: "  我的发行商  " } });
  page.renderSources();
  assert.equal(page.button("保存名称").disabled, false);
  page.button("保存名称").onClick();
  await page.settle();
  assert.deepEqual(Array.from(page.sourceWrites[0]), ["patchmoe", "  我的发行商  "]);
  assert.match(page.renderSources(), /我的发行商/);
  assert.equal(page.state.report.installs[0].discovery, previous);
  page.state.rejectName = true;
  input().onChange({ target: { value: "保留草稿" } });
  page.renderSources();
  page.button("保存名称").onClick();
  await page.settle();
  assert.match(page.renderSources(), /保存被拒绝/);
  assert.equal(input().value, "保留草稿");
  assert.equal(page.state.report.sources[0].name, "我的发行商");
});

test("game subscriptions display the latest global source name without rescanning", () => {
  const item = game();
  item.sources = [{ id: "patchmoe", name: "Old name", enabled: true, subscribed: true,
    matched: false, role: "publisher", pages: [] }] as Install["sources"];
  const page = mount(item);
  page.state.report.sources = [{ id: "patchmoe", name: "我的发行商" }];
  page.render("sources");
  assert.ok(page.controls.some(control => control.name === "PanelSection" && control.props.title === "我的发行商"));
  assert.ok(!page.controls.some(control => control.name === "PanelSection" && control.props.title === "Old name"));
});

test("the default page has three native panels and neutral empty results with query controls", () => {
  const page = mount(game());
  const html = page.render();
  const nav = page.controls.find(control => control.name === "SidebarNavigation")!.props;
  assert.equal(nav.page, "/patchhub/game/patches");
  assert.deepEqual(Array.from(nav.pages.filter((entry: any) => typeof entry === "object"), (entry: any) => entry.title), ["补丁管理", "操作记录", "游戏订阅"]);
  assert.match(html, /AppID.*1.*Build.*123/);
  assert.match(html, /暂无补丁结果/);
  assert.match(html, /重新查询/);
  assert.doesNotMatch(html, /概览|query-secret|查询失败|覆盖不完整|尚未完成/);
  assert.ok(!page.controls.some(control => control.name === "DropdownItem" && control.props.label === "偏好的补丁语言"));
  page.button("安装路径与游戏库 ▾").onClick();
  assert.match(page.render(), /\/library\/game.*复制安装路径.*\/library.*复制游戏库路径/);
});

test("query animation follows a running task and ends for failed or cancelled tasks", () => {
  const task: sourceTasks.SourceTaskSnapshot = { task_id: "query", kind: "rematch_game", key: "game", instances: ["game"], appid: 1, state: "running", progress: null, matches: {}, result: null, error: null };
  const page = mount(game(), task);
  assert.match(page.render(), /class="patch-query-spinner"/);
  for (const state of ["failed", "cancelled", "completed"] as const) {
    page.state.sourceTasks = { query: { ...task, state } };
    assert.doesNotMatch(page.render(), /class="patch-query-spinner"/);
  }
});

test("the game list omits source query diagnostics and retains bottom settings and scan controls", () => {
  const page = mount(game());
  page.state.report.diagnostics = [{ code: "source_refresh_failed", message: "query-secret" }, { code: "discovery_http_503", message: "query-secret" }, { code: "game_write_failed", path: "/game" }];
  const html = page.renderList();
  assert.doesNotMatch(html, /查询失败|HTTP 503|query-secret|来源覆盖不完整/);
  assert.match(html, /暂无补丁结果/);
  assert.match(html, /写入游戏文件失败/);
  assert.equal(page.button("全局设置 ›").disabled, undefined);
  assert.equal(page.button("重新扫描").disabled, false);
});

test("a retained failed rematch does not surface its error in global source tasks", () => {
  const task: sourceTasks.SourceTaskSnapshot = { task_id: "query", kind: "rematch_game", key: "game", instances: ["game"], appid: 1, state: "failed", progress: null, matches: {}, result: null, error: "source_refresh_failed", error_detail: "query-secret" };
  const page = mount(game(), task);
  const html = page.renderSources();
  assert.doesNotMatch(html, /来源任务|query-secret|查询失败|来源覆盖不完整/);
  assert.match(html, /匹配全部游戏/);
});

test("eligible local ZIP import stays optional even with no automatic download", async () => {
  const item = game();
  item.patches = [patch({ actions: { automatic: unavailable, local_archive: available, overwrite: available, reinstall: unavailable, primary: { input: "unavailable", operation: "overwrite", label: "install" } } })];
  const page = mount(item);
  const html = page.render();
  assert.match(html, /未找到下载链接/);
  assert.equal(page.button("安装补丁").disabled, true);
  assert.doesNotMatch(html, /ZIP 路径|请先选择本地/);
  page.button("导入本地 ZIP（可选） ▾").onClick();
  page.render();
  page.controls.find(control => control.name === "TextField")!.props.onChange({ target: { value: "/home/deck/Downloads/p.zip" } });
  page.render();
  await page.button("使用本地 ZIP 安装").onClick();
  assert.deepEqual(Array.from(page.installs[0]), ["game", "release", null, "overwrite", "request-1", "/home/deck/Downloads/p.zip"]);
});

test("source-only candidates accept a manual URL and use the same installation task", async () => {
  const item = game(); item.discovery!.candidates = [candidate];
  const page = mount(item);
  const html = page.render();
  assert.match(html, /未找到下载链接/);
  assert.match(html, /打开补丁来源页/);
  assert.doesNotMatch(html, /尚待确认|线索用于核对|待核对来源/);
  page.button("手动输入下载链接 ▾").onClick();
  page.render();
  page.controls.find(control => control.name === "TextField")!.props.onChange({ target: { value: "https://downloads.example.test/download?id=1" } });
  page.render();
  assert.equal(page.button("下载安装补丁").disabled, false);
  await page.button("下载安装补丁").onClick();
  const args = page.installs[0];
  assert.deepEqual(Array.from(args.slice(0, 6)), ["game", null, null, "overwrite", "request-1", ""]);
  assert.equal(JSON.stringify(args[6]), JSON.stringify({ source_id: "publisher", candidate_id: "candidate-1", url: "https://downloads.example.test/download?id=1" }));
});

test("patch and exact mirror selectors remain native Decky dropdowns", () => {
  const item = game();
  item.patches = [patch({ release_id: "first", candidate_id: "candidate-1", selection_id: "mirror-a", download_url: "https://a.example.test/p.zip" }), patch({ release_id: "first", candidate_id: "candidate-1", selection_id: "mirror-b", download_url: "https://b.example.test/p.zip" }), patch({ release_id: "second" })];
  const page = mount(item); page.render();
  const dropdowns = page.controls.filter(control => control.name === "DropdownItem");
  assert.equal(dropdowns.find(control => control.props.label === "补丁")!.props.rgOptions.length, 2);
  const mirrors = dropdowns.find(control => control.props.label === "选择下载项")!.props;
  assert.deepEqual(Array.from(mirrors.rgOptions, (entry: any) => entry.data), ["mirror-a", "mirror-b"]);
  mirrors.onChange({ data: "mirror-b" });
  page.render();
  assert.equal(page.controls.find(control => control.name === "DropdownItem" && control.props.label === "选择下载项")!.props.selectedOption, "mirror-b");
});
