import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
const output = ts.transpileModule(readFileSync(new URL('../src/app-update-view.tsx', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const exports: Record<string, any> = {};
runInNewContext(output, { exports });
test('whole update progress uses the worker percentage and actual stage', () => {
  assert.equal(exports.updateProgressLabel({ state: 'preparing', stage: 'downloading', percent: 40 }), '40% · 下载中');
  assert.equal(exports.updateProgressLabel({ state: 'applying', stage: 'verifying', percent: 95 }), '95% · 验证中');
  assert.equal(exports.updateProgressLabel({ state: 'preparing', stage: 'cancelling', percent: 80 }), '80% · 正在取消任务并收尾');
  assert.equal(exports.updateProgressLabel({ state: 'preparing' }), '0% · 准备更新');
  assert.equal(exports.updateExecuting({ state: 'completed' }), false);
});
