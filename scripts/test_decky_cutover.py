from contextlib import closing
import importlib.util
import json
import io
import sys
from unittest.mock import patch
from contextlib import redirect_stdout
from pathlib import Path
import sqlite3
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('cutover', Path(__file__).with_name('check-decky-cutover.py'))
cutover = importlib.util.module_from_spec(spec); spec.loader.exec_module(cutover)


class CutoverTests(unittest.TestCase):
    def test_ended_tasks_are_safe_but_unknown_or_inflight_is_blocked(self):
        baseline = {'instances': 1, 'tasks': [{'state': 'cancelled'}], 'transactions': [{'state': 'committed'}]}
        self.assertEqual(cutover.check_rpc(baseline), {'tasks': 1, 'transactions': 1})
        for name, states in [('tasks', ['running', 'cancelling', 'rolling_back', None]), ('transactions', ['planned', 'applying', 'rolling_back', 'needs_recovery', None])]:
            for state in states:
                with self.subTest(state=state):
                    data = {**baseline, name: [{'state': state}]}
                    with self.assertRaisesRegex(ValueError, 'unfinished'):
                        cutover.check_rpc(data)

    def test_unfinished_old_journal_blocks_without_modification(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); path = root / 'publisher/release/instance/journal.sqlite'
            path.parent.mkdir(parents=True)
            with closing(sqlite3.connect(path)) as database:
                database.execute('CREATE TABLE jobs(document TEXT)')
                database.execute('INSERT INTO jobs VALUES (?)', (json.dumps({'transaction': {'state': 'needs_recovery'}}),))
                database.commit()
            before = path.read_bytes()
            with self.assertRaisesRegex(ValueError, 'unfinished_journal'):
                cutover.check_journals(root)
            self.assertEqual(path.read_bytes(), before)
            with closing(sqlite3.connect(path)) as database:
                database.execute('UPDATE jobs SET document=?', (json.dumps({'transaction': {'state': 'rolled_back', 'instance_id': 'known-instance'}}),))
                database.commit()
            before = path.read_bytes()
            self.assertEqual(cutover.check_journals(root)[0]['states'], ['rolled_back'])
            self.assertEqual(path.read_bytes(), before)

    def test_committed_journal_is_safe_without_marker_publication(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); path = root / 'publisher/release/instance/journal.sqlite'
            path.parent.mkdir(parents=True)
            with closing(sqlite3.connect(path)) as database:
                database.execute('CREATE TABLE jobs(document TEXT)')
                database.execute('INSERT INTO jobs VALUES (?)', (json.dumps({'transaction': {'state': 'committed', 'instance_id': '42|old-library|game'}}),))
                database.commit()
            snapshot = root / 'rpc.json'
            snapshot.write_text(json.dumps({'instances': 1, 'tasks': [], 'transactions': [{'state': 'committed'}]}))
            command = ['check', '--rpc-snapshot', str(snapshot), '--state-dir', str(root)]
            with patch.object(sys, 'argv', command), redirect_stdout(io.StringIO()) as output:
                cutover.main()
            self.assertTrue(json.loads(output.getvalue())['safe_to_replace'])
            self.assertFalse((root / 'installer/rust-v1/preexisting-installations.json').exists())
            with patch.object(sys, 'argv', command + ['--write-protection']):
                with self.assertRaises(SystemExit):
                    cutover.main()

    def test_wrong_state_directory_cannot_override_committed_rpc_snapshot(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); snapshot = root / 'rpc.json'
            snapshot.write_text(json.dumps({'instances': 1, 'tasks': [], 'transactions': [{'state': 'committed'}]}))
            with patch.object(sys, 'argv', ['check', '--rpc-snapshot', str(snapshot), '--state-dir', str(root)]):
                with self.assertRaisesRegex(ValueError, 'snapshot_mismatch'):
                    cutover.main()

    def test_bad_journal_is_not_silently_skipped(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); (root / 'journal.sqlite').write_bytes(b'bad journal')
            with self.assertRaises(sqlite3.DatabaseError):
                cutover.check_journals(root)


if __name__ == '__main__':
    unittest.main()
