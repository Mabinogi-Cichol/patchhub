"""Package already-built clients without game assets or private signing keys."""
from pathlib import Path
import argparse
import hashlib
import json
import zipfile
from update_manifest import prepare as prepare_update

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / '.artifacts'
INSTALLER_FILES = (
    ('crates/installer-format/Cargo.toml', None, 'format-manifest'),
    ('crates/installer-worker/Cargo.toml', None, 'worker-manifest'),
    ('LICENSE', 'LICENSE', 'project-license'),
    ('scripts/licenses/sfextract-MIT.txt', 'licenses/sfextract-MIT.txt', 'reference-license'),
)
SFEXTRACT_COMMIT = '21f5c5595a70e9284fff83476c0e307d5b92f573'
PRODUCTION_BACKEND = {'bridge.py', 'source_service.py', 'tasks.py', 'plugin_settings.py', 'state_host.py', 'protocol.py', 'protocol_validation.py', 'v5.schema.json', 'app_update.py', 'install-decky.py'}


def validate_sbom(sbom, plugin, decky):
    data = json.loads(sbom.read_text(encoding='utf-8'))
    try:
        properties = {item['name']: item['value'] for item in data['metadata']['properties']}
        matches = [item for item in data['components'] if item.get('name') == 'steam-patch-rust-installer']
    except (KeyError, TypeError) as error:
        raise ValueError('Regenerate SBOM for current dependencies before packaging') from error
    for name, path in [('cargo', 'Cargo.lock'), ('npm', 'package-lock.json')]:
        if properties.get(f'steam-patch:{name}-lock-sha256') != hashlib.sha256((ROOT / path).read_bytes()).hexdigest():
            raise ValueError('Regenerate SBOM for current lockfiles before packaging')
    if len(matches) != 1:
        raise ValueError('Regenerate SBOM with the Rust installer before packaging')
    installer = matches[0]
    details = {item['name']: item['value'] for item in installer.get('properties', [])}
    if (installer.get('version') != '1' or installer.get('licenses') != [{'expression': 'MIT'}]
            or details.get('steam-patch:reference-commit') != SFEXTRACT_COMMIT):
        raise ValueError('Regenerate SBOM for the current Rust installer before packaging')
    for source, packaged, prefix in INSTALLER_FILES:
        digest = hashlib.sha256((ROOT / source).read_bytes()).hexdigest()
        if (details.get(f'steam-patch:{prefix}-source') != source
                or details.get(f'steam-patch:{prefix}-sha256') != digest
                or (packaged and details.get(f'steam-patch:{prefix}-path') != packaged)):
            raise ValueError(f'Regenerate SBOM for current {source} before packaging')
        if decky and packaged:
            bundled = plugin / packaged
            if not bundled.is_file() or hashlib.sha256(bundled.read_bytes()).hexdigest() != digest:
                raise ValueError(f'Rebuild Decky plugin for current {source} before packaging')


