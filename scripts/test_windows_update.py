"""Exercise the actual PowerShell updater in isolated program directories."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import zipfile

from update_manifest import prepare

SCRIPT = Path(__file__).with_name('windows-update.ps1')


@unittest.skipUnless(os.name == 'nt', 'Windows updater uses Windows PowerShell')
class WindowsUpdateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.build = tempfile.TemporaryDirectory()
        cls.executable = Path(cls.build.name) / 'probe.exe'
        source = Path(cls.build.name) / 'probe.cs'
        source.write_text('''using System;
using System.IO;
using System.Threading;
using System.Collections.Generic;
using System.Web.Script.Serialization;
class Probe {
 static int Main(string[] args) {
  var json = new JavaScriptSerializer();
  var request = json.Deserialize<Dictionary<string,object>>(File.ReadAllText(args[1]));
  string work = (string)request["work"];
  if (File.Exists(Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "fail-probe"))) return 7;
  File.WriteAllText(Path.Combine(work,"ready.json"), json.Serialize(new { pid=System.Diagnostics.Process.GetCurrentProcess().Id, version=(string)request["version"] }));
  while (!File.Exists(Path.Combine(work,"commit.json"))) {
   if(File.Exists(Path.Combine(work,"abort.json"))) return 1;
   Thread.Sleep(20);
  }
  File.WriteAllText(Path.Combine(work,"ack.json"),"{}");
  return 0;
 }
}''', encoding='utf-8')
        subprocess.run(['powershell.exe', '-NoProfile', '-NonInteractive', '-Command',
            'Add-Type -Path $env:PROBE_SOURCE -ReferencedAssemblies System.Web.Extensions -OutputAssembly $env:PROBE_OUTPUT -OutputType ConsoleApplication'],
            env={**os.environ, 'PROBE_SOURCE': str(source), 'PROBE_OUTPUT': str(cls.executable)}, check=True, capture_output=True)

    @classmethod
    def tearDownClass(cls):
        cls.build.cleanup()

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.target = self.root / '程序 with spaces'
        self.target.mkdir()
        self.bundle = self.root / 'bundle'
        self.bundle.mkdir()
        self.work = self.root / 'transaction'
        self.work.mkdir()
        for root, version in ((self.target, '0.3.0'), (self.bundle, '0.4.0')):
            shutil.copyfile(self.executable, root / 'steam_patch_windows.exe')
            (root / 'steam-patch.exe').write_text(version)
            (root / 'sf-worker.exe').write_text(version)
        (self.target / 'obsolete.dll').write_text('old')
        (self.bundle / 'added.dll').write_text('new')
        prepare(self.target, '0.3.0')
        (self.target / 'user.txt').write_text('preserve')

    def tearDown(self):
        self.tmp.cleanup()

    def apply(self, corrupt=False):
        prepare(self.bundle, '0.4.0')
        archive = self.work / 'package.zip'
        with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as bundle:
            for file in self.bundle.iterdir():
                bundle.write(file, file.name)
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        (self.work / 'request.json').write_text(json.dumps({'target': str(self.target), 'version':'0.4.0', 'sha256':'0'*64 if corrupt else digest}), encoding='utf-8')
        (self.work / 'apply.json').write_text('{}')
        process = subprocess.run(['powershell.exe', '-NoProfile', '-NonInteractive', '-ExecutionPolicy','Bypass',
            '-File', str(SCRIPT), '-Target', str(self.target), '-Work', str(self.work)], capture_output=True, text=True)
        return process

    def test_zip_update_preserves_extra_files_and_removes_obsolete_files(self):
        result = self.apply()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.target / 'steam-patch.exe').read_text(), '0.4.0')
        self.assertEqual((self.target / 'user.txt').read_text(), 'preserve')
        self.assertFalse((self.target / 'obsolete.dll').exists())
        self.assertTrue((self.target / 'added.dll').exists())
        self.assertFalse((self.target / '.steam-patch-update.json').exists())
        self.assertFalse((self.work / 'backup').exists())

    def test_failed_probe_restores_old_files_and_manifest(self):
        (self.bundle / 'fail-probe').write_text('yes')
        result = self.apply()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('initialization failed', result.stderr)
        self.assertEqual((self.target / 'steam-patch.exe').read_text(), '0.3.0')
        self.assertTrue((self.target / 'obsolete.dll').exists())
        self.assertFalse((self.target / 'added.dll').exists())
        self.assertEqual(json.loads((self.target / 'update-manifest.json').read_text())['version'], '0.3.0')

    def test_bad_hash_never_modifies_program(self):
        result = self.apply(corrupt=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('SHA-256 mismatch', result.stderr)
        self.assertEqual((self.target / 'steam-patch.exe').read_text(), '0.3.0')
        self.assertFalse((self.target / '.steam-patch-update.json').exists())

    def test_new_managed_file_cannot_overwrite_unmanaged_user_file(self):
        (self.target / 'added.dll').write_text('user supplied')
        result = self.apply()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('unmanaged user file', result.stderr)
        self.assertEqual((self.target / 'added.dll').read_text(), 'user supplied')
        self.assertEqual((self.target / 'steam-patch.exe').read_text(), '0.3.0')

    def test_interrupted_transaction_restores_old_files_before_next_launch(self):
        backup = self.work / 'backup'
        backup.mkdir()
        shutil.copyfile(self.target / 'steam-patch.exe', backup / 'steam-patch.exe')
        (self.target / 'steam-patch.exe').write_text('partially replaced')
        (self.target / 'new-file.dll').write_text('partial new file')
        (self.work / 'journal.json').write_text(json.dumps({'target':str(self.target), 'phase':'applying', 'registry':None,
            'entries':[{'path':'steam-patch.exe', 'existed':True}, {'path':'new-file.dll','existed':False}]}), encoding='utf-8')
        (self.target / '.steam-patch-update.json').write_text(json.dumps({'target':str(self.target),'work':str(self.work)}), encoding='utf-8')
        result = subprocess.run(['powershell.exe','-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass',
            '-File',str(SCRIPT),'-Mode','Recover','-Target',str(self.target)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.target / 'steam-patch.exe').read_text(), '0.3.0')
        self.assertFalse((self.target / 'new-file.dll').exists())
        self.assertFalse(self.work.exists())

    def test_uninstall_after_zip_update_reads_current_manifest(self):
        result = self.apply()
        self.assertEqual(result.returncode, 0, result.stderr)
        result = subprocess.run(['powershell.exe','-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass',
            '-File',str(SCRIPT),'-Mode','Uninstall','-Target',str(self.target)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(sorted(p.name for p in self.target.iterdir()), ['user.txt'])

    def test_unrelated_transaction_marker_survives_failed_preparation(self):
        marker = self.target / '.steam-patch-update.json'
        saved = {'target':str(self.target), 'work':str(self.root / 'other-work')}
        marker.write_text(json.dumps(saved), encoding='utf-8')
        result = self.apply(corrupt=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(json.loads(marker.read_text()), saved)
