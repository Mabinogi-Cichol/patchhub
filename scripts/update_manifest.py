"""Build the shared Windows ZIP/NSIS managed-file contract."""
import json
from pathlib import Path
import shutil
from project_version import project_version

ROOT = Path(__file__).resolve().parents[1]


def prepare(bundle, version=None, *, root=ROOT):
    bundle = Path(bundle)
    version = version or project_version(root)
    shutil.copyfile(root / 'scripts/windows-update.ps1', bundle / 'windows-update.ps1')
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
