"""Application release checks belong to the host, independent of patch Core."""
import asyncio
import json
import re
import time
import urllib.error
import urllib.request
import os
import shutil
import subprocess
import signal
import ssl
from pathlib import Path

from plugin_settings import _write_json

REPOSITORY = "Mabinogi-Cichol/steam-hentai-patch"
API = f"https://api.github.com/repos/{REPOSITORY}/releases/latest"
ASSET = "steam-patch-manager-decky.zip"
INTERVAL = 24 * 60 * 60


def version(value):
    match = re.fullmatch(r"v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?", value)
    if not match:
        raise ValueError("无法识别版本")
    if '+' in value and any(not part for part in value.split('+', 1)[1].split('.')):
        raise ValueError('无法识别版本')
    pre = match[4]
    if pre and any(not part or (part.isdigit() and len(part) > 1 and part[0] == '0') for part in pre.split('.')):
        raise ValueError("无法识别版本")
    return tuple(int(match[i]) for i in (1, 2, 3)), pre


def newer(remote, local):
    r, rp = version(remote)
    l, lp = version(local)
    if r != l:
        return r > l
    if rp is None or lp is None:
        return rp is None and lp is not None
    for a, b in zip(rp.split('.'), lp.split('.')):
        if a != b:
            if a.isdigit() and b.isdigit():
                return int(a) > int(b)
            if a.isdigit() != b.isdigit():
                return not a.isdigit()
            return a > b
    return len(rp.split('.')) > len(lp.split('.'))


def candidate(release, current, asset=ASSET):
    version(current)
    tag = release['tag_name']
    version(tag)
    if release.get('draft') or release.get('prerelease') or version(tag)[1] is not None:
        raise ValueError("Release 不是正式版")
    result = {'current': current, 'tag': tag, 'release_id': release['id'],
              'notes': release.get('body') or '', 'published_at': release.get('published_at'),
              'page': f"https://github.com/{REPOSITORY}/releases/tag/{tag}",
              'state': 'up_to_date'}
    if not newer(tag, current):
        return result
    selected = {}
    for name in (asset, 'SHA256SUMS.txt'):
        matches = [a for a in release.get('assets', []) if a.get('name') == name]
        if len(matches) != 1:
            return {**result, 'state': 'unavailable', 'error': '新版已发布，当前平台更新包尚不可用'}
        item = matches[0]
        expected = f"https://github.com/{REPOSITORY}/releases/download/{tag}/{name}"
        if item.get('browser_download_url') != expected:
            raise ValueError('附件不属于官方冻结版本')
        selected[name] = item
    return {**result, 'state': 'available', 'asset': selected[asset], 'sums': selected['SHA256SUMS.txt']}


class RateLimited(Exception):
    def __init__(self, until):
        self.until = until
        super().__init__('GitHub 请求受限，请稍后重试')


