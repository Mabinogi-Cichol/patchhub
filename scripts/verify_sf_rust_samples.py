"""Offline static-member parity through the production parser binary.

Compare the worker list and streamed payloads with an independent Python decoder
or a previously completed extraction. No installer code or game path is used.
"""
import argparse
import hashlib
import json
import mmap
from pathlib import Path
import subprocess
import time
import zlib

import sf95_audit as sf

ROOT = Path(__file__).resolve().parents[1]
P2 = ROOT / '.artifacts/p2'
LOCAL = ROOT / '.artifacts/generic-exe-local'


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def minimal_members(rows):
    return [{'path': row['destination'], 'size': row['unpacked_size'],
             'crc32': row['crc32']} for row in rows]


def verify_stream(worker, source, binding, reference, completed=None):
    task = subprocess.Popen([str(worker), str(source)], stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    members = []
    try:
        task.stdin.write(json.dumps({'action': 'extract_members', **binding}).encode())
        task.stdin.close()
        stream = task.stdout
        assert json.loads(stream.readline()) == {'extract': 1, 'members': len(reference['files'])}
        with source.open('rb') as exe, mmap.mmap(exe.fileno(), 0, access=mmap.ACCESS_READ) as data:
            decoder = sf.Decoder()
            for i, row in enumerate(reference['files']):
                assert json.loads(stream.readline()) == {'index': i, 'path': row['destination'], 'size': row['unpacked_size']}
                if completed:
                    expected = completed / 'payload' / row['destination']
                    digest = hashlib.sha256()
                    crc = 0
                    remaining = row['unpacked_size']
                    with expected.open('rb') as check:
                        while remaining:
                            chunk = stream.read(min(65536, remaining))
                            assert chunk, 'truncated member stream'
                            assert chunk == check.read(len(chunk)), 'payload bytes differ'
                            remaining -= len(chunk)
                            digest.update(chunk)
                            crc = zlib.crc32(chunk, crc)
                        assert not check.read(1), 'reference size differs'
                    info = {'size': row['unpacked_size'], 'crc32': crc, 'sha256': digest.hexdigest()}
                else:
                    class Compare:
                        def write(self, chunk):
                            assert stream.read(len(chunk)) == chunk, 'payload bytes differ'
                    info, _ = decoder.decode(sf.Reader(data, row['payload_offset']), row['packed_size'],
                        row['unpacked_size'], row['crc32'], compressed=row['compressed'], sink=Compare())
                assert info['crc32'] == row['crc32'], 'member CRC differs'
                assert json.loads(stream.readline()) == {'index': i, **info}, 'payload digest differs'
                members.append({'path': row['destination'], **info})
        assert json.loads(stream.readline()) == {'done': True}
        assert stream.read(1) == b'', 'trailing extraction data'
        errors = task.stderr.read().decode('utf8')
        assert task.wait() == 0, errors
    finally:
        if task.poll() is None:
            task.kill()
        task.wait()
    return members


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--platform', choices=['windows', 'linux'], default='windows')
    parser.add_argument('--synthetic-only', action='store_true')
    parser.add_argument('--skip-synthetic', action='store_true')
    parser.add_argument('--appid', action='append')
    parser.add_argument('--sample', action='append', default=[], metavar='APPID=PATH')
    parser.add_argument('--worker', type=Path)
    parser.add_argument('--reference-dir', type=Path, help='Completed extraction reference for one explicit sample')
    args = parser.parse_args()
    if args.reference_dir and len(args.sample) != 1:
        parser.error('--reference-dir requires exactly one --sample')
    worker = args.worker or ROOT / ('target/release/patchhub-worker.exe' if args.platform == 'windows' else 'target/linux/release/patchhub-worker')
    worker = worker.resolve()
    run = f'{args.platform}-{time.time_ns()}'
    P2.mkdir(parents=True, exist_ok=True)
    records = []

    def check(label, source, expected=None, completed=None):
        root = P2 / (run + '-' + label)
        root.mkdir(parents=True)
        if completed:
            report_root = completed / 'reports'
            preflight = json.loads((report_root / 'preflight.json').read_text(encoding='utf8'))
            assert sha(source) == preflight['source_sha256']
            assert source.stat().st_size == preflight['source_size']
            reference = {'files': json.loads((report_root / 'file-table.json').read_text(encoding='utf8'))}
            binding = {'exe_sha256': preflight['source_sha256'], 'exe_size': preflight['source_size']}
        else:
            with source.open('rb') as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as data:
                reference = sf.audit(data, supported_versions=('9.5', '9.6', '9.7'), decode_payloads=False)
            binding = {'exe_sha256': reference['exe']['sha256'], 'exe_size': reference['exe']['size']}
        listed = minimal_members(reference['files'])
        if expected is not None:
            assert listed == expected
        print('start', label, 'list_members', flush=True)
        task = subprocess.run([str(worker), str(source)], input=json.dumps({'action': 'list_members', **binding}).encode(), capture_output=True)
        assert task.returncode == 0, task.stdout.decode('utf8') + task.stderr.decode('utf8')
        response = json.loads(task.stdout)
        assert response == {'schema_version': 1, 'action': 'list_members', 'exe_sha256': binding['exe_sha256'], 'members': listed}
        (root / 'list-members.json').write_text(json.dumps(response, ensure_ascii=True, indent=2), encoding='utf8')
        print('start', label, 'extract_members', flush=True)
        members = verify_stream(worker, source, binding, reference, completed)
        (root / 'members.json').write_text(json.dumps(members, ensure_ascii=True, indent=2), encoding='utf8')
        records.append({'sample': label, 'source': str(source), 'pass': True, **binding,
            'members': len(members), 'bytes': sum(member['size'] for member in members),
            'manifest_sha256': hashlib.sha256(json.dumps(members, sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
            'worker_sha256': sha(worker), 'evidence': str(root)})
        print('pass', label, len(members), flush=True)

    try:
        if not args.skip_synthetic:
            fixtures = json.loads((ROOT / 'crates/installer-format/tests/fixtures/sf/worker-golden.json').read_text(encoding='utf8'))['cases']
            for i, fixture in enumerate(fixtures):
                source = P2 / (run + f'-synthetic-{i}.exe')
                source.write_bytes(bytes.fromhex(fixture['data_hex']))
                check('synthetic-' + fixture['name'], source, fixture['members'])
        if not args.synthetic_only:
            samples = [(appid, LOCAL / (appid + '-archive.exe')) for appid in (args.appid or ([] if args.sample else ['2303800', '3606360', '3721970']))]
            samples += [(s.split('=', 1)[0], Path(s.split('=', 1)[1]).resolve()) for s in args.sample]
            for appid, source in samples:
                assert appid.isdecimal(), 'sample label must be an AppID'
                check(appid, source, completed=args.reference_dir)
    except Exception as error:
        records.append({'pass': False, 'error': str(error)})
        raise
    finally:
        summary = P2 / (run + '-summary.json')
        summary.write_text(json.dumps({'platform': args.platform, 'checks': records}, indent=2), encoding='utf8')
        print('summary', summary, flush=True)


if __name__ == '__main__':
    main()
