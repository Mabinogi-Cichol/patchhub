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
const clickEvent = { currentTarget: { ownerDocument: { defaultView: { performance: { timeOrigin: 456 } } } } };
const tick = () => new Promise(resolve => setImmediate(resolve));

function mount() {
  const values: any[] = [], controls: any[] = [], timers = new Map(), cleanups: any[] = [], requests: any[] = [], modals: any[] = [], toasts: any[] = [], navigations: string[] = [];
  let hook = 0, first = true, closed = 0;
  const native: Record<string, any> = {};
  for (const name of ['DialogButton', 'Field', 'PanelSection', 'PanelSectionRow']) native[name] = (props: any) => {
    controls.push({ name, props });
    if (name === 'DialogButton') props.ref?.(clickEvent.currentTarget);
    return React.createElement('div', {}, props.children);
  };
  native.showModal = (element: any) => { modals.push(element); return { Close() { closed++; } }; };
  const exports: Record<string, any> = {};
  const modules: Record<string, any> = {
    '@decky/api': { callable: (method: string) => (...args: any[]) => new Promise((resolve, reject) => requests.push({ method, args, resolve, reject })), toaster: { toast(value: any) { toasts.push(value); } } },
    '@decky/ui': native, './route': { settingsRoutes: { cache: '/settings' } },
    './app-update-view': { updateExecuting: (e: any) => ['preparing', 'applying'].includes(e?.state),
      updateProgressLabel: (e: any) => `${e.percent ?? 0}% · 准备更新` },
    react: { ...React,
      useState(initial: any) { const i = hook++; if (!(i in values)) values[i] = initial; return [values[i], (v: any) => { values[i] = v; }]; },
      useRef(initial: any) { const i = hook++; if (!(i in values)) values[i] = { current: initial }; return values[i]; },
      useEffect(effect: any) { if (first) cleanups.push(effect()); },
    },
  };
  native.Navigation = { Navigate(path: string) { navigations.push(path); }, CloseSideMenus() {} };
  const schedule = (fn: any) => { const id = {}; timers.set(id, fn); return id; };
  runInNewContext(output, { exports, require: (id: string) => modules[id] ?? require(id), console, performance: { timeOrigin: 123 },
    setTimeout: schedule, clearTimeout: (id: any) => timers.delete(id), setInterval: schedule, clearInterval: (id: any) => timers.delete(id) });
  const render = () => { hook = 0; controls.length = 0; const html = renderToStaticMarkup(React.createElement(exports.AppUpdatePanel)); first = false; return html; };
  const button = (label: string) => controls.find(c => c.name === 'DialogButton' && c.props.children?.props?.children === label)!.props;
  return { render, button, requests, timers, modals, toasts, navigations, exports, closed: () => closed, cleanup: () => cleanups.forEach(fn => fn?.()) };
}

test('update replaces the same single control with progress, without a modal or overlapping polls', async () => {
  const ui = mount(); ui.render();
  assert.equal(ui.requests.length, 1); assert.equal(ui.timers.size, 0);
  assert.equal(ui.button('检查更新').style.width, '100%');
  assert.equal(ui.button('检查更新').style.minWidth, 0);
  ui.requests.shift().resolve({ state: 'available', current: '0.4.0', tag: 'v0.5.0' }); await tick();
  const html = ui.render(); assert.doesNotMatch(html, /发布时间|稍后|跳过|查看更新进度/);
  ui.button('更新至 v0.5.0').onClick(clickEvent);
  ui.button('更新至 v0.5.0').onClick(clickEvent);
  assert.equal(ui.requests.length, 1);
  assert.match(ui.render(), /0%/);
  const start = ui.requests.shift(); assert.equal(start.method, 'start_app_update'); assert.deepEqual(start.args, ['v0.5.0', 456]);
  start.resolve({ state: 'available', execution: { state: 'preparing', percent: 32 } }); await tick();
  assert.match(ui.render(), /32%/); assert.equal(ui.modals.length, 0);
  const [id, timer] = [...ui.timers.entries()][0]; ui.timers.delete(id); void timer();
  assert.equal(ui.requests.length, 1); assert.equal(ui.timers.size, 0);
  ui.requests.shift().reject(new Error('reload')); await tick();
  assert.match(ui.render(), /32%/); assert.doesNotMatch(ui.render(), /更新未完成/);
  ui.cleanup(); assert.equal(ui.timers.size, 0);
});

