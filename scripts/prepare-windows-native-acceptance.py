"""Prepare a signed ZIP and synthetic Steam library for Windows native acceptance.

Writes only a new OS temporary directory and an ignored evidence directory.
Requires the already-built release CLI; never scans or mutates a real Steam library.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--core', type=Path, default=ROOT / 'target/release/patchhub-core.exe')
    args = parser.parse_args()
    core = args.core.resolve(strict=True)
    base = Path(tempfile.mkdtemp(prefix='patchhub-windows-acceptance-'))
    library = base / 'Steam'
    game = library / 'steamapps/common/Native acceptance'
    game.mkdir(parents=True)
    (library / 'steamapps/appmanifest_42.acf').write_text(
        '"AppState" { "appid" "42" "name" "Windows 临时验收" "installdir" "Native acceptance" '
        '"buildid" "100" "StateFlags" "4" "UserConfig" { "language" "schinese" } }', encoding='utf-8')
    (game / 'patch.dat').write_bytes(b'original temporary data')
    (game / 'save.dat').write_bytes(b'untouched synthetic save')
    archive = base / 'patch.zip'
    # The previous 142 MB fixture exposed multiple progress samples. Use a similarly
    # sized stored member to observe native close during writing without CPU-heavy compression.
    block = bytes(range(256)) * 4096
    installed = hashlib.sha256()
    with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_STORED) as zipped:
        with zipped.open('patch.dat', 'w') as member:
            for _ in range(128):
                member.write(block)
                installed.update(block)
    with archive.open('rb') as stream:
        archive_hash = hashlib.file_digest(stream, 'sha256').hexdigest()
    catalog = {'schema_version': 1, 'revision': 1, 'releases': [{
        'patch_id': 'native-acceptance', 'release_id': 'native-acceptance-v1', 'appid': 42,
        'name': '临时 ZIP 原生关闭验收', 'version': '1', 'language': 'zh-CN',
        'category': 'content_restore', 'source_page': 'https://example.invalid/acceptance',
        'support_level': 'managed_zip', 'platforms': ['windows'], 'verified_buildids': ['100'],
        'detection': [{'path': 'patch.dat', 'sha256': installed.hexdigest()}],
        'recipe': {'artifact_sha256': archive_hash, 'files': [{
            'source': 'patch.dat', 'destination': 'patch.dat', 'original_sha256': None,
            'installed_sha256': installed.hexdigest()}]}}]}
    catalog_path = base / 'catalog.json'
    catalog_path.write_text(json.dumps(catalog), encoding='utf-8')
    for command in [
        ['keygen', str(base / 'private'), str(base / 'keys.json'), 'native-acceptance'],
        ['sign-catalog', str(catalog_path), str(base / 'private'), 'native-acceptance', str(base / 'signed.json'), '86400'],
    ]:
        subprocess.run([str(core), *command], check=True, capture_output=True)
    evidence = ROOT / '.artifacts/windows-flutter/p7-native' / base.name
    evidence.mkdir(parents=True)
    spec = {'binary': str(core), 'state': str(base / 'state'), 'resources': str(ROOT / 'catalog'),
            'roots': [str(library)], 'signed_catalog': str(base / 'signed.json'),
            'trust_keys': str(base / 'keys.json'), 'appid': 42, 'release_id': 'native-acceptance-v1',
            'archive': str(archive), 'game_directory': str(game), 'evidence_directory': str(evidence)}
    spec_path = evidence / 'spec.json'
    spec_path.write_text(json.dumps(spec, ensure_ascii=False, indent=2), encoding='utf-8')
    print(spec_path)

if __name__ == '__main__':
    main()
