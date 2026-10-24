import asyncio
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch
import importlib.util
import sys
import types

from app_update import AppUpdate, ASSET, REPOSITORY, candidate, newer, version, RateLimited


def release():
    tag = 'v0.4.0'
    return {'id': 8, 'tag_name': tag, 'assets': [{'id': i, 'name': name,
        'browser_download_url': f'https://github.com/{REPOSITORY}/releases/download/{tag}/{name}'}
        for i, name in enumerate((ASSET, 'SHA256SUMS.txt'))]}


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


class MaintenanceTests(unittest.TestCase):
    def setUp(self):
        sys.modules.setdefault('decky', types.ModuleType('decky'))
        spec = importlib.util.spec_from_file_location('app_update_plugin', Path(__file__).parents[1] / 'main.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.plugin = module.Plugin()
        self.plugin.tasks = Mock()
        self.plugin.tasks.busy.return_value = False
        self.plugin.sources = Mock()
        self.plugin.sources.task_status.return_value = []

    def test_source_and_inflight_calls_block_update_without_closing_gate(self):
        self.plugin.sources.task_status.return_value = [{'state':'running'}]
        with self.assertRaises(RuntimeError):
            self.plugin._prepare_app_update()
        self.assertFalse(getattr(self.plugin, '_app_update_gate', False))
        self.plugin.sources.task_status.return_value = []
        self.plugin._active_host_calls = 2
        with self.assertRaises(RuntimeError):
            self.plugin._prepare_app_update()

    def test_idle_host_enters_maintenance(self):
        self.plugin._active_host_calls = 1
        self.assertTrue(self.plugin._prepare_app_update()['ready'])
        self.assertTrue(self.plugin._app_update_gate)


class ServiceTests(unittest.IsolatedAsyncioTestCase):
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
