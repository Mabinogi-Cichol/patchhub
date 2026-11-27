const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
class Element {
  constructor() { this.children = []; this.events = {}; this.dataset = {}; this.value = ''; this.hidden = false; this.files = []; }
  addEventListener(name, cb) { this.events[name] = cb; }
  append(...items) { this.children.push(...items); }
  replaceChildren(...items) { this.children = items; }
  showModal() { this.open = true; }
  close() { this.open = false; }
  setAttribute() {}
  focus() {}
  select() { this.selected = true; }
  reportValidity() { return true; }
  click() { this.events.click?.({ target: this }); }
}
const elements = new Map();
const nodes = function* (element) {
  yield element;
  for (const child of element.children ?? []) yield* nodes(child);
};
let copied;
const document = {
  getElementById(id) { if (!elements.has(id)) elements.set(id, new Element()); return elements.get(id); },
  querySelector(selector) {
    return [...nodes(elements.get('sources'))].find(node => node.dataset?.sourceTask &&
      selector === `[data-source-task="${node.dataset.sourceTask}"]`);
  },
  createElement() { return new Element(); },
  createTextNode(text) { return { textContent: text }; },
  execCommand(command) {
    assert.equal(command, 'copy');
    const prompt = elements.get('prompt');
    assert.equal(prompt.selected, true);
    copied = prompt.value;
    return true;
  }
};
const calls = [];
let failImport = true;
let failFirstUpdate = false;
let failRefresh = false;
const source = { id:'one', name:'示例', role:'publisher', origin:'builtin', enabled:true, status:'fresh', page_hosts:['example.com'], download_hosts:[], entry_count:1, interval_hours:24, script:{api_version:1,sha256:'a'} };
const imported = { ...source, id:'two', name:'自定义来源', origin:'added' };
const failed = { ...imported, id:'three' };
let sources = [source];
const prompt = '完整 AI 提示词\n第二行';
const fetch = async (path, options) => {
  calls.push({path, ...options});
  if (path === '/api/sources' && options.method === 'GET' && failRefresh) throw new Error('offline');
  if (path === '/api/script-prompt') return {ok:true,json:async()=>({prompt})};
  if (path === '/api/updates') return {ok:true,json:async()=>({tasks:[{source_id:'one',task_id:'task-1'}]})};
  if (path === '/api/tasks/task-1') { failRefresh = true; return {ok:true,json:async()=>({task_id:'task-1',running:false,cancel_requested:true,progress:null,result:null,error:{code:'task_cancelled'},control_error:null})}; }
  if (path === '/api/tasks/task-2') return {ok:true,json:async()=>({task_id:'task-2',running:false,error:null,control_error:null})};
  if (path === '/api/sources' && options.method === 'GET') return {ok:true, json:async()=>({sources,settings_revision:1})};
  if (path === '/api/sources' && options.method === 'POST') {
    if (failImport) return {ok:false,status:400,json:async()=>({code:'invalid_script',detail:'错误'})};
    if (failFirstUpdate) {
      sources.push(failed);
      return {ok:true,json:async()=>({rule:{id:failed.id},task_error:{code:'busy'}})};
    }
    sources = [source, imported];
    return {ok:true,json:async()=>({rule:{id:imported.id},task_id:'task-2'})};
  }
  return {ok:true,json:async()=>({})};
};
const script = fs.readFileSync('apps/remote-web/app.js','utf8').replace(/export \{\};?\s*$/, '');
vm.runInNewContext(script, {document, fetch, window:{setTimeout}, navigator:{}, CSS:{escape:s=>s}, confirm:()=>true, console, Error, Map, encodeURIComponent});
const tick = () => new Promise(resolve => setTimeout(resolve, 0));
const slot = id => document.querySelector(`[data-source-task="${id}"]`);
(async()=>{
  document.getElementById('key').value='012345';
  elements.get('login-form').events.submit({preventDefault(){}});
  await tick();
  assert.equal(calls[0].headers.Authorization,'Bearer 012345');
  assert.equal(document.getElementById('workspace').hidden,false);
  elements.get('import').click();
  assert.equal(elements.get('editor').open,true);
  await tick();
  elements.get('prompt-copy').click();
  assert.equal(copied,prompt);
  assert.match(elements.get('prompt-feedback').textContent,/已复制/);
  assert.equal(elements.get('prompt-box').open,false);
  elements.get('code').value='return {}';
  elements.get('save').click();
  await tick();
  assert.equal(elements.get('code').value,'return {}');
  assert.match(elements.get('editor-notice').textContent,/invalid_script/);
  assert.equal(JSON.parse(calls.find(call => call.method === 'POST').body).code,'return {}');
  assert.equal(elements.get('editor').open,true);
  failImport=false;
  elements.get('save').click();
  await tick(); await tick();
  assert.equal(elements.get('editor').open,false);
  assert.equal(calls.at(-1).path,'/api/sources');
  assert.equal(slot('two').children[0].children[0].textContent,'更新完成');
  assert.equal(slot('one').children.length,0);
  failFirstUpdate = true;
  elements.get('import').click();
  elements.get('save').click();
  await tick();
  assert.equal(slot('three').children[0].textContent,'首次更新未启动：busy');
  assert.equal(elements.get('notice').textContent,'脚本已保存');
  elements.get('update-all').click();
  await tick(); await tick();
  assert.match(elements.get('notice').textContent,/任务已结束，但刷新来源失败/);
  assert.equal(slot('one').children[0].children[0].textContent,'更新已取消');
  assert.equal(slot('two').children[0].children[0].textContent,'更新完成');
  console.log('remote web protocol interaction: passed');
})().catch(error=>{console.error(error);process.exitCode=1});
