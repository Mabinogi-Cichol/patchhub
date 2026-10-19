"""Freeze synthetic Python SF oracle cases for Rust regression (development only).

No real publisher EXE or script is embedded. Test-generated PE packages contain
inert runtime labels and tiny payloads, stored as text, never executed.
"""
import copy
import hashlib
import importlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import sf95_audit as sf
import sf9_graph as graph
import sf9_metadata as metadata
import sf9_lua as lua
import sf9_behavior as behavior
import sf95_worker as worker
from test_sf9_behavior import Fixture
from test_sf95_audit import sample

DEST = ROOT / 'crates/installer-format/tests/fixtures/sf'
DEST.mkdir(parents=True, exist_ok=True)
originals = {'lua':lua.parse, 'behavior':behavior.lower, 'graph':graph.decode,
             'inventory':metadata.inventory, 'audit':sf.audit}
inputs = {key:{} for key in originals}

def store(kind, args):
    encoded = json.dumps(args, ensure_ascii=True, sort_keys=True, separators=(',', ':'))
    inputs[kind][hashlib.sha256(encoded.encode()).hexdigest()] = json.loads(encoded)

# Preserve the original synthetic regression corpus independently of test discovery.
for kind, cases in json.loads((ROOT / 'scripts/fixtures/sf-reference-inputs.json').read_text(encoding='utf8')).items():
    for args in cases: store(kind, args)

def capture(kind):
    fn = originals[kind]
    def wrapped(*args, **kwargs):
        if kind == 'lua': store(kind, {'source':args[0]})
        elif kind == 'behavior': store(kind, {'graph':copy.deepcopy(args[0]),'files':copy.deepcopy(args[1]),'version':args[2]})
        elif kind == 'graph': store(kind, {'data_hex':bytes(args[0]).hex(),'version':args[1]})
        elif kind == 'inventory': store(kind, {'data_hex':bytes(args[0]).hex()})
        elif kind == 'audit' and len(args[0]) < 100_000:
            store(kind, {'data_hex':bytes(args[0]).hex()})
        return fn(*args, **kwargs)
    return wrapped

mods = [importlib.import_module(n) for n in ('sf9_graph','sf9_metadata','sf9_lua','sf9_behavior','sf95_worker',
        'test_sf9_behavior','test_sf9_metadata','test_sf95_audit','test_sf95_worker')]
changes = []
for mod in mods:
    for attr,value in list(vars(mod).items()):
        for kind,fn in originals.items():
            if value is fn:
                changes.append((mod,attr,value))
                setattr(mod,attr,capture(kind))
suite = unittest.TestSuite(unittest.defaultTestLoader.loadTestsFromModule(m) for m in mods if m.__name__.startswith('test_'))
try:
    result = unittest.TextTestRunner(stream=sys.stderr).run(suite)
    if not result.wasSuccessful(): raise SystemExit('Python baseline tests failed')
finally:
    for mod,attr,value in changes: setattr(mod,attr,value)

# The direct Compiler tests also become full graph/behavior regression inputs.
for source in [r'File.Delete("%AppFolder%\\identity.dat")',
        'local x="File"; _G[x].Run("payload")', '_NeedsReboot=true; System.Reboot()',
        'local f=File.Run; f("payload")','while true do Screen.Next() end',
        'function evil() _NeedsReboot=true end evil(); if _NeedsReboot then System.Reboot() end',
        '-- safe\nFile.Run("bad")', '--[=[ File.Run("bad") ]=]\nif false then File.Run("bad") else Screen.Next() end',
        'if File.GetCRC("%AppFolder%\\\\anything.dat") == 4294967295 then Screen.Next() end']:
    f=Fixture(); data=f.metadata(script=source)
    g=graph.decode(data,'9.5'); files=sf.file_table(data)[0]
    store('behavior',{'graph':g,'files':files,'version':'9.5'})
    store('lua',{'source':source})

cases=[]
for version in ('9.5','9.6','9.7'):
    for name in ('payload.bin','补丁-测试.bin'):
        f=Fixture(); data=f.metadata(version,name)
        with patch('test_sf95_audit.metadata',return_value=data): raw=sample([(b'unused',f.packed)],version=version+'.0.0')
        store('graph',{'data_hex':data.hex(),'version':version})
        store('inventory',{'data_hex':data.hex()})
        store('audit',{'data_hex':raw.hex()})
        full_report=worker.audit_behavior(raw)
        report=worker.audit_behavior(raw, decode_payloads=False)
        descriptor={'action':'convert_behavior','exe_sha256':report['exe']['sha256'],'exe_size':len(raw),
            'metadata_sha256':report['metadata']['sha256'],'behavior_sha256':report['behavior_sha256'],
            'payload':[{'path':r['destination'],'size':r['unpacked_size'],'sha256':r['payload']['sha256']} for r in full_report['files']]}
        with tempfile.TemporaryDirectory() as directory:
            p=Path(directory); (p/'input.exe').write_bytes(raw)
            with (p/'input.exe').open('rb') as stream: converted=worker.convert(stream,descriptor,p/'normalized.zip')
        cases.append({'name':version+'-'+name,'data_hex':raw.hex(),'report':report,'descriptor':descriptor,'conversion':converted})

for kind,values in inputs.items():
    out=[]
    for key,args in sorted(values.items()):
        case={'id':key,'input':args}
        try:
            if kind=='lua': expected=lua.parse(args['source'])
            elif kind=='behavior': expected=behavior.lower(args['graph'],args['files'],args['version'])
            elif kind=='graph': expected=graph.decode(bytes.fromhex(args['data_hex']),args['version'])
            elif kind=='inventory': expected=metadata.inventory(bytes.fromhex(args['data_hex']))
            else: expected=sf.audit(bytes.fromhex(args['data_hex']),supported_versions=('9.5','9.6','9.7'),metadata_inspector=metadata.inventory)
            case['expected']=expected
        except (ValueError,KeyError,IndexError,UnicodeError) as error: case['error']=str(error)
        out.append(case)
    (DEST/(kind+'-golden.json')).write_text(json.dumps({'schema_version':1,'oracle':'Python synthetic test fixtures','cases':out},ensure_ascii=True,sort_keys=True,indent=2)+'\n',encoding='utf8')
    print(kind,len(out))
(DEST/'worker-golden.json').write_text(json.dumps({'schema_version':1,'oracle':'sf95_worker synthetic behavior/convert','cases':cases},ensure_ascii=True,sort_keys=True,indent=2)+'\n',encoding='utf8')
print('worker',len(cases))
