"""P2 offline parity evidence. Real bytes are parsed only by an isolated fixed Rust worker.

Consumes existing local samples and frozen Python reports; never downloads,
executes publisher code, or writes game directories. Windows LPAC and Linux
bwrap use the development-only sf_sandbox_check example.
"""
import argparse,hashlib,json,shutil,struct,subprocess,time,zipfile,re
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
P2=ROOT/'.artifacts/p2'

def sha(path):
    h=hashlib.sha256()
    with path.open('rb') as stream:
        while b:=stream.read(64*1024): h.update(b)
    return h.hexdigest()

def diff(a,b,path='$'):
    if a==b:return None
    if isinstance(a,dict) and isinstance(b,dict):
        if a.keys()!=b.keys():return f'{path}: keys differ'
        for k in a:
            found=diff(a[k],b[k],path+'.'+k)
            if found:return found
    elif isinstance(a,list) and isinstance(b,list):
        if len(a)!=len(b):return f'{path}: lengths differ'
        for i,(v,w) in enumerate(zip(a,b)):
            found=diff(v,w,f'{path}[{i}]')
            if found:return found
    else:return f'{path}: {a!r} != {b!r}'

class FrozenZipInfo(zipfile.ZipInfo):
    """Keep the frozen CPython 3.12 force-ZIP64 bytes on Python 3.10 too."""
    def FileHeader(self, zip64=None):
        if zip64:
            self.extract_version=max(self.extract_version,45)
            self.create_version=max(self.create_version,45)
        header=super().FileHeader(zip64)
        if zip64:
            header=bytearray(header)
            struct.pack_into('<H',header,4,45)
            struct.pack_into('<II',header,18,0xffffffff,0xffffffff)
            return bytes(header)
        return header

