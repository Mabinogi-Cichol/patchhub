const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
class Element {
  constructor() { this.children = []; this.events = {}; this.value = ''; this.hidden = false; this.files = []; }
  addEventListener(name, cb) { this.events[name] = cb; }
  append(...items) { this.children.push(...items); }
  replaceChildren(...items) { this.children = items; }
  scrollIntoView() {}
  reportValidity() { return true; }
  click() { this.events.click?.({ target: this }); }
}
const elements = new Map();
const document = { getElementById(id) { if (!elements.has(id)) elements.set(id, new Element()); return elements.get(id); }, createElement() { return new Element(); }, createTextNode(text) { return { textContent: text }; } };
const calls = [];
let failImport = true;
let failRefresh = false;
const source = { id:'one', name:'示例', role:'publisher', origin:'builtin', enabled:true, status:'fresh', page_hosts:['example.com'], download_hosts:[], entry_count:1, interval_hours:24, script:{api_version:1,sha256:'a'} };
const fetch = async (path, options) => {
  calls.push({path, ...options});
  if (path === '/api/sources' && options.method === 'GET' && failRefresh) throw new Error('offline');
  if (path === '/api/updates') return {ok:true,json:async()=>({task_ids:['task-1']})};
  if (path === '/api/tasks/task-1') { failRefresh = true; return {ok:true,json:async()=>({task_id:'task-1',running:false,cancel_requested:true,progress:null,result:null,error:{code:'task_cancelled'},control_error:null})}; }
  if (path === '/api/sources' && options.method === 'GET') return {ok:true, json:async()=>({sources:[source],settings_revision:1})};
  if (path === '/api/sources' && options.method === 'POST' && failImport) return {ok:false,status:400,json:async()=>({code:'invalid_script',detail:'错误'})};
  return {ok:true,json:async()=>({})};
};
const script = fs.readFileSync('apps/remote-web/app.js','utf8').replace(/export \{\};?\s*$/, '');
vm.runInNewContext(script, {document, fetch, window:{setTimeout}, confirm:()=>true, console, Error, Map, encodeURIComponent});
const tick = () => new Promise(resolve => setTimeout(resolve, 0));
(async()=>{
  document.getElementById('key').value='012345';
  elements.get('login-form').events.submit({preventDefault(){}});
  await tick();
  assert.equal(calls[0].headers.Authorization,'Bearer 012345');
  assert.equal(document.getElementById('workspace').hidden,false);
  elements.get('import').click();
  elements.get('code').value='return {}';
  elements.get('save').click();
  await tick();
  assert.equal(elements.get('code').value,'return {}');
  assert.match(elements.get('notice').textContent,/invalid_script/);
  assert.equal(JSON.parse(calls[1].body).code,'return {}');
  failImport=false;
  elements.get('save').click();
  await tick(); await tick();
  assert.equal(elements.get('editor').hidden,true);
  assert.equal(calls.at(-1).path,'/api/sources');
  elements.get('update-all').click();
  await tick(); await tick();
  assert.match(elements.get('notice').textContent,/任务已结束，但刷新来源失败/);
  assert.equal(elements.get('tasks').children[0].children.length,1);
  assert.equal(elements.get('tasks').children[0].children[0].textContent,'更新已取消');
  console.log('remote web protocol interaction: passed');
})().catch(error=>{console.error(error);process.exitCode=1});
