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
const output = ts.transpileModule(`${source}\nexport { GameManager, QuickAccess, GlobalSources, Manager };`, { compilerOptions: {
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
  const methods: string[] = [];
  const removals: unknown[][] = [];
  let pending = Promise.resolve();
  const state: any = { report: { installs: [item], sources: [], diagnostics: [] }, transactions: [], busy: null, sourceTasks: task ? { [task.task_id]: task } : {}, error: null, note: null, errorInstance: null, noteInstance: null };
  const native: Record<string, any> = {};
  for (const name of ["Button", "ButtonItem", "DialogButton", "Field", "Focusable", "PanelSection", "PanelSectionRow", "DropdownItem", "TextField", "ToggleField"]) {
    native[name] = (props: any) => {
      controls.push({ name, props });
      if (name === "Button" || name === "ButtonItem" || name === "DialogButton") return React.createElement("button", { disabled: props.disabled }, props.children);
      if (name === "TextField") return React.createElement("input", { "aria-label": props.label, value: props.value, readOnly: true });
      return React.createElement("div", { "data-control": name, "data-label": props.label }, props.label, props.children);
    };
  }
  native.SidebarNavigation = (props: any) => {
    controls.push({ name: "SidebarNavigation", props });
    return props.pages.find((page: any) => page.route === props.page)?.content;
  };
  native.showModal = () => ({ Close() {} });
  native.useParams = () => ({ instance: "sources" });
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
      methods.push(method);
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
      if (method === "remove_source") {
        removals.push(args);
        return (state.report.sources ?? []).filter((source: any) => source.id !== args[0]);
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
    "./game-artwork": { GameArtwork: (props: any) => {
      controls.push({ name: "GameArtwork", props }); return null;
    } },
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
  const renderSettings = () => { hook = 0; controls.length = 0; return renderToStaticMarkup(React.createElement(exports.Manager)); };
  const button = (label: string) => controls.find(control => control.name === "ButtonItem" && React.Children.toArray(control.props.children).join("") === label)!.props;
  return { render, renderList, renderSources, renderSettings, controls, button, installs, state, sourceWrites, scriptWrites, projections, copies, methods, removals, settle: () => pending };
}

test("artwork follows the selected game and only appears in patch details", () => {
  const page = mount(game());
  page.render();
  assert.equal(page.controls.find(control => control.name === "GameArtwork")?.props.appid, 1);
  assert.equal(page.controls.some(control => control.name === "Field" && control.props.label === "游戏"), false);
  assert.equal(page.controls.some(control => control.name === "PanelSection" && control.props.title === "当前游戏"), false);
  for (const panel of ["records", "sources"]) {
    page.render(panel);
    assert.equal(page.controls.some(control => control.name === "GameArtwork"), false);
  }
  page.renderList();
  assert.equal(page.controls.some(control => control.name === "GameArtwork"), false);
});

test("all native sidebar pages hide duplicate content titles and retain sidebar labels", () => {
  const page = mount(game());
  page.render();
  const gameNav = page.controls.find(control => control.name === "SidebarNavigation")!.props;
  assert.equal(gameNav.showTitle, true);
  assert.deepEqual(Array.from(gameNav.pages.filter((p: any) => typeof p === "object"), (p: any) => [p.title, p.hideTitle]),
    [["补丁管理", true], ["操作记录", true], ["游戏订阅", true]]);
  page.renderSettings();
  const settingsNav = page.controls.find(control => control.name === "SidebarNavigation")!.props;
  assert.equal(settingsNav.showTitle, true);
  assert.deepEqual(Array.from(settingsNav.pages.filter((p: any) => typeof p === "object"), (p: any) => [p.title, p.hideTitle]),
    [["管理订阅源", true], ["缓存、备份与更新", true]]);
});

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

test("builtin sources are viewed read-only and user sources can be deleted", async () => {
  const page = mount(game());
  const script = { api_version: 1, sha256: "a".repeat(64) };
  page.state.report.sources = [
    { id: "patchmoe", name: "Shiravune", enabled: true, role: "publisher", origin: "builtin", pages: [], interval_hours: 0, entry_count: 0, revision: 0, script },
    { id: "mine", name: "我的脚本", enabled: true, role: "publisher", origin: "added", pages: [], interval_hours: 0, entry_count: 0, revision: 0, script },
  ];
  page.state.script = { rule: { id: "patchmoe", name: "Shiravune", adapter: "lua", pages: [] }, api_version: 1, code: "return {}", sha256: script.sha256 };
  page.state.expandedSources = { patchmoe: true };
  page.renderSources();
  assert.match(page.renderSources(), /内置来源随版本更新/);
  assert.equal(page.controls.some(control => control.name === "ButtonItem" && control.props.children === "编辑脚本"), false);
  assert.equal(page.controls.some(control => control.name === "ButtonItem" && control.props.children === "删除来源"), false);
  page.button("查看脚本").onClick(); await new Promise<void>(resolve => setImmediate(resolve)); page.renderSources();
  const editor = () => page.controls.find(control => control.name === "ScriptSourceEditor")!.props;
  assert.equal(editor().readOnly, true);
  assert.equal(editor().document.rule.id, "patchmoe");
  editor().onClose(); page.renderSources();

  page.state.expandedSources = { mine: true };
  page.renderSources();
  assert.equal(page.controls.some(control => control.name === "ButtonItem" && control.props.children === "查看脚本"), false);
  page.button("删除来源").onClick(); page.renderSources();
  assert.match(page.renderSources(), /删除后该来源不再参与更新和匹配/);
  page.button("取消").onClick(); page.renderSources();
  assert.equal(page.removals.length, 0);
  page.button("删除来源").onClick(); page.renderSources();
  page.button("确认删除").onClick(); await page.settle();
  assert.deepEqual(Array.from(page.removals[0] as unknown[]), ["mine"]);
  assert.equal((page.state.report.sources as unknown[]).length, 1);
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
  assert.ok(html.indexOf("重新查询") < html.indexOf("AppID"));
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
  assert.doesNotMatch(html, /打开补丁来源页/);
  page.controls.find(control => control.name === "DialogButton")!.props.onClick();
  assert.match(page.render(), /打开补丁来源页/);
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
  assert.equal(page.controls.some(control => control.name === "DropdownItem" && control.props.label === "选择补丁"), true);
  const selectionIndex = page.controls.findIndex(control => control.name === "DropdownItem" && control.props.label === "选择补丁");
  const downloadIndex = page.controls.findIndex(control => control.name === "DropdownItem" && control.props.label === "选择下载项");
  const installIndex = page.controls.findIndex(control => control.name === "ButtonItem" && control.props.children === "安装补丁");
  const detailsIndex = page.controls.findIndex(control => control.name === "DialogButton" && control.props["aria-controls"]);
  const queryIndex = page.controls.findIndex(control => control.name === "ButtonItem" && control.props.children === "重新查询");
  assert.ok(selectionIndex < downloadIndex && downloadIndex < installIndex && installIndex < detailsIndex && detailsIndex < queryIndex);
  const appidIndex = page.controls.findIndex(control => control.name === "Field" && control.props.label === "AppID");
  const buildIndex = page.controls.findIndex(control => control.name === "Field" && control.props.label === "Build");
  const pathsIndex = page.controls.findIndex(control => control.name === "ButtonItem" && control.props.children?.[0] === "安装路径与游戏库");
  assert.ok(queryIndex < appidIndex && appidIndex < buildIndex && buildIndex < pathsIndex);
  assert.equal(page.controls.filter(control => control.name === "ButtonItem").at(-1)?.props.children?.[0], "安装路径与游戏库");
  page.controls.find(control => control.name === "DialogButton" && control.props["aria-controls"])!.props.onClick();
  page.render();
  const dropdowns = page.controls.filter(control => control.name === "DropdownItem");
  const patches = dropdowns.find(control => control.props.label === "选择补丁")!.props;
  assert.equal(patches.rgOptions.length, 2);
  const mirrors = dropdowns.find(control => control.props.label === "选择下载项")!.props;
  assert.deepEqual(Array.from(mirrors.rgOptions, (entry: any) => entry.data), ["mirror-a", "mirror-b"]);
  mirrors.onChange({ data: "mirror-b" });
  page.render();
  assert.equal(page.controls.find(control => control.name === "DropdownItem" && control.props.label === "选择下载项")!.props.selectedOption, "mirror-b");
  assert.equal(page.controls.find(control => control.name === "DialogButton")!.props["aria-expanded"], true);
  page.controls.find(control => control.name === "DropdownItem" && control.props.label === "选择补丁")!.props.onChange(patches.rgOptions[1]);
  page.render();
  assert.equal(page.controls.find(control => control.name === "DropdownItem" && control.props.label === "选择补丁")!.props.selectedOption, patches.rgOptions[1].data);
  assert.equal(page.controls.find(control => control.name === "DialogButton")!.props["aria-expanded"], true);
});

test("patch language, version, conditions and source link fold together while installation stays available", () => {
  const item = game();
  item.discovery!.candidates = [{ ...candidate, version_raw: "3", requires_dlc: [42] }];
  item.patches = [patch({ candidate_id: "candidate-1", support_level: "publisher_exe",
    download_url: "https://downloads.example.test/patch.exe", download_label: "主下载" })];
  const page = mount(item);
  const toggle = () => page.controls.find(control => control.name === "DialogButton" && control.props["aria-controls"])!.props;
  const collapsed = page.render();
  assert.equal(toggle()["aria-expanded"], false);
  assert.doesNotMatch(collapsed, /downloads\.example\.test|当前状态|安装方式|匹配源|补丁语言|补丁版本|适用条件|打开来源网页/);
  assert.equal(page.button("安装补丁").disabled, false);
  const before = page.methods.length;
  toggle().onClick();
  const expanded = page.render();
  assert.equal(toggle()["aria-expanded"], true);
  assert.match(expanded, /downloads\.example\.test\/patch\.exe/);
  assert.match(expanded, /当前状态.*兼容性.*安装方式/);
  assert.match(expanded, /补丁语言.*补丁版本.*3.*适用条件.*需要 DLC 42/);
  assert.equal(page.button("打开来源网页 ↗").disabled, undefined);
  toggle().onClick();
  assert.doesNotMatch(page.render(), /downloads\.example\.test/);
  assert.equal(toggle()["aria-expanded"], false);
  assert.equal(page.methods.length, before);
  assert.equal(page.button("安装补丁").disabled, false);
});

test("dynamic EXE hides unsupported file preview and keeps installation; ZIP and signed releases keep preview", async () => {
  const item = game();
  item.patches = [patch({ selection_id: "exe-selection-one-1", support_level: "publisher_exe" }),
    patch({ release_id: "zip", selection_id: "zip-selection-two-1", support_level: "publisher_zip" }),
    patch({ release_id: "signed" })];
  const page = mount(item);
  const html = page.render();
  assert.doesNotMatch(html, /查看文件变更|exe_check_required/);
  assert.equal(page.button("安装补丁").disabled, false);
  assert.ok(!page.methods.includes("preview"));
  const selector = () => page.controls.find(control => control.name === "DropdownItem" && control.props.label === "选择补丁")!.props;
  for (const id of ["zip-selection-two-1", "signed"]) {
    selector().onChange(selector().rgOptions.find((option: any) => option.data === id));
    page.render();
    assert.equal(page.button("查看文件变更（只读）").disabled, false);
    page.button("查看文件变更（只读）").onClick();
    await page.settle();
  }
  assert.equal(page.methods.filter(method => method === "preview").length, 2);
  selector().onChange(selector().rgOptions.find((option: any) => option.data === "exe-selection-one-1"));
  assert.doesNotMatch(page.render(), /查看文件变更|exe_check_required/);
  assert.equal(page.button("安装补丁").disabled, false);
});

test("source-only patch selector stays outside the fold while its metadata and links stay inside", () => {
  const item = game();
  item.discovery!.candidates = [candidate, { ...candidate, candidate_id: "candidate-2", patch_title: "另一个补丁" }];
  const page = mount(item);
  const collapsed = page.render();
  assert.ok(collapsed.indexOf("手动输入下载链接") < collapsed.indexOf("补丁详情"));
  assert.ok(collapsed.indexOf("补丁详情") < collapsed.indexOf("重新查询"));
  assert.equal(page.controls.some(control => control.name === "DropdownItem" && control.props.label === "选择补丁"), true);
  assert.doesNotMatch(collapsed, /补丁语言|补丁版本|适用条件|打开补丁来源页/);
  assert.ok(page.button("手动输入下载链接 ▾"));
  page.controls.find(control => control.name === "DialogButton")!.props.onClick();
  page.render();
  const selector = page.controls.find(control => control.name === "DropdownItem" && control.props.label === "选择补丁")!.props;
  assert.equal(selector.rgOptions.length, 2);
  selector.onChange(selector.rgOptions[1]);
  assert.match(page.render(), /另一个补丁/);
  assert.equal(page.controls.find(control => control.name === "DialogButton")!.props["aria-expanded"], true);
  assert.ok(page.button("打开补丁来源页 ↗"));
});
