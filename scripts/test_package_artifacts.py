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

    def write(self, root, path, content=b'fixture'):
        target = root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)

    def sbom_fixture(self, root):
        self.write(root, 'Cargo.lock', b'locked cargo')
        self.write(root, 'package-lock.json', b'locked npm')
        self.write(root, 'apps/decky/plugin-output/bin/steam-patch')
        sources = {
            'scripts/sf95_audit.py': ('backend/exe_worker/sf95_audit.py', 'bundled-audit', b'VERSION = "sf95-audit-v-test"\n'),
            'scripts/sf95_worker.py': ('backend/exe_worker/sf95_worker.py', 'bundled-worker', b'import sf95_audit\n'),
            'LICENSE': ('LICENSE', 'project-license', b'MIT project license\n'),
            'scripts/licenses/sfextract-MIT.txt': ('backend/exe_worker/sfextract-MIT.txt', 'bundled-license', b'MIT reference license\n'),
        }
        properties = [{'name': f'steam-patch:{name}-lock-sha256',
                       'value': hashlib.sha256((root / path).read_bytes()).hexdigest()}
                      for name, path in [('cargo', 'Cargo.lock'), ('npm', 'package-lock.json')]]
        sf95 = [{'name': 'steam-patch:reference-commit', 'value': packager.SFEXTRACT_COMMIT}]
        for source, (destination, prefix, content) in sources.items():
            self.write(root, source, content)
            self.write(root, 'apps/decky/plugin-output/' + destination, content)
            sf95.extend([{'name': f'steam-patch:{prefix}-path', 'value': destination},
                         {'name': f'steam-patch:{prefix}-sha256', 'value': hashlib.sha256(content).hexdigest()}])
        inventory = {'metadata': {'properties': properties}, 'components': [
            {'name': 'steam-patch-sf95-worker', 'version': 'sf95-audit-v-test',
             'licenses': [{'expression': 'MIT'}], 'properties': sf95}]}
        self.write(root, '.artifacts/dependencies.cdx.json', json.dumps(inventory).encode())
        return sources

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

    def test_current_sbom_and_decky_bundle_package_together(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.sbom_fixture(root)
            self.run_package(root, 'decky')
            with zipfile.ZipFile(root / '.artifacts/steam-patch-manager-decky.zip') as package:
                self.assertIn('steam-patch-manager/dependencies.cdx.json', package.namelist())
                self.assertIn('steam-patch-manager/LICENSE', package.namelist())
                self.assertIn('steam-patch-manager/backend/exe_worker/sfextract-MIT.txt', package.namelist())

    def test_windows_sbom_checks_sources_without_requiring_decky_bundle(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.sbom_fixture(root)
            for path in ['target/release/steam-patch-windows.exe', 'target/release/steam-patch.exe',
                         'catalog/catalog.json', 'catalog/signed-catalog.json', 'catalog/trust-keys.json',
                         'docs/client-guide.md', 'docs/privacy.md']:
                self.write(root, path)
            self.write(root, 'apps/decky/plugin-output/backend/exe_worker/sf95_worker.py', b'outdated')
            self.run_package(root, 'windows')
            with zipfile.ZipFile(root / '.artifacts/steam-patch-manager-windows-portable.zip') as package:
                self.assertIn('dependencies.cdx.json', package.namelist())

    def test_missing_component_and_each_changed_source_refuse_before_zip(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sources = self.sbom_fixture(root)
            sbom = root / '.artifacts/dependencies.cdx.json'
            inventory = json.loads(sbom.read_text())
            inventory['components'] = []
            sbom.write_text(json.dumps(inventory))
            with self.assertRaisesRegex(ValueError, 'Regenerate SBOM'):
                self.run_package(root, 'decky')
            self.assertFalse(list((root / '.artifacts').glob('*.zip')))
            for source in sources:
                self.sbom_fixture(root)
                changed = b'VERSION = "sf95-audit-v-other"\n' if source == 'scripts/sf95_audit.py' else b'changed source'
                self.write(root, source, changed)
                with self.subTest(source=source), self.assertRaisesRegex(ValueError, 'Regenerate SBOM'):
                    self.run_package(root, 'decky')
                self.assertFalse(list((root / '.artifacts').glob('*.zip')))

    def test_outdated_decky_worker_or_license_refuses_before_zip(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sources = self.sbom_fixture(root)
            for destination, _, _ in sources.values():
                self.sbom_fixture(root)
                self.write(root, 'apps/decky/plugin-output/' + destination, b'old plugin file')
                with self.subTest(destination=destination), self.assertRaisesRegex(ValueError, 'Rebuild Decky plugin'):
                    self.run_package(root, 'decky')
                self.assertFalse(list((root / '.artifacts').glob('*.zip')))


if __name__ == '__main__':
    unittest.main()
