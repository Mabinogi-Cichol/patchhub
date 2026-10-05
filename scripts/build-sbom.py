"""Create a dependency inventory from locks and the bundled SF95 worker.

Run cargo metadata --locked --format-version 1 into a local JSON file first.
Includes build/development and target-specific dependencies; not a reachability audit.
"""
import ast
import base64
import hashlib
import json
from pathlib import Path
import sys
import tomllib
from urllib.parse import quote

root = Path(__file__).resolve().parents[1]
metadata = json.loads(Path(sys.argv[1]).read_text(encoding='utf-8-sig'))
cargo_lock = tomllib.loads((root / 'Cargo.lock').read_text(encoding='utf-8'))
checksums = {(p['name'], p['version'], p.get('source')): p.get('checksum')
             for p in cargo_lock['package']}
components = {}


def component(ecosystem, name, version, license_expression=None):
    ref = f'pkg:{ecosystem}/{quote(name, safe="/")}@{quote(version, safe="")}'
    value = {'type': 'library', 'bom-ref': ref, 'name': name, 'version': version, 'purl': ref}
    if isinstance(license_expression, str) and license_expression:
        expression = ' OR '.join(part.strip() for part in license_expression.split('/'))
        value['licenses'] = [{'expression': expression}]
        if expression != license_expression:
            value['properties'] = [{'name': 'steam-patch:declared-license', 'value': license_expression}]
    else:
        value['properties'] = [{'name': 'steam-patch:license-review', 'value': 'required'}]
    components.setdefault(ref, value)
    return components[ref]


for package in metadata['packages']:
    value = component('cargo', package['name'], package['version'], package.get('license'))
    checksum = checksums.get((package['name'], package['version'], package.get('source')))
    if checksum:
        value['hashes'] = [{'alg': 'SHA-256', 'content': checksum}]

lock = json.loads((root / 'package-lock.json').read_text(encoding='utf-8'))
for location, package in lock['packages'].items():
    if 'node_modules/' not in location or package.get('link'):
        continue
    name = package.get('name') or location.rsplit('node_modules/', 1)[1]
    value = component('npm', name, package['version'], package.get('license'))
    integrity = package.get('integrity', '')
    if integrity.startswith('sha512-'):
        value['hashes'] = [{'alg': 'SHA-512', 'content': base64.b64decode(integrity[7:]).hex()}]


def sf95_version(path):
    """Read the declared parser version without importing developer code."""
    module = ast.parse(path.read_text(encoding='utf-8'), filename=str(path))
    versions = [node.value.value for node in module.body
                if isinstance(node, ast.Assign)
                and any(isinstance(target, ast.Name) and target.id == 'VERSION' for target in node.targets)
                and isinstance(node.value, ast.Constant) and isinstance(node.value.value, str)]
    if len(versions) != 1:
        raise ValueError('sf95_audit.py must declare one literal VERSION')
    return versions[0]


audit = root / 'scripts/sf95_audit.py'
worker = root / 'scripts/sf95_worker.py'
license_file = root / 'scripts/licenses/sfextract-MIT.txt'
project_license = root / 'LICENSE'
sf95 = component('generic', 'steam-patch-sf95-worker', sf95_version(audit), 'MIT')
sf95['properties'] = [
    {'name': 'steam-patch:implementation', 'value': 'Maintained in-tree SF95 parser and sandbox worker; not the complete sfextract runtime.'},
    {'name': 'steam-patch:project-license-path', 'value': 'LICENSE'},
    {'name': 'steam-patch:project-license-sha256', 'value': hashlib.sha256(project_license.read_bytes()).hexdigest()},
    {'name': 'steam-patch:bundled-audit-path', 'value': 'backend/exe_worker/sf95_audit.py'},
    {'name': 'steam-patch:bundled-audit-sha256', 'value': hashlib.sha256(audit.read_bytes()).hexdigest()},
    {'name': 'steam-patch:bundled-worker-path', 'value': 'backend/exe_worker/sf95_worker.py'},
    {'name': 'steam-patch:bundled-worker-sha256', 'value': hashlib.sha256(worker.read_bytes()).hexdigest()},
    {'name': 'steam-patch:reference-repository', 'value': 'https://github.com/CybercentreCanada/sfextract'},
    {'name': 'steam-patch:reference-commit', 'value': '21f5c5595a70e9284fff83476c0e307d5b92f573'},
    {'name': 'steam-patch:reference-scope', 'value': 'SF9.5 file-table layout reference; upstream runtime is not bundled.'},
    {'name': 'steam-patch:license-source', 'value': 'CybercentreCanada/sfextract MIT notice at the reference commit; text bundled with the plugin.'},
    {'name': 'steam-patch:bundled-license-path', 'value': 'backend/exe_worker/sfextract-MIT.txt'},
    {'name': 'steam-patch:bundled-license-sha256', 'value': hashlib.sha256(license_file.read_bytes()).hexdigest()},
]

output = root / '.artifacts/dependencies.cdx.json'
output.parent.mkdir(exist_ok=True)
output.write_text(json.dumps({
    'bomFormat': 'CycloneDX', 'specVersion': '1.6', 'version': 1,
    'metadata': {'component': {'type': 'application', 'name': 'Steam Patch Manager', 'version': '0.1.0'},
                 'properties': [{'name': 'steam-patch:inventory-scope',
                                 'value': 'All resolved Rust and locked npm dependencies, including build, development and target-specific packages, plus the bundled in-tree SF95 worker; no reachability analysis.'},
                                {'name': 'steam-patch:cargo-lock-sha256', 'value': hashlib.sha256((root / 'Cargo.lock').read_bytes()).hexdigest()},
                                {'name': 'steam-patch:npm-lock-sha256', 'value': hashlib.sha256((root / 'package-lock.json').read_bytes()).hexdigest()}]},
    'components': sorted(components.values(), key=lambda item: item['bom-ref'])
}, indent=2) + '\n', encoding='utf-8')
missing = [item['purl'] for item in components.values() if 'licenses' not in item]
print(json.dumps({'components': len(components), 'license_metadata_missing': missing,
                  'output': str(output)}, indent=2))
