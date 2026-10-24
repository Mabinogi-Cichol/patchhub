"""Build the shared Windows ZIP/NSIS managed-file contract."""
import json
from pathlib import Path
import re
import shutil
import os

ROOT = Path(__file__).resolve().parents[1]


def project_version():
    cargo = re.search(r'(?m)^version = "([^"]+)"', (ROOT / 'Cargo.toml').read_text(encoding='utf-8'))[1]
    flutter = re.search(r'(?m)^version: ([^+\s]+)', (ROOT / 'apps/windows-flutter/pubspec.yaml').read_text(encoding='utf-8'))[1]
    decky = json.loads((ROOT / 'apps/decky/package.json').read_text(encoding='utf-8'))['version']
    if len({cargo, flutter, decky}) != 1:
        raise ValueError('Cargo/Flutter/Decky versions differ')
    tag = os.environ.get('GITHUB_REF', '')
    if tag.startswith('refs/tags/') and tag != f'refs/tags/v{cargo}':
        raise ValueError('Release Tag differs from application version')
    return cargo


def prepare(bundle, version=None):
    bundle = Path(bundle)
    version = version or project_version()
    shutil.copyfile(ROOT / 'scripts/windows-update.ps1', bundle / 'windows-update.ps1')
    (bundle / 'app-version.json').write_text(json.dumps({'version': version}) + '\n', encoding='utf-8')
    files = sorted({p.relative_to(bundle).as_posix() for p in bundle.rglob('*') if p.is_file()
                    and p.name not in ('install.ini', 'uninstall.exe', 'update-manifest.json',
                                       '.patchhub-maintenance.lock', '.patchhub-update.json')}
                   | {'update-manifest.json'})
    (bundle / 'update-manifest.json').write_text(json.dumps({'schema': 1, 'version': version, 'files': files}, indent=2) + '\n', encoding='utf-8')
    return files


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument('bundle', type=Path)
    args = parser.parse_args()
    prepare(args.bundle)
