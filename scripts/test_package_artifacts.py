"""Packaging boundaries: fixed Rust runtimes, provenance and no second engine."""
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
    def write(self, root, path, data=b'fixture'):
        target = root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)

    def run_package(self, root, platform):
        with patch.object(packager, 'ROOT', root), patch.object(packager, 'OUTPUT', root / '.artifacts'), patch.object(sys, 'argv', ['package-artifacts.py', '--platform', platform]):
            packager.main()

    def fixture(self, root, windows=False):
        self.write(root, 'Cargo.lock', b'cargo lock')
        self.write(root, 'package-lock.json', b'npm lock')
        plugin = root / 'apps/decky/plugin-output'
        for name in packager.PRODUCTION_BACKEND:
            self.write(plugin, 'backend/' + name)
        for name in ('protocol.py', 'v4.schema.json'):
            self.write(root, 'packages/contracts/generated/' + name)
        files = []
        for name in ('steam-patch', 'sf-worker'):
            data = b'\x7fELF' + name.encode()
            self.write(plugin, 'bin/' + name, data)
            files.append({'path': 'bin/' + name, 'size': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
        self.write(plugin, 'bin/runtime.json', json.dumps({'protocol_version': 4, 'worker_protocol': 1, 'files': files}).encode())
        properties = [{'name': 'steam-patch:reference-commit', 'value': packager.SFEXTRACT_COMMIT}]
        for source, dest, prefix in packager.INSTALLER_FILES:
            data = source.encode()
            self.write(root, source, data)
            if dest:
                self.write(plugin, dest, data)
                properties.append({'name': f'steam-patch:{prefix}-path', 'value': dest})
            properties.extend([{'name': f'steam-patch:{prefix}-source', 'value': source},
                               {'name': f'steam-patch:{prefix}-sha256', 'value': hashlib.sha256(data).hexdigest()}])
        inventory = {'metadata': {'properties': [
            {'name': f'steam-patch:{name}-lock-sha256', 'value': hashlib.sha256((root / path).read_bytes()).hexdigest()}
            for name, path in [('cargo', 'Cargo.lock'), ('npm', 'package-lock.json')]]},
            'components': [{'name': 'steam-patch-rust-installer', 'version': '1', 'licenses': [{'expression': 'MIT'}], 'properties': properties}]}
        self.write(root, '.artifacts/dependencies.cdx.json', json.dumps(inventory).encode())
        if windows:
            for path in ('target/release/steam-patch-windows.exe', 'target/release/steam-patch.exe', 'target/release/sf-worker.exe',
                         'catalog/catalog.json', 'catalog/signed-catalog.json', 'catalog/trust-keys.json', 'docs/client-guide.md', 'docs/privacy.md'):
                self.write(root, path)
        return plugin

    def test_decky_runtime_permissions_and_provenance(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); self.fixture(root); self.run_package(root, 'decky')
            with zipfile.ZipFile(root / '.artifacts/steam-patch-manager-decky.zip') as archive:
                for name in ('steam-patch', 'sf-worker'):
                    self.assertEqual(archive.getinfo('steam-patch-manager/bin/' + name).external_attr >> 16 & 0o777, 0o755)
                self.assertIn('steam-patch-manager/dependencies.cdx.json', archive.namelist())
                self.assertIn('steam-patch-manager/licenses/sfextract-MIT.txt', archive.namelist())

    def test_windows_does_not_require_decky(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); self.fixture(root, windows=True)
            self.write(root, 'apps/decky/plugin-output/bin/sf-worker', b'bad old Linux runtime')
            self.run_package(root, 'windows')
            self.assertFalse((root / '.artifacts/steam-patch-manager-decky.zip').exists())
            with zipfile.ZipFile(root / '.artifacts/steam-patch-manager-windows-portable.zip') as archive:
                self.assertIn('sf-worker.exe', archive.namelist())

    def test_rejects_second_production_engine(self):
        for name in ('publisher.py', 'source_engine.py', 'exe_worker'):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp); plugin = self.fixture(root)
                self.write(plugin, 'backend/' + name)
                with self.assertRaisesRegex(ValueError, 'unexpected or missing'):
                    self.run_package(root, 'decky')
                self.assertFalse(list((root / '.artifacts').glob('*.zip')))

    def test_missing_or_tampered_runtime_refuses_before_zip(self):
        for missing in (True, False):
            with self.subTest(missing=missing), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp); plugin = self.fixture(root)
                binary = plugin / 'bin/sf-worker'
                if missing: binary.unlink()
                else: binary.write_bytes(b'MZ Windows binary')
                with self.assertRaises((ValueError, FileNotFoundError)):
                    self.run_package(root, 'decky')
                self.assertFalse(list((root / '.artifacts').glob('*.zip')))

    def test_generated_contract_tampering_refuses_before_zip(self):
        for name in ('protocol.py', 'v4.schema.json'):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp); plugin = self.fixture(root)
                self.write(plugin, 'backend/' + name, b'old contract')
                with self.assertRaisesRegex(ValueError, 'generated contract mismatch'):
                    self.run_package(root, 'decky')
                self.assertFalse(list((root / '.artifacts').glob('*.zip')))

    def test_missing_sbom_refuses(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); self.fixture(root); (root / '.artifacts/dependencies.cdx.json').unlink()
            with self.assertRaisesRegex(FileNotFoundError, 'SBOM'):
                self.run_package(root, 'decky')

    def test_changed_sources_or_bundled_license_refuse(self):
        for source, dest, _ in packager.INSTALLER_FILES:
            with self.subTest(source=source), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp); self.fixture(root); self.write(root, source, b'changed')
                with self.assertRaisesRegex(ValueError, 'Regenerate SBOM'):
                    self.run_package(root, 'decky')
            if dest:
                with self.subTest(dest=dest), tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp); plugin = self.fixture(root); self.write(plugin, dest, b'changed')
                    with self.assertRaisesRegex(ValueError, 'Rebuild Decky plugin'):
                        self.run_package(root, 'decky')


if __name__ == '__main__':
    unittest.main()
