// Execute the shipped bridge in a fake Steam context, never a real browser.
import { readFileSync } from 'node:fs';
import { webcrypto } from 'node:crypto';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';

const source = readFileSync(new URL('./install-decky.py', import.meta.url), 'utf8');
const bridge = source.match(/BRIDGE = r"""([\s\S]*?)"""/)[1];
const name = 'Steam 黄油补丁助手';
const tick = () => new Promise(resolve => setImmediate(resolve));

function environment() {
  const seen = [], forwarded = [], timers = new Set();
  const settings = {pluginOrder:['alpha',name,'beta'], frozenPlugins:[name], hiddenPlugins:[], disabled_plugins:[]};
  const backend = {
    runningCalls:new Map(), eventListeners:new Map(), ws:{readyState:1}, sequence:0,
    addEventListener(event, listener) {
      if (!this.eventListeners.has(event)) this.eventListeners.set(event,new Set());
      this.eventListeners.get(event).add(listener);
    },
    removeEventListener(event, listener) { this.eventListeners.get(event)?.delete(listener); },
    emit(event,...args) { for (const listener of this.eventListeners.get(event)||[]) listener(...args); },
    call(route,...args) {
      seen.push({route,args});
      const id=++this.sequence;
      this.runningCalls.set(id,{message:{route,args}});
      return Promise.resolve().then(()=>this.dispatch(route,...args)).then(result=>{
        this.runningCalls.delete(id); return result;
      },error=>{this.runningCalls.delete(id);throw error;});
    },
    async dispatch(route,...args) {
      switch (route) {
        case 'updater/get_version_info': return {current:'v3.2.9'};
        case 'loader/get_plugins': return [{name,version:'0.2.0',disabled:false}];
        case 'utilities/get_user_info': return {path:'/home/deck'};
        case 'utilities/settings/get': return [...settings[args[0]]];
        case 'utilities/settings/set': settings[args[0]]=[...args[1]]; return;
        case 'loader/call_plugin_method': return args[1]==='list_tasks'?[]:{protocol_version:5,methods:['scan','get_capabilities']};
        case 'utilities/install_plugin':
          this.emit('loader/add_plugin_install_prompt',name,args[2],'request-actual',args[3],args[4]); return;
        case 'utilities/confirm_plugin_install': return;
        case 'utilities/cancel_plugin_install': return;
        case 'utilities/uninstall_plugin': return;
        default: throw new Error(route);
      }
    },
  };
  const handler = function addPluginInstallPrompt(...args) { forwarded.push(args); }.bind(null);
  backend.addEventListener('loader/add_plugin_install_prompt',handler);
  const loader = {plugins:[{name,version:'0.2.0',titleView:{}}]};
  const window = {DeckyBackend:backend,DeckyPluginLoader:loader};
  const run = vm.runInNewContext(`(${bridge})`, {
    window,Map,Set,Promise,crypto:webcrypto,
    fetch:async()=>({ok:true,arrayBuffer:async()=>new Uint8Array([1,2,3]).buffer}),
    setTimeout(fn) {timers.add(fn);return fn;},clearTimeout(fn){timers.delete(fn);},
  });
  const install = {id:'job-1',kind:'install',version:'0.2.0',hash:'a'.repeat(64),artifact:'file:///tmp/test.zip',installType:1,checkTasks:true};
  return {run,backend,loader,window,seen,forwarded,settings,handler,timers,install};
}

async function finished(e) {
  for(let i=0;i<20;i++) {
    await tick();
    const result=await e.run('status',{id:e.install.id});
    if(result.done) return result;
  }
  throw new Error('bridge did not settle');
}

test('confirm exact event request ID; suppress own modal and restore original handler',async()=>{
  const e=environment();
  await e.run('launch',e.install);
  assert.equal((await finished(e)).ok,true);
  assert.deepEqual(e.seen.find(x=>x.route==='utilities/confirm_plugin_install').args,['request-actual']);
  assert.equal(e.forwarded.length,0);
  assert.deepEqual([...e.backend.eventListeners.get('loader/add_plugin_install_prompt')],[e.handler]);
  assert.equal(e.timers.size,0);
});

test('foreign hash/version prompts are forwarded and never confirmed',async()=>{
  const e=environment(),dispatch=e.backend.dispatch.bind(e.backend);
  e.backend.dispatch=async function(route,...args) {
    if(route==='utilities/install_plugin') {
      this.emit('loader/add_plugin_install_prompt',name,'0.2.0','wrong-hash','b'.repeat(64),1);
      this.emit('loader/add_plugin_install_prompt',name,'0.1.0','wrong-version','a'.repeat(64),1);
      return;
    }
    return dispatch(route,...args);
  };
  await e.run('launch',e.install);
  await tick();
  assert.equal(e.forwarded.length,2);
  assert.equal((await e.run('status',{id:'job-1'})).done,false);
  for(const timer of e.timers) timer();
  assert.equal((await finished(e)).ok,false);
  assert.equal(e.seen.filter(x=>x.route==='utilities/confirm_plugin_install').length,0);
  assert.deepEqual([...e.backend.eventListeners.get('loader/add_plugin_install_prompt')],[e.handler]);
});

