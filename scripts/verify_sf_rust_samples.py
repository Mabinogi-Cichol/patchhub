"""Offline Rust/Python parity through the production direct parser session.

Analyze metadata, then compare each streamed payload byte, CRC and SHA against
Python's independent decoder. No installer code or game directory is executed.
Use --sample APPID=PATH for local samples outside generic-exe-local.
"""
import argparse
import hashlib
import json
import mmap
from pathlib import Path
import shutil
import subprocess
import time

import sf95_audit as sf
from sf95_worker import audit_behavior

ROOT = Path(__file__).resolve().parents[1]
P2 = ROOT / '.artifacts/p2'
LOCAL = ROOT / '.artifacts/generic-exe-local'


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def diff(a, b, path='$'):
    if a == b: return None
    if isinstance(a, dict) and isinstance(b, dict):
        if a.keys() != b.keys(): return f'{path}: keys differ'
        for key in a:
            found = diff(a[key], b[key], path + '.' + key)
            if found: return found
    elif isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b): return f'{path}: lengths differ'
        for i, (v, w) in enumerate(zip(a, b)):
            found = diff(v, w, f'{path}[{i}]')
            if found: return found
    return f'{path}: {a!r} != {b!r}'


def verify_stream(path, source, report):
    members = []
    with path.open('rb') as stream, source.open('rb') as exe, mmap.mmap(exe.fileno(), 0, access=mmap.ACCESS_READ) as data:
        assert json.loads(stream.readline()) == {'extract': 1, 'members': len(report['files'])}
        decoder = sf.Decoder()
        class Compare:
            def write(self, chunk):
                assert stream.read(len(chunk)) == chunk, 'payload bytes differ'
        for i, row in enumerate(report['files']):
            assert json.loads(stream.readline()) == {'index': i, 'path': row['destination'], 'size': row['unpacked_size']}
            info, _ = decoder.decode(sf.Reader(data, row['payload_offset']), row['packed_size'],
                row['unpacked_size'], row['crc32'], compressed=row['compressed'], sink=Compare())
            assert json.loads(stream.readline()) == {'index': i, **info}, 'payload digest differs'
            members.append({'path': row['destination'], **info})
        assert json.loads(stream.readline()) == {'done': True}
        assert stream.read(1) == b'', 'trailing extraction data'
    return members


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--platform', choices=['windows', 'linux'], default='windows')
    parser.add_argument('--synthetic-only', action='store_true')
    parser.add_argument('--appid', action='append')
    parser.add_argument('--sample', action='append', default=[], metavar='APPID=PATH')
    args = parser.parse_args()
    runner = ROOT / ('target/release/examples/sf_worker_check.exe' if args.platform == 'windows' else 'target/linux/release/examples/sf_worker_check')
    worker = runner.parent.parent / ('patchhub-worker.exe' if args.platform == 'windows' else 'patchhub-worker')
    run = f'{args.platform}-{time.time_ns()}'
    P2.mkdir(parents=True, exist_ok=True)
    LOCAL.mkdir(parents=True, exist_ok=True)
    records = []

    def check(label, source, expected=None):
        root = P2 / (run + '-' + label)
        (root / 'input').mkdir(parents=True)
        shutil.copyfile(source, root / 'input/archive.exe')
        with source.open('rb') as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as data:
            reference = audit_behavior(data, decode_payloads=False)
        reference = json.loads(json.dumps(reference))
        if expected is not None: assert reference == expected, diff(reference, expected)
        (LOCAL / (label + '-behavior-analysis.json')).write_text(json.dumps(reference, ensure_ascii=True, indent=2), encoding='utf8')
        binding = {'exe_sha256': reference['exe']['sha256'], 'exe_size': reference['exe']['size']}
        def invoke(descriptor, action):
            (root / 'descriptor.json').write_text(json.dumps(descriptor), encoding='utf8')
            print('start', label, action, flush=True)
            task = subprocess.run([str(runner), str(root)], cwd=ROOT, capture_output=True, text=True)
            if task.returncode: raise RuntimeError(task.stdout + task.stderr)
            evidence = json.loads((root / 'result.json').read_text(encoding='utf8'))
            (root / (action + '-result.json')).write_text(json.dumps(evidence, indent=2), encoding='utf8')
            return evidence
        evidence = invoke({'action': 'analyze_behavior', **binding}, 'analyze')
        assert evidence['response']['report'] == reference, diff(evidence['response']['report'], reference)
        assert not evidence['response']['installable']
        payload = [{'path': row['destination'], 'size': row['unpacked_size'], **{k: row[k] for k in ('crc32', 'payload_offset', 'packed_size', 'compressed')}} for row in reference['files']]
        invoke({'action': 'extract_behavior', **binding, 'payload': payload}, 'extract')
        members = verify_stream(root / 'members.stream', source, reference)
        (root / 'members.json').write_text(json.dumps(members, indent=2), encoding='utf8')
        records.append({'sample': label, 'source': str(source), 'pass': True, 'exe_sha256': binding['exe_sha256'],
            'metadata_sha256': reference['metadata']['sha256'], 'members': len(members),
            'manifest_sha256': hashlib.sha256(json.dumps(members, sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
            'worker_sha256': sha(worker), 'evidence': str(root)})
        print('pass', label, len(members), flush=True)

    try:
        fixtures = json.loads((ROOT / 'crates/installer-format/tests/fixtures/sf/worker-golden.json').read_text(encoding='utf8'))['cases']
        for i, fixture in enumerate(fixtures):
            source = P2 / (run + f'-synthetic-{i}.exe')
            source.write_bytes(bytes.fromhex(fixture['data_hex']))
            check('synthetic-' + fixture['name'], source, fixture['report'])
        if not args.synthetic_only:
            samples = [(appid, LOCAL / (appid + '-archive.exe')) for appid in (args.appid or ([] if args.sample else ['2303800', '3606360', '3721970']))]
            samples += [(s.split('=', 1)[0], Path(s.split('=', 1)[1]).resolve()) for s in args.sample]
            for appid, source in samples:
                assert appid.isdecimal(), 'sample label must be an AppID'
                check(appid, source)
    except Exception as error:
        records.append({'pass': False, 'error': str(error)})
        raise
    finally:
        summary = P2 / (run + '-summary.json')
        summary.write_text(json.dumps({'platform': args.platform, 'checks': records}, indent=2), encoding='utf8')
        print('summary', summary, flush=True)


if __name__ == '__main__': main()
