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
        for name, value in (('TARGET', self.target), ('DATA', self.data), ('CONNECTIONS', self.connections)):
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

    def make_app(self, folder, build):
        (folder / 'Contents/Resources/agent').mkdir(parents=True)
        info = {'CFBundleName': 'Nero', 'CFBundleVersion': build, 'CFBundleShortVersionString': '0.0.1',
                'CFBundleIdentifier': installer.IDENTIFIER}
        (folder / 'Contents/Info.plist').write_bytes(plistlib.dumps(info))
        metadata = {'version': '0.0.1', 'build': build, 'bundleIdentifier': installer.IDENTIFIER,
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

    def install(self, with_zip=False):
        return installer.install(self.app, self.dmg, self.work,
                                 archive=self.archive if with_zip else None)

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


if __name__ == '__main__':
    unittest.main()
