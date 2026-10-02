"""Exercise platform packaging with isolated fixture files."""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location('packager', Path(__file__).with_name('package-artifacts.py'))
packager = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packager)


class PackagingTests(unittest.TestCase):
    def run_package(self, root, platform):
        with patch.object(packager, 'ROOT', root), patch.object(packager, 'OUTPUT', root / '.artifacts'), patch.object(sys, 'argv', ['package-artifacts.py', '--platform', platform]):
            packager.main()

    def write(self, root, path):
        target = root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(b'fixture')

    def test_windows_without_decky(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for path in ['target/release/steam-patch-windows.exe', 'target/release/steam-patch.exe', 'catalog/catalog.json', 'catalog/signed-catalog.json', 'catalog/trust-keys.json', 'docs/client-guide.md', 'docs/privacy.md']:
                self.write(root, path)
            self.run_package(root, 'windows')
            output = root / '.artifacts'
            records = json.loads((output / 'checksums-windows.json').read_text())
            archive = output / records[0]['file']
            self.assertEqual(records[0]['sha256'], hashlib.sha256(archive.read_bytes()).hexdigest())
            with zipfile.ZipFile(archive) as package:
                self.assertIn('resources/signed-catalog.json', package.namelist())
            self.assertFalse((output / 'steam-patch-manager-decky.zip').exists())

    def test_decky_without_windows(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write(root, 'apps/decky/plugin-output/bin/steam-patch')
            self.run_package(root, 'decky')
            with zipfile.ZipFile(root / '.artifacts/steam-patch-manager-decky.zip') as package:
                info = package.getinfo('steam-patch-manager/bin/steam-patch')
                self.assertEqual((info.external_attr >> 16) & 0o777, 0o755)
            self.assertTrue((root / '.artifacts/checksums-decky.json').is_file())

    def test_missing_inputs_do_not_create_archive(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaises(FileNotFoundError):
                self.run_package(root, 'windows')
            self.assertFalse(list((root / '.artifacts').glob('*.zip')))


if __name__ == '__main__':
    unittest.main()