def validate_decky(plugin):
    actual = {path.name for path in (plugin / 'backend').iterdir()}
    if actual != PRODUCTION_BACKEND:
        raise ValueError('Rebuild Decky plugin: unexpected or missing production backend modules')
    for name in ('protocol.py', 'v5.schema.json'):
        source = ROOT / 'packages/contracts/generated' / name
        bundled = plugin / 'backend' / name
        if source.read_bytes() != bundled.read_bytes():
            raise ValueError('Rebuild Decky plugin: generated contract mismatch')
    runtime = json.loads((plugin / 'bin/runtime.json').read_text(encoding='utf-8'))
    if runtime.get('protocol_version') != 5 or runtime.get('worker_protocol') != 1:
        raise ValueError('Rebuild Decky plugin: invalid runtime manifest')
    files = runtime.get('files', [])
    if {item.get('path') for item in files} != {'bin/steam-patch', 'bin/sf-worker'} or len(files) != 2:
        raise ValueError('Rebuild Decky plugin: incomplete runtime manifest')
    for item in files:
        binary = plugin / item['path']
        data = binary.read_bytes()
        if (not data.startswith(b'\x7fELF') or len(data) != item.get('size')
                or hashlib.sha256(data).hexdigest() != item.get('sha256')):
            raise ValueError('Rebuild Decky plugin: runtime hash mismatch')


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
    bundle = ROOT / 'apps/windows-flutter/build/windows/x64/runner/Release'
    windows_files = None
    if platform in ('all', 'windows'):
        # Add documentation/SBOM before finalizing the exact ZIP/NSIS inventory.
        import shutil
        if not (OUTPUT / 'dependencies.cdx.json').is_file():
            raise FileNotFoundError('Generate the current SBOM before packaging')
        for source, name in ((ROOT / 'docs/client-guide.md', 'client-guide.md'),
                             (ROOT / 'docs/privacy.md', 'privacy.md'),
                             (ROOT / 'LICENSE', 'LICENSE'),
                             (OUTPUT / 'dependencies.cdx.json', 'dependencies.cdx.json')):
            shutil.copyfile(source, bundle / name)
        (bundle / 'licenses').mkdir(exist_ok=True)
        shutil.copyfile(ROOT / 'scripts/licenses/sfextract-MIT.txt', bundle / 'licenses/sfextract-MIT.txt')
        windows_files = set(prepare_update(bundle))
    windows = [(path, path.relative_to(bundle).as_posix(), path.suffix == '.exe')
               for path in sorted(bundle.rglob('*')) if path.is_file()
               and (windows_files is None or path.relative_to(bundle).as_posix() in windows_files)
               and path.relative_to(bundle).as_posix() not in ('client-guide.md', 'privacy.md', 'LICENSE', 'licenses/sfextract-MIT.txt', 'dependencies.cdx.json')]
    for name in ('client-guide.md', 'privacy.md'):
        windows.append((ROOT / 'docs' / name, name, False))
    for source, packaged, _ in INSTALLER_FILES:
        if packaged:
            windows.append((ROOT / source, packaged, False))
    plugin = ROOT / 'apps/decky/plugin-output'
    if platform in ('all', 'decky'):
        if not plugin.is_dir():
            raise FileNotFoundError('Build apps/decky package:plugin first')
        validate_decky(plugin)
    decky = [(path, 'steam-patch-manager/' + path.relative_to(plugin).as_posix(),
              path.relative_to(plugin).as_posix() in ('bin/steam-patch', 'bin/sf-worker'))
             for path in sorted(plugin.rglob('*')) if path.is_file()]
    sbom = OUTPUT / 'dependencies.cdx.json'
    if not sbom.is_file():
        raise FileNotFoundError('Generate the current SBOM before packaging')
    if sbom.is_file():
        validate_sbom(sbom, plugin, platform in ('all', 'decky'))
        windows.append((sbom, sbom.name, False))
        decky.append((sbom, 'steam-patch-manager/' + sbom.name, False))
    selected = []
    if platform in ('all', 'windows'):
        for name in ('steam_patch_windows.exe', 'steam-patch.exe', 'sf-worker.exe',
                     'flutter_windows.dll', 'msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll', 'file_selector_windows_plugin.dll',
                     'url_launcher_windows_plugin.dll', 'window_manager_plugin.dll',
                     'screen_retriever_windows_plugin.dll', 'data/icudtl.dat', 'data/app.so',
                     'data/flutter_assets/AssetManifest.bin', 'data/flutter_assets/NOTICES.Z',
                     'data/flutter_assets/assets/protocol-v5.schema.json',
                     'resources/catalog.json', 'resources/signed-catalog.json', 'resources/trust-keys.json',
                     'resources/licenses/Flutter-LICENSE', 'resources/licenses/Dart-LICENSE'):
            if not (bundle / name).is_file():
                raise FileNotFoundError(f'Missing Flutter runtime input: {name}')
        if ((bundle / 'data/flutter_assets/assets/protocol-v5.schema.json').read_bytes()
                != (ROOT / 'packages/contracts/generated/v5.schema.json').read_bytes()):
            raise ValueError('Rebuild Flutter: generated contract mismatch')
        inventory = json.loads(sbom.read_text(encoding='utf-8'))
        properties = {p['name']: p['value'] for p in inventory['metadata']['properties']}
        if properties.get('steam-patch:dart-lock-sha256') != hashlib.sha256((ROOT / 'apps/windows-flutter/pubspec.lock').read_bytes()).hexdigest():
            raise ValueError('Regenerate SBOM with current Flutter dependencies before packaging')
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
