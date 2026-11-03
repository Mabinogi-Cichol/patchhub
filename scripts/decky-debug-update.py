#!/usr/bin/env python3
"""One command: Devkit upload, loopback source, real Decky UI update and cleanup.

Inputs are two complete, version-consistent clean Decky ZIPs built from the
debug-capable code. No GitHub Release is created. Requires Node 22+, OpenSSH,
the running Devkit client, and an already trusted known_hosts file.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shlex
import socket
import subprocess
import sys
import time
import urllib.request
import uuid
import zipfile

ROOT = Path(__file__).resolve().parents[1]
CEF = ROOT / '.agents/skills/decky-deploy-verify/scripts/cef.mjs'


def get(url):
    with urllib.request.urlopen(url, timeout=10) as response:
        return json.load(response)


def package_version(path):
    with zipfile.ZipFile(path) as archive:
        return json.loads(archive.read('patchhub/package.json'))['version']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--current-package', type=Path, required=True)
    parser.add_argument('--target-package', type=Path, required=True)
    parser.add_argument('--known-hosts', type=Path, required=True)
    parser.add_argument('--devkit-port', type=int, default=32010)
    parser.add_argument('--cef-forward-port', type=int, default=18081)
    args = parser.parse_args()
    current, target = package_version(args.current_package), package_version(args.target_package)
    sys.path.insert(0, str(ROOT / 'apps/decky/backend'))
    from app_update import newer
    if not newer(target, current):
        raise ValueError('Target package must be newer than current package')
    device = get(f'http://127.0.0.1:{args.devkit_port}/selected_devkit')
    key = get(f'http://127.0.0.1:{args.devkit_port}/ssh_key_path')['key_path']
    destination = device['login'] + '@' + device['address']
    ssh_args = ['-i', key, '-o', 'BatchMode=yes', '-o', 'StrictHostKeyChecking=yes',
                '-o', 'UserKnownHostsFile=' + str(args.known_hosts.resolve())]
    session = 'debug-update-' + time.strftime('%Y%m%d-%H%M%S') + '-' + uuid.uuid4().hex[:8]
    output = ROOT / '.artifacts' / session
    output.mkdir(parents=True)
    # Resolve the actual paired user's home; never assume deck or a historical IP.
    def ssh(command, **kwargs):
        return subprocess.run(['ssh', *ssh_args, destination, command], check=True, **kwargs)
    home = ssh('printf %s "$HOME"', capture_output=True, text=True).stdout
    stage = home + '/patchhub-validation/' + session
    remote = 'python3 ' + shlex.quote(stage + '/decky_debug_update_device.py')
    service = tunnel = None
    boot = None
    cleaned = staged = False
    events = []
    def save(name, value):
        (output / name).write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding='utf-8')
    endpoint = f'http://127.0.0.1:{args.cef_forward_port}'
    def cef(title, expression=None, screenshot=None):
        command = ['node', str(CEF), '--endpoint', endpoint, '--target', title]
        if expression:
            (output / 'expression.js').write_text(expression, encoding='utf-8')
            command += ['--expression-file', str(output / 'expression.js')]
        if screenshot:
            command += ['--screenshot', str(output / screenshot)]
        result = subprocess.run(command, check=True, capture_output=True, text=True, encoding='utf-8')
        data = json.loads(result.stdout)
        return data.get('evaluation', {}).get('result', {}).get('value')
    try:
        ssh('mkdir -p ' + shlex.quote(stage))
        staged = True
        manifest = {kind + '.zip': hashlib.sha256(path.read_bytes()).hexdigest() for kind, path in [('current', args.current_package), ('target', args.target_package)]}
        (output / 'inputs.json').write_text(json.dumps(manifest))
        for source, name in [(output / 'inputs.json', 'inputs.json'), (args.current_package, 'current.zip'), (args.target_package, 'target.zip'),
                             (ROOT / 'scripts/install-decky.py', 'install-decky.py'),
                             (ROOT / 'scripts/decky_debug_update_device.py', 'decky_debug_update_device.py')]:
            subprocess.run(['scp', *ssh_args, str(source.resolve()), destination + ':' + stage + '/' + name], check=True)
        (output / 'session.json').write_text(json.dumps({'device': {k: device.get(k) for k in ('name', 'address', 'login')},
            'current': current, 'target': target, 'stage': stage}, indent=2))
        with (output / 'http.log').open('w') as log:
            service = subprocess.Popen(['ssh', *ssh_args, destination, remote + ' serve'],
                                       stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True)
            ready = json.loads(service.stdout.readline())
            save('server.json', ready)
            tunnel = subprocess.Popen(['ssh', *ssh_args, '-N', '-o', 'ExitOnForwardFailure=yes',
                                       '-L', f'127.0.0.1:{args.cef_forward_port}:127.0.0.1:8080', destination],
                                      stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL)
            # Wait for this local SSH listener, then check remote CEF once.
            while True:
                if tunnel.poll() is not None:
                    raise RuntimeError('CEF SSH tunnel exited')
                try:
                    with socket.create_connection(('127.0.0.1', args.cef_forward_port), timeout=1):
                        break
                except ConnectionRefusedError:
                    time.sleep(.2)
            get(endpoint + '/json/list')  # Fail on an actual remote connection error.
            boot = subprocess.Popen(['ssh', *ssh_args, destination, remote + ' bootstrap'],
                                    stdout=(output / 'bootstrap.log').open('w'), stderr=subprocess.STDOUT)
            notice = None
            while boot.poll() is None or notice is None:
                if boot.poll() not in (None, 0):
                    raise RuntimeError('Bootstrap failed; see bootstrap.log')
                tabs = get(endpoint + '/json/list')
                for tab in tabs:
                    if tab['title'] == 'SharedJSContext':
                        continue
                    probe = cef(tab['title'], "(()=>{const t=" + json.dumps('发现新版 v' + target) + ";const e=[...document.querySelectorAll('*')].find(e=>e.childElementCount===0&&e.textContent.includes(t));return e?{text:e.textContent,html:e.parentElement.outerHTML}:null})()")
                    if probe:
                        notice = {'title': tab['title'], **probe}
                        cef(tab['title'], screenshot='native-tips.png')
                        save('notice.json', notice)
                        break
                if notice and boot.poll() == 0:
                    break
                time.sleep(.2)
            # Click the actual native notice; it owns navigation to the update page.
            cef(notice['title'], "(()=>{const e=[...document.querySelectorAll('*')].find(e=>e.childElementCount===0&&e.textContent.includes(" + json.dumps('发现新版 v' + target) + "));if(!e)throw Error('Native notice disappeared');e.click();return true})()")
            main_title = next(t['title'] for t in get(endpoint + '/json/list') if 'browserType=4' in t.get('url', ''))
            while True:
                row = cef(main_title, "(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.innerText.includes(" + json.dumps('更新至 v' + target) + "));if(!b)return null;b.scrollIntoView({block:'center'});return {text:b.innerText,timeOrigin:performance.timeOrigin}})()")
                if row:
                    save('before-click.json', row)
                    cef(main_title, screenshot='update-ready.png')
                    break
                time.sleep(.2)
            # Exercise the real cancellable business path instead of waiting
            # for an idle device. This is the ordinary no-argument scan RPC.
            active = cef('SharedJSContext', "(()=>{const b=DeckyBackend;const calls=()=>[...b.runningCalls.values()].filter(r=>r.message?.route==='loader/call_plugin_method'&&r.message.args?.[0]==='PatchHub').map(r=>r.message.args[1]);const before=calls();if(!before.includes('scan'))window.__patchhubDebugScan=b.call('loader/call_plugin_method','PatchHub','scan').then(()=>({completed:true}),e=>({error:String(e)}));return {before,after:calls()}})()")
            save('inflight-before-click.json', active)
            if 'scan' not in active['after']:
                raise RuntimeError('The device test did not establish an in-flight scan')
            cef(main_title, "(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.innerText.includes(" + json.dumps('更新至 v' + target) + "));if(!b||b.disabled)throw Error('Update button unavailable');b.click();return true})()")
            worker = home + '/.cache/patchhub-app-update-debug/result.json'
            last = None
            while True:
                state = json.loads(ssh('if test -f ' + shlex.quote(worker) + '; then cat ' + shlex.quote(worker) + '; else printf null; fi', capture_output=True, text=True).stdout)
                if not state:
                    # DOM click dispatches the asynchronous RPC; the detached
                    # worker file is not guaranteed to exist in this same tick.
                    label = cef(main_title, "(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')?.includes('Steam 客户端'));return b?.innerText})()")
                    if label and label.startswith('更新至'):
                        tips = [t for t in get(endpoint + '/json/list') if t['title'].startswith('notificationtoasts')]
                        save('start-error.json', {'label': label, 'tips': [cef(t['title'], 'document.body.innerText') for t in tips]})
                        raise RuntimeError('Update RPC did not start a worker; see start-error.json')
                    time.sleep(.2)
                    continue
                if state != last:
                    events.append(state)
                    save('progress.json', events)
                    print(json.dumps(state, ensure_ascii=False), flush=True)
                    last = state
                    if state.get('stage') == 'downloading' and 0 < state.get('percent', 0) < 80:
                        cef(main_title, screenshot='http-progress.png')
                if state.get('state') == 'failed':
                    raise RuntimeError('Product updater failed: ' + str(state))
                if state.get('state') == 'completed':
                    if state.get('restart') != 'requested':
                        raise RuntimeError('Steam restart was not requested: ' + str(state))
                    break
                time.sleep(.3)
            # A restart is expected to temporarily remove CEF. Wait for the real
            # Steam PID change and new frontend, rather than invoking restart directly.
            before = json.loads(ssh('cat ' + shlex.quote(stage + '/baseline.json'), capture_output=True, text=True).stdout)
            while True:
                pid = ssh("pgrep -x steam || true", capture_output=True, text=True).stdout.strip()
                if pid != before['steam_pid']:
                    try:
                        tabs = get(endpoint + '/json/list')
                        shared = cef('SharedJSContext', "(()=>{const p=DeckyPluginLoader?.plugins.filter(p=>p.name==='PatchHub');return {versions:p?.map(p=>p.version)}})()")
                        if shared['versions'] == [target]:
                            break
                    except (OSError, subprocess.CalledProcessError, KeyError):
                        pass
                time.sleep(.5)
            ssh(remote + ' inspect')
            cef('SharedJSContext', "(()=>{DFL.Navigation.Navigate('/patchhub/sources/cache');DFL.Navigation.CloseSideMenus();return true})()")
            main_title = next(t['title'] for t in get(endpoint + '/json/list') if 'browserType=4' in t.get('url', ''))
            while True:
                loaded = cef(main_title, "(()=>{return {text:document.body.innerText,timeOrigin:performance.timeOrigin}})()")
                if '当前版本 ' + target in loaded['text'] and '正在扫描…' not in loaded['text']:
                    if loaded['timeOrigin'] == row['timeOrigin']:
                        raise RuntimeError('Steam page context did not restart')
                    save('new-frontend.json', loaded)
                    cef(main_title, "(()=>{const e=[...document.querySelectorAll('*')].find(e=>e.childElementCount===0&&e.textContent===" + json.dumps('当前版本 ' + target) + " );if(!e)throw Error('New version row missing');e.scrollIntoView({block:'center'});return true})()")
                    cef(main_title, screenshot='new-version.png')
                    break
                time.sleep(.2)
            if not any(e.get('stage') == 'downloading' and 0 < e.get('percent', 0) < 80 for e in events):
                raise RuntimeError('No intermediate real HTTP percentage observed')
    finally:
        if boot is not None and boot.poll() is None:
            boot.wait()
        try:
            if boot is not None:
                ssh(remote + ' clean')
            cleaned = True
        finally:
            if service:
                service.stdin.close()
                service.wait()
            if tunnel:
                tunnel.terminate()
                tunnel.wait()
            if staged:
                subprocess.run(['scp', *ssh_args, '-r', destination + ':' + stage + '/.', str(output)], check=True)
            if cleaned and staged:
                # Verify the exact per-run directory before recursive deletion.
                cleanup = "from pathlib import Path;import shutil;p=Path(" + repr(stage) + ");assert p.resolve().parent==Path.home()/'patchhub-validation' and p.name==" + repr(session) + ";shutil.rmtree(p);assert not p.exists()"
                ssh('python3 -c ' + shlex.quote(cleanup))
            save('cleanup.json', {'remote_staging_removed': cleaned, 'service_stopped': service is None or service.poll() is not None,
                                  'physical_controller': 'not tested'})
    after = json.loads((output / 'after.json').read_text(encoding='utf-8'))
    before = json.loads((output / 'baseline.json').read_text(encoding='utf-8'))
    if after['boot'] != before['boot'] or after['steam_pid'] == before['steam_pid']:
        raise RuntimeError('Expected Steam-only restart was not proved')
    print('Complete debug UI update passed. Evidence: ' + str(output))


if __name__ == '__main__':
    main()
