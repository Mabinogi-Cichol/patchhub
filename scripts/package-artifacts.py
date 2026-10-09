"""Package already-built clients without game assets or private signing keys."""
from pathlib import Path
import argparse
import ast
import hashlib
import json
import zipfile

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / '.artifacts'
SF95_FILES = (
    ('scripts/sf95_audit.py', 'backend/exe_worker/sf95_audit.py', 'bundled-audit'),
    ('scripts/sf95_worker.py', 'backend/exe_worker/sf95_worker.py', 'bundled-worker'),
    ('scripts/sf9_metadata.py', 'backend/exe_worker/sf9_metadata.py', 'bundled-metadata'),
    ('scripts/sf9_graph.py', 'backend/exe_worker/sf9_graph.py', 'bundled-graph'),
    ('scripts/sf9_lua.py', 'backend/exe_worker/sf9_lua.py', 'bundled-lua'),
    ('scripts/sf9_behavior.py', 'backend/exe_worker/sf9_behavior.py', 'bundled-behavior'),
    ('LICENSE', 'LICENSE', 'project-license'),
    ('scripts/licenses/sfextract-MIT.txt', 'backend/exe_worker/sfextract-MIT.txt', 'bundled-license'),
)
SFEXTRACT_COMMIT = '21f5c5595a70e9284fff83476c0e307d5b92f573'


def sf95_version(path):
    module = ast.parse(path.read_text(encoding='utf-8'), filename=str(path))
    versions = [node.value.value for node in module.body
                if isinstance(node, ast.Assign)
                and any(isinstance(target, ast.Name) and target.id == 'VERSION' for target in node.targets)
                and isinstance(node.value, ast.Constant) and isinstance(node.value.value, str)]
    if len(versions) != 1:
        raise ValueError('SF95 audit version is missing; rebuild SBOM after fixing the worker')
    return versions[0]


def validate_sbom(sbom, plugin, decky):
    data = json.loads(sbom.read_text(encoding='utf-8'))
    try:
        properties = {item['name']: item['value'] for item in data['metadata']['properties']}
        matches = [item for item in data['components'] if item.get('name') == 'steam-patch-sf95-worker']
    except (KeyError, TypeError) as error:
        raise ValueError('Regenerate SBOM for current dependencies before packaging') from error
    for name, path in [('cargo', 'Cargo.lock'), ('npm', 'package-lock.json')]:
        if properties.get(f'steam-patch:{name}-lock-sha256') != hashlib.sha256((ROOT / path).read_bytes()).hexdigest():
            raise ValueError('Regenerate SBOM for current lockfiles before packaging')
    if len(matches) != 1:
        raise ValueError('Regenerate SBOM with the maintained SF95 worker before packaging')
    sf95 = matches[0]
    details = {item['name']: item['value'] for item in sf95.get('properties', [])}
    if (sf95.get('version') != sf95_version(ROOT / SF95_FILES[0][0])
            or sf95.get('licenses') != [{'expression': 'MIT'}]
            or details.get('steam-patch:reference-commit') != SFEXTRACT_COMMIT):
        raise ValueError('Regenerate SBOM for the current SF95 worker before packaging')
    for source, packaged, prefix in SF95_FILES:
        digest = hashlib.sha256((ROOT / source).read_bytes()).hexdigest()
        if (details.get(f'steam-patch:{prefix}-path') != packaged
                or details.get(f'steam-patch:{prefix}-sha256') != digest):
            raise ValueError(f'Regenerate SBOM for current {source} before packaging')
        if decky:
            bundled = plugin / packaged
            if not bundled.is_file() or hashlib.sha256(bundled.read_bytes()).hexdigest() != digest:
                raise ValueError(f'Rebuild Decky plugin for current {source} before packaging')


def archive(name, entries):
    target = OUTPUT / name
    with zipfile.ZipFile(target, 'w', compression=zipfile.ZIP_DEFLATED) as package:
        for source, destination, executable in entries:
            info = zipfile.ZipInfo(destination)
            info.create_system = 3
            info.external_attr = (0o100755 if executable else 0o100644) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            package.writestr(info, source.read_bytes())
    return {'file': target.name, 'size': target.stat().st_size,
            'sha256': hashlib.sha256(target.read_bytes()).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform', choices=['all', 'windows', 'decky'], default='all')
    platform = parser.parse_args().platform
    OUTPUT.mkdir(exist_ok=True)
    windows = [(ROOT / 'target/release/steam-patch-windows.exe', 'steam-patch-windows.exe', True),
               (ROOT / 'target/release/steam-patch.exe', 'steam-patch.exe', True),
               (ROOT / 'target/release/sf-worker.exe', 'sf-worker.exe', True)]
    for name in ['catalog.json', 'signed-catalog.json', 'trust-keys.json']:
        windows.append((ROOT / 'catalog' / name, 'resources/' + name, False))
    windows.append((ROOT / 'docs/client-guide.md', 'client-guide.md', False))
    windows.append((ROOT / 'docs/privacy.md', 'privacy.md', False))
    plugin = ROOT / 'apps/decky/plugin-output'
    if platform in ('all', 'decky') and not plugin.is_dir():
        raise FileNotFoundError('Build apps/decky package:plugin first')
    decky = [(path, 'steam-patch-manager/' + path.relative_to(plugin).as_posix(),
              path.relative_to(plugin).as_posix() == 'bin/steam-patch')
             for path in sorted(plugin.rglob('*')) if path.is_file()]
    sbom = OUTPUT / 'dependencies.cdx.json'
    if sbom.is_file():
        validate_sbom(sbom, plugin, platform in ('all', 'decky'))
        windows.append((sbom, sbom.name, False))
        decky.append((sbom, 'steam-patch-manager/' + sbom.name, False))
    selected = []
    if platform in ('all', 'windows'):
        selected.append(('steam-patch-manager-windows-portable.zip', windows))
    if platform in ('all', 'decky'):
        selected.append(('steam-patch-manager-decky.zip', decky))
    for _, entries in selected:
        for source, _, _ in entries:
            if not source.is_file():
                raise FileNotFoundError(f'Missing build input: {source}')
    result = [archive(name, entries) for name, entries in selected]
    checksum_name = 'checksums.json' if platform == 'all' else f'checksums-{platform}.json'
    (OUTPUT / checksum_name).write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
