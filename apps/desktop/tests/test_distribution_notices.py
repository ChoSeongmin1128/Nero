import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('distribution_notices', Path(__file__).parents[1] / 'scripts/distribution-notices.py')
notices = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notices)


class DistributionNoticeTests(unittest.TestCase):
    def fixture(self, root):
        (root / 'Cargo.lock').write_text('synthetic locked graph\n')
        (root / 'THIRD_PARTY_NOTICES.md').write_text('Synthetic distribution notices\n')
        folder = root / 'crate'
        folder.mkdir()
        (folder / 'LICENSE').write_text('Synthetic upstream copyright and permission notice\n')
        ids = ['nero-desktop', 'nero-cli', 'runtime', 'transitive', 'build-tool', 'dev-only', 'derive', 'macro-helper']
        packages = [dict(id=name, name=name, version='1.0.0', license='ISC', repository='https://example.test/source',
                         source='registry+https://example.test/index', authors=['Upstream'],
                         manifest_path=str(folder / 'Cargo.toml'),
                         targets=[{'kind': ['proc-macro' if name == 'derive' else 'lib']}]) for name in ids]
        def dep(name, kind=None):
            return {'pkg': name, 'dep_kinds': [{'kind': kind, 'target': None}]}
        edges = {'nero-desktop': [dep('runtime'), dep('build-tool', 'build'), dep('dev-only', 'dev'), dep('derive')],
                 'nero-cli': [dep('runtime')], 'runtime': [dep('transitive')], 'derive': [dep('macro-helper')]}
        document = {'packages': packages, 'workspace_members': ['nero-desktop', 'nero-cli'],
                    'resolve': {'nodes': [{'id': name, 'deps': edges.get(name, [])} for name in ids]}}
        model_folder = root / 'apps/desktop/licenses/models'
        model_folder.mkdir(parents=True)
        license_bytes = b'Synthetic model license text\n'
        (model_folder / 'LICENSE').write_bytes(license_bytes)
        entries = []
        for name in ['config/models/nemotron-asr.json', 'config/models/whisper-turbo.json',
                     'apps/desktop/models/diarization.json', 'apps/desktop/models/summary.json']:
            model = {'repository': name, 'revision': 'a' * 40,
                     'files': [{'name': 'model.bin', 'bytes': 5, 'sha256': 'b' * 64}]}
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(model))
            entries.append({'manifest': name, 'model': model, 'license': 'MIT',
                            'notices': [{'path': 'LICENSE', 'sha256': notices.sha(license_bytes),
                                         'source': 'https://example.test/pinned/LICENSE'}]})
        (model_folder / 'manifest.json').write_text(json.dumps(entries))
        return document

    def test_target_runtime_graph_excludes_build_dev_and_proc_macro_code(self):
        with tempfile.TemporaryDirectory() as folder:
            document = self.fixture(Path(folder))
            self.assertEqual([p['name'] for p in notices.runtime_packages(document)], ['runtime', 'transitive'])

    def test_packaged_resources_require_every_runtime_and_model_notice_on_both_targets(self):
        for target in notices.TARGETS:
            with self.subTest(target=target), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                document = self.fixture(root)
                resource = root / ('Nero.app/Contents/Resources' if target.startswith('aarch64') else 'installed-resources')
                output = resource / 'native/licenses/distribution'
                before = (root / 'config/models/nemotron-asr.json').read_bytes()
                manifest = notices.prepare(output, target, document, root)
                self.assertEqual(notices.verify(output, target, document, root), manifest)
                self.assertEqual((root / 'config/models/nemotron-asr.json').read_bytes(), before)
                missing = output / 'rust/runtime-1.0.0/LICENSE'
                missing.rename(root / 'omitted-LICENSE')
                with self.assertRaisesRegex(ValueError, 'Missing or linked notice'):
                    notices.verify(output, target, document, root)
                notices.prepare(output, target, document, root)
                (output / 'models/LICENSE').write_text('modified')
                with self.assertRaisesRegex(ValueError, 'changed packaged notice'):
                    notices.verify(output, target, document, root)

    def test_manifest_cannot_omit_a_runtime_package_or_relabel_another_target(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            document = self.fixture(root)
            output = root / 'generated'
            manifest = notices.prepare(output, notices.TARGETS[0], document, root)
            manifest['packages'].pop()
            (output / 'manifest.json').write_text(json.dumps(manifest))
            with self.assertRaisesRegex(ValueError, 'runtime graph'):
                notices.verify(output, notices.TARGETS[0], document, root)
            notices.prepare(output, notices.TARGETS[0], document, root)
            with self.assertRaisesRegex(ValueError, 'runtime graph'):
                notices.verify(output, notices.TARGETS[1], document, root)

    def test_unreviewed_model_pin_or_missing_source_license_stops_generation(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            document = self.fixture(root)
            pin = root / 'apps/desktop/models/summary.json'
            model = json.loads(pin.read_text())
            model['revision'] = 'c' * 40
            pin.write_text(json.dumps(model))
            with self.assertRaisesRegex(ValueError, 'pinned model'):
                notices.prepare(root / 'generated', notices.TARGETS[0], document, root)
            (root / 'crate/LICENSE').rename(root / 'LICENSE-not-in-crate')
            supplements = root / 'apps/desktop/licenses/rust-supplements'
            supplements.mkdir()
            (supplements / 'manifest.json').write_text('[]')
            with self.assertRaisesRegex(ValueError, 'Missing upstream license text'):
                notices.prepare(root / 'generated', notices.TARGETS[0], document, root)

    def test_regeneration_removes_stale_target_notices_and_rejects_linked_output(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            document = self.fixture(root)
            output = root / 'generated'
            notices.prepare(output, notices.TARGETS[0], document, root)
            (output / 'stale-Windows-NOTICE').write_text('stale')
            notices.prepare(output, notices.TARGETS[1], document, root)
            self.assertFalse((output / 'stale-Windows-NOTICE').exists())
            notices.verify(output, notices.TARGETS[1], document, root)
            if hasattr(Path, 'symlink_to'):
                try:
                    (root / 'linked').symlink_to(output, target_is_directory=True)
                except OSError:
                    return
                with self.assertRaisesRegex(ValueError, 'Invalid generated'):
                    notices.prepare(root / 'linked', notices.TARGETS[1], document, root)


if __name__ == '__main__':
    unittest.main()
