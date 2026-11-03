"""Device half of decky-debug-update.py; run only inside its private staging directory."""
import argparse
import functools
import http.server
import importlib.util
import json
from pathlib import Path
import secrets
import shutil
import sys
import threading
import time
import zipfile

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('installer', ROOT / 'install-decky.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
PLUGIN = m.loader_root() / 'plugins/patchhub'
WORKER = Path.home() / '.cache/patchhub-app-update-debug'


def save(name, value):
    (ROOT / name).write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding='utf-8')


def rpc(loader, method):
    return loader.cdp.evaluate("DeckyBackend.call('loader/call_plugin_method','PatchHub'," + json.dumps(method) + ")")


def baseline(loader):
    tasks = rpc(loader, 'list_tasks')
    sources = rpc(loader, 'source_task_status')
    m.require(all(t['state'] in m.TERMINAL for t in tasks + sources), 'Active tasks; stop debug deployment')
    m.require(not (Path.home() / '.cache/patchhub-decky-install/journal.json').exists(), 'Product update requires recovery')
    return {'started_at': time.time(), 'probe': loader.call('probe'), 'tasks': tasks, 'sources': sources,
            'transactions': rpc(loader, 'list_transactions'),
            'boot': Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
            'steam_pid': m.subprocess.check_output(['pgrep', '-x', 'steam'], text=True).strip()}


def install(loader, archive):
    """Debug deployment: use Loader, retain no old plugin backup. Product updater is unchanged."""
    baseline(loader)
    package = {**m.validate_package(archive), 'sha256': m.digest(archive)}
    state = m.State(ROOT, {'jobs': [], 'settings': loader.call('probe')['settings']})
    loader.mutate(state, kind='install', name=m.NAME, artifact=archive.as_uri(),
                  version=package['version'], hash=package['sha256'], installType=1, checkTasks=True)
    m.Installer(ROOT, PLUGIN, loader).restore_settings(state)
    m.verify_files(PLUGIN, package['files'])
    loader.verify(ROOT, package)
    (ROOT / 'journal.json').unlink()
    return package