def canonical_oracle(prior, report, output):
    # Validate historical ZIP payloads against the frozen full report, then
    # encode the frozen CPython 3.12 force_zip64=True local/central headers.
    # Python 3.10 otherwise retains version 20 and actual local sizes for small
    # files, despite carrying ZIP64 extras; this development oracle normalizes it.
    rows=sorted(report['files'],key=lambda r:r['destination'])
    with zipfile.ZipFile(prior) as source, zipfile.ZipFile(output,'x',compression=zipfile.ZIP_STORED,allowZip64=True) as archive:
        assert sorted(source.namelist())==[r['destination'] for r in rows],'prior artifact manifest changed'
        for row in rows:
            info=source.getinfo(row['destination'])
            assert info.compress_type==zipfile.ZIP_STORED and info.file_size==row['unpacked_size'],'prior artifact size/type changed'
            entry=FrozenZipInfo(row['destination'],date_time=(1980,1,1,0,0,0))
            entry.compress_type=zipfile.ZIP_STORED;entry.create_system=3;entry.external_attr=0o100644<<16;entry.file_size=row['unpacked_size']
            h=hashlib.sha256();size=0
            with source.open(info) as stream,archive.open(entry,'w',force_zip64=True) as sink:
                while data:=stream.read(64*1024):
                    size+=len(data);assert size<=row['unpacked_size'];h.update(data);sink.write(data)
            assert size==row['unpacked_size'] and h.hexdigest()==row['payload']['sha256'],'prior payload changed'
    return sha(output)

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--platform',choices=['windows','linux'],default='windows')
    ap.add_argument('--synthetic-only',action='store_true');ap.add_argument('--appid',action='append');ap.add_argument('--reuse-run')
    args=ap.parse_args();P2.mkdir(parents=True,exist_ok=True)
    run=args.reuse_run or f'{args.platform}-{time.time_ns()}'
    assert re.fullmatch(args.platform+r'-[0-9]+',run),'invalid reuse run'
    runner=ROOT/('target/release/examples/sf_sandbox_check.exe' if args.platform=='windows' else 'target/linux/release/examples/sf_sandbox_check')
    records=[]
    def invoke(label,descriptor,raw=None,source=None):
        root=P2/(run+'-'+label)
        evidence_path=root/'result.json'
        if args.reuse_run and evidence_path.exists():
            current=ROOT/('target/release/sf-worker.exe' if args.platform=='windows' else 'target/linux/release/sf-worker')
            copied=root/'runtime'/current.name
            assert sha(copied)==sha(current),'worker changed; cannot reuse'
            assert json.loads((root/'descriptor.json').read_text('utf8'))==descriptor,'descriptor changed; cannot reuse'
            evidence=json.loads(evidence_path.read_text('utf8'))
            assert evidence['exit_code']==0,'failed worker result cannot be reused'
            print('reuse',label,flush=True)
            return root,evidence
        (root/'input').mkdir(parents=True)
        archive=root/'input/archive.exe'
        if source:shutil.copyfile(source,archive)
        else:archive.write_bytes(raw)
        (root/'descriptor.json').write_text(json.dumps(descriptor),encoding='utf8')
        print('start',label,flush=True)
        task=subprocess.run([str(runner),str(root)],cwd=ROOT,capture_output=True,text=True)
        evidence_path=root/'result.json'
        if not evidence_path.exists():raise RuntimeError(task.stderr+' '+task.stdout)
        evidence=json.loads(evidence_path.read_text('utf8'))
        if task.returncode:raise RuntimeError(str(evidence))
        return root,evidence
    try:
        fixtures=json.loads((ROOT/'crates/installer-format/tests/fixtures/sf/worker-golden.json').read_text('utf8'))['cases']
        for fixture in [c for c in fixtures if c['name'].endswith('补丁-测试.bin')]:
            root,evidence=invoke('synthetic-'+fixture['name'].split('-')[0],fixture['descriptor'],raw=bytes.fromhex(fixture['data_hex']))
            assert evidence['response']==fixture['conversion'],diff(evidence['response'],fixture['conversion'])
            records.append({'sample':fixture['name'],'kind':'synthetic','action':'convert_behavior','pass':True,
                'artifact_sha256':evidence['response']['artifact_sha256'],'evidence':str(root/'result.json'),'wall_ms':evidence['wall_ms']})
            print('pass',fixture['name'],evidence['wall_ms'],flush=True)
        if not args.synthetic_only:
            for appid in args.appid or ['2303800','3606360','3721970']:
                local=ROOT/'.artifacts/generic-exe-local';source=local/(appid+'-archive.exe')
                report=json.loads((local/(appid+'-behavior-analysis.json')).read_text('utf8'))
                assert source.stat().st_size==report['exe']['size'] and sha(source)==report['exe']['sha256'],'stale Python oracle'
                analysis={'action':'analyze_behavior','exe_sha256':report['exe']['sha256'],'exe_size':report['exe']['size']}
                root,evidence=invoke(appid+'-analyze',analysis,source=source)
                assert evidence['response']['installable'] is False
                observed=evidence['response']['report'];assert observed==report,diff(observed,report)
                records.append({'sample':appid,'kind':'existing_offline','action':'analyze_behavior','pass':True,'installer':report['installer'],
                    'exe_sha256':report['exe']['sha256'],'metadata_sha256':report['metadata']['sha256'],'behavior_sha256':report['behavior_sha256'],
                    'evidence':str(root/'result.json'),'wall_ms':evidence['wall_ms']})
                print('pass',appid,'analysis',evidence['wall_ms'],flush=True)
                descriptor={**analysis,'action':'convert_behavior','metadata_sha256':report['metadata']['sha256'],
                    'behavior_sha256':report['behavior_sha256'],
                    'payload':[{'path':f['destination'],'size':f['unpacked_size'],'sha256':f['payload']['sha256']} for f in report['files']]}
                root,evidence=invoke(appid+'-convert',descriptor,source=source)
                prior=ROOT/'.artifacts/generic-exe-convert'/appid/'normalized.zip'
                oracle=root/'python-current-normalized.zip'
                if oracle.exists():oracle.unlink()
                expected=canonical_oracle(prior,report,oracle);observed=sha(root/'output/normalized.zip')
                assert observed==expected==evidence['response']['artifact_sha256'],'artifact differs'
                assert oracle.stat().st_size==evidence['response']['artifact_size']
                records.append({'sample':appid,'kind':'existing_offline','action':'convert_behavior','pass':True,
                    'artifact_sha256':observed,'artifact_size':evidence['response']['artifact_size'],'python_oracle':str(oracle),
                    'evidence':str(root/'result.json'),'wall_ms':evidence['wall_ms']})
                print('pass',appid,'convert',evidence['wall_ms'],flush=True)
    except Exception as error:
        records.append({'pass':False,'error':str(error)});raise
    finally:
        summary=P2/(run+'-summary.json');summary.write_text(json.dumps({'platform':args.platform,'checks':records},ensure_ascii=False,indent=2),encoding='utf8')
        print('summary',summary,flush=True)
if __name__=='__main__':main()
