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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rpc-snapshot', required=True, type=Path)
    parser.add_argument('--state-dir', type=Path)
    args = parser.parse_args()
    if args.rpc_snapshot.stat().st_size > 8 * 1024 * 1024:
        raise ValueError('cutover_size_limit')
    result = check_rpc(json.loads(args.rpc_snapshot.read_text(encoding='utf-8-sig')))
    if args.state_dir:
        result['journals'] = check_journals(args.state_dir)
        if sum(len(journal['states']) for journal in result['journals']) != result['transactions']:
            raise ValueError('cutover_state_snapshot_mismatch')
    journals_verified = 'journals' in result and (
        sum(len(journal['states']) for journal in result['journals']) == result['transactions']
        and all(state in TRANSACTION_TERMINAL for journal in result['journals'] for state in journal['states'])
    )
    safe = result['transactions'] == 0 or journals_verified
    print(json.dumps({'safe_to_replace': safe, **result}, indent=2))


if __name__ == '__main__':
    main()