test('tasks starting after preflight cancel pending request before confirmation',async()=>{
  const e=environment(),dispatch=e.backend.dispatch.bind(e.backend);
  e.backend.dispatch=async(route,...args)=>route==='loader/call_plugin_method'?[{state:'running'}]:dispatch(route,...args);
  await e.run('launch',e.install);
  const result=await finished(e);
  assert.equal(result.ok,false);
  assert.equal(result.error,'active_or_unknown_tasks');
  assert.equal(e.seen.filter(x=>x.route==='utilities/confirm_plugin_install').length,0);
  assert.deepEqual(e.seen.find(x=>x.route==='utilities/cancel_plugin_install').args,['request-actual']);
});

test('live confirm promise stays busy after caller disconnect; cannot forget or launch rollback',async()=>{
  const e=environment(),dispatch=e.backend.dispatch.bind(e.backend);
  let resolve;
  e.backend.dispatch=async(route,...args)=>route==='utilities/confirm_plugin_install'?new Promise(r=>{resolve=r;}):dispatch(route,...args);
  await e.run('launch',e.install);
  await tick();
  const status=await e.run('status',{id:'job-1'});
  assert.equal(status.busy,true);
  assert.equal(status.done,false);
  await assert.rejects(e.run('launch',{...e.install,id:'rollback'}),/busy/);
  await assert.rejects(e.run('forget',{id:'job-1'}),/busy/);
  await assert.rejects(e.backend.call('loader/call_plugin_method',name,'start_install'),/update_in_progress/);
  assert.equal((await e.backend.call('loader/call_plugin_method','other','start_install')).protocol_version,5);
  resolve();
  assert.equal((await finished(e)).ok,true);
});

test('restore only own settings membership and neighbour position; retain added plugins',async()=>{
  const e=environment();
  e.settings.pluginOrder=['new','alpha','beta'];
  e.settings.hiddenPlugins=['another'];
  e.settings.frozenPlugins=['new'];
  await e.run('launch',{id:'job-1',kind:'settings',settings:{pluginOrder:['alpha',name,'beta'],hiddenPlugins:[name],frozenPlugins:[],disabled_plugins:[]}});
  assert.equal((await finished(e)).ok,true);
  assert.deepEqual(e.settings.pluginOrder,['new','alpha',name,'beta']);
  assert.deepEqual(e.settings.hiddenPlugins,['another',name]);
  assert.deepEqual(e.settings.frozenPlugins,['new']);
});

test('reject unsupported version and disconnected or foreign busy Loader',async()=>{
  const e=environment(),dispatch=e.backend.dispatch.bind(e.backend);
  e.backend.dispatch=async(route,...args)=>route==='updater/get_version_info'?{current:'v4.0.0'}:dispatch(route,...args);
  await assert.rejects(e.run('probe',{}),/unsupported_loader_version/);
  e.backend.ws.readyState=3;
  await assert.rejects(e.run('probe',{}),/unsupported_loader_adapter/);
  e.backend.ws.readyState=1;
  e.backend.runningCalls.set(100,{message:{route:'utilities/install_plugins'}});
  await assert.rejects(e.run('launch',e.install),/busy/);
});

test('runtime requires backend/frontend versions, real RPC and HTTP bundle hash',async()=>{
  const e=environment();
  const hash=Buffer.from(await webcrypto.subtle.digest('SHA-256',new Uint8Array([1,2,3]))).toString('hex');
  assert.equal(await e.run('runtime',{present:true,version:'0.2.0',disabled:false,bundle:hash}),true);
  await assert.rejects(e.run('runtime',{present:true,version:'0.2.0',disabled:false,bundle:'0'.repeat(64)}),/http_hash/);
  e.loader.plugins[0].titleView=undefined;
  await assert.rejects(e.run('runtime',{present:true,version:'0.2.0',disabled:false,bundle:hash}),/frontend_version_or_instance/);
});

test('transaction guard survives install completion until verification or recovery releases it',async()=>{
  const e=environment(),previous=e.backend.call;
  await e.run('acquire',{id:'transaction'});
  await e.run('launch',e.install);
  assert.equal((await finished(e)).ok,true);
  assert.equal(await e.run('guard',{id:'transaction'}),true);
  await assert.rejects(e.backend.call('loader/call_plugin_method',name,'start_install'),/update_in_progress/);
  await e.run('forget',{id:'job-1'});
  await e.run('release',{id:'transaction'});
  assert.equal(e.backend.call,previous);
  assert.equal(e.window.__steamPatchTerminalGuard,undefined);
  assert.equal(e.window.__steamPatchTerminalJobs,undefined);
});

