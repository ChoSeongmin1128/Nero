import importlib.util
import io
import json
from pathlib import Path
import plistlib
import subprocess
import sys
import tarfile
import tempfile
import unittest

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('mac_update', Path(__file__).parents[1] / 'scripts/package-macos-update.py')
mac = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mac)


class MacUpdatePackageTests(unittest.TestCase):
    def fixture(self, root):
        app = root / 'Nero.app'
        (app / 'Contents/MacOS').mkdir(parents=True)
        (app / 'Contents/Resources').mkdir()
        executable = app / 'Contents/MacOS/nero-desktop'
        executable.write_bytes(b'synthetic executable')
        executable.chmod(0o755)
        build = dict(formatVersion=1, version='0.0.1', build='44', bundleIdentifier=mac.IDENTIFIER,
                     sourceCommit='a' * 40, sourceTree='b' * 40, sourceDirty=False,
                     hostPlatform='darwin', hostArchitecture='arm64')
        (app / 'Contents/Resources/build-info.json').write_text(json.dumps(build))
        (app / 'Contents/Info.plist').write_bytes(plistlib.dumps(dict(
            CFBundleIdentifier=mac.IDENTIFIER, CFBundleName='Nero',
            CFBundleShortVersionString='0.0.1', CFBundleVersion='44')))
        receipt = dict(bundle_id=mac.IDENTIFIER, version='0.0.1', build='44',
                       source_commit='a' * 40, source_tree='b' * 40, architecture='arm64',
                       signed=True, notarized=True, source_dirty=False,
                       app_tree_sha256=mac.installer.tree_hash(mac.installer.app_tree(app)))
        path = root / 'release.json'
        path.write_text(json.dumps(receipt))
        return app, path

    def test_roundtrip_keeps_executable_mode_and_binds_receipt_to_bytes(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            app, receipt = self.fixture(root)
            out = root / 'Nero44.app.tar.gz'
            checked = []
            notices_checked = []
            result = mac.package(app, out, receipt, trust=lambda value: checked.append(value),
                                 notices=lambda value: notices_checked.append(value))
            self.assertEqual(len(checked), 2)
            self.assertEqual(len(notices_checked), 2)
            self.assertEqual(result['artifact']['sha256'], mac.installer.sha(out))
            self.assertEqual(json.loads(Path(str(out) + '.build-info.json').read_text())['sourceCommit'], 'a' * 40)
            with self.assertRaisesRegex(RuntimeError, 'already exists'):
                mac.package(app, out, receipt, trust=lambda _: None, notices=lambda _: None)

    def test_missing_distribution_notices_prevent_final_updater_package(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            app, receipt = self.fixture(root)
            out = root / 'Nero44.app.tar.gz'
            with self.assertRaises(subprocess.CalledProcessError):
                mac.package(app, out, receipt, trust=lambda _: None)
            self.assertFalse(out.exists())

    def test_modified_signed_bundle_never_creates_package(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            app, receipt = self.fixture(root)
            (app / 'Contents/MacOS/nero-desktop').write_bytes(b'modified')
            out = root / 'Nero44.app.tar.gz'
            with self.assertRaisesRegex(RuntimeError, 'changed after'):
                mac.package(app, out, receipt, trust=lambda _: None)
            self.assertFalse(out.exists())

    def test_archive_rejects_escaping_paths_links_and_special_entries(self):
        cases = [('Nero.app/../../escape', None), ('Other.app/file', None),
                 ('Nero.app/link', '/tmp/outside')]
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for index, (name, link) in enumerate(cases):
                archive = root / f'{index}.tar.gz'
                with tarfile.open(archive, 'w:gz') as stream:
                    member = tarfile.TarInfo(name)
                    if link:
                        member.type = tarfile.SYMTYPE
                        member.linkname = link
                    else:
                        member.size = 1
                    stream.addfile(member, None if link else io.BytesIO(b'x'))
                with self.assertRaises((RuntimeError, tarfile.FilterError)):
                    mac.extract_archive(archive, root / f'out{index}')


if __name__ == '__main__':
    unittest.main()
