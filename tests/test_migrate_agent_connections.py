import fcntl
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    'migrate_agent_connections', Path(__file__).parents[1] / 'apps/desktop/scripts/migrate-agent-connections.py'
)
migration = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(migration)


class AgentConnectionMigrationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.connections = self.root / 'connections'
        self.old = self.root / 'old data'
        self.new = self.root / 'new data'
        self.originals = {}
        for client in migration.CLIENTS:
            folder = self.connections / client
            (folder / 'sessions').mkdir(parents=True)
            value = {'schemaVersion': 1, 'client': client,
                     'token': '11111111-1111-4111-8111-111111111111',
                     'generation': 'a' * 64, 'dataDir': str(self.old),
                     'extra': {'preserved': 1.0}, 'escaped': '\\u1234'}
            raw = ('\n' + json.dumps(value, ensure_ascii=True, indent=2) + '\n').encode()
            path = folder / 'state.json'
            path.write_bytes(raw)
            path.chmod(0o600)
            self.originals[client] = raw
            (folder / 'releases').mkdir()
            (folder / 'releases/immutable.bin').write_bytes(b'historical package')

    def prepare(self):
        plan = migration.prepare(self.connections, self.old, self.new)
        self.addCleanup(migration.release, plan)
        return plan

    def test_changes_only_data_dir_token_and_rolls_back_exact_bytes(self):
        plan = self.prepare()
        migration.apply(plan)
        receipt = migration.verify(plan)
        for client in migration.CLIENTS:
            path = self.connections / client / 'state.json'
            expected = self.originals[client].replace(
                json.dumps(str(self.old)).encode(), json.dumps(str(self.new)).encode())
            self.assertEqual(path.read_bytes(), expected)
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertEqual((path.parent / 'releases/immutable.bin').read_bytes(), b'historical package')
            self.assertEqual(receipt[client]['before'], migration.fingerprint(self.originals[client]))
        self.assertNotIn('11111111', json.dumps(receipt))
        self.assertFalse(self.old.exists())
        self.assertFalse(self.new.exists())
        migration.rollback(plan)
        for client in migration.CLIENTS:
            self.assertEqual((self.connections / client / 'state.json').read_bytes(), self.originals[client])

    def test_preparation_rejects_live_session_and_releases_its_locks(self):
        lock = self.connections / 'codex/sessions/session.lock'
        with lock.open('w+b') as stream:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaisesRegex(RuntimeError, 'active Nero agent'):
                self.prepare()
        plan = self.prepare()
        self.assertEqual(len(plan['entries']), 2)

    def test_apply_rechecks_live_sessions_before_writing(self):
        plan = self.prepare()
        lock = self.connections / 'codex/sessions/session.lock'
        with lock.open('w+b') as stream:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(RuntimeError):
                migration.apply(plan)
        self.assertEqual(plan['applied'], [])

    def test_prepare_lock_blocks_concurrent_connection_changes(self):
        plan = self.prepare()
        with self.assertRaises(RuntimeError):
            self.prepare()
        migration.release(plan)
        self.prepare()

    def test_concurrent_state_change_is_not_overwritten(self):
        plan = self.prepare()
        path = self.connections / 'codex/state.json'
        path.write_bytes(b'changed')
        with self.assertRaisesRegex(RuntimeError, 'changed before'):
            migration.apply(plan)
        self.assertEqual(path.read_bytes(), b'changed')
        self.assertEqual((self.connections / 'claude/state.json').read_bytes(), self.originals['claude'])

    def test_partial_write_failure_rolls_back_only_its_changes(self):
        plan = self.prepare()
        write = migration.atomic_write
        calls = 0

        def failing(path, value, mode):
            nonlocal calls
            calls += 1
            if calls == 2:
                raise OSError('synthetic write failure')
            write(path, value, mode)

        with patch.object(migration, 'atomic_write', side_effect=failing):
            with self.assertRaises(OSError):
                migration.apply(plan)
        migration.rollback(plan)
        for client in migration.CLIENTS:
            self.assertEqual((self.connections / client / 'state.json').read_bytes(), self.originals[client])

    def test_duplicate_or_mismatching_state_is_rejected(self):
        path = self.connections / 'claude/state.json'
        for value in [b'{"dataDir":"a","dataDir":"b"}',
                      self.originals['claude'].replace(b'"schemaVersion": 1', b'"schemaVersion": 2'),
                      self.originals['claude'].replace(b'"client": "claude"', b'"client": "codex"')]:
            with self.subTest(value=value[:30]):
                path.write_bytes(value)
                with self.assertRaises(RuntimeError):
                    self.prepare()

    def test_partial_rollback_failure_can_resume_without_reverting_a_restored_state(self):
        plan = self.prepare()
        migration.apply(plan)
        write = migration.atomic_write
        calls = 0

        def failing(path, value, mode):
            nonlocal calls
            calls += 1
            if calls == 2:
                raise OSError('synthetic rollback failure')
            write(path, value, mode)

        with patch.object(migration, 'atomic_write', side_effect=failing):
            with self.assertRaises(OSError):
                migration.rollback(plan)
        self.assertEqual([entry['client'] for entry in plan['applied']], ['claude'])
        migration.rollback(plan)
        self.assertFalse(plan['applied'])
        for client in migration.CLIENTS:
            self.assertEqual((self.connections / client / 'state.json').read_bytes(), self.originals[client])

    def test_linked_state_is_rejected(self):
        path = self.connections / 'claude/state.json'
        external = self.root / 'external.json'
        path.rename(external)
        path.symlink_to(external)
        with self.assertRaises(RuntimeError):
            self.prepare()
        self.assertEqual(external.read_bytes(), self.originals['claude'])

    def test_missing_client_state_is_not_initialized(self):
        path = self.connections / 'codex/state.json'
        path.rename(self.root / 'preserved.json')
        plan = self.prepare()
        migration.apply(plan)
        self.assertFalse(path.exists())
        self.assertEqual(len(plan['entries']), 1)


if __name__ == '__main__':
    unittest.main()
