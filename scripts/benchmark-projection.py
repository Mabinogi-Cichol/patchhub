"""Protocol v4 projection benchmark; instruments a disposable source copy only.

Timings use the plain debug binary. Counters use a separate binary whose response
must equal the plain response. No user state, network or production source writes.
Hash counters cover the shared core evidence reader. Historical P0/P1 runs remain
in their original evidence files and Git revisions.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import tempfile
import time
import zipfile
import threading

REPO = Path(__file__).resolve().parents[1]
CRATES = ('core', 'cli', 'protocol', 'installer-format', 'installer-worker')
MARKER = 'P0_HASH_COUNTS:'

def digest(data):
    return hashlib.sha256(data).hexdigest()

def once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('Instrumentation anchor drifted: ' + old[:90])
    return text.replace(old, new, 1)

def instrument(path, start_anchor, end_anchor):
    text = path.read_text(encoding='utf-8')
    start = text.index(start_anchor)
    end = text.index(end_anchor, start) if end_anchor else len(text)
    part = text[start:end]
    brace = part.index('{') + 1
    part = part[:brace] + '\n    P0_ATTEMPTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);' + part[brace:]
    part = once(part, 'hasher.update(&buffer[..count]);', 'P0_BYTES.fetch_add(count as u64, std::sync::atomic::Ordering::Relaxed);\n        hasher.update(&buffer[..count]);')
    anchor = 'format!("{:x}", hasher.finalize())'
    part = once(part, anchor, '{ P0_COMPLETED.fetch_add(1, std::sync::atomic::Ordering::Relaxed); ' + anchor + ' }')
    counters = '''
static P0_ATTEMPTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static P0_COMPLETED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static P0_BYTES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub fn p0_hash_counts() -> [u64; 3] {
    let order = std::sync::atomic::Ordering::Relaxed;
    [P0_ATTEMPTS.load(order), P0_COMPLETED.load(order), P0_BYTES.load(order)]
}
'''
    path.write_text(text[:start] + part + text[end:] + counters, encoding='utf-8')

def build_pair(scratch, cargo, profile):
    source = scratch / 'source'
    source.mkdir()
    manifest = (REPO / 'Cargo.toml').read_text(encoding='utf-8')
    manifest, count = re.subn(r'members = \[[^\n]+\]', 'members = ' + json.dumps(['crates/' + c for c in CRATES]), manifest, count=1)
    if count != 1:
        raise ValueError('workspace members anchor drifted')
    (source / 'Cargo.toml').write_text(manifest, encoding='utf-8')
    shutil.copy2(REPO / 'Cargo.lock', source / 'Cargo.lock')
    (source / 'catalog').mkdir()
    shutil.copy2(REPO / 'catalog/trust-keys.json', source / 'catalog/trust-keys.json')
    for c in CRATES:
        dest = source / 'crates' / c
        dest.mkdir(parents=True)
        shutil.copy2(REPO / 'crates' / c / 'Cargo.toml', dest / 'Cargo.toml')
        shutil.copytree(REPO / 'crates' / c / 'src', dest / 'src')
        resources = REPO / 'crates' / c / 'resources'
        if resources.is_dir():
            shutil.copytree(resources, dest / 'resources')
    env = os.environ.copy()
    if cargo == REPO / '.tools/cargo/bin/cargo.exe':
        env['CARGO_HOME'] = str(REPO / '.tools/cargo')
        env['RUSTUP_HOME'] = str(REPO / '.tools/rustup')
    env['PATH'] = str(cargo.parent) + os.pathsep + env.get('PATH', '')
    target = REPO / '.local/p0-projection-target'
    env['CARGO_TARGET_DIR'] = str(target)
    suffix = '.exe' if os.name == 'nt' else ''
    def build(label):
        print('Building isolated ' + label + ' ' + profile + ' CLI', flush=True)
        command = [str(cargo), 'build', '--offline', '-p', 'steam-patch-cli']
        if profile == 'release':
            command.append('--release')
        subprocess.run(command, cwd=source, env=env, check=True, timeout=900)
        binary = scratch / (label + suffix)
        shutil.copy2(target / profile / ('steam-patch' + suffix), binary)
        return binary
    plain = build('plain')
    instrument(source / 'crates/core/src/detection.rs', 'fn read_digest_with(', '\n#[cfg(test)]\nmod tests')
    count_expression = 'serde_json::json!({"evidence": steam_patch_core::detection::p0_hash_counts()})'
    main = source / 'crates/cli/src/main.rs'
    text = once(main.read_text(encoding='utf-8'), 'fn main() -> ExitCode {', '''fn main() -> ExitCode {
    struct P0Summary;
    impl Drop for P0Summary {
        fn drop(&mut self) {
            eprintln!("P0_HASH_COUNTS:{}", COUNT_EXPRESSION);
        }
    }
    let _p0_summary = P0Summary;'''.replace('COUNT_EXPRESSION', count_expression))
    main.write_text(text, encoding='utf-8')
    return plain, build('counted'), True

def fixture(parent, name, manifests, files, mixed, library_names=('Steam 测试库', '第二库')):
    directory = parent / name
    libraries = [directory / name for name in library_names]
    for lib in libraries:
        (lib / 'steamapps/common').mkdir(parents=True)
    quote = lambda x: json.dumps(str(x), ensure_ascii=False)
    index = '"libraryfolders" { ' + ' '.join(f'"{i}" {{ "path" {quote(lib)} }}' for i, lib in enumerate(libraries)) + ' }'
    (libraries[0] / 'steamapps/libraryfolders.vdf').write_text(index, encoding='utf-8')
    payload = bytes(range(256)) * 1024
    sha = digest(payload)
    releases = []
    for i in range(manifests):
        appid = 1000000 + i // 2  # Two instances per AppID, in distinct libraries.
        lib = libraries[i % 2]
        game = lib / 'steamapps/common' / f'Game {i // 2:04d}'
        game.mkdir()
        acf = f'"AppState" {{ "appid" "{appid}" "name" "Fixture {appid}" "installdir" "{game.name}" "buildid" "1" "StateFlags" "4" }}'
        (lib / f'steamapps/appmanifest_{appid}.acf').write_text(acf, encoding='utf-8')
        mappings = []
        for f in range(files):
            path = f'payload-{f}.bin'
            target = game / path
            if mixed and f == 2:
                pass
            elif mixed and f == 3:
                target.mkdir()  # Unsafe/not-file evidence must be unknown.
            else:
                target.write_bytes(payload if not mixed or f != 1 else payload[::-1])
            mappings.append({'source': path, 'destination': path, 'original_sha256': None, 'installed_sha256': sha})
        if files and i % 2 == 0:
            releases.append({'patch_id': f'patch-{appid}', 'release_id': f'release-{appid}', 'appid': appid,
                             'name': f'Patch {appid}', 'version': '1', 'language': 'en', 'category': 'fixture',
                             'source_page': 'https://example.invalid/fixture', 'support_level': 'managed_zip',
                             'platforms': ['windows', 'steamos'], 'verified_buildids': ['1'],
                             'detection': [{'path': 'payload-0.bin', 'sha256': sha}],
                             'recipe': {'artifact_sha256': '0'*64, 'files': mappings}, 'artifact': None})
    catalog = directory / 'catalog.json'
    catalog.write_text(json.dumps({'schema_version': 1, 'revision': 1, 'releases': releases}), encoding='utf-8')
    return {'roots': [str(libraries[0])], 'catalog': str(catalog),
            'platform': 'windows' if os.name == 'nt' else 'steamos'}

def rpc(binary, method, params, version):
    request_id = params.get('request_id', 'p0-projection')
    params = dict(params)
    params.pop('request_id', None)
    platform = 'windows' if os.name == 'nt' else 'steamos'
    host = {key: params.pop(key, [] if key == 'roots' else platform if key == 'platform' else None)
            for key in ('roots', 'catalog', 'signed_catalog', 'trust_keys', 'state_dir', 'platform')}
    if method == 'install_patch':
        params['cleanup_policy'] = 'disabled'
    params['host'] = host
    request = {'protocol_version': version, 'request_id': request_id, 'method': method, 'params': params}
    data = json.dumps(request).encode() + b'\n'
    if len(data) > 64*1024:
        raise ValueError('request exceeds protocol bound')
    started = time.perf_counter()
    if method == 'install_patch':
        # Keep stdin open while the task runs: EOF is an intentional cancel.
        task = subprocess.Popen([str(binary), 'serve-task'], stdin=subprocess.PIPE,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        deadline = threading.Timer(120, task.kill)
        deadline.start()
        try:
            task.stdin.write(data)
            task.stdin.flush()
            stdout = task.stdout.read(8*1024*1024 + 1)
            stderr = task.stderr.read()
            code = task.wait(timeout=5)
            task.stdin.close()
            if code:
                raise ValueError('fixture install task failed: ' + stderr.decode('utf-8'))
            process = subprocess.CompletedProcess(task.args, code, stdout, stderr)
        finally:
            deadline.cancel()
            if task.poll() is None:
                task.kill()
                task.wait()
    else:
        process = subprocess.run([str(binary), 'serve'], input=data, capture_output=True, check=True, timeout=120)
    elapsed = time.perf_counter() - started
    if len(process.stdout) > 8*1024*1024:
        raise ValueError('response exceeds protocol bound')
    lines = [json.loads(line) for line in process.stdout.splitlines()]
    terminals = [line for line in lines if 'ok' in line]
    if len(terminals) != 1:
        raise ValueError('expected one terminal RPC response')
    response = terminals[0]
    if (response.get('protocol_version') != version or not response.get('ok')
            or response.get('request_id') != request_id):
        raise ValueError(response)
    return response, elapsed, len(process.stdout), process.stderr.decode('utf-8')

def scan(binary, params, version):
    return rpc(binary, 'scan', params, version)

def dynamic_fixture(parent, binary, version):
    """Seed only disposable state/game fixtures, through the real signed installer.

    The scan then removes catalog input, projecting the source-scoped receipt.
    No download or executable worker is used; archives and signing keys are local.
    """
    params = fixture(parent, 'dynamic-managed', 8, 4, False, ('a', 'b'))
    directory = Path(params['catalog']).parent
    catalog = json.loads(Path(params['catalog']).read_text(encoding='utf-8'))
    payload = bytes(range(256)) * 1024
    archive = directory / 'fixture.zip'
    with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_STORED) as output:
        for index in range(4):
            output.writestr(f'payload-{index}.bin', payload)
    for release in catalog['releases']:
        release['release_id'] = f"patchmoe-{release['appid']}"
        release['recipe']['artifact_sha256'] = digest(archive.read_bytes())
    Path(params['catalog']).write_text(json.dumps(catalog), encoding='utf-8')
    for manifest in directory.rglob('appmanifest_*.acf'):
        text = manifest.read_text(encoding='utf-8')
        text = text.rstrip().removesuffix('}') + ' "UserConfig" { "language" "english" } }'
        manifest.write_text(text, encoding='utf-8')
    private, keys, signed = (directory / name for name in ('fixture.key', 'keys.json', 'signed.json'))
    subprocess.run([str(binary), 'keygen', str(private), str(keys), 'fixture-key'], check=True, capture_output=True)
    subprocess.run([str(binary), 'sign-catalog', params['catalog'], str(private), 'fixture-key', str(signed), '3600'], check=True, capture_output=True)
    state = directory / 'state'
    rows = scan(binary, {'roots': params['roots']}, version)[0]['result']['installs']
    for index, row in enumerate(rows):
        installed = rpc(binary, 'install_patch', {
            'roots': params['roots'], 'instance_id': row['game']['instance_id'],
            'release_id': f"patchmoe-{row['game']['appid']}", 'signed_catalog': str(signed),
            'trust_keys': str(keys), 'state_dir': str(state), 'archive': str(archive),
            'operation': 'overwrite', 'request_id': f'fixture-install-{index}'}, version)[0]
        if (installed['result']['disposition'] != 'completed'
                or installed['result']['state'] != 'committed'):
            raise ValueError(installed)
    rpc(binary, 'list_sources', {'roots': params['roots'], 'state_dir': str(state)}, version)
    now = int(time.time())
    entries = [{'title': f"Fixture {release['appid']}", 'aliases': [], 'appid': release['appid'],
                'url': 'https://download.patch.moe/fixture.zip',
                'source_page': 'https://www.patch.moe/fixture', 'language': 'en', 'date': None}
               for release in catalog['releases']]
    cache = state / 'sources/rust-v1/cache/patchmoe.json'
    cache.parent.mkdir(exist_ok=True)
    cache.write_text(json.dumps({'entries': entries, 'etag': None, 'modified': None,
                                'cached_at': now, 'checked_at': now}), encoding='utf-8')
    return {'roots': params['roots'], 'state_dir': str(state), 'platform': params['platform']}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--runs', type=int, default=6)
    parser.add_argument('--cargo', type=Path, default=REPO / '.tools/cargo/bin/cargo.exe')
    parser.add_argument('--binary', type=Path, help='Current CLI for fixture timing only; hash counts remain unmeasured')
    parser.add_argument('--protocol-version', type=int, choices=[4], default=4)
    parser.add_argument('--profile', choices=['debug', 'release'], default='debug')
    parser.add_argument('--real-root', type=Path, action='append', default=[], help='Explicit Steam root; manifest-only, no state/catalog, read-only')
    args = parser.parse_args()
    if args.runs < 2:
        parser.error('--runs must be >= 2')
    parent = REPO / '.local'
    parent.mkdir(exist_ok=True)
    paths = [REPO / 'Cargo.toml', REPO / 'Cargo.lock', REPO / 'catalog/trust-keys.json'] + [p for c in CRATES for p in sorted((REPO / 'crates' / c).rglob('*')) if p.is_file() and (p.suffix == '.rs' or p.name == 'Cargo.toml' or 'resources' in p.parts)]
    before = {str(p.relative_to(REPO)): digest(p.read_bytes()) for p in paths}
    cases = []
    snapshots = {}
    with tempfile.TemporaryDirectory(prefix='projection-p0-', dir=parent) as temporary:
        scratch = Path(temporary).resolve()
        if scratch.parent != parent.resolve():
            raise ValueError('scratch escapes fixture parent')
        if args.binary:
            plain, counted, shared_evidence = args.binary.resolve(strict=True), None, True
        else:
            # Rustup dispatches by argv[0]; resolving cargo's Unix symlink to rustup
            # would turn `cargo build` into the unrelated `rustup build` command.
            cargo = args.cargo.absolute()
            if not cargo.is_file():
                raise FileNotFoundError(cargo)
            plain, counted, shared_evidence = build_pair(scratch, cargo, args.profile)
        binary_hashes = {'plain': digest(plain.read_bytes()), 'counted': digest(counted.read_bytes()) if counted else None}
        for name, manifests, files, mixed in [('manifest-only', 500, 0, False), ('recipe-all-match', 24, 4, False), ('recipe-mixed-evidence', 24, 4, True)]:
            params = fixture(scratch, name, manifests, files, mixed)
            durations, expected = [], None
            for _ in range(args.runs):
                response, elapsed, size, stderr = scan(plain, params, args.protocol_version)
                if stderr or (expected is not None and response != expected):
                    raise ValueError('unexpected stderr or unstable projection: ' + stderr)
                expected = response
                durations.append(elapsed)
            rows = response['result']['installs']
            assert len(rows) == manifests
            assert len({r['game']['instance_id'] for r in rows}) == manifests
            assert len({r['game']['library_path'] for r in rows}) == 2
            if not mixed:
                assert not response['result']['diagnostics']
            assert all(len(r['patches']) == (1 if files else 0) for r in rows)
            if files:
                assert all(r['patches'][0]['detection']['status'] == ('unknown' if mixed else 'installed_external') for r in rows)
                statuses = ['match', 'mismatch', 'missing', 'unreadable_or_unsafe'] if mixed else ['match']*files
                assert all([f['status'] for f in r['patches'][0]['detection']['files']] == statuses for r in rows)
            def normalize(value):
                if isinstance(value, str):
                    return value.replace(str(scratch), '$FIXTURE')
                if isinstance(value, list):
                    return [normalize(x) for x in value]
                if isinstance(value, dict):
                    return {k: normalize(v) for k, v in value.items()}
                return value
            snapshots[name] = normalize(expected['result'])
            counts = None
            if counted:
                result, _, _, stderr = scan(counted, params, args.protocol_version)
                if result != expected:
                    raise ValueError('instrumentation changed projection')
                lines = stderr.splitlines()
                if len(lines) != 1 or not lines[0].startswith(MARKER):
                    raise ValueError('invalid counter output: ' + stderr)
                counts = json.loads(lines[0][len(MARKER):])
                complete = manifests * (2 if mixed else files)
                expected_counts = {'evidence': [manifests*files, complete, complete*262144]}
                assert counts == expected_counts, counts
            cases.append({'fixture': name, 'manifests': manifests, 'libraries': 2, 'unique_appids': manifests//2,
                          'files_per_patch': files, 'payload_bytes': 262144 if files else 0, 'response_bytes': size,
                          'first_run_seconds': durations[0], 'warm_median_seconds': statistics.median(durations[1:]),
                          'warm_max_seconds': max(durations[1:]), 'runs_seconds': durations,
                          'hash_counts_attempts_completed_bytes': counts, 'counted_response_equal': True if counted else None})
        if shared_evidence:
            params = dynamic_fixture(scratch, plain, args.protocol_version)
            durations, expected = [], None
            for _ in range(args.runs):
                response, elapsed, size, stderr = scan(plain, params, args.protocol_version)
                if stderr or (expected is not None and response != expected):
                    raise ValueError('unstable dynamic projection: ' + stderr)
                expected = response
                durations.append(elapsed)
            rows = response['result']['installs']
            assert len(rows) == 8 and all(len(r['patches']) == 1 for r in rows)
            assert all(r['patches'][0]['detection']['status'] == 'installed_managed' for r in rows)
            assert all(len(r['patches'][0]['detection']['files']) == 4 for r in rows)
            counts = None
            if counted:
                result, _, _, stderr = scan(counted, params, args.protocol_version)
                if result != expected:
                    raise ValueError('instrumentation changed dynamic projection')
                lines = stderr.splitlines()
                assert len(lines) == 1 and lines[0].startswith(MARKER), stderr
                counts = json.loads(lines[0][len(MARKER):])
                assert counts == {'evidence': [32, 32, 32*262144]}, counts
            snapshots['dynamic-managed'] = normalize(expected['result'])
            cases.append({'fixture': 'dynamic-managed', 'manifests': 8, 'libraries': 2, 'unique_appids': 4,
                          'files_per_patch': 4, 'payload_bytes': 262144, 'response_bytes': size,
                          'first_run_seconds': durations[0], 'warm_median_seconds': statistics.median(durations[1:]),
                          'warm_max_seconds': max(durations[1:]), 'runs_seconds': durations,
                          'hash_counts_attempts_completed_bytes': counts, 'counted_response_equal': True if counted else None})
        if args.real_root:
            params = {'roots': [str(p.resolve(strict=True)) for p in args.real_root]}
            durations = []
            for _ in range(args.runs):
                response, elapsed, size, stderr = scan(plain, params, args.protocol_version)
                if stderr:
                    raise ValueError(stderr)
                durations.append(elapsed)
            result = response['result']
            selected = [r['game'] for r in result['installs'] if r['game']['appid'] in (2743810, 2303800, 3137110)]
            cases.append({'fixture': 'real-manifest-only', 'roots': params['roots'],
                          'manifests': len(result['installs']), 'target_instances': selected,
                          'diagnostics': result['diagnostics'], 'response_bytes': size,
                          'runs_seconds': durations, 'first_run_seconds': durations[0],
                          'warm_median_seconds': statistics.median(durations[1:]),
                          'warm_max_seconds': max(durations[1:]), 'hash_counts_attempts_completed_bytes': None})
    after = {str(p.relative_to(REPO)): digest(p.read_bytes()) for p in paths}
    if before != after:
        raise ValueError('production sources changed during baseline')
    report = {'schema_version': 1, 'protocol_version': args.protocol_version, 'platform': os.name,
              'profile': args.profile if not args.binary else 'caller-supplied',
              'clock': 'perf_counter: process startup + serve + full scan/serialization', 'cold_cache_measured': False,
              'network_used': False, 'installer_state_used': bool(shared_evidence), 'installer_state_scope': 'disposable fixture only',
              'steam_metadata_read': bool(args.real_root), 'game_files_written': False, 'fixture_game_files_written': True,
              'hash_scope': 'shared core detection read_digest; excludes manifest/identity hashing',
              'production_sources_unchanged': True, 'source_sha256': before, 'binary_sha256': binary_hashes, 'cases': cases}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    snapshot_path = args.output.with_name(args.output.stem + '-snapshots.json')
    snapshot_path.write_text(json.dumps(snapshots, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    report['fixture_snapshots'] = snapshot_path.name
    report['fixture_snapshots_sha256'] = digest(snapshot_path.read_bytes())
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'output': str(args.output), 'cases': cases}, ensure_ascii=False, indent=2))

if __name__ == '__main__':
    main()
