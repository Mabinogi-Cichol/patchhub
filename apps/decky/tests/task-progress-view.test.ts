import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { TaskSnapshot } from "../src/task-controller.ts";

// Use the actual JSX with native-component stand-ins: no Steam runtime is needed.
const require = createRequire(import.meta.url);
const controls: Record<string, any>[] = [];
const native = {
  ModalRoot: (props: any) => { controls.push(props); return React.createElement("section", {}, props.children); },
  Focusable: (props: any) => React.createElement("div", { "data-close-hint": props.onCancelActionDescription }, props.children),
  ProgressBar: () => React.createElement("progress"),
};
const source = readFileSync(new URL("../src/task-progress-view.tsx", import.meta.url), "utf8");
const output = ts.transpileModule(source, { compilerOptions: {
  jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022,
} }).outputText;
const exports: Record<string, any> = {};
runInNewContext(output, { exports, require: (id: string) => {
  if (id === "@decky/ui") return native;
  if (id === "@steam-patch/ui") return { reason: (code: string) => `原因：${code}` };
  if (id === "./cache") return { formatSize: (bytes: number) => `${bytes} B` };
  if (id === "./task-progress") return { taskProgressPercent: () => 50 };
  return require(id);
} });
function task(state: TaskSnapshot["state"]): TaskSnapshot {
  return { task_id: "one", kind: "install", state, stage: "applying", substage: "writing", bytes_done: 5,
    total_bytes: 10, files_done: 1, total_files: 2, current_file: "data.bin", archive_bytes: 10,
    elapsed_seconds: 1, speed_bps: 0, eta_seconds: null, result: null, error: null };
}

test("progress has only the native close hint and all native exits use its handler", () => {
  controls.length = 0;
  let closed = 0;
  const html = renderToStaticMarkup(React.createElement(exports.TaskProgressView, {
    task: task("running"), game: "游戏", patch: "补丁", onClose: () => { closed++; },
  }));
  assert.match(html, /data-close-hint="关闭并取消安装"/);
  assert.match(html, /<progress/);
  assert.doesNotMatch(html, /<button|关闭进度窗口|安装仍会继续/);
  const modal = controls.at(-1)!;
  assert.equal(modal.bDisableBackgroundDismiss, true);
  assert.equal(modal.bHideCloseIcon, true);
  modal.onCancel(); modal.closeModal(); modal.onEscKeypress();
  assert.equal(closed, 3);
});

test("every terminal result contains result and native close hint without progress or action buttons", () => {
  for (const [state, message] of [["completed", "安装已完成"], ["cancelled", "已安全取消"], ["failed", "安装失败"]] as const) {
    const html = renderToStaticMarkup(React.createElement(exports.TaskResultView, { task: task(state), onClose() {} }));
    assert.match(html, new RegExp(message));
    assert.match(html, /data-close-hint="关闭"/);
    assert.doesNotMatch(html, /<button|<progress|已处理|文件：|已用|当前阶段|data.bin/);
  }
});

test("recovery failure and interrupted work never claim safe cancellation", () => {
  const value = task("failed");
  value.result = { transaction_id: "tx", instance_id: "game", release_id: "patch", state: "needs_recovery", error: "recovery_conflict" };
  const html = renderToStaticMarkup(React.createElement(exports.TaskResultView, { task: value, onClose() {} }));
  assert.match(html, /仍需检查并恢复原文件/);
  assert.match(html, /recovery_conflict/);
  assert.doesNotMatch(html, /已安全取消/);
  value.result = null; value.error = "task_interrupted";
  assert.match(renderToStaticMarkup(React.createElement(exports.TaskResultView, { task: value, onClose() {} })), /仍需检查并恢复原文件/);
});


test("all nine installer stages and applying substages render native progress", () => {
  const stages = {
    connecting: "正在连接", downloading: "下载补丁", validating: "检查已下载补丁",
    analyzing_installer: "分析安装包", extracting: "提取文件", planning: "检查安装条件",
    applying: "写入文件", rolling_back: "恢复原文件", cleanup: "清理临时文件",
  };
  for (const [stage, label] of Object.entries(stages)) {
    const html = renderToStaticMarkup(React.createElement(exports.TaskProgressView, {
      task: { ...task("running"), stage, substage: null }, game: "游戏", patch: "补丁", onClose() {},
    }));
    assert.match(html, new RegExp(label), stage);
  }
  for (const [substage, label] of [["backing_up", "保护现有文件"], ["writing", "写入文件"], ["committing", "提交安装"]]) {
    const html = renderToStaticMarkup(React.createElement(exports.TaskProgressView, {
      task: { ...task("running"), substage }, game: "游戏", patch: "补丁", onClose() {},
    }));
    assert.match(html, new RegExp(label), substage);
  }
});

test("recovery metadata overrides a cancelled-looking host state", () => {
  const value = task("cancelled");
  value.result = { transaction_id: "tx", instance_id: "game", release_id: "patch", state: "needs_recovery",
    recovery: { required: true, code: "recovery_conflict", detail: "game file changed" } };
  const html = renderToStaticMarkup(React.createElement(exports.TaskResultView, { task: value, onClose() {} }));
  assert.match(html, /仍需检查并恢复原文件/);
  assert.match(html, /game file changed/);
  assert.doesNotMatch(html, /已安全取消/);
});
