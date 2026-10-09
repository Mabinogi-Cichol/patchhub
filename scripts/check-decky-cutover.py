"""Read-only deployment gate; never shipped as a production legacy reader.

Run against the still-installed old plugin's list_tasks/list_transactions RPC
snapshot, and optionally inspect its journal files before replacement. An
unresolved job blocks cutover; use the old engine to recover it. No deletion,
import, authorization, ID mapping or state restore is performed here.
"""
from contextlib import closing
import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import tempfile

TASK_TERMINAL = {'completed', 'failed', 'cancelled'}
TRANSACTION_TERMINAL = {'committed', 'uninstalled', 'rolled_back', 'superseded'}


def check_rpc(snapshot):
    data = snapshot.get('evaluation', {}).get('result', {}).get('value', snapshot)
    if not isinstance(data, dict) or data.get('instances') != 1:
        raise ValueError('cutover_plugin_identity_unconfirmed')
    for name, terminal in (('tasks', TASK_TERMINAL), ('transactions', TRANSACTION_TERMINAL)):
        rows = data.get(name)
        if not isinstance(rows, list) or len(rows) > 8192:
            raise ValueError('cutover_invalid_snapshot')
        if any(not isinstance(row, dict) or row.get('state') not in terminal for row in rows):
            raise ValueError('cutover_unfinished_' + name)
    return {'tasks': len(data['tasks']), 'transactions': len(data['transactions'])}


def plain(path):
    for part in (path, *path.parents):
        if part.is_symlink() or (hasattr(part, 'is_junction') and part.is_junction()):
            raise ValueError('cutover_linked_path')


def check_journals(state):
    state = Path(state).absolute()
    plain(state)
    if not state.is_dir():
        raise ValueError('cutover_state_missing')
    journals = []
    if (state / 'journal.sqlite').exists():
        journals.append(state / 'journal.sqlite')
    publisher = state / 'publisher'
    plain(publisher)
    if publisher.exists():
        count = 0
        for directory, children, files in os.walk(publisher, followlinks=False):
            count += 1
            if count > 8192:
                raise ValueError('cutover_size_limit')
            for name in children + files:
                plain(Path(directory) / name)
            if 'journal.sqlite' in files:
                journals.append(Path(directory) / 'journal.sqlite')
    result = []
    for path in sorted(journals):
        plain(path)
        if path.stat().st_size > 64 * 1024 * 1024:
            raise ValueError('cutover_size_limit')
        with closing(sqlite3.connect(path.as_uri() + '?mode=ro', uri=True)) as database:
            rows = database.execute('SELECT document FROM jobs LIMIT 8193').fetchall()
        if len(rows) > 8192:
            raise ValueError('cutover_size_limit')
        states = []
        preserved = set()
        for (document,) in rows:
            if not isinstance(document, str) or len(document.encode('utf-8')) > 8 * 1024 * 1024:
                raise ValueError('cutover_invalid_journal')
            job = json.loads(document)
            transaction = job.get('transaction') if isinstance(job, dict) else None
            if not isinstance(transaction, dict) or transaction.get('state') not in TRANSACTION_TERMINAL:
                raise ValueError('cutover_unfinished_journal')
            states.append(transaction['state'])
            if transaction['state'] == 'committed':
                instance = transaction.get('instance_id')
                if not isinstance(instance, str) or not instance or len(instance.encode('utf-8')) > 1024 or any(ord(c) < 32 for c in instance):
                    raise ValueError('cutover_invalid_instance')
                preserved.add(instance)
        result.append({'path': path.relative_to(state).as_posix(), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(), 'states': states, 'preserved_instances': sorted(preserved)})
    return result


def publish_protection(state, journals):
    """Persist blockers only; carries no old release/profile/receipt permission."""
    path = Path(state).absolute() / 'installer/rust-v1/preexisting-installations.json'
    plain(path)
    instances = {instance for journal in journals for instance in journal['preserved_instances']}
    appids = {int(instance.split('|', 1)[0]) for instance in instances if instance.split('|', 1)[0].isdigit()}
    if len(appids) == 0 and instances:
        raise ValueError('cutover_invalid_instance')
    if path.exists():
        if path.stat().st_size > 1024 * 1024:
            raise ValueError('cutover_invalid_protection')
        previous = json.loads(path.read_text(encoding='utf-8'))
        if (not isinstance(previous, dict) or set(previous) != {'schema_version', 'instances', 'appids'}
                or previous['schema_version'] != 1 or not isinstance(previous['instances'], list)
                or any(not isinstance(item, str) for item in previous['instances'])):
            raise ValueError('cutover_invalid_protection')
        instances.update(previous['instances'])
        if not isinstance(previous['appids'], list) or any(type(appid) is not int or not 1 <= appid <= 4294967295 for appid in previous['appids']):
            raise ValueError('cutover_invalid_protection')
        appids.update(previous['appids'])
    if len(instances) > 8192:
        raise ValueError('cutover_size_limit')
    data = json.dumps({'schema_version': 1, 'instances': sorted(instances), 'appids': sorted(appids)}, ensure_ascii=False).encode('utf-8')
    if len(data) > 1024 * 1024:
        raise ValueError('cutover_size_limit')
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    plain(path)
    descriptor, name = tempfile.mkstemp(prefix='.preexisting-', dir=path.parent)
    try:
        with os.fdopen(descriptor, 'wb') as stream:
            stream.write(data); stream.flush(); os.fsync(stream.fileno())
        os.replace(name, path)
        if os.name != 'nt':
            directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
            try: os.fsync(directory)
            finally: os.close(directory)
    finally:
        Path(name).unlink(missing_ok=True)
    return {'protected_instances': len(instances), 'sha256': hashlib.sha256(data).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rpc-snapshot', required=True, type=Path)
    parser.add_argument('--state-dir', type=Path)
    parser.add_argument('--write-protection', action='store_true', help='After stopping old task admission, atomically publish write blockers only')
    args = parser.parse_args()
    if args.rpc_snapshot.stat().st_size > 8 * 1024 * 1024:
        raise ValueError('cutover_size_limit')
    result = check_rpc(json.loads(args.rpc_snapshot.read_text(encoding='utf-8-sig')))
    if args.state_dir:
        result['journals'] = check_journals(args.state_dir)
        if sum(len(journal['states']) for journal in result['journals']) != result['transactions']:
            raise ValueError('cutover_state_snapshot_mismatch')
    if args.write_protection:
        if not args.state_dir: raise ValueError('cutover_state_required')
        result['protection'] = publish_protection(args.state_dir, result['journals'])
    protected = not any(journal['preserved_instances'] for journal in result.get('journals', []))
    if args.write_protection: protected = True
    elif args.state_dir and not protected:
        marker = args.state_dir / 'installer/rust-v1/preexisting-installations.json'
        if marker.is_file():
            plain(marker)
            if marker.stat().st_size > 1024 * 1024: raise ValueError('cutover_size_limit')
            guard = json.loads(marker.read_text(encoding='utf-8'))
            expected = {instance for journal in result['journals'] for instance in journal['preserved_instances']}
            protected = guard.get('schema_version') == 1 and expected.issubset(set(guard.get('instances', []))) and {int(instance.split('|', 1)[0]) for instance in expected}.issubset(set(guard.get('appids', [])))
    elif not args.state_dir:
        protected = result['transactions'] == 0
    print(json.dumps({'safe_to_replace': protected, 'protection_required': not protected, **result}, indent=2))


if __name__ == '__main__':
    main()
