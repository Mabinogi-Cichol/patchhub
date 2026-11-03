"""Debug harness failure boundaries; no network or real device writes."""
import importlib.util
import json
from pathlib import Path
import subprocess
import shlex
import sys
import tempfile
import types
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location('debug_update', Path(__file__).with_name('decky-debug-update.py'))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class DebugHarnessTests(unittest.TestCase):
    def test_upload_failure_cleans_only_its_staging_without_installing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, version in [('current', '0.5.3'), ('target', '0.5.4')]:
                with zipfile.ZipFile(root / (name + '.zip'), 'w') as archive:
                    archive.writestr('patchhub/package.json', json.dumps({'version': version}))
            commands = []
            def run(command, **kwargs):
                commands.append(command)
                if command[0] == 'scp' and '-r' not in command:
                    raise subprocess.CalledProcessError(1, command)
                return subprocess.CompletedProcess(command, 0, stdout='/home/paired-user', stderr='')
            def get(url):
                return {'key_path': 'paired-key'} if url.endswith('ssh_key_path') else {
                    'name': 'selected-device', 'address': '192.0.2.1', 'login': 'paired-user'}
            argv = ['decky-debug-update.py', '--current-package', str(root / 'current.zip'),
                    '--target-package', str(root / 'target.zip'), '--known-hosts', str(root / 'known_hosts')]
            with patch.object(m, 'ROOT', root), patch.object(m, 'get', get), patch.object(m.subprocess, 'run', run), \
                    patch.object(sys, 'argv', argv), patch.dict(sys.modules, {'app_update': types.SimpleNamespace(newer=lambda a, b: True)}):
                with self.assertRaises(subprocess.CalledProcessError):
                    m.main()
            self.assertFalse(any(' bootstrap' in c[-1] or ' clean' in c[-1] for c in commands))
            cleanup = next(c[-1] for c in commands if c[-1].startswith('python3 -c '))
            cleanup = shlex.split(cleanup)[2]
            self.assertIn("p.resolve().parent==Path.home()/'patchhub-validation'", cleanup)
            self.assertIn('/home/paired-user/patchhub-validation/debug-update-', cleanup)
            self.assertIn('StrictHostKeyChecking=yes', commands[0])
            record = next((root / '.artifacts').glob('*/cleanup.json'))
            self.assertTrue(json.loads(record.read_text())['remote_staging_removed'])


if __name__ == '__main__':
    unittest.main()
