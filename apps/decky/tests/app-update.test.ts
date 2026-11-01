import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

const require = createRequire(import.meta.url);
const output = ts.transpileModule(readFileSync(new URL('../src/app-update.tsx', import.meta.url), 'utf8'), {
  compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const tick = () => new Promise(resolve => setImmediate(resolve));

function mount() {
  const values: any[] = [], controls: any[] = [], timers = new Map(), cleanups: any[] = [], requests: any[] = [], modals: any[] = [];
  let hook = 0, first = true, closed = 0;
  const native: Record<string, any> = {};
  for (const name of ['ButtonItem', 'Field', 'PanelSection', 'PanelSectionRow']) native[name] = (props: any) => {
    controls.push({ name, props });
    return React.createElement('div', {}, props.children);
  };
  native.showModal = (element: any) => { modals.push(element); return { Close() { closed++; } }; };
  const exports: Record<string, any> = {};
  const modules: Record<string, any> = {
    '@decky/api': { callable: (method: string) => (...args: any[]) => new Promise((resolve, reject) => requests.push({ method, args, resolve, reject })), toaster: { toast() {} } },
    '@decky/ui': native, './route': { settingsRoutes: { cache: '/settings' } },
    './app-update-view': { updateExecuting: (e: any) => ['preparing', 'applying'].includes(e?.state),
      AppUpdateProgress: () => React.createElement('div', {}, '更新进度'), AppUpdateDialogView: () => null },
    react: { ...React,
      useState(initial: any) { const i = hook++; if (!(i in values)) values[i] = initial; return [values[i], (v: any) => { values[i] = v; }]; },
      useEffect(effect: any) { if (first) cleanups.push(effect()); },
    },
  };
  const schedule = (fn: any) => { const id = {}; timers.set(id, fn); return id; };
  runInNewContext(output, { exports, require: (id: string) => modules[id] ?? require(id), console,
    setTimeout: schedule, clearTimeout: (id: any) => timers.delete(id), setInterval: schedule, clearInterval: (id: any) => timers.delete(id) });
  const render = () => { hook = 0; controls.length = 0; const html = renderToStaticMarkup(React.createElement(exports.AppUpdatePanel)); first = false; return html; };
  const button = (label: string) => controls.find(c => c.name === 'ButtonItem' && c.props.children === label)!.props;
  return { render, button, requests, timers, modals, exports, closed: () => closed, cleanup: () => cleanups.forEach(fn => fn?.()) };
}

test('start opens progress for the accepted update, and status polls never overlap', async () => {
  const ui = mount(); ui.render();
  assert.equal(ui.requests.length, 1); assert.equal(ui.timers.size, 0);
  ui.requests.shift().resolve({ state: 'available', tag: 'v0.5.0' }); await tick(); ui.render();
  assert.equal(ui.timers.size, 1);
  ui.button('更新插件').onClick(); ui.render();
  assert.equal(ui.button('更新插件').disabled, true);
  const start = ui.requests.shift(); assert.equal(start.method, 'start_app_update'); assert.deepEqual(start.args, ['v0.5.0']);
  start.resolve({ state: 'available', execution: { state: 'preparing' } }); await tick(); ui.render();
  assert.equal(ui.modals.length, 1); assert.equal(ui.modals[0].props.initial.execution.state, 'preparing');
  ui.button('查看更新进度').onClick();
  assert.equal(ui.closed(), 1); assert.equal(ui.modals.length, 2);
  assert.equal(ui.requests.length, 0);
  const [id, timer] = [...ui.timers.entries()][0]; ui.timers.delete(id); void timer();
  assert.equal(ui.requests.length, 1); assert.equal(ui.timers.size, 0);
  ui.requests.shift().reject(new Error('reload')); await tick();
  assert.match(ui.render(), /正在等待插件重载或重新连接/);
  assert.doesNotMatch(ui.render(), /更新未完成/);
  ui.cleanup(); assert.equal(ui.timers.size, 0);
});

test('plugin reload resumes an active update dialog and dismount closes it', async () => {
  const ui = mount();
  const stop = ui.exports.startAppUpdateNotifications();
  ui.requests.find(r => r.method === 'get_app_update_status').resolve({ state: 'idle', execution: { state: 'applying', stage: 'verifying' } });
  ui.requests.find(r => r.method === 'claim_app_update_notice').resolve(null);
  await tick();
  assert.equal(ui.modals.length, 1); assert.equal(ui.modals[0].props.initial.execution.stage, 'verifying');
  stop(); assert.equal(ui.closed(), 1); assert.equal(ui.timers.size, 0);
});