def fetch_release():
    request = urllib.request.Request(API, headers={'User-Agent': 'steam-patch-manager',
        'Accept': 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28'})
    try:
        # Loader's frozen interpreter resolves its bundled CA location incorrectly on
        # SteamOS; use the OS trust bundle, retaining full TLS verification.
        with urllib.request.urlopen(request, timeout=30, context=ssl.create_default_context(cafile='/etc/ssl/cert.pem')) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        if error.code in (403, 429) and (error.headers.get('Retry-After') or error.headers.get('X-RateLimit-Remaining') == '0'):
            until = (time.time() + int(error.headers['Retry-After']) if error.headers.get('Retry-After')
                     else int(error.headers['X-RateLimit-Reset']))
            raise RateLimited(until) from error
        raise


class AppUpdate:
    def __init__(self, root, state_dir, fetch=fetch_release, clock=time.time):
        self.root = Path(root)
        self.path = Path(state_dir) / 'host-v3' / 'app-update.json'
        self.current = json.loads((self.root / 'package.json').read_text(encoding='utf-8'))['version']
        self.fetch, self.clock = fetch, clock
        self.data = {}
        load_error = None
        try:
            if self.path.exists():
                self.data = json.loads(self.path.read_text(encoding='utf-8'))
        except (OSError, ValueError) as error:
            load_error = f'更新记录读取失败：{error}'
        self.result = self.data.get('candidate', {'state': 'idle', 'current': self.current})
        if self.result.get('current') != self.current:
            self.data.pop('last_attempt_at', None)
            self.result = {'state': 'idle', 'current': self.current}
        if load_error:
            self.result = {'state': 'failed', 'current': self.current, 'error': load_error}
        self.pending = None
        self.worker_dir = Path.home() / '.cache' / 'steam-patch-app-update'

    def snapshot(self):
        worker = self.worker_dir / 'result.json'
        execution = json.loads(worker.read_text(encoding='utf-8')) if worker.exists() else None
        identity_file = self.worker_dir / 'worker.json'
        if execution and execution.get('state') in ('preparing', 'applying') and identity_file.exists():
            identity = json.loads(identity_file.read_text(encoding='utf-8'))
            process = Path('/proc') / str(identity['pid']) / 'cmdline'
            command = process.read_bytes().split(b'\0') if process.exists() else []
            if os.fsencode(str(self.worker_dir / 'install-decky.py')) not in command:
                # The worker can publish its terminal result while this snapshot
                # was checking /proc. Never replace a completed/failed result.
                latest = json.loads(worker.read_text(encoding='utf-8'))
                if latest.get('state') in ('completed', 'failed'):
                    execution = latest
                else:
                    execution = {'state': 'failed', 'error': '更新进程已退出，请重试或恢复未完成的更新'}
                    _write_json(worker, execution)
        return {**self.result, 'execution': execution, 'skipped': bool(self.result.get('tag')) and self.data.get('skipped_tag') == self.result.get('tag'),
                'retry_at': self.data.get('retry_at'),
                'needs_recovery': (Path.home() / '.cache/steam-patch-decky-install/journal.json').exists()}

    def start(self, tag=None, recover=False):
        if not recover and (self.result.get('state') != 'available' or tag != self.result.get('tag')):
            raise ValueError('更新候选已变化，请重新检查')
        self.worker_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
        status = self.snapshot().get('execution') or {}
        if status.get('state') in ('preparing', 'applying'):
            raise RuntimeError('更新已在进行，请查看更新结果或完成恢复')
        # This trusted script is copied outside the directory Loader will remove.
        python = shutil.which('python3')
        if not python:
            raise RuntimeError('未找到 SteamOS Python 3，请使用终端安装入口检查运行环境')
        shutil.copyfile(self.root / 'backend' / 'install-decky.py', self.worker_dir / 'install-decky.py')
        request = self.worker_dir / 'request.json'
        _write_json(request, {'candidate': self.result, 'plugin': str(self.root), 'status': str(self.worker_dir / 'result.json'), 'recover': recover})
        _write_json(self.worker_dir / 'result.json', {'state': 'preparing'})
        with (self.worker_dir / 'worker.log').open('wb') as log:
            environment = dict(os.environ)
            if 'LD_LIBRARY_PATH_ORIG' in environment:
                environment['LD_LIBRARY_PATH'] = environment.pop('LD_LIBRARY_PATH_ORIG')
            else:
                environment.pop('LD_LIBRARY_PATH', None)
            worker = subprocess.Popen([python, str(self.worker_dir / 'install-decky.py'), '--gui-request', str(request)],
                stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True, close_fds=True, env=environment)
        _write_json(self.worker_dir / 'worker.json', {'pid': worker.pid})
        return self.snapshot()

    def cancel(self):
        state = self.snapshot().get('execution') or {}
        if state.get('state') != 'preparing':
            raise RuntimeError('插件正在替换或恢复，请等待实际结果')
        identity = json.loads((self.worker_dir / 'worker.json').read_text(encoding='utf-8'))
        process = Path('/proc') / str(identity['pid'])
        command = (process / 'cmdline').read_bytes().split(b'\0')
        if os.fsencode(str(self.worker_dir / 'install-decky.py')) not in command:
            raise RuntimeError('更新进程身份已变化，请检查更新状态')
        os.kill(identity['pid'], signal.SIGTERM)
        return self.snapshot()

    async def check(self, manual=False):
        if self.pending:
            await self.pending
            return self.snapshot()
        now = self.clock()
        last = self.data.get('last_attempt_at', 0)
        if now < self.data.get('retry_at', 0) or (not manual and 0 <= now - last < INTERVAL):
            return self.snapshot()
        self.pending = asyncio.create_task(self._check(now))
        try:
            await self.pending
        finally:
            self.pending = None
        return self.snapshot()

    async def _check(self, now):
        self.data['last_attempt_at'] = now
        self.result = {'state': 'checking', 'current': self.current}
        try:
            version(self.current)
            self.result = candidate(await asyncio.to_thread(self.fetch), self.current)
            self.data['last_success_at'] = now
            self.data.pop('retry_at', None)
        except RateLimited as error:
            self.data['retry_at'] = error.until
            self.result = {'state': 'failed', 'current': self.current, 'error': str(error)}
        except Exception as error:
            self.result = {'state': 'failed', 'current': self.current, 'error': str(error)}
        self.data['candidate'] = self.result
        _write_json(self.path, self.data)

    def skip(self, tag):
        if tag != self.result.get('tag'):
            raise ValueError('更新候选已变化，请重新检查')
        self.data['skipped_tag'] = tag
        _write_json(self.path, self.data)
        return self.snapshot()

    def claim_notice(self):
        tag = self.result.get('tag')
        if self.result.get('state') != 'available' or tag in (self.data.get('skipped_tag'), self.data.get('last_notified_tag')):
            return None
        self.data['last_notified_tag'] = tag
        _write_json(self.path, self.data)
        return self.snapshot()

    async def close(self):
        if self.pending:
            await self.pending
