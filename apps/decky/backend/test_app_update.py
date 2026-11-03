import asyncio
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, AsyncMock, patch
import importlib.util
import sys
import types

from app_update import AppUpdate, ASSET, REPOSITORY, candidate, newer, version, RateLimited


def release():
    tag = 'v0.4.0'
    return {'id': 8, 'tag_name': tag, 'assets': [{'id': i, 'name': name,
        'browser_download_url': f'https://github.com/{REPOSITORY}/releases/download/{tag}/{name}'}
        for i, name in enumerate((ASSET, 'SHA256SUMS.txt'))]}


class RestartRetryTests(unittest.TestCase):
    def test_retry_launches_restart_only_and_rejects_a_second_request(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'package.json').write_text('{"version":"0.5.3"}')
            (root / 'backend').mkdir()
            (root / 'backend/install-decky.py').write_text('# fixture')
            service = AppUpdate(root, root)
            service.worker_dir = root / 'worker'
            service.worker_dir.mkdir()
            (service.worker_dir / 'result.json').write_text(json.dumps({
                'state': 'completed', 'version': 'v0.5.3', 'restart': 'failed'}))
            with patch('app_update.shutil.which', return_value='python3'), patch('app_update.subprocess.Popen', return_value=Mock(pid=123)):
                service.start(restart_only=True, client_context=123)
            request = json.loads((service.worker_dir / 'request.json').read_text())
            self.assertTrue(request['restart_only'])
            self.assertEqual(request['installed_version'], 'v0.5.3')
            self.assertEqual(request['client_context'], 123)
            self.assertEqual(request['candidate']['tag'], 'v0.5.3')
            result = service.snapshot()['execution']
            self.assertEqual(result['state'], 'completed')
            self.assertEqual(result['version'], 'v0.5.3')
            service.snapshot = Mock(return_value={'execution': {'state': 'applying', 'stage': 'restarting'}})
            with self.assertRaises(RuntimeError):
                service.start(restart_only=True)


class VersionTests(unittest.TestCase):
    def test_shared_contract(self):
        fixtures = json.loads((Path(__file__).parents[3] / 'scripts/fixtures/app-update.json').read_text())
        for case in fixtures['versions']:
            self.assertEqual(newer(case['remote'], case['local']), case['newer'])
        for value in fixtures['invalid']:
            with self.assertRaises(ValueError):
                version(value)

    def test_missing_duplicate_and_foreign_assets(self):
        data = release()
        self.assertEqual(candidate(data, '0.3.0')['state'], 'available')
        data['assets'].append(data['assets'][0])
        self.assertEqual(candidate(data, '0.3.0')['state'], 'unavailable')
        data['assets'] = release()['assets']
        data['assets'][0]['browser_download_url'] = 'https://example.com/program.zip'
        with self.assertRaises(ValueError):
            candidate(data, '0.3.0')


class MaintenanceTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        from tasks import TaskManager
        from source_service import SourceService
        from bridge import CoreBridge
        sys.modules.setdefault('decky', types.ModuleType('decky'))
        spec = importlib.util.spec_from_file_location('app_update_plugin', Path(__file__).parents[1] / 'main.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.plugin = module.Plugin()
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        binary = root / 'binary'
        binary.touch()
        self.plugin.bridge = CoreBridge(binary)
        self.plugin.bridge._preflight_done = True
        self.plugin.bridge.recover = AsyncMock(return_value=[])
        self.plugin.tasks = TaskManager()
        self.plugin.sources = SourceService(self.plugin.bridge, root)
        self.plugin.app_update = Mock()
        self.plugin.app_update.start.return_value = {'execution': {'state': 'preparing', 'percent': 0}}
        self.plugin.app_update.check = AsyncMock(return_value={'state': 'available'})

    async def test_click_starts_download_while_writer_cancels_and_prepare_waits_for_rollback(self):
        from test_tasks import ControlledProcess
        process = ControlledProcess(restoring=True)
        async def work(task):
            return await self.plugin.bridge.install_patch('instance', 'zip-selection-0123456789abcdef01234567-42', operation='overwrite')
        with patch('bridge.asyncio.create_subprocess_exec', return_value=process):
            receipt = self.plugin.tasks.start('install', work)
            while process.request is None:
                await asyncio.sleep(0)
            result = await self.plugin.start_app_update('v0.5.4')
            self.assertEqual(result['execution']['state'], 'preparing')
            self.plugin.app_update.start.assert_called_once()
            await asyncio.wait_for(process.cancel_received.wait(), 1)
            self.assertEqual(process.messages[1], b'cancel\n')
            prepare = asyncio.create_task(self.plugin.prepare_app_update())
            await asyncio.sleep(0)
            self.assertFalse(prepare.done())
            self.assertEqual(self.plugin.tasks.status(receipt['task_id'])['state'], 'rolling_back')
            self.plugin.bridge.recover.assert_not_awaited()
            self.assertEqual((await self.plugin.check_app_update())['state'], 'available')
            with self.assertRaisesRegex(RuntimeError, 'plugin_update_in_progress'):
                await self.plugin.update_source('patchmoe')
            process.finish('rolled_back')
            self.assertTrue((await prepare)['ready'])
            self.assertEqual(self.plugin.tasks.status(receipt['task_id'])['state'], 'cancelled')
            self.plugin.bridge.recover.assert_awaited_once()

    async def test_active_scan_is_cancelled_and_does_not_start_late_matching(self):
        started = asyncio.Event()
        async def scan():
            started.set()
            await asyncio.Event().wait()
        self.plugin.bridge.scan = scan
        scan_call = asyncio.create_task(self.plugin.scan())
        await started.wait()
        await self.plugin.start_app_update('v0.5.4')
        self.assertTrue((await self.plugin.prepare_app_update())['ready'])
        with self.assertRaisesRegex(RuntimeError, 'app_update_cancelled'):
            await scan_call
        self.assertIsNone(self.plugin._start_pending_match({'installs': []}))

    async def test_failed_source_cancellation_blocks_replacement(self):
        from bridge import CoreError
        started = asyncio.Event()
        async def call(method, params, on_event=None, cancel=None):
            started.set()
            while not cancel.is_set():
                await asyncio.sleep(0)
            raise CoreError('source_disk_failed', 'write did not finish')
        self.plugin.bridge._call = call
        self.plugin.sources.start_update_source('patchmoe')
        await started.wait()
        await self.plugin.start_app_update('v0.5.4')
        with self.assertRaisesRegex(RuntimeError, '取消收尾失败'):
            await self.plugin.prepare_app_update()
        self.plugin.bridge.recover.assert_not_awaited()
        self.assertFalse(self.plugin.sources._closed)

    async def test_unresolved_recovery_blocks_replacement(self):
        self.plugin.bridge.recover.return_value = [{'state': 'needs_recovery'}]
        await self.plugin.start_app_update('v0.5.4')
        with self.assertRaisesRegex(RuntimeError, '恢复尚未完成'):
            await self.plugin.prepare_app_update()


class ServiceTests(unittest.IsolatedAsyncioTestCase):
    async def test_recovery_launch_is_applying_and_cannot_be_cancelled_as_download(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'package.json').write_text('{"version":"0.3.0"}')
            (root / 'backend').mkdir()
            (root / 'backend/install-decky.py').write_text('# fixture')
            service = AppUpdate(root, root)
            service.worker_dir = root / 'worker'
            service.snapshot = lambda: {'execution': json.loads((service.worker_dir / 'result.json').read_text())} if (service.worker_dir / 'result.json').exists() else {}
            with patch('app_update.shutil.which', return_value='python3'), patch('app_update.subprocess.Popen', return_value=Mock(pid=123)):
                self.assertEqual(service.start(recover=True)['execution'], {'state': 'applying', 'stage': 'recovering'})
            with self.assertRaisesRegex(RuntimeError, '正在替换或恢复'):
                service.cancel()

    async def test_worker_completion_during_process_check_is_preserved(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'package.json').write_text('{"version":"0.3.0"}')
            service = AppUpdate(root, root)
            service.worker_dir = root / 'worker'
            service.worker_dir.mkdir()
            result = service.worker_dir / 'result.json'
            result.write_text('{"state":"applying"}')
            (service.worker_dir / 'worker.json').write_text('{"pid":123}')
            def finish(_):
                result.write_text('{"state":"completed","version":"v0.4.0"}')
                return b''
            original_exists = Path.exists
            with patch.object(Path, 'exists', lambda p: True if p == Path('/proc/123/cmdline') else original_exists(p)), patch.object(Path, 'read_bytes', finish):
                self.assertEqual(service.snapshot()['execution']['state'], 'completed')
            self.assertEqual(json.loads(result.read_text())['state'], 'completed')

    async def test_cache_skip_manual_failure_and_rate_limit(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'package.json').write_text('{"version":"0.3.0"}')
            calls = []
            def fetch():
                calls.append(1)
                return release()
            service = AppUpdate(root, root, fetch=fetch, clock=lambda: 100000)
            self.assertEqual((await service.check())['state'], 'available')
            self.assertEqual(service.claim_notice()['tag'], 'v0.4.0')
            self.assertIsNone(service.claim_notice())
            self.assertTrue(service.skip('v0.4.0')['skipped'])
            await service.check()
            self.assertEqual(len(calls), 1)
            await service.check(manual=True)
            self.assertEqual(len(calls), 2)
            def limited():
                raise RateLimited(200000)
            service.fetch = limited
            self.assertEqual((await service.check(manual=True))['state'], 'failed')
            service.fetch = fetch
            await service.check(manual=True)
            self.assertEqual(len(calls), 2)

    async def test_upgrade_does_not_reuse_old_available_candidate(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'package.json').write_text('{"version":"0.4.0"}')
            state = root / 'host-v3'
            state.mkdir()
            (state / 'app-update.json').write_text(json.dumps({'last_attempt_at':100000,
                'candidate':candidate(release(), '0.3.0')}))
            service = AppUpdate(root, root, fetch=release, clock=lambda:100001)
            self.assertEqual((await service.check())['state'], 'up_to_date')

    async def test_checks_are_coalesced(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'package.json').write_text('{"version":"0.3.0"}')
            calls = []
            def fetch():
                calls.append(1)
                return release()
            service = AppUpdate(root, root, fetch=fetch, clock=lambda: 100000)
            await asyncio.gather(service.check(True), service.check(True))
            self.assertEqual(len(calls), 1)


class DebugSourceTests(unittest.IsolatedAsyncioTestCase):
    async def test_debug_state_is_isolated_and_production_rejects_local_assets(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'package.json').write_text('{"version":"0.3.0"}')
            host = root / 'host-v3'
            host.mkdir()
            production = host / 'app-update.json'
            production.write_text('{"last_notified_tag":"v0.4.0","retry_at":99999999999}')
            before = production.read_bytes()
            source = 'http://127.0.0.1:43210/session'
            (root / 'debug-update-source.json').write_text(json.dumps({'base_url': source}))
            data = release()
            for item in data['assets']:
                item['browser_download_url'] = f"{source}/download/v0.4.0/{item['name']}"
            service = AppUpdate(root, root, fetch=lambda: data)
            self.assertEqual((await service.check(manual=True))['state'], 'available')
            self.assertIsNotNone(service.claim_notice())
            self.assertIsNone(service.claim_notice())
            self.assertEqual(production.read_bytes(), before)
            self.assertEqual(service.worker_dir.name, 'patchhub-app-update-debug')
            with self.assertRaises(ValueError):
                candidate(data, '0.3.0')

    async def test_marker_rejects_non_loopback_or_unscoped_sources(self):
        from app_update import debug_source
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.assertIsNone(debug_source(root))
            for source in ['http://example.com:80/session', 'http://127.0.0.1:80',
                           'https://127.0.0.1:80/session', 'http://u@127.0.0.1:80/session',
                           'http://127.0.0.1:80/session?x=1']:
                (root / 'debug-update-source.json').write_text(json.dumps({'base_url': source}))
                with self.assertRaises(ValueError):
                    debug_source(root)
