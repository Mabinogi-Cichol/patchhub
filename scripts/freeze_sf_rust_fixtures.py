"""Freeze static SF member fixtures from the independent Python decoder.

Development only: synthetic packages contain inert runtime labels and tiny
payloads. No installer or installer script is executed or interpreted.
"""
import json
from pathlib import Path
from unittest.mock import patch
import sf95_audit as sf
from test_sf9_behavior import Fixture
from test_sf95_audit import sample

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / 'crates/installer-format/tests/fixtures/sf/worker-golden.json'

def main():
    cases = []
    for version in ('9.5', '9.6', '9.7'):
        for name, display_blob in [('payload.bin', None), ('补丁-测试.bin', None), ('display-raw.bin', b'\x02\xa9 ')]:
            fixture = Fixture()
            metadata = fixture.metadata(version, name, display_blob=display_blob)
            with patch('test_sf95_audit.metadata', return_value=metadata):
                raw = sample([(b'unused', fixture.packed)], version=version + '.0.0')
            report = sf.audit(raw, supported_versions=('9.5', '9.6', '9.7'))
            cases.append({'name': version + '-' + name, 'data_hex': raw.hex(),
                'descriptor': {'action': 'list_members', 'exe_sha256': report['exe']['sha256'], 'exe_size': len(raw)},
                'members': [{'path': row['destination'], 'size': row['unpacked_size'], 'crc32': row['crc32']} for row in report['files']],
                'conversion': {'files': [{'path': row['destination'], 'size': row['unpacked_size'], 'sha256': row['payload']['sha256']} for row in report['files']]}})
    DEST.write_text(json.dumps({'schema_version': 1,
        'oracle': 'Frozen independent Python SF container bytes, static members and content SHA-256. No behavior-selection oracle.',
        'cases': cases}, ensure_ascii=True, indent=2) + '\n', encoding='utf8')
    print('static SF fixtures:', len(cases))

if __name__ == '__main__':
    main()
