"""Exercise single-field bumps, preservation, CLI boundaries and stale metadata."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch

import project_version as versions
from update_manifest import prepare


class ProjectVersionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.environment = patch.dict(os.environ, {'GITHUB_REF': ''})
        self.environment.start()
        self.addCleanup(self.environment.stop)
        for relative in ('Cargo.toml', 'Cargo.lock', 'package-lock.json',
                         'apps/decky/package.json', 'packages/ui/package.json',
                         'apps/windows-flutter/pubspec.yaml', 'scripts/project_version.py',
                         'scripts/windows-update.ps1'):
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(versions.ROOT / relative, target)
        self.members = tomllib.loads(self.read('Cargo.toml'))['workspace']['members']
        for member in self.members:
            target = self.root / member / 'Cargo.toml'
            target.parent.mkdir(parents=True)
            shutil.copyfile(versions.ROOT / member / 'Cargo.toml', target)

    def read(self, relative):
        return (self.root / relative).read_text(encoding='utf-8')

    def snapshot(self):
        return {p.relative_to(self.root).as_posix(): p.read_bytes()
                for p in self.root.rglob('*') if p.is_file()}

    def bump(self, version):
        current = versions.project_version(self.root)
        path = self.root / 'Cargo.toml'
        path.write_text(self.read('Cargo.toml').replace(f'version = "{current}"',
                                                     f'version = "{version}"', 1), encoding='utf-8')

    def test_one_field_bump_syncs_all_consumers_without_dependency_changes(self):
        old_cargo = tomllib.loads(self.read('Cargo.lock'))
        old_npm = json.loads(self.read('package-lock.json'))
        names = {tomllib.loads(self.read(member + '/Cargo.toml'))['package']['name'] for member in self.members}
        self.bump('9.8.7')
        changed = versions.synchronize(self.root)
        self.assertEqual(set(changed), {'apps/decky/package.json', 'packages/ui/package.json',
                                      'apps/windows-flutter/pubspec.yaml', 'Cargo.lock', 'package-lock.json'})
        self.assertEqual(versions.check(self.root), '9.8.7')
        new_cargo = tomllib.loads(self.read('Cargo.lock'))
        for old, new in zip(old_cargo['package'], new_cargo['package'], strict=True):
            expected = {**old, 'version': '9.8.7'} if old['name'] in names and 'source' not in old else old
            self.assertEqual(new, expected)
        new_npm = json.loads(self.read('package-lock.json'))
        for location, old in old_npm['packages'].items():
            if location not in ('apps/decky', 'packages/ui'):
                self.assertEqual(new_npm['packages'][location], old)
        self.assertIn('version: 9.8.7+1', self.read('apps/windows-flutter/pubspec.yaml'))
        self.assertEqual(json.loads(self.read('apps/decky/package.json'))['dependencies']['@patchhub/ui'], '9.8.7')
        self.assertEqual(versions.synchronize(self.root), [])

    def test_check_lists_differences_without_writing(self):
        self.bump('9.8.7')
        before = self.snapshot()
        with self.assertRaises(ValueError) as error:
            versions.check(self.root)
        for relative in ('Cargo.lock', 'package-lock.json', 'pubspec.yaml', 'apps/decky/package.json', 'packages/ui/package.json'):
            self.assertIn(relative, str(error.exception))
        self.assertEqual(self.snapshot(), before)

    def test_matching_fields_do_not_rewrite_files(self):
        before = self.snapshot()
        self.assertEqual(versions.synchronize(self.root), [])
        self.assertEqual(self.snapshot(), before)

    def test_tag_mismatch_refuses_before_sync(self):
        self.bump('9.8.7')
        before = self.snapshot()
        with patch.dict(os.environ, {'GITHUB_REF': 'refs/tags/v9.8.6'}):
            with self.assertRaisesRegex(ValueError, 'Release Tag differs'):
                versions.synchronize(self.root)
        self.assertEqual(self.snapshot(), before)

    def test_semver_and_flutter_metadata_mapping(self):
        for version, expected in (('9.8.7-rc.1', '9.8.7-rc.1+1'),
                                  ('9.8.7+42', '9.8.7+42'),
                                  ('9.8.7-rc.1+build.sha', '9.8.7-rc.1+build.sha')):
            with self.subTest(version=version):
                self.bump(version)
                versions.synchronize(self.root)
                self.assertEqual(versions.check(self.root), version)
                self.assertIn(f'version: {expected}', self.read('apps/windows-flutter/pubspec.yaml'))

    def test_invalid_primary_version_never_changes_derived_files(self):
        original = self.read('Cargo.toml')
        current = versions.project_version(self.root)
        for invalid in ('01.2.3', '1.2', '1.2.3-01', '1.2.3+', 'v1.2.3', '1.2.3-a..b'):
            with self.subTest(version=invalid):
                (self.root / 'Cargo.toml').write_text(original.replace(f'version = "{current}"',
                                                                    f'version = "{invalid}"', 1), encoding='utf-8')
                before = self.snapshot()
                with self.assertRaisesRegex(ValueError, 'valid SemVer'):
                    versions.synchronize(self.root)
                self.assertEqual(self.snapshot(), before)

    def test_missing_workspace_lock_entry_refuses_without_partial_sync(self):
        self.bump('9.8.7')
        (self.root / 'Cargo.lock').write_text('version = 4\n', encoding='utf-8')
        before = self.snapshot()
        with self.assertRaisesRegex(ValueError, 'missing workspace packages'):
            versions.synchronize(self.root)
        self.assertEqual(self.snapshot(), before)

    def test_existing_lock_comments_and_dependencies_are_preserved(self):
        self.bump('9.8.7')
        path = self.root / 'Cargo.lock'
        path.write_text('# fixture header\n' + self.read('Cargo.lock'), encoding='utf-8')
        versions.synchronize(self.root)
        self.assertTrue(self.read('Cargo.lock').startswith('# fixture header\n'))

    def test_cli_reads_copied_root_and_check_rejects_unsynchronized_bump(self):
        script = self.root / 'scripts/project_version.py'
        self.bump('9.8.7')
        result = subprocess.run([sys.executable, str(script), '--check'], capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn('out of sync', result.stderr)
        result = subprocess.run([sys.executable, str(script), '--sync'], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        result = subprocess.run([sys.executable, str(script)], capture_output=True, text=True)
        self.assertEqual(result.stdout.strip(), '9.8.7')

    def test_windows_metadata_refreshes_from_primary_after_bump(self):
        bundle = self.root / 'bundle'
        bundle.mkdir()
        prepare(bundle, '0.0.1', root=self.root)
        self.bump('9.8.7')
        versions.synchronize(self.root)
        files = prepare(bundle, root=self.root)
        self.assertEqual(json.loads((bundle / 'app-version.json').read_text())['version'], '9.8.7')
        self.assertEqual(json.loads((bundle / 'update-manifest.json').read_text())['version'], '9.8.7')
        self.assertEqual(set(files), {p.name for p in bundle.iterdir()})

    @unittest.skipUnless(os.name == 'nt', 'Windows preview entrypoint requires PowerShell and cmd')
    def test_windows_preview_syncs_before_build_and_refreshes_old_metadata(self):
        for relative in ('scripts/build-windows-flutter.ps1', 'scripts/update_manifest.py',
                         'apps/windows-flutter/.flutter-version', 'scripts/licenses/sfextract-MIT.txt'):
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(versions.ROOT / relative, target)
        for relative in ('catalog/catalog.json', 'catalog/signed-catalog.json', 'catalog/trust-keys.json',
                         'LICENSE', 'packages/contracts/generated/v5.schema.json',
                         'target/release/patchhub-core.exe', 'target/release/patchhub-worker.exe'):
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text('fixture', encoding='utf-8')
        (self.root / 'apps/windows-flutter/assets').mkdir()
        sdk = self.root / 'fake sdk'
        (sdk / 'bin/cache/dart-sdk').mkdir(parents=True)
        for relative in ('LICENSE', 'bin/cache/dart-sdk/LICENSE'):
            (sdk / relative).write_text('fixture license', encoding='utf-8')
        (sdk / 'bin/flutter.bat').write_text(
            '@echo off\nif "%1"=="--version" (\n'
            '  if "%2"=="--machine" (echo {"frameworkVersion":"3.47.7"}) else (echo Flutter 3.47.7)\n'
            ')\nexit /b 0\n', encoding='utf-8')
        bundle = self.root / 'apps/windows-flutter/build/windows/x64/runner/Release'
        bundle.mkdir(parents=True)
        prepare(bundle, '0.0.1', root=self.root)
        self.bump('9.8.7')
        result = subprocess.run(['pwsh', '-NoProfile', '-File', str(self.root / 'scripts/build-windows-flutter.ps1'),
                                 '-Flutter', str(sdk / 'bin/flutter.bat'), '-SkipRust', '-Preview'],
                                capture_output=True, text=True, encoding='utf-8')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(versions.check(self.root), '9.8.7')
        self.assertEqual(json.loads((bundle / 'app-version.json').read_text())['version'], '9.8.7')
        self.assertEqual(json.loads((bundle / 'update-manifest.json').read_text())['version'], '9.8.7')

    def test_direct_decky_staging_syncs_package_before_copying(self):
        for relative in ('scripts/sync-project-version.mjs', 'apps/decky/scripts/package-plugin.mjs'):
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(versions.ROOT / relative, target)
        app = self.root / 'apps/decky'
        for relative in ('main.py', 'plugin.json', 'README.md', 'dist/index.js',
                         'backend/bridge.py', 'backend/source_service.py', 'backend/tasks.py',
                         'backend/plugin_settings.py', 'backend/state_host.py',
                         'backend/protocol_validation.py', 'backend/app_update.py'):
            target = app / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text('fixture', encoding='utf-8')
        for relative in ('LICENSE', 'scripts/install-decky.py', 'scripts/licenses/sfextract-MIT.txt',
                         'packages/contracts/generated/protocol.py', 'packages/contracts/generated/v5.schema.json',
                         'catalog/catalog.json', 'catalog/signed-catalog.json', 'catalog/trust-keys.json'):
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text('fixture', encoding='utf-8')
        for name in ('patchhub-core', 'patchhub-worker'):
            target = self.root / 'target/linux/release' / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(b'\x7fELFfixture')
        (app / 'plugin-output').mkdir()
        (app / 'plugin-output/package.json').write_text('{"version":"0.0.1"}', encoding='utf-8')
        self.bump('9.8.7')
        result = subprocess.run(['node', str(app / 'scripts/package-plugin.mjs')], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(json.loads((app / 'plugin-output/package.json').read_text())['version'], '9.8.7')
        self.assertEqual(versions.check(self.root), '9.8.7')


if __name__ == '__main__':
    unittest.main()
