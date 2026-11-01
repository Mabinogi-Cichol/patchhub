import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { formatSize } from '../src/cache.ts';

const require = createRequire(import.meta.url);
const controls: Record<string, any>[] = [];
const native = Object.fromEntries(['DialogButton', 'Focusable', 'ModalRoot', 'ProgressBar'].map(name => [name, (props: any) => {
  controls.push({ name, props });
  return React.createElement(name === 'DialogButton' ? 'button' : 'div', {}, props.children);
}]));
const output = ts.transpileModule(readFileSync(new URL('../src/app-update-view.tsx', import.meta.url), 'utf8'), {
  compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const exports: Record<string, any> = {};
runInNewContext(output, { exports, require: (id: string) => id === '@decky/ui' ? native : id === './cache' ? { formatSize } : require(id) });

function progress(execution: any) {
  controls.length = 0;
  const html = renderToStaticMarkup(React.createElement(exports.AppUpdateProgress, { execution }));
  return { html, bar: controls.find(c => c.name === 'ProgressBar')!.props };
}

test('known download size displays real bytes and percentage; unknown size never invents progress', () => {
  const known = progress({ state: 'preparing', stage: 'downloading', bytes: 1024, total: 4096, file: 'patchhub-decky.zip' });
  assert.match(known.html, /下载更新包.*25%/);
  assert.match(known.html, /1.0 KiB \/ 4.0 KiB/);
  assert.equal(known.bar.nProgress, 25);
  assert.equal(known.bar.indeterminate, false);
  const unknown = progress({ state: 'preparing', stage: 'downloading', bytes: 1024, total: 0 });
  assert.match(unknown.html, /总量未知/);
  assert.doesNotMatch(unknown.html, /%/);
  assert.equal(unknown.bar.indeterminate, true);
  assert.equal(progress({ state: 'preparing' }).bar.indeterminate, true);
});

test('validation, install, verification, recovery and cleanup remain indeterminate', () => {
  for (const [stage, label] of Object.entries({ validating: '校验更新包', backing_up: '备份当前插件', installing: '安装新版插件', verifying: '验证新版插件', recovering: '恢复安装前状态', cleanup: '清理更新材料' })) {
    const value = progress({ state: stage === 'validating' ? 'preparing' : 'applying', stage });
    assert.match(value.html, new RegExp(label));
    assert.equal(value.bar.indeterminate, true);
    assert.doesNotMatch(value.html, /%/);
  }
});

test('closing only hides progress, download cancellation is separate and unavailable during replacement', () => {
  let closed = 0, cancelled = 0;
  const render = (state: string, error?: string) => {
    controls.length = 0;
    return renderToStaticMarkup(React.createElement(exports.AppUpdateDialogView, {
      execution: { state, error: state === 'failed' ? 'fixture error' : undefined }, busy: false, error,
      onClose: () => { closed++; }, onCancelDownload: () => { cancelled++; },
    }));
  };
  render('preparing');
  controls.find(c => c.name === 'ModalRoot')!.props.onCancel();
  assert.equal(closed, 1); assert.equal(cancelled, 0);
  controls.find(c => c.name === 'DialogButton' && c.props.children === '取消下载')!.props.onClick();
  assert.equal(cancelled, 1);
  const applying = render('applying', 'RPC unavailable');
  assert.doesNotMatch(applying, /取消下载|更新未完成/);
  assert.match(applying, /等待插件重载或重新连接/);
  assert.match(render('failed'), /更新未完成：fixture error/);
  assert.match(render('completed'), /更新已完成/);
});
