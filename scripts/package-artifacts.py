"""Package already-built clients without game assets or private signing keys."""
from pathlib import Path
import hashlib
import json
import zipfile

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / '.artifacts'


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
    OUTPUT.mkdir(exist_ok=True)
    windows = [(ROOT / 'target/release/steam-patch-windows.exe', 'steam-patch-windows.exe', True),
               (ROOT / 'target/release/steam-patch.exe', 'steam-patch.exe', True)]
    for name in ['catalog.json', 'signed-catalog.json', 'trust-keys.json']:
        windows.append((ROOT / 'catalog' / name, 'resources/' + name, False))
    windows.append((ROOT / 'docs/client-guide.md', 'client-guide.md', False))
    plugin = ROOT / 'apps/decky/plugin-output'
    if not plugin.is_dir():
        raise FileNotFoundError('Build apps/decky package:plugin first')
    decky = [(path, 'steam-patch-manager/' + path.relative_to(plugin).as_posix(),
              path.relative_to(plugin).as_posix() == 'bin/steam-patch')
             for path in sorted(plugin.rglob('*')) if path.is_file()]
    result = [archive('steam-patch-manager-windows-portable.zip', windows),
              archive('steam-patch-manager-decky.zip', decky)]
    (OUTPUT / 'checksums.json').write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
