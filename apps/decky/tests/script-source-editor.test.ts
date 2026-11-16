import { scriptSourceError } from "../src/script-source-error.ts";
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
const resources = new URL("../../../crates/core/resources/", import.meta.url);
const rule = JSON.parse(readFileSync(new URL("sources.json", resources), "utf8")).sources
  .find((source: { id: string }) => source.id === "kagura-en");
const bundledCode = readFileSync(new URL("source-scripts/kagura-en.lua", resources), "utf8");
const document = { rule: { ...rule, adapter: "lua" }, api_version: 1, sha256: "a".repeat(64), code: bundledCode };

const require = createRequire(import.meta.url);
const output = ts.transpileModule(readFileSync(new URL("../src/script-source-editor.tsx", import.meta.url), "utf8"), {
  compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

// Exercise actual JSX and native control callbacks using the existing page-test
// approach. Steam focus navigation and the on-device keyboard remain device QA.
function mount(document?: any, readOnly?: boolean) {
  const values: any[] = []; let hook = 0; let closed = false; let reject: unknown = false;
  const controls: any[] = []; const writes: any[][] = []; const copies: string[] = [];
  const native: Record<string, any> = {};
  for (const name of ["ButtonItem", "DropdownItem", "ToggleField", "Focusable", "PanelSection", "PanelSectionRow", "TextField"]) {
    native[name] = (props: any) => {
      controls.push({ name, props });
      if (name === "ButtonItem") return React.createElement("button", { disabled: props.disabled }, props.children);
      if (name === "TextField") return React.createElement("input", { value: props.value, readOnly: true });
      return React.createElement("div", {}, props.children);
    };
  }
  const exports: Record<string, any> = {};
  const modules: Record<string, any> = {
    "./script-source-error": { scriptSourceError },
    "react": { ...React, useState(initial: any) {
      const index = hook++; if (!(index in values)) values[index] = typeof initial === "function" ? initial() : initial;
      return [values[index], (value: any) => { values[index] = typeof value === "function" ? value(values[index]) : value; }];
    }, useRef: (current: any) => ({ current }) },
    "@decky/ui": native,
  };
  runInNewContext(output, { exports, require: (id: string) => modules[id] ?? require(id),
    navigator: { clipboard: { writeText: async (text: string) => { copies.push(text); } } } });
  function render() {
    hook = 0; controls.length = 0;
    return renderToStaticMarkup(React.createElement(exports.ScriptSourceEditor, { document, readOnly,
      onClose: () => { closed = true; }, onSave: async (...args: any[]) => {
        writes.push(args); if (reject) throw typeof reject === "boolean" ? Error("script_source_syntax_error: source/custom.lua:8: 完整诊断") : reject;
      } }));
  }
  const button = (label: string) => controls.find(c => c.name === "ButtonItem" && c.props.children === label).props;
  const input = (label: string) => controls.filter(c => c.name === "Focusable")
    .map(c => React.Children.toArray(c.props.children.props.children).find((child: any) => child.type === "textarea") as any)
    .find(element => element.props["aria-label"] === label).props;
  const field = (label: string) => controls.find(c => c.props.label === label).props;
  return { render, button, input, field, controls, writes, copies, closed: () => closed, reject: (value: unknown) => { reject = value; } };
}

function fill(editor: ReturnType<typeof mount>) {
  for (const [label, value] of [
    ["来源 ID（必填）", rule.id], ["规则名称（必填）", rule.name],
  ]) editor.field(label).onChange({ target: { value } });
  for (const [label, value] of [
    ["起始页面（必填，每行一项）", rule.pages.join("\n")],
    ["页面域名（必填，每行一项）", rule.page_hosts.join("\n")],
    ["下载域名（每行一项）", rule.download_hosts.join("\n")],
  ]) editor.input(label).onChange({ target: { value } });
}

test("large script import failures preserve named fields and Lua", async () => {
  const editor = mount(); editor.render();
  fill(editor);
  const code = `-- ${"中文".repeat(20000)}\n${bundledCode}`;
  editor.input("Lua 源码").onChange({ target: { value: code } });
  editor.render(); editor.reject(true);
  await editor.button("保存并导入").onClick();
  assert.match(editor.render(), /source\/custom.lua:8: 完整诊断/);
  assert.equal(editor.input("Lua 源码").value, code);
  assert.equal(editor.field("来源 ID（必填）").value, rule.id);
  assert.equal(editor.closed(), false);
  const [raw, version, submitted, replace] = editor.writes[0];
  assert.deepEqual(JSON.parse(raw).pages, rule.pages);
  assert.deepEqual([version, submitted, replace], [1, code, false]);
  editor.reject(false); await editor.button("保存并导入").onClick();
  assert.equal(editor.closed(), true);
});

test("edit locks ID and preserves optional values including empty strings and zero columns", async () => {
  const original = { ...document.rule, url: "", language: "", table_id: "补丁:列表",
    priority: 0, download_hosts: [], columns: { title: 0, download: 22, date: 0, language: null, appid: null } };
  const editor = mount({ ...document, rule: original });
  assert.doesNotMatch(editor.render(), new RegExp(document.sha256));
  assert.equal(editor.field("来源 ID（必填）").disabled, true);
  await editor.button(`保存并替换 ${rule.id}`).onClick();
  const [raw, version, code, replace] = editor.writes[0];
  const saved = JSON.parse(raw);
  for (const key of ["id", "url", "language", "table_id", "priority", "columns", "download_hosts"]) {
    assert.deepEqual(saved[key], original[key]);
  }
  assert.deepEqual([version, code, replace], [1, bundledCode, true]);
  assert.equal(editor.closed(), true);
});

test("builtin fields are read-only and copy the exact bundled source", async () => {
  const editor = mount(document, true);
  assert.doesNotMatch(editor.render(), /保存并替换|查看脚本示例/);
  assert.equal(editor.input("Lua 源码").disabled, true);
  assert.equal(editor.field("规则名称（必填）").disabled, true);
  await editor.button("复制 Lua 源码").onClick();
  assert.equal(editor.copies[0], bundledCode);
  assert.equal(editor.writes.length, 0);
});

test("real Decky Python Exception shows the full Lua error and keeps the draft", async () => {
  const editor = mount(); editor.render();
  fill(editor);
  editor.input("Lua 源码").onChange({ target: { value: "return {" } });
  const detail = "script_source_syntax_error: source/custom.lua:1: unexpected symbol near <eof>\nException: text inside a Lua diagnostic";
  const failure = { name: "Python Exception", pythonTraceback: `Traceback (most recent call last):\n  File loader.py, line 1\nException: ${detail}\n`, toString: () => "Python Exception" };
  editor.reject(failure); editor.render();
  await editor.button("保存并导入").onClick();
  assert.match(editor.render(), /source\/custom.lua:1/);
  assert.match(editor.render(), /text inside a Lua diagnostic/);
  assert.equal(scriptSourceError(failure), detail);
  assert.equal(editor.input("Lua 源码").value, "return {");
  assert.equal(editor.closed(), false);
  assert.equal(scriptSourceError(Error("clipboard denied")), "Error: clipboard denied");
});
