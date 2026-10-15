"""Freeze CPython ZIP64 boundary framing without allocating or writing huge payloads."""
import json
import sys
import zipfile
from pathlib import Path


class Capture:
    def __init__(self, position):
        self.position = position
        self.data = bytearray()

    def tell(self):
        return self.position

    def write(self, data):
        self.data.extend(data)
        self.position += len(data)
        return len(data)

    def flush(self):
        pass


def main():
    if sys.version_info[:2] != (3, 12):
        raise SystemExit('Regenerate the frozen ZIP byte oracle with CPython 3.12.')
    limit = zipfile.ZIP64_LIMIT
    boundaries = [
        ('classic', 15, 0, 77),
        ('size-at-limit', limit, 0, limit + 62),
        ('size-above-limit', limit + 1, 0, limit + 63),
        ('offset-at-limit', 0, limit, limit + 62),
        ('offset-above-limit', 0, limit + 1, limit + 63),
        ('size-and-offset-extras', limit + 1, limit + 1, 2 * limit + 128),
        ('size-and-offset-extras-beyond-32bit', 5 * 2**30, 5 * 2**30 + 123, 2 * (5 * 2**30) + 128),
        ('end-at-limit', 0, 0, limit),
        ('end-above-limit', 0, 0, limit + 1),
        ('unicode-size-above-limit', limit + 1, 0, limit + 80),
    ]
    cases = []
    for label, size, offset, central_offset in boundaries:
        name = '补丁-测试.bin' if label.startswith('unicode') else 'payload.bin'
        info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_STORED
        info.create_system = 3
        info.create_version = info.extract_version = 45
        info.external_attr = 0o100644 << 16
        info.file_size = info.compress_size = size
        info.header_offset = offset
        info.CRC = 0x12345678
        capture = Capture(central_offset)
        archive = object.__new__(zipfile.ZipFile)
        archive.fp = capture
        archive.filelist = [info]
        archive.start_dir = central_offset
        archive._allowZip64 = True
        archive._comment = b''
        archive.mode = 'w'
        try:
            archive._write_end_record()
        finally:
            archive.fp = None
        cases.append({'id': label, 'name': name, 'size': size,
                      'header_offset': offset, 'central_offset': central_offset,
                      'expected_hex': capture.data.hex()})
    output = Path(__file__).resolve().parents[1] / 'crates/installer-format/tests/fixtures/sf/zip64-boundary-golden.json'
    output.write_text(json.dumps({'schema_version': 1, 'oracle': 'CPython 3.12 ZIP_STORED central/end framing, no payload allocation', 'cases': cases}, ensure_ascii=True, sort_keys=True, indent=2) + '\n', encoding='utf8')
    print('ZIP64 boundary framing:', len(cases), 'frozen cases')


if __name__ == '__main__':
    main()
