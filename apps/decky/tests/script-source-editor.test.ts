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
const bundledCode = readFileSync(new URL("source-scripts/kagura-en.lua", resources), "utf8");
const document = {
  source_id: "kagura-en",
  name: "Kagura Games 海外",
  rule: {
    id: "kagura-en",
    adapter: "lua",
    enabled: true,
    role: "publisher",
    official_title_match: false,
    priority: 20,
    pages: ["https://www.kaguragames.com/patch/"],
    page_hosts: ["www.kaguragames.com"],
    download_hosts: ["kaguraserver.com", "kaguragamespatch.com"],
  },
  api_version: 1,
  sha256: "a".repeat(64),
  code: bundledCode,
};

const require = createRequire(import.meta.url);
const output = ts.transpileModule(readFileSync(new URL("../src/script-source-editor.tsx", import.meta.url), "utf8"), {
  compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

function mount(doc?: any, readOnly?: boolean, onLoadPrompt?: () => Promise<string>) {
  const values: any[] = []; let hook = 0; let closed = false; let reject: unknown = false;
  const controls: any[] = []; const writes: any[][] = []; const copies: string[] = [];
  const native: Record<string, any> = {};
  for (const name of ["ButtonItem", "Focusable", "PanelSection", "PanelSectionRow"]) {
    native[name] = (props: any) => {
      controls.push({ name, props });
      if (name === "ButtonItem") return React.createElement("button", { disabled: props.disabled }, props.children);
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
    return renderToStaticMarkup(React.createElement(exports.ScriptSourceEditor, {
      document: doc, readOnly, onLoadPrompt,
      onClose: () => { closed = true; },
      onSave: async (...args: any[]) => {
        writes.push(args);
        if (reject) throw typeof reject === "boolean" ? Error("script_source_syntax_error: source/custom.lua:8: 完整诊断") : reject;
      },
    }));
  }
  const button = (label: string) => controls.find(c => c.name === "ButtonItem" && c.props.children === label).props;
  const hasButton = (label: string) => controls.some(c => c.name === "ButtonItem" && c.props.children === label);
  const input = (label: string) => controls.filter(c => c.name === "Focusable")
    .map(c => React.Children.toArray(c.props.children.props.children).find((child: any) => child.type === "textarea") as any)
    .find(element => element.props["aria-label"] === label).props;
  return { render, button, hasButton, input, controls, writes, copies, closed: () => closed, reject: (value: unknown) => { reject = value; } };
}

test("create mode copies AI prompt to clipboard and keeps form free of manual rule fields", async () => {
  const prompt = "https://raw.githubusercontent.com/Mabinogi-Cichol/patchhub/main/docs/custom-subscription-sources.md";
  const editor = mount(undefined, false, async () => prompt);
  const html = editor.render();
  assert.doesNotMatch(html, /来源 ID（必填）|规则名称（必填）|安装布局/);
  await editor.button("复制 AI 提示词").onClick();
  assert.equal(editor.copies[0], prompt);
  assert.match(editor.render(), /AI 提示词已复制/);
});

test("large script import failures preserve Lua draft and omit source_id in create mode", async () => {
  const editor = mount(); editor.render();
  const code = `-- ${"中文".repeat(20000)}\n${bundledCode}`;
  editor.input("Lua 源码").onChange({ target: { value: code } });
  editor.render(); editor.reject(true);
  await editor.button("保存并导入").onClick();
  assert.match(editor.render(), /source\/custom.lua:8: 完整诊断/);
  assert.equal(editor.input("Lua 源码").value, code);
  assert.equal(editor.closed(), false);
  assert.deepEqual(editor.writes[0], [code, null, 1]);
  editor.reject(false); await editor.button("保存并导入").onClick();
  assert.equal(editor.closed(), true);
});

test("edit displays read-only summary, locks source_id and hides prompt button", async () => {
  const editor = mount(document, false, async () => "unused");
  const html = editor.render();
  assert.doesNotMatch(html, new RegExp(document.sha256));
  assert.equal(editor.hasButton("复制 AI 提示词"), false);
  assert.match(html, /来源 ID：kagura-en/);
  assert.match(html, /显示名称：Kagura Games 海外/);
  assert.match(html, /允许访问域名：www\.kaguragames\.com/);
  assert.match(html, /允许下载域名：kaguraserver\.com · kaguragamespatch\.com/);
  await editor.button("保存并替换 kagura-en").onClick();
  assert.deepEqual(editor.writes[0], [bundledCode, "kagura-en", 1]);
  assert.equal(editor.closed(), true);
});

test("builtin fields are read-only and copy the exact bundled source", async () => {
  const editor = mount(document, true, async () => "unused");
  const html = editor.render();
  assert.doesNotMatch(html, /保存并替换|复制 AI 提示词/);
  assert.equal(editor.input("Lua 源码").disabled, true);
  await editor.button("复制 Lua 源码").onClick();
  assert.equal(editor.copies[0], bundledCode);
  assert.equal(editor.writes.length, 0);
});

test("real Decky Python Exception shows the full Lua error and keeps the draft", async () => {
  const editor = mount(); editor.render();
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
