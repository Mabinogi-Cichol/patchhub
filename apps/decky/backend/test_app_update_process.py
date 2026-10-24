"""Exercise the host's real detached worker launch, without installing a plugin."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest


@unittest.skipIf(os.name == 'nt', 'Decky process sessions are POSIX')
class DetachedUpdateTests(unittest.TestCase):
    def test_worker_survives_host_termination_and_plugin_directory_removal(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            plugin = root / 'plugin'
            (plugin / 'backend').mkdir(parents=True)
            (plugin / 'package.json').write_text('{"version":"0.3.0"}')
            worker_dir = root / 'worker'
            worker_source = '''import json, sys, time
from pathlib import Path
request = json.loads(Path(sys.argv[2]).read_text())
root = Path(request['status']).parent
(root / 'started').write_text('ready')
while not (root / 'continue').exists(): time.sleep(.01)
(root / 'survived').write_text('yes')
'''
            (plugin / 'backend/install-decky.py').write_text(worker_source)
            parent = root / 'parent.py'
            backend = Path(__file__).parent.resolve()
            parent.write_text(f'''import sys, time
from pathlib import Path
sys.path.insert(0, {str(backend)!r})
from app_update import AppUpdate
service = AppUpdate(Path({str(plugin)!r}), Path({str(root)!r}))
service.worker_dir = Path({str(worker_dir)!r})
service.result = {{'state':'available','tag':'v0.4.0'}}
service.start('v0.4.0')
while True: time.sleep(1)
''')
            host = subprocess.Popen([sys.executable, str(parent)])
            try:
                deadline = time.monotonic() + 10  # Test harness only: report a failed launch.
                while not (worker_dir / 'started').exists():
                    if host.poll() is not None or time.monotonic() >= deadline:
                        self.fail('detached worker did not start')
                    time.sleep(.01)
                host.terminate()
                host.wait(timeout=10)
                (plugin / 'backend/install-decky.py').unlink()
                (plugin / 'backend').rmdir()
                (plugin / 'package.json').unlink()
                plugin.rmdir()
                (worker_dir / 'continue').write_text('continue')
                while not (worker_dir / 'survived').exists():
                    if time.monotonic() >= deadline:
                        self.fail('worker did not survive parent termination')
                    time.sleep(.01)
                self.assertEqual((worker_dir / 'survived').read_text(), 'yes')
            finally:
                if host.poll() is None:
                    host.terminate()
                    host.wait(timeout=10)