def serve():
    for name, expected in m.json_read(ROOT / 'inputs.json').items():
        m.require(name in ('current.zip', 'target.zip') and m.digest(ROOT / name) == expected, 'Uploaded ZIP SHA-256 mismatch')
    session = secrets.token_hex(12)
    class Handler(http.server.SimpleHTTPRequestHandler):
        def copyfile(self, source, output):
            # Deliberate test transfer pacing gives the real UI time to render
            # intermediate percentages; this is not a production download limit.
            while block := source.read(64 * 1024):
                output.write(block)
                output.flush()
                time.sleep(.05)
        def log_message(self, fmt, *args):
            print(fmt % args, file=sys.stderr, flush=True)
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Handler, directory=str(ROOT / 'http')))
    base = f'http://127.0.0.1:{server.server_port}/{session}'
    web = ROOT / 'http' / session
    web.mkdir(parents=True)
    for kind in ('current', 'target'):
        with zipfile.ZipFile(ROOT / (kind + '.zip')) as original, zipfile.ZipFile(ROOT / (kind + '-debug.zip'), 'w') as debug:
            m.require('patchhub/debug-update-source.json' not in original.namelist(), 'Input must be a clean package')
            for info in original.infolist():
                debug.writestr(info, original.read(info))
            info = zipfile.ZipInfo('patchhub/debug-update-source.json')
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            debug.writestr(info, json.dumps({'base_url': base}))
    target = ROOT / 'target-debug.zip'
    package = m.validate_package(target)
    tag = 'v' + package['version']
    download = web / 'download' / tag
    download.mkdir(parents=True)
    shutil.copyfile(target, download / m.ASSET)
    (download / 'SHA256SUMS.txt').write_text(f'{m.digest(target)}  {m.ASSET}\n')
    release = {'id': session, 'tag_name': tag, 'draft': False, 'prerelease': False,
               'body': 'Local debug update: real HTTP / production installer',
               'assets': [{'id': i, 'name': name, 'size': (download / name).stat().st_size,
                           'digest': 'sha256:' + m.digest(download / name),
                           'browser_download_url': f'{base}/download/{tag}/{name}'}
                          for i, name in enumerate((m.ASSET, 'SHA256SUMS.txt'))]}
    (web / 'latest').write_text(json.dumps(release))
    (web / 'tags').mkdir()
    (web / 'tags' / tag).write_text(json.dumps(release))
    save('source.json', {'base_url': base, 'release': release, 'manifest': package})
    print(json.dumps({'ready': True, 'base_url': base, 'tag': tag}), flush=True)
    # Closing the owning SSH stdin stops this one server, including on Ctrl-C.
    def stop():
        sys.stdin.read()
        server.shutdown()
    threading.Thread(target=stop, daemon=True).start()
    try:
        server.serve_forever()
    finally:
        server.server_close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['serve', 'bootstrap', 'inspect', 'clean'])
    action = parser.parse_args().action
    m.require(ROOT.parent == Path.home() / 'patchhub-validation' and ROOT.name.startswith('debug-update-'), 'Invalid staging directory')
    if action == 'serve':
        serve()
        return
    loader = m.Loader()
    try:
        if action == 'bootstrap':
            save('baseline.json', baseline(loader))
            status = rpc(loader, 'get_app_update_status')
            m.require((status.get('execution') or {}).get('state') not in ('preparing', 'applying'), 'Previous debug update is active')
            if WORKER.exists():
                m.no_links(WORKER)
                shutil.copytree(WORKER, ROOT / 'previous-debug', dirs_exist_ok=True)
                shutil.rmtree(WORKER)
            (Path.home() / '.local/share/patchhub/host-v3/app-update-debug.json').unlink(missing_ok=True)
            save('bootstrap.json', install(loader, ROOT / 'current-debug.zip'))
        elif action == 'inspect':
            save('after.json', {**baseline(loader), 'status': rpc(loader, 'get_app_update_status'),
                                'scan': rpc(loader, 'scan')})
            since = str(int(m.json_read(ROOT / 'baseline.json')['started_at']))
            journal = m.subprocess.check_output(['journalctl', '-u', 'plugin_loader', '--since', '@' + since, '--no-pager'], text=True)
            lines = [line for line in journal.splitlines() if 'app_update_cancel' in line or 'Installing PatchHub' in line]
            (ROOT / 'cancellation.log').write_text('\n'.join(lines) + '\n', encoding='utf-8')
            m.require(any('app_update_cancel_started' in line and "'scan'" in line for line in lines), 'No real in-flight scan was cancelled')
            end = next(i for i, line in enumerate(lines) if 'app_update_cancel_completed' in line)
            target = m.validate_package(ROOT / 'target.zip')['version']
            m.require(any('Installing PatchHub' in line and 'Version: ' + target in line for line in lines[end + 1:]), 'Loader installed before business cancellation completed')
        else:
            status = rpc(loader, 'get_app_update_status')
            m.require((status.get('execution') or {}).get('state') not in ('preparing', 'applying'), 'Debug worker is still active; preserve staging')
            # The test proved the newer version loaded. Restore the supplied
            # clean baseline so a fictitious higher test version cannot suppress
            # future production updates. This is an input package, not a backup.
            version = m.json_read(PLUGIN / 'package.json')['version']
            versions = {m.validate_package(ROOT / name)['version'] for name in ('current.zip', 'target.zip')}
            m.require(version in versions, 'Unexpected running version; preserve staging')
            save('clean-install.json', install(loader, ROOT / 'current.zip'))
            m.require(not (PLUGIN / 'debug-update-source.json').exists(), 'Debug marker survived cleanup')
            debug_cache = Path.home() / '.local/share/patchhub/host-v3/app-update-debug.json'
            debug_cache.unlink(missing_ok=True)
            if WORKER.exists():
                m.no_links(WORKER)
                m.require(WORKER.parent == Path.home() / '.cache' and WORKER.name == 'patchhub-app-update-debug', 'Invalid debug cache')
                shutil.copytree(WORKER, ROOT / 'worker-evidence', dirs_exist_ok=True)
                shutil.rmtree(WORKER)
            save('clean.json', {'marker_removed': True, 'debug_cache_removed': True, 'probe': loader.call('probe')})
    finally:
        loader.cdp.close()


if __name__ == '__main__':
    main()