test('plugin mount does not reopen update progress', async () => {
  const ui = mount(); const stop = ui.exports.startAppUpdateNotifications();
  assert.equal(ui.requests.length, 1);
  assert.equal(ui.requests.shift().method, 'check_app_update');
  stop();
  assert.equal(ui.modals.length, 0);
});

test('startup awaits a fresh check then posts a native clickable update tip', async () => {
  const ui = mount(); const stop = ui.exports.startAppUpdateNotifications();
  const check = ui.requests.shift(); assert.equal(check.method, 'check_app_update');
  assert.equal(ui.requests.length, 0); assert.equal(ui.timers.size, 0);
  check.resolve({ state: 'available', tag: 'v0.6.0' }); await tick();
  const claim = ui.requests.shift(); assert.equal(claim.method, 'claim_app_update_notice');
  claim.resolve({ state: 'available', tag: 'v0.6.0' }); await tick();
  assert.equal(ui.toasts.length, 1); assert.equal(ui.toasts[0].title, '发现新版 v0.6.0');
  ui.toasts[0].onClick(clickEvent); assert.deepEqual(ui.navigations, ['/settings']);
  assert.equal(ui.modals.length, 0); assert.equal(ui.timers.size, 1);
  stop(); assert.equal(ui.timers.size, 0);
});

test('dismount during startup check does not claim a notification or schedule work', async () => {
  const ui = mount(); const stop = ui.exports.startAppUpdateNotifications();
  stop(); ui.requests.shift().resolve({ state: 'available' }); await tick();
  assert.equal(ui.requests.length, 0); assert.equal(ui.toasts.length, 0); assert.equal(ui.timers.size, 0);
});

test('no new notice stays quiet after the startup check', async () => {
  const ui = mount(); const stop = ui.exports.startAppUpdateNotifications();
  ui.requests.shift().resolve({ state: 'up_to_date' }); await tick();
  ui.requests.shift().resolve(null); await tick();
  assert.equal(ui.toasts.length, 0); stop();
});

test('a completed request from an old Steam context does not keep the new client reloading', async () => {
  const ui = mount(); ui.render();
  ui.requests.shift().resolve({ state: 'idle', current: '0.5.0', execution: {
    state: 'completed', restart: 'requested', restart_context: 122,
  } }); await tick(); assert.doesNotMatch(ui.render(), /正在重新加载/);
  assert.ok(ui.button('检查更新')); ui.cleanup();
});

test('reload failure retries only the UI reload', async () => {
  const ui = mount(); ui.render();
  ui.requests.shift().resolve({ state: 'idle', execution: {
    state: 'completed', restart: 'failed', restart_error: 'fixture',
  } }); await tick(); ui.render(); ui.button('重新加载界面').onClick(clickEvent);
  assert.equal(ui.requests[0].method, 'restart_app_update_client');
  assert.deepEqual(ui.requests[0].args, [456]); ui.cleanup();
});


test('reload indicator uses the rendering page context rather than SharedJSContext', async () => {
  const ui = mount(); ui.render();
  ui.requests.shift().resolve({ state: 'idle', execution: {
    state: 'completed', restart: 'requested', restart_context: 456,
  } }); await tick();
  assert.match(ui.render(), /正在重新加载界面/);
  const [id, timer] = [...ui.timers.entries()][0]; ui.timers.delete(id); void timer();
  ui.requests.shift().resolve({ state: 'idle', execution: {
    state: 'completed', restart: 'requested', restart_context: 123,
  } }); await tick();
  assert.doesNotMatch(ui.render(), /正在重新加载/);
  ui.cleanup();
});
