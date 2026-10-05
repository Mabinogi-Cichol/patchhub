#!/usr/bin/env node
// Connect to Steam CEF through a caller-managed loopback SSH tunnel.
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { dirname } from 'node:path';

const usage = `Usage: node cef.mjs [--endpoint http://127.0.0.1:18080] [--list]
  --target TITLE --expression-file FILE [--output JSON]
  --target TITLE --screenshot PNG [--output JSON]
  --timeout-ms N    Per-operation timeout (default 25000, maximum 60000)
No mutation is performed automatically. Review any supplied JS before use.`;

function parse(argv) {
  const options = { endpoint: 'http://127.0.0.1:18080', timeout: 25000 };
  const fields = new Map([
    ['--endpoint', 'endpoint'], ['--target', 'target'],
    ['--expression-file', 'expression'], ['--screenshot', 'screenshot'],
    ['--output', 'output'], ['--timeout-ms', 'timeout'],
  ]);
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === '--help' || arg === '-h') options.help = true;
    else if (arg === '--list') options.list = true;
    else if (fields.has(arg)) {
      const value = argv[++i];
      if (!value || value.startsWith('--')) throw new Error(`Missing value for ${arg}`);
      options[fields.get(arg)] = value;
    } else throw new Error(`Unknown option: ${arg}`);
  }
  if (options.help) return options;
  options.timeout = Number(options.timeout);
  if (!Number.isInteger(options.timeout) || options.timeout < 1 || options.timeout > 60000)
    throw new Error('timeout-ms must be an integer from 1 to 60000');
  const endpoint = new URL(options.endpoint);
  if (!['http:', 'https:'].includes(endpoint.protocol) ||
      !['127.0.0.1', 'localhost', '[::1]'].includes(endpoint.hostname) ||
      endpoint.username || endpoint.password || endpoint.pathname !== '/' || endpoint.search || endpoint.hash)
    throw new Error('endpoint must be a loopback HTTP origin without credentials');
  options.endpoint = endpoint;
  const action = options.expression || options.screenshot;
  if (options.list && action) throw new Error('--list cannot be combined with evaluation or screenshot');
  if (action && !options.target) throw new Error('--target is required for evaluation or screenshot');
  if (!action) options.list = true;
  return options;
}

async function save(path, data) {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, data);
}

async function openCDP(url, timeout) {
  const socket = new WebSocket(url);
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => { socket.close(); reject(new Error('CEF connection timeout')); }, timeout);
    socket.onopen = () => { clearTimeout(timer); resolve(); };
    socket.onerror = () => { clearTimeout(timer); reject(new Error('CEF connection failed')); };
    socket.onclose = () => { clearTimeout(timer); reject(new Error('CEF connection closed before opening')); };
  });
  let sequence = 0;
  const pending = new Map();
  const failAll = error => {
    for (const item of pending.values()) { clearTimeout(item.timer); item.reject(error); }
    pending.clear();
  };
  socket.onclose = () => failAll(new Error('CEF connection closed'));
  socket.onerror = () => failAll(new Error('CEF WebSocket error'));
  socket.onmessage = event => {
    let message;
    try { message = JSON.parse(event.data); }
    catch { failAll(new Error('Invalid CDP JSON response')); return; }
    const item = pending.get(message.id);
    if (!item) return;
    pending.delete(message.id);
    clearTimeout(item.timer);
    if (message.error) item.reject(new Error(`CDP: ${message.error.message ?? JSON.stringify(message.error)}`));
    else item.resolve(message.result);
  };
  return {
    call(method, params = {}) {
      return new Promise((resolve, reject) => {
        const id = ++sequence;
        const timer = setTimeout(() => {
          pending.delete(id);
          reject(new Error(`CDP timeout: ${method}`));
        }, timeout);
        pending.set(id, { resolve, reject, timer });
        try { socket.send(JSON.stringify({ id, method, params })); }
        catch (error) { clearTimeout(timer); pending.delete(id); reject(error); }
      });
    },
    close() { failAll(new Error('CEF client closed')); socket.close(); },
  };
}

async function main() {
  const options = parse(process.argv.slice(2));
  if (options.help) { console.log(usage); return; }
  const response = await fetch(new URL('/json/list', options.endpoint), {
    signal: AbortSignal.timeout(options.timeout),
  });
  if (!response.ok) throw new Error(`CEF target list HTTP ${response.status}`);
  const tabs = await response.json();
  if (!Array.isArray(tabs)) throw new Error('CEF target list is not an array');
  let result;
  if (options.list) {
    result = tabs.map(({ id, title, type, url }) => ({ id, title, type, url }));
  } else {
    const matches = tabs.filter(tab => tab.title === options.target);
    if (matches.length !== 1) throw new Error(`Expected one target titled ${options.target}; found ${matches.length}`);
    const tab = matches[0];
    const socketURL = new URL(tab.webSocketDebuggerUrl);
    if (!['ws:', 'wss:'].includes(socketURL.protocol) || socketURL.username || socketURL.password)
      throw new Error('Invalid CEF WebSocket URL');
    // Device advertises its own loopback port; reuse the local forwarded origin.
    socketURL.host = options.endpoint.host;
    socketURL.protocol = options.endpoint.protocol === 'https:' ? 'wss:' : 'ws:';
    const cdp = await openCDP(socketURL, options.timeout);
    result = { target: { id: tab.id, title: tab.title, url: tab.url } };
    try {
      if (options.expression) {
        const expression = await readFile(options.expression, 'utf8');
        const evaluation = await cdp.call('Runtime.evaluate', {
          expression, returnByValue: true, awaitPromise: true,
        });
        if (evaluation.exceptionDetails)
          throw new Error(`JavaScript exception: ${evaluation.exceptionDetails.exception?.description ?? evaluation.exceptionDetails.text}`);
        result.evaluation = evaluation;
      }
      if (options.screenshot) {
        const screenshot = await cdp.call('Page.captureScreenshot', { format: 'png', fromSurface: true });
        const bytes = Buffer.from(screenshot.data ?? '', 'base64');
        if (bytes.length < 8 || bytes.subarray(0, 8).toString('hex') !== '89504e470d0a1a0a')
          throw new Error('CDP screenshot is not PNG data');
        await save(options.screenshot, bytes);
        result.screenshot = { path: options.screenshot, bytes: bytes.length };
      }
    } finally { cdp.close(); }
  }
  const json = JSON.stringify(result, null, 2) + '\n';
  if (options.output) await save(options.output, json);
  console.log(json.trimEnd());
}

main().catch(error => { console.error(error.message); process.exitCode = 1; });
