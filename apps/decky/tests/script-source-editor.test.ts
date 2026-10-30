import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { scriptRuleExample, scriptCodeExample } from "../src/script-source-examples.ts";

const require = createRequire(import.meta.url);
const output = ts.transpileModule(readFileSync(new URL("../src/script-source-editor.tsx", import.meta.url), "utf8"), {
  compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

// Exercise actual JSX and native control callbacks using the existing page-test
// approach. Steam focus navigation and the on-device keyboard remain device QA.
function mount(document?: any) {
  const values: any[] = []; let hook = 0; let closed = false; let reject = false;
  const controls: any[] = []; const writes: any[][] = []; const copies: string[] = [];
  const native: Record<string, any> = {};
  for (const name of ["ButtonItem", "Focusable", "PanelSection", "PanelSectionRow", "TextField"]) {
    native[name] = (props: any) => {
      controls.push({ name, props });
      if (name === "ButtonItem") return React.createElement("button", { disabled: props.disabled }, props.children);
      if (name === "TextField") return React.createElement("input", { value: props.value, readOnly: true });
      return React.createElement("div", {}, props.children);
    };
  }
  const exports: Record<string, any> = {};
  const modules: Record<string, any> = {
    "react": { ...React, useState(initial: any) {
      const index = hook++; if (!(index in values)) values[index] = initial;
      return [values[index], (value: any) => { values[index] = value; }];
    }, useRef: (current: any) => ({ current }) },
    "@decky/ui": native,
    "./script-source-examples": { scriptRuleExample, scriptCodeExample },
  };
  runInNewContext(output, { exports, require: (id: string) => modules[id] ?? require(id),
    navigator: { clipboard: { writeText: async (text: string) => { copies.push(text); } } } });
  function render() {
    hook = 0; controls.length = 0;
    return renderToStaticMarkup(React.createElement(exports.ScriptSourceEditor, { document,
      onClose: () => { closed = true; }, onSave: async (...args: any[]) => {
        writes.push(args); if (reject) throw Error("script_source_syntax_error: source/custom.lua:8: 完整诊断");
      } }));
  }
  const button = (label: string) => controls.find(c => c.name === "ButtonItem" && c.props.children === label).props;
  const input = (label: string) => controls.filter(c => c.name === "Focusable")
    .map(c => React.Children.toArray(c.props.children.props.children).find((child: any) => child.type === "textarea") as any)
    .find(element => element.props["aria-label"] === label).props;
  return { render, button, input, controls, writes, copies, closed: () => closed, reject: (value: boolean) => { reject = value; } };
}

test("script examples equal the documented author files", () => {
  const root = new URL("../../../docs/features/script-source-adapters/examples/publisher/", import.meta.url);
  assert.deepEqual(JSON.parse(scriptRuleExample), JSON.parse(readFileSync(new URL("source.json", root), "utf8")));
  assert.equal(scriptCodeExample, readFileSync(new URL("adapter.lua", root), "utf8").replaceAll("\r\n", "\n"));
});

test("large script import failures preserve both inputs; examples do not modify drafts", async () => {
  const editor = mount(); editor.render();
  const code = `-- ${"中文".repeat(20000)}\nreturn {}`;
  editor.input("脚本规则 JSON").onChange({ target: { value: scriptRuleExample } });
  editor.input("Lua 源码").onChange({ target: { value: code } });
  editor.render(); editor.reject(true);
  await editor.button("保存并导入").onClick();
  assert.match(editor.render(), /source\/custom.lua:8: 完整诊断/);
  assert.equal(editor.input("Lua 源码").value, code);
  assert.equal(editor.input("脚本规则 JSON").value, scriptRuleExample);
  assert.equal(editor.closed(), false);
  assert.deepEqual(Array.from(editor.writes[0]), [scriptRuleExample, 1, code, false]);
  editor.button("查看脚本示例").onClick(); editor.render();
  await editor.button("复制源码示例").onClick();
  await Promise.resolve();
  assert.equal(editor.copies[0], scriptCodeExample);
  assert.equal(editor.input("Lua 源码").value, code);
  editor.reject(false); await editor.button("保存并导入").onClick();
  assert.equal(editor.closed(), true);
});

test("read document shows full metadata and replacement is bound to the original ID", async () => {
  const rule = JSON.parse(scriptRuleExample);
  const document = { rule, api_version: 1, sha256: "a".repeat(64), code: "return {}" };
  const editor = mount(document);
  assert.match(editor.render(), new RegExp(document.sha256));
  assert.equal(editor.input("Lua 源码").value, document.code);
  editor.input("脚本规则 JSON").onChange({ target: { value: JSON.stringify({ ...rule, id: "different" }) } }); editor.render();
  await editor.button(`保存并替换 ${rule.id}`).onClick();
  assert.match(editor.render(), /替换时来源 ID 必须为/);
  assert.equal(editor.writes.length, 0);
  editor.input("脚本规则 JSON").onChange({ target: { value: scriptRuleExample } }); editor.render();
  await editor.button(`保存并替换 ${rule.id}`).onClick();
  assert.equal(editor.writes[0][3], true);
  assert.equal(editor.closed(), true);
});
