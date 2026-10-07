"""Check the inventory against disposable lockfiles and bundled worker files."""
import base64
import hashlib
import json
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
            audit = b'VERSION = "sf95-audit-v-test"\n'
            worker = b'import sf95_audit\n'
            license_text = b'MIT License\n'
            project_license = b'MIT License\nCopyright (c) test contributors\n'
            (scripts / 'sf95_audit.py').write_bytes(audit)
            (scripts / 'sf95_worker.py').write_bytes(worker)
            metadata_decoder = b'VERSION = "sf9-metadata-v-test"\n'
            (scripts / 'sf9_metadata.py').write_bytes(metadata_decoder)
            for module in ('sf9_graph', 'sf9_lua', 'sf9_behavior'):
                (scripts / f'{module}.py').write_bytes(b'# synthetic module\n')
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
            metadata = root / 'metadata.json'
            metadata.write_text(json.dumps({'packages': [{'name': 'locked-rust', 'version': '1.2.3',
                'source': 'registry+https://example.test', 'license': 'MIT'}]}), encoding='utf-8')

            result = subprocess.run([sys.executable, str(scripts / 'build-sbom.py'), str(metadata)],
                                    cwd=root, capture_output=True, text=True, check=True)
            summary = json.loads(result.stdout)
            inventory = json.loads((root / '.artifacts/dependencies.cdx.json').read_text(encoding='utf-8'))
            components = {entry['name']: entry for entry in inventory['components']}
            self.assertEqual(summary['components'], 3)
            self.assertEqual(set(components), {'locked-rust', 'locked-npm', 'steam-patch-sf95-worker'})
            self.assertEqual(components['locked-rust']['hashes'][0]['content'], 'ab' * 32)
            self.assertEqual(components['locked-npm']['hashes'][0]['content'], npm_digest.hex())
            sf95 = components['steam-patch-sf95-worker']
            self.assertEqual(sf95['version'], 'sf95-audit-v-test')
            self.assertEqual(sf95['licenses'], [{'expression': 'MIT'}])
            properties = {entry['name']: entry['value'] for entry in sf95['properties']}
            self.assertEqual(properties['steam-patch:bundled-audit-sha256'], hashlib.sha256(audit).hexdigest())
            self.assertEqual(properties['steam-patch:bundled-worker-sha256'], hashlib.sha256(worker).hexdigest())
            self.assertEqual(properties['steam-patch:bundled-metadata-sha256'], hashlib.sha256(metadata_decoder).hexdigest())
            self.assertEqual(properties['steam-patch:bundled-license-sha256'], hashlib.sha256(license_text).hexdigest())
            self.assertEqual(properties['steam-patch:project-license-path'], 'LICENSE')
            self.assertEqual(properties['steam-patch:project-license-sha256'], hashlib.sha256(project_license).hexdigest())
            self.assertEqual(properties['steam-patch:reference-commit'], '21f5c5595a70e9284fff83476c0e307d5b92f573')
            self.assertIn('not the complete sfextract runtime', properties['steam-patch:implementation'])
            self.assertIn('MIT notice', properties['steam-patch:license-source'])
            self.assertIn('build, development and target-specific packages',
                          inventory['metadata']['properties'][0]['value'])


if __name__ == '__main__':
    unittest.main()
