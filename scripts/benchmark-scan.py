"""Measure manifest-only discovery in an isolated synthetic two-library fixture.

Usage: python benchmark-scan.py CORE_BINARY [FIXTURE_PARENT]
The first run is not a cold-cache measurement: fixture creation warms filesystem caches.
"""
import json
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile
import time


def quoted(value):
    return json.dumps(str(value), ensure_ascii=False)


def main():
    binary = Path(sys.argv[1]).resolve(strict=True)
    parent = Path(sys.argv[2] if len(sys.argv) > 2 else '.local').resolve(strict=True)
    count = 500
    with tempfile.TemporaryDirectory(prefix='steam-scan-benchmark-', dir=parent) as directory:
        fixture = Path(directory).resolve()
        if fixture.parent != parent:
            raise ValueError('Fixture must remain inside requested parent')
        libraries = [fixture / 'Steam 测试库', fixture / '另一个 Steam 库']
        for library in libraries:
            (library / 'steamapps/common').mkdir(parents=True)
        index = '"libraryfolders" { ' + ' '.join(
            f'"{i}" {{ "path" {quoted(library)} }}' for i, library in enumerate(libraries)
        ) + ' }'
        (libraries[0] / 'steamapps/libraryfolders.vdf').write_text(index, encoding='utf-8')
        for i in range(count):
            appid = 1000000 + i
            library = libraries[i % 2]
            name = f'基准游戏 {i:04d}'
            (library / 'steamapps/common' / name).mkdir()
            manifest = f'"AppState" {{ "appid" "{appid}" "name" {quoted(name)} "installdir" {quoted(name)} "buildid" "1" "StateFlags" "4" }}'
            (library / f'steamapps/appmanifest_{appid}.acf').write_text(manifest, encoding='utf-8')
        request = {'protocol_version': 1, 'request_id': 'benchmark', 'method': 'scan',
                   'params': {'roots': [str(libraries[0])]}}
        durations = []
        for _ in range(6):
            started = time.perf_counter()
            process = subprocess.run([str(binary), 'serve'], input=json.dumps(request).encode()+b'\n',
                                     capture_output=True, check=True, timeout=120)
            durations.append(time.perf_counter() - started)
            response = json.loads(process.stdout)
            assert response['ok'], response
            result = response['result']
            assert not result['diagnostics'], result['diagnostics']
            assert sorted(row['game']['appid'] for row in result['installs']) == list(range(1000000, 1000000+count))
            assert len({row['game']['library_path'] for row in result['installs']}) == 2
        print(json.dumps({'manifests': count, 'libraries': 2, 'matched_patches': 0,
                          'platform': sys.platform, 'first_run_seconds': durations[0],
                          'warm_median_seconds': statistics.median(durations[1:]),
                          'warm_max_seconds': max(durations[1:]),
                          'runs_seconds': durations, 'cold_cache_measured': False}, indent=2))


if __name__ == '__main__':
    main()