test('cancellation RPC failure still restores the original prompt handler and call function',async()=>{
  const e=environment(),previous=e.backend.call,dispatch=e.backend.dispatch.bind(e.backend);
  e.backend.dispatch=async(route,...args)=>{
    if (route==='loader/call_plugin_method') return [{state:'running'}];
    if (route==='utilities/cancel_plugin_install') throw new Error('cancel_failed');
    return dispatch(route,...args);
  };
  await e.run('launch',e.install);
  assert.equal((await finished(e)).ok,false);
  assert.equal(e.backend.call,previous);
  assert.deepEqual([...e.backend.eventListeners.get('loader/add_plugin_install_prompt')],[e.handler]);
});

const legacyName = 'Steam 补丁管理';

test('legacy plugin is discovered and its tasks are read using its installed name', async () => {
  const e=environment(), dispatch=e.backend.dispatch.bind(e.backend);
  e.loader.plugins[0].name=legacyName;
  e.backend.dispatch=async(route,...args)=>route==='loader/get_plugins'?[{name:legacyName,version:'0.2.0',disabled:false}]:dispatch(route,...args);
  const probe=await e.run('probe',{});
  assert.equal(probe.plugins[0].name,legacyName);
  assert.equal(probe.instances.length,1);
  await e.run('tasks',{});
  assert.equal(e.seen.find(x=>x.route==='loader/call_plugin_method').args[0],legacyName);
});

test('renamed install unloads old name before confirmation and checks its tasks', async () => {
  const e=environment(), dispatch=e.backend.dispatch.bind(e.backend);
  e.backend.dispatch=async(route,...args)=>route==='loader/get_plugins'?[{name:legacyName,version:'0.2.0',disabled:false}]:dispatch(route,...args);
  e.loader.plugins[0].name=legacyName;
  await e.run('launch',{...e.install,name});
  assert.equal((await finished(e)).ok,true);
  assert.equal(e.seen.find(x=>x.route==='utilities/install_plugin').args[1],name);
  assert.equal(e.seen.find(x=>x.route==='loader/call_plugin_method').args[0],legacyName);
  assert.equal(e.seen.find(x=>x.route==='utilities/uninstall_plugin').args[0],legacyName);
  assert.ok(e.seen.findIndex(x=>x.route==='utilities/uninstall_plugin') < e.seen.findIndex(x=>x.route==='utilities/confirm_plugin_install'));
});

test('settings replace old name, preserve neighbour order and can restore old identity', async () => {
  const e=environment();
  e.settings.pluginOrder=['alpha',legacyName,'beta'];
  e.settings.frozenPlugins=[legacyName];
  const target={...e.settings,pluginOrder:['alpha',name,'beta'],frozenPlugins:[name]};
  await e.run('launch',{id:e.install.id,kind:'settings',name,settings:target});
  assert.equal((await finished(e)).ok,true);
  assert.deepEqual(e.settings.pluginOrder,['alpha',name,'beta']);
  assert.deepEqual(e.settings.frozenPlugins,[name]);
  await e.run('forget',{id:e.install.id});
  const original={...e.settings,pluginOrder:['alpha',legacyName,'beta'],frozenPlugins:[legacyName]};
  await e.run('launch',{id:e.install.id,kind:'settings',name:legacyName,settings:original});
  assert.equal((await finished(e)).ok,true);
  assert.deepEqual(e.settings.pluginOrder,['alpha',legacyName,'beta']);
  assert.deepEqual(e.settings.frozenPlugins,[legacyName]);
});

test('runtime absence and duplicate checks cover both identities', async () => {
  const e=environment(), dispatch=e.backend.dispatch.bind(e.backend);
  e.loader.plugins[0].name=legacyName;
  e.backend.dispatch=async(route,...args)=>route==='loader/get_plugins'?[{name:legacyName,version:'0.2.0',disabled:false}]:dispatch(route,...args);
  await assert.rejects(e.run('runtime',{present:false,name}),/plugin_still_present/);
  e.backend.dispatch=async(route,...args)=>route==='loader/get_plugins'?[{name},{name:legacyName}]:dispatch(route,...args);
  await assert.rejects(e.run('probe',{}),/duplicate_plugin/);
});

test('rollback unloads renamed identity and confirms original metadata name', async () => {
  const e=environment(), dispatch=e.backend.dispatch.bind(e.backend);
  e.backend.dispatch=async(route,...args)=>{
    if(route==='utilities/install_plugin') {
      e.backend.emit('loader/add_plugin_install_prompt',args[1],args[2],'rollback-request',args[3],args[4]);
      return;
    }
    return dispatch(route,...args);
  };
  await e.run('launch',{...e.install,name:legacyName,checkTasks:false});
  assert.equal((await finished(e)).ok,true);
  assert.equal(e.seen.find(x=>x.route==='utilities/uninstall_plugin').args[0],name);
  assert.equal(e.seen.find(x=>x.route==='utilities/install_plugin').args[1],legacyName);
});
