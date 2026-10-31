"""Check the inventory against disposable lockfiles and bundled worker files."""
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name('build-sbom.py')


class BuildSbomTests(unittest.TestCase):
    def test_locked_dependencies_and_bundled_sf95_are_disclosed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            scripts = root / 'scripts'
            licenses = scripts / 'licenses'
            licenses.mkdir(parents=True)
            shutil.copyfile(SCRIPT, scripts / 'build-sbom.py')
            shutil.copyfile(SCRIPT.with_name('project_version.py'), scripts / 'project_version.py')
            (root / 'Cargo.toml').write_text('[workspace.package]\nversion = "9.8.7-rc.1"\n', encoding='utf-8')
            for name in ('installer-format', 'installer-worker'):
                manifest = root / 'crates' / name / 'Cargo.toml'
                manifest.parent.mkdir(parents=True)
                manifest.write_bytes(name.encode())
            license_text = b'MIT reference license\n'
            project_license = b'MIT License\nCopyright (c) test contributors\n'
            (licenses / 'sfextract-MIT.txt').write_bytes(license_text)
            (root / 'LICENSE').write_bytes(project_license)
            cargo_checksum = 'ab' * 32
            (root / 'Cargo.lock').write_text(f'''version = 3
[[package]]
name = "locked-rust"
version = "1.2.3"
source = "registry+https://example.test"
checksum = "{cargo_checksum}"
''', encoding='utf-8')
            npm_digest = bytes(range(64))
            (root / 'package-lock.json').write_text(json.dumps({'packages': {
                '': {'name': 'app', 'version': '0.1.0'},
                'node_modules/locked-npm': {'version': '4.5.6', 'license': 'Apache-2.0',
                    'dev': True, 'integrity': 'sha512-' + base64.b64encode(npm_digest).decode()},
            }}), encoding='utf-8')
            for name in ('Lua-5.4-LICENSE.txt', 'mlua-MIT.txt'):
                (licenses / name).write_text(name, encoding='utf-8')
            metadata = root / 'metadata.json'
            metadata.write_text(json.dumps({'packages': [{'name': 'locked-rust', 'version': '1.2.3',
                'source': 'registry+https://example.test', 'license': 'MIT'},
                {'name': 'lua-src', 'version': '551.0.2', 'license': 'MIT'},
                {'name': 'mlua', 'version': '0.12.2', 'license': 'MIT'}]}), encoding='utf-8')

            result = subprocess.run([sys.executable, str(scripts / 'build-sbom.py'), str(metadata)],
                                    cwd=root, capture_output=True, text=True, check=True,
                                    env={**os.environ, 'GITHUB_REF': ''})
            summary = json.loads(result.stdout)
            inventory = json.loads((root / '.artifacts/dependencies.cdx.json').read_text(encoding='utf-8'))
            components = {entry['name']: entry for entry in inventory['components']}
            self.assertEqual(inventory['metadata']['component']['version'], '9.8.7-rc.1')
            self.assertEqual(summary['components'], 5)
            self.assertEqual(set(components), {'locked-rust', 'locked-npm', 'patchhub-rust-installer', 'lua-src', 'mlua'})
            self.assertEqual(components['locked-rust']['hashes'][0]['content'], 'ab' * 32)
            self.assertEqual(components['locked-npm']['hashes'][0]['content'], npm_digest.hex())
            lua = {p['name']: p['value'] for p in components['lua-src']['properties']}
            self.assertEqual(lua['patchhub:license-path'], 'licenses/Lua-5.4-LICENSE.txt')
            self.assertEqual(lua['patchhub:license-sha256'], hashlib.sha256(b'Lua-5.4-LICENSE.txt').hexdigest())
            self.assertIn('Lua 5.4.9', lua['patchhub:bundled-runtime'])
            self.assertIn('statically embedded', lua['patchhub:bundled-runtime'])
            sf95 = components['patchhub-rust-installer']
            self.assertEqual(sf95['version'], '1')
            self.assertEqual(sf95['licenses'], [{'expression': 'MIT'}])
            properties = {entry['name']: entry['value'] for entry in sf95['properties']}
            self.assertEqual(properties['patchhub:format-manifest-sha256'], hashlib.sha256(b'installer-format').hexdigest())
            self.assertEqual(properties['patchhub:reference-license-sha256'], hashlib.sha256(license_text).hexdigest())
            self.assertEqual(properties['patchhub:project-license-path'], 'LICENSE')
            self.assertEqual(properties['patchhub:project-license-sha256'], hashlib.sha256(project_license).hexdigest())
            self.assertEqual(properties['patchhub:reference-commit'], '21f5c5595a70e9284fff83476c0e307d5b92f573')
            self.assertIn('Python developer oracles are not bundled', properties['patchhub:implementation'])
            self.assertIn('build, development and target-specific packages',
                          inventory['metadata']['properties'][0]['value'])


if __name__ == '__main__':
    unittest.main()
