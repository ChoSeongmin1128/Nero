import importlib.util
import json
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    'install_macos', Path(__file__).parents[1] / 'apps/desktop/scripts/install-macos.py'
)
installer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(installer)


class MacOSInstallTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.target = self.root / 'Applications/Nero.app'
        self.app = self.root / 'candidate/Nero.app'
        self.data = self.root / 'data'
        self.legacy_data = self.root / 'legacy-data'
        self.connections = self.root / 'connections'
        self.work = self.root / 'installation'
        self.archive = self.root / 'release.zip'
        self.dmg = self.root / 'release.dmg'
        self.source = 'a' * 40
        self.source_tree = 'b' * 40
        self.signature = {
            'Identifier': installer.IDENTIFIER, 'TeamIdentifier': 'ABCDE12345', 'CDHash': 'c' * 40,
        }
        self.commands = []
        self.moves = []
        for folder in (self.data, self.connections, self.work):
            folder.mkdir()
        self.make_app(self.target, '24')
        self.make_app(self.app, '25')
        (self.data / 'store.lock').write_bytes(b'')
        (self.data / 'original.wav').write_bytes(b'synthetic original audio')
        (self.data / 'models').mkdir()
        (self.data / 'models/model.gguf').write_bytes(b'synthetic model bytes')
        for client in ('claude', 'codex'):
            folder = self.connections / client
            folder.mkdir()
            (folder / 'state.json').write_text('{}')
        self.archive.write_bytes(b'synthetic zip')
        self.dmg.write_bytes(b'synthetic dmg')
        self.receipt = {
            'version': '0.0.1', 'build': '25', 'bundle_id': installer.IDENTIFIER,
            'source_commit': self.source, 'source_tree': self.source_tree, 'source_dirty': False,
            'signed': True, 'notarized': True, 'notary': {'status': 'Accepted'},
            'app_signature': self.signature, 'app_tree_sha256': installer.tree_hash(installer.app_tree(self.app)),
            'zip_sha256': installer.sha(self.archive), 'dmg_sha256': installer.sha(self.dmg),
            'verification': {'embedded_app_matches_source': True},
        }
        self.write_receipts()
        for name, value in (('TARGET', self.target), ('DATA', self.data),
                            ('LEGACY_DATA', self.legacy_data), ('CONNECTIONS', self.connections)):
            mock = patch.object(installer, name, value)
            mock.start()
            self.addCleanup(mock.stop)
        mock = patch.object(installer, 'run', side_effect=self.run_command)
        mock.start()
        self.addCleanup(mock.stop)
        mock = patch.object(installer.time, 'sleep')
        mock.start()
        self.addCleanup(mock.stop)
        self.real_rename = installer.os.rename
        mock = patch.object(installer.os, 'rename', side_effect=self.rename)
        mock.start()
        self.addCleanup(mock.stop)

    def make_app(self, folder, build, version='0.0.1'):
        (folder / 'Contents/Resources/agent').mkdir(parents=True, exist_ok=True)
        info = {'CFBundleName': 'Nero', 'CFBundleVersion': build, 'CFBundleShortVersionString': version,
                'CFBundleIdentifier': installer.IDENTIFIER}
        (folder / 'Contents/Info.plist').write_bytes(plistlib.dumps(info))
        metadata = {'version': version, 'build': build, 'bundleIdentifier': installer.IDENTIFIER,
                    'sourceCommit': self.source, 'sourceTree': self.source_tree, 'sourceDirty': False}
        (folder / 'Contents/Resources/build-info.json').write_text(json.dumps(metadata))
        (folder / 'Contents/Resources/agent/nero').write_bytes(b'synthetic agent ' + build.encode())

    def write_receipts(self):
        for artifact in (self.archive, self.dmg):
            Path(str(artifact) + '.receipt.json').write_text(json.dumps(self.receipt))

    def run_command(self, *args):
        self.commands.append(args)
        output = ''
        if args[0] == 'git':
            output = self.source_tree + '\n'
        elif args[:3] == ('codesign', '-d', '--verbose=4'):
            output = 'Authority=Developer ID Application: Synthetic\nflags=runtime\n'
            output += ''.join(key + '=' + value + '\n' for key, value in self.signature.items())
        elif args[0] == str(self.target / 'Contents/Resources/agent/nero'):
            output = json.dumps({'meetings': [], 'sourceVersion': 'sha256:' + 'd' * 64,
                                 'pagination': {'offset': 0, 'limit': 200, 'hasMore': False}})
        elif args[0] not in ('ps', 'codesign', 'xcrun', 'spctl'):
            raise AssertionError('Unexpected external command: ' + str(args))
        return subprocess.CompletedProcess(args, 0, stdout=output, stderr='')

    def rename(self, source, destination):
        self.moves.append((source, destination))
        self.real_rename(source, destination)

    def install(self, with_zip=False, reset_from=None):
        return installer.install(self.app, self.dmg, self.work,
                                 archive=self.archive if with_zip else None,
                                 prepublication_reset_from=reset_from)

    def prepare_prepublication_reset(self, current_version='0.3.0', current_build='49',
                                    version='0.0.1', build='50'):
        self.make_app(self.target, current_build, current_version)
        self.make_app(self.app, build, version)
        self.receipt.update(version=version, build=build,
                            app_tree_sha256=installer.tree_hash(installer.app_tree(self.app)))
        self.write_receipts()

    def test_prepublication_reset_requires_the_explicit_source_and_records_the_transition(self):
        self.prepare_prepublication_reset()
        journal = self.data / 'pending.json'
        journal.write_bytes(b'{"from":"0.2.0+47","target":"0.3.0+49"}\n')
        data_before = installer.inventory(self.data)
        models_before = installer.model_metadata()
        connections_before = installer.connection_states()
        with self.assertRaisesRegex(RuntimeError, 'downgrade refused'):
            self.install()
        self.assertEqual(self.moves, [])
        self.assertFalse((self.work / 'installation.json').exists())
        result = self.install(reset_from='0.3.0+49')
        expected = {'schemaVersion': 1, 'flag': '--prepublication-reset-from', 'value': '0.3.0+49',
                    'from': {'version': '0.3.0', 'build': '49'},
                    'to': {'version': '0.0.1', 'build': '50'}}
        self.assertEqual(result['prepublicationReset'], expected)
        recorded = installer.load(self.work / 'installation.json')
        self.assertEqual(recorded['prepublicationReset'], expected)
        self.assertTrue(recorded['writerLockHeld'])
        events = [json.loads(line) for line in (self.work / 'installation-events.jsonl').read_text().splitlines()]
        self.assertEqual(events[0]['prepublicationReset'], expected)
        self.assertEqual(installer.inventory(self.data), data_before)
        self.assertEqual(installer.model_metadata(), models_before)
        self.assertEqual(installer.connection_states(), connections_before)
        self.moves.clear()
        for reset_from in ('0.3.0+49', None):
            repeated = self.install(reset_from=reset_from)
            self.assertTrue(repeated['alreadyInstalled'])
            self.assertFalse(repeated['changesMade'])
            self.assertEqual(repeated['prepublicationReset'], expected)
        self.assertEqual(self.moves, [])

    def test_prepublication_reset_refuses_other_source_target_and_flag_values(self):
        cases = [
            {'current_version': '0.2.0'}, {'current_build': '48'}, {'current_build': '50'},
            {'current_build': '049'}, {'version': '0.0.2'}, {'version': '0.0.01'},
            {'build': '49'}, {'build': '48'}, {'reset_from': '0.2.0+49'},
            {'reset_from': '0.3.0+48'},
        ]
        for case in cases:
            with self.subTest(case=case):
                values = dict(case)
                reset_from = values.pop('reset_from', '0.3.0+49')
                self.prepare_prepublication_reset(**values)
                with self.assertRaisesRegex(RuntimeError, '[Pp]republication reset'):
                    self.install(reset_from=reset_from)
                self.assertEqual(self.moves, [])
                self.assertFalse((self.work / 'data-before-manifest.json').exists())

    def test_prepublication_reset_refuses_later_builds_and_cannot_reuse_the_source_exception(self):
        self.prepare_prepublication_reset(build='51')
        with self.assertRaisesRegex(RuntimeError, 'target 0.0.1/build50'):
            self.install(reset_from='0.3.0+49')
        self.assertEqual(self.moves, [])
        self.prepare_prepublication_reset()
        self.install(reset_from='0.3.0+49')
        self.moves.clear()
        self.make_app(self.app, '51')
        self.receipt.update(build='51', app_tree_sha256=installer.tree_hash(installer.app_tree(self.app)))
        self.write_receipts()
        with self.assertRaisesRegex(RuntimeError, 'target 0.0.1/build50'):
            self.install(reset_from='0.3.0+49')
        self.assertEqual(self.moves, [])

    def test_prepublication_reset_retry_requires_its_original_installation_receipt(self):
        self.prepare_prepublication_reset()
        self.install(reset_from='0.3.0+49')
        self.moves.clear()
        receipt = installer.load(self.work / 'installation.json')
        receipt['prepublicationReset']['from']['build'] = '48'
        (self.work / 'installation.json').write_text(json.dumps(receipt))
        with self.assertRaisesRegex(RuntimeError, 'does not record this prepublication reset'):
            self.install(reset_from='0.3.0+49')
        self.assertEqual(self.moves, [])

    def test_prepublication_reset_does_not_add_authorization_to_an_ordinary_installation(self):
        self.prepare_prepublication_reset(current_version='0.0.1')
        self.install()
        self.moves.clear()
        with self.assertRaisesRegex(RuntimeError, 'does not record this prepublication reset'):
            self.install(reset_from='0.3.0+49')
        self.assertEqual(self.moves, [])

    def test_prepublication_reset_preserves_release_lock_and_idle_guards(self):
        self.prepare_prepublication_reset()
        self.receipt['notary'] = {'status': 'Rejected'}
        self.write_receipts()
        with self.assertRaisesRegex(RuntimeError, 'notarization'):
            self.install(reset_from='0.3.0+49')
        self.receipt['notary'] = {'status': 'Accepted'}
        self.write_receipts()
        with patch.object(installer.fcntl, 'flock', side_effect=BlockingIOError()):
            with self.assertRaisesRegex(RuntimeError, 'writer lock'):
                self.install(reset_from='0.3.0+49')
        with patch.object(installer, 'idle_snapshot', side_effect=RuntimeError('A recording is active')):
            with self.assertRaisesRegex(RuntimeError, 'recording is active'):
                self.install(reset_from='0.3.0+49')
        self.assertEqual(self.moves, [])
        self.assertFalse((self.work / 'data-before-manifest.json').exists())

    def test_prepublication_reset_verification_failure_restores_both_app_bundles(self):
        self.prepare_prepublication_reset()
        original = installer.app_tree(self.target)
        candidate = installer.app_tree(self.app)
        data_before = installer.inventory(self.data)
        run = self.run_command

        def fail_installed(*args):
            if args == ('codesign', '--verify', '--deep', '--strict', str(self.target)):
                raise RuntimeError('synthetic installed verification failure')
            return run(*args)

        with patch.object(installer, 'run', side_effect=fail_installed):
            with self.assertRaisesRegex(RuntimeError, 'installed verification failure'):
                self.install(reset_from='0.3.0+49')
        self.assertEqual(installer.app_tree(self.target), original)
        self.assertEqual(installer.app_tree(self.app), candidate)
        self.assertEqual(installer.inventory(self.data), data_before)
        self.assertFalse((self.work / 'installation.json').exists())

    def test_prepublication_reset_cannot_be_combined_with_identity_migration(self):
        self.prepare_prepublication_reset()
        self.use_old_identity()
        with self.assertRaisesRegex(RuntimeError, 'current Nero identity'):
            self.install(reset_from='0.3.0+49')
        self.assertEqual(self.moves, [])

    def test_prepublication_reset_cli_passes_the_explicit_source(self):
        with patch.object(installer, 'install', return_value={}) as install, patch('builtins.print'):
            installer.main(['--app', str(self.app), '--dmg', str(self.dmg), '--work', str(self.work),
                            '--prepublication-reset-from', '0.3.0+49'])
        self.assertEqual(install.call_args.kwargs['prepublication_reset_from'], '0.3.0+49')

    def test_success_moves_bundles_and_records_hashes_without_copying_data(self):
        candidate_inode = self.app.stat().st_ino
        previous_inode = self.target.stat().st_ino
        data_before = installer.inventory(self.data)
        model_before = installer.model_metadata()
        result = self.install()
        self.assertEqual(result['build'], '25')
        self.assertEqual(self.target.stat().st_ino, candidate_inode)
        self.assertEqual((self.work / 'previous-installed.app').stat().st_ino, previous_inode)
        self.assertFalse(self.app.exists())
        self.assertFalse((self.work / 'data-before').exists())
        self.assertEqual(installer.inventory(self.data), data_before)
        self.assertEqual(installer.model_metadata(), model_before)
        receipt = installer.load(self.work / 'installation.json')
        self.assertFalse(receipt['dataBackupCreated'])
        self.assertFalse(receipt['userDataCopiedOrMoved'])
        self.assertFalse(receipt['appCopied'])
        self.assertFalse(receipt['guiVerified'])
        self.assertEqual(installer.load(self.work / 'data-before-manifest.json'), data_before)
        self.assertEqual(len(self.moves), 4)
        self.assertTrue(self.install()['alreadyInstalled'])

    def test_dmg_only_install_does_not_require_zip_or_its_receipt(self):
        self.archive.rename(self.root / 'unused.zip')
        Path(str(self.archive) + '.receipt.json').rename(self.root / 'unused-zip-receipt.json')
        self.assertEqual(self.install()['build'], '25')

    def test_dmg_only_install_preserves_hash_signature_notary_and_embedded_app_guards(self):
        for field, invalid in (('dmg_sha256', '0' * 64), ('app_tree_sha256', '0' * 64),
                               ('app_signature', {}), ('notary', {'status': 'Rejected'}),
                               ('verification', {'embedded_app_matches_source': False})):
            with self.subTest(field=field):
                original = self.receipt[field]
                self.receipt[field] = invalid
                self.write_receipts()
                with self.assertRaises(RuntimeError):
                    self.install()
                self.assertEqual(self.moves, [])
                self.receipt[field] = original
        self.write_receipts()

    def test_optional_zip_is_checked_when_supplied(self):
        self.assertEqual(self.install(with_zip=True)['build'], '25')

    def test_installed_verification_failure_restores_old_and_candidate_in_order(self):
        original = installer.app_tree(self.target)
        candidate = installer.app_tree(self.app)
        run = self.run_command

        def fail_installed(*args):
            if args == ('codesign', '--verify', '--deep', '--strict', str(self.target)):
                raise RuntimeError('synthetic installed verification failure')
            return run(*args)

        with patch.object(installer, 'run', side_effect=fail_installed):
            with self.assertRaisesRegex(RuntimeError, 'installed verification failure'):
                self.install()
        stage = self.target.with_name('.Nero-install.app')
        previous = self.work / 'previous-installed.app'
        self.assertEqual(self.moves, [(self.app, stage), (self.target, previous), (stage, self.target),
                                      (self.target, stage), (previous, self.target), (stage, self.app)])
        self.assertEqual(installer.app_tree(self.target), original)
        self.assertEqual(installer.app_tree(self.app), candidate)
        self.assertFalse((self.work / 'installation.json').exists())
        self.assertFalse(previous.exists())

    def test_stage_verification_failure_returns_candidate_without_moving_installed_app(self):
        run = self.run_command
        stage = self.target.with_name('.Nero-install.app')

        def fail_stage(*args):
            if args == ('codesign', '--verify', '--deep', '--strict', str(stage)):
                raise RuntimeError('synthetic stage verification failure')
            return run(*args)

        with patch.object(installer, 'run', side_effect=fail_stage):
            with self.assertRaisesRegex(RuntimeError, 'stage verification failure'):
                self.install()
        self.assertEqual(self.moves, [(self.app, stage), (stage, self.app)])
        self.assertFalse((self.work / 'installation.json').exists())
        self.assertEqual(plistlib.loads((self.target / 'Contents/Info.plist').read_bytes())['CFBundleVersion'], '24')

    def test_candidate_promotion_failure_restores_previous_before_returning_candidate(self):
        stage = self.target.with_name('.Nero-install.app')
        previous = self.work / 'previous-installed.app'
        rename = self.rename

        def fail_promotion(source, destination):
            if source == stage and destination == self.target:
                raise OSError('synthetic promotion failure')
            return rename(source, destination)

        with patch.object(installer.os, 'rename', side_effect=fail_promotion):
            with self.assertRaisesRegex(OSError, 'promotion failure'):
                self.install()
        self.assertEqual(self.moves, [(self.app, stage), (self.target, previous),
                                      (previous, self.target), (stage, self.app)])
        self.assertFalse((self.work / 'installation.json').exists())

    def test_receipt_mismatch_and_downgrade_are_refused_before_changes(self):
        self.receipt['zip_sha256'] = '0' * 64
        self.write_receipts()
        with self.assertRaisesRegex(RuntimeError, 'ZIP hash'):
            self.install(with_zip=True)
        self.assertEqual(self.moves, [])
        self.receipt['zip_sha256'] = installer.sha(self.archive)
        self.write_receipts()
        info_path = self.target / 'Contents/Info.plist'
        info = plistlib.loads(info_path.read_bytes())
        info['CFBundleVersion'] = '26'
        info_path.write_bytes(plistlib.dumps(info))
        with self.assertRaisesRegex(RuntimeError, 'downgrade refused'):
            self.install()
        self.assertEqual(self.moves, [])

    def test_active_latest_job_is_refused_without_replacing_any_app(self):
        states = self.data / 'processing/job-1/job-state'
        states.mkdir(parents=True)
        (states / '00000000000000000001.json').write_text(json.dumps({'id': 'job-1', 'status': 'completed'}))
        (states / '00000000000000000002.json').write_text(json.dumps({'id': 'job-1', 'status': 'queued'}))
        with self.assertRaisesRegex(RuntimeError, 'processing job is active'):
            self.install()
        self.assertEqual(self.moves, [])
        self.assertFalse((self.work / 'data-before-manifest.json').exists())

    def test_writer_lock_contention_preserves_installed_app_and_data(self):
        with patch.object(installer.fcntl, 'flock', side_effect=BlockingIOError()):
            with self.assertRaisesRegex(RuntimeError, 'writer lock'):
                self.install()
        self.assertEqual(self.moves, [])
        self.assertFalse((self.work / 'data-before-manifest.json').exists())

    def test_unreadable_subdirectory_cannot_produce_a_partial_preservation_manifest(self):
        def unreadable(folder, *, onerror):
            onerror(PermissionError('synthetic unreadable directory'))
            return iter(())

        with patch.object(installer.os, 'walk', side_effect=unreadable):
            for inventory in (installer.inventory, installer.app_tree):
                with self.assertRaisesRegex(PermissionError, 'unreadable directory'):
                    inventory(self.data)
            with self.assertRaisesRegex(PermissionError, 'unreadable directory'):
                installer.model_metadata()

    def use_old_identity(self):
        self.real_rename(self.data, self.legacy_data)
        info_path = self.target / 'Contents/Info.plist'
        info = plistlib.loads(info_path.read_bytes())
        info['CFBundleIdentifier'] = installer.LEGACY_IDENTIFIER
        info_path.write_bytes(plistlib.dumps(info))
        for client in ('claude', 'codex'):
            (self.connections / client / 'state.json').write_text(json.dumps({
                'schemaVersion': 1, 'client': client, 'token': '11111111-1111-4111-8111-111111111111',
                'generation': 'a' * 64, 'dataDir': str(self.legacy_data),
            }))

    def test_identity_migration_renames_under_lock_without_copying_recordings_or_models(self):
        self.use_old_identity()
        before = installer.inventory(self.legacy_data)
        inode = self.legacy_data.stat().st_ino
        model = (self.legacy_data / 'models/model.gguf').stat()
        self.install()
        self.assertFalse(self.legacy_data.exists())
        self.assertEqual(self.data.stat().st_ino, inode)
        self.assertEqual(installer.inventory(self.data), before)
        self.assertEqual((self.data / 'models/model.gguf').stat().st_ino, model.st_ino)
        self.assertEqual(installer.load(self.data / installer.MIGRATION_RECEIPT),
                         installer.migration_receipt(self.legacy_data, 'completed'))
        receipt = installer.load(self.work / 'installation.json')
        self.assertTrue(receipt['identityMigration'])
        self.assertTrue(receipt['dataRenamedWithoutCopy'])
        self.assertFalse(receipt['dataBackupCreated'])

    def test_identity_migration_refuses_two_roots_and_held_writer_lock(self):
        self.use_old_identity()
        self.data.mkdir()
        with self.assertRaisesRegex(RuntimeError, 'Both identity'):
            self.install()
        self.data.rename(self.root / 'conflicting-data-preserved')
        self.moves.clear()
        with patch.object(installer.fcntl, 'flock', side_effect=BlockingIOError()):
            with self.assertRaisesRegex(RuntimeError, 'writer lock'):
                self.install()
        self.assertTrue(self.legacy_data.exists())
        self.assertFalse(self.data.exists())
        self.assertEqual(self.moves, [])

    def test_identity_migration_rename_failure_retains_old_data_and_returns_candidate(self):
        self.use_old_identity()
        before = installer.inventory(self.legacy_data)
        rename = self.rename
        def refuse(source, destination):
            if source == self.legacy_data and destination == self.data:
                raise OSError('synthetic data rename failure')
            return rename(source, destination)
        with patch.object(installer.os, 'rename', side_effect=refuse):
            with self.assertRaisesRegex(OSError, 'data rename failure'):
                self.install()
        self.assertEqual(installer.inventory(self.legacy_data), before)
        self.assertFalse((self.legacy_data / installer.MIGRATION_RECEIPT).exists())
        self.assertFalse(self.data.exists())
        self.assertTrue(self.app.exists())

    def test_failed_new_app_validation_rolls_back_data_identity_and_app(self):
        self.use_old_identity()
        before = installer.inventory(self.legacy_data)
        run = self.run_command
        def refuse(*args):
            if args == ('codesign', '--verify', '--deep', '--strict', str(self.target)):
                raise RuntimeError('synthetic installed verification failure')
            return run(*args)
        rename = self.rename
        restored = []
        def observe_restore(source, destination):
            if source == self.work / 'previous-installed.app' and destination == self.target:
                restored.append(True)
                self.assertTrue(self.legacy_data.is_dir())
                self.assertFalse(self.data.exists())
                for client in ('claude', 'codex'):
                    self.assertEqual(installer.load(self.connections / client / 'state.json')['dataDir'],
                                     str(self.legacy_data))
            return rename(source, destination)
        with patch.object(installer, 'run', side_effect=refuse), \
                patch.object(installer.os, 'rename', side_effect=observe_restore):
            with self.assertRaisesRegex(RuntimeError, 'installed verification failure'):
                self.install()
        self.assertEqual(installer.inventory(self.legacy_data), before)
        self.assertFalse(self.data.exists())
        self.assertEqual(plistlib.loads((self.target / 'Contents/Info.plist').read_bytes())['CFBundleIdentifier'],
                         installer.LEGACY_IDENTIFIER)
        self.assertEqual(restored, [True])

    def test_old_app_cannot_reenter_between_library_rename_and_new_app_promotion(self):
        self.use_old_identity()
        rename = self.rename
        attempts = []
        def launch_if_available(source, destination):
            result = rename(source, destination)
            if source == self.legacy_data and destination == self.data:
                attempts.append(self.target.exists())
                if self.target.exists():
                    self.legacy_data.mkdir()
                    (self.legacy_data / 'store.lock').write_bytes(b'')
                self.assertTrue((self.work / 'previous-installed.app').is_dir())
            return result
        with patch.object(installer.os, 'rename', side_effect=launch_if_available):
            self.install()
        self.assertEqual(attempts, [False])
        self.assertFalse(self.legacy_data.exists())
        self.assertTrue(self.data.is_dir())

    def test_pending_or_mismatched_migration_receipt_blocks_normal_and_repeated_installation(self):
        expected = installer.migration_receipt(self.legacy_data, 'completed')
        path = self.data / installer.MIGRATION_RECEIPT
        invalid_values = [dict(expected, status='pending'), dict(expected, sourceData='/other'),
                          dict(expected, destinationData='/other'), dict(expected, extra=True),
                          dict(expected, schemaVersion=True)]
        for value in invalid_values:
            with self.subTest(value=value):
                path.write_text(json.dumps(value))
                with self.assertRaisesRegex(RuntimeError, 'Identity migration'):
                    self.install()
                self.assertEqual(self.moves, [])
        path.write_text(json.dumps(expected)[:-1] + ', "status": "completed"}')
        with self.assertRaisesRegex(RuntimeError, 'duplicate fields'):
            self.install()
        path.write_text(json.dumps(expected))
        self.install()
        self.moves.clear()
        path.write_text(json.dumps(dict(expected, status='pending')))
        with self.assertRaisesRegex(RuntimeError, 'Identity migration is incomplete'):
            self.install()
        self.assertEqual(self.moves, [])

    def test_recreated_old_library_stops_migration_without_merging_or_discarding_either_root(self):
        self.use_old_identity()
        run = self.run_command
        recreated = []
        def recreate_old_library(*args):
            if args == ('codesign', '--verify', '--deep', '--strict', str(self.target)):
                self.legacy_data.mkdir()
                (self.legacy_data / 'reentered.txt').write_bytes(b'newly created user data')
                recreated.append(True)
            return run(*args)
        with patch.object(installer, 'run', side_effect=recreate_old_library):
            with self.assertRaisesRegex(RuntimeError, 'preserve the stage and rollback app'):
                self.install()
        self.assertEqual(recreated, [True])
        self.assertEqual((self.legacy_data / 'reentered.txt').read_bytes(), b'newly created user data')
        self.assertEqual((self.data / 'original.wav').read_bytes(), b'synthetic original audio')
        self.assertFalse((self.legacy_data / 'original.wav').exists())
        self.assertFalse((self.data / 'reentered.txt').exists())
        self.assertTrue((self.work / 'previous-installed.app').is_dir())
        self.assertTrue(self.target.with_name('.Nero-install.app').is_dir())
        self.assertFalse(self.target.exists())


if __name__ == '__main__':
    unittest.main()
