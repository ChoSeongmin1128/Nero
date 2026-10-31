import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import struct
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('whisper_prepare', Path(__file__).with_name('prepare.py'))
p = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p)


class PackageTests(unittest.TestCase):
    def pe_fixture(self, path, dependencies=()):
        data = bytearray(4096)
        data[:2] = b'MZ'
        struct.pack_into('<I', data, 60, 128)
        data[128:134] = b'PE\0\0\x64\x86'
        struct.pack_into('<H', data, 134, 1)
        struct.pack_into('<H', data, 148, 240)
        data[152:154] = b'\x0b\x02'
        struct.pack_into('<IIII', data, 392 + 8, 2048, 4096, 2048, 1024)
        if dependencies:
            struct.pack_into('<II', data, 152 + 112 + 8, 4096, (len(dependencies) + 1) * 20)
            for i, name in enumerate(dependencies):
                struct.pack_into('<I', data, 1024 + i * 20 + 12, 4096 + 512 + i * 256)
                raw = name.encode() + b'\0'
                data[1536 + i * 256:1536 + i * 256 + len(raw)] = raw
        path.write_bytes(data)

    def fixture(self, root, target='windows-x64'):
        names = ['share/licenses/whisper.cpp/LICENSE']
        if target == 'windows-x64':
            names += ['bin/' + name for name in ('whisper-cli.exe', 'whisper.dll', 'ggml.dll',
                'ggml-base.dll', 'ggml-cpu-x64.dll', 'msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll',
                'nero-vulkan-probe.exe')]
            names += ['vulkan/bin/whisper-cli.exe', 'share/licenses/vulkan-sdk/LICENSE.txt']
            names += [f"share/licenses/{item['repository'].split('/')[-1]}/{license}"
                      for item in p.SPEC['targets']['windows-x64']['vulkan']['dependencies']
                      for license in item['licenses']]
        else:
            names += ['bin/whisper-cli', 'lib/libwhisper.1.dylib', 'lib/libggml-metal.0.dylib']
        for name in names:
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            if target == 'windows-x64' and path.suffix in ('.exe', '.dll'):
                self.pe_fixture(path, ['vulkan-1.dll'] if name.startswith('vulkan/') else [])
            else:
                path.write_bytes(b'\xcf\xfa\xed\xfe fixture' if not name.endswith('LICENSE') else b'license')
        receipt = {'runtime': copy.deepcopy(p.SPEC), 'target': target,
                   'sourceArchiveHashesVerified': True, 'files': p.package_files(root)}
        if target == 'windows-x64':
            receipt['windowsBuild'] = {'flags': p.SPEC['targets'][target]['vulkan']['cmakeFlags'],
                'toolchainReceiptSha256': 'a'*64, 'cmakeCacheSha256': 'b'*64, 'compiler': {'sha256': 'c'*64}}
            pe = p.load_module('nero_test_pe', Path(__file__).with_name('windows_pe.py'))
            receipt['windowsDependencies'] = pe.verify_closure(root)
        self.write_receipt(root, receipt)
        return receipt

    def test_missing_vulkan_build_or_dependency_evidence_is_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            receipt = self.fixture(root)
            receipt.pop('windowsBuild')
            self.write_receipt(root, receipt)
            with self.assertRaisesRegex(ValueError, 'build evidence'):
                p.verify_staged(root)
            receipt = self.fixture(root)
            pe = p.load_module('nero_test_pe', Path(__file__).with_name('windows_pe.py'))
            for dependencies in [['unpackaged.dll'], ['vulkan-1.dll']]:
                self.pe_fixture(root / 'bin/nero-vulkan-probe.exe', dependencies)
                with self.assertRaises(ValueError):
                    pe.verify_closure(root)

    def write_receipt(self, root, receipt):
        (root / 'runtime.json').write_text(json.dumps(receipt), encoding='utf-8')

    def test_file_mutation_and_unexpected_server_are_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.fixture(root)
            p.verify_staged(root, 'windows-x64')
            (root / 'bin/whisper-cli.exe').write_bytes(b'different')
            with self.assertRaisesRegex(ValueError, 'differ'):
                p.verify_staged(root, 'windows-x64')
            (root / 'bin/whisper-server.exe').write_bytes(b'server')
            with self.assertRaises(ValueError):
                p.package_files(root)

    def test_missing_cpu_dispatch_or_license_is_rejected(self):
        for missing in ('bin/ggml-cpu-x64.dll', 'share/licenses/whisper.cpp/LICENSE'):
            with self.subTest(missing=missing), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                receipt = self.fixture(root)
                receipt['files'] = [item for item in receipt['files'] if item['path'] != missing]
                self.write_receipt(root, receipt)
                with self.assertRaisesRegex(ValueError, 'missing'):
                    p.package_receipt(root)

    def test_source_or_target_changes_are_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            receipt = self.fixture(root)
            with self.assertRaises(ValueError):
                p.verify_staged(root, 'macos-arm64')
            receipt['runtime']['commit'] = '0' * 40
            self.write_receipt(root, receipt)
            with self.assertRaisesRegex(ValueError, 'pinned'):
                p.verify_staged(root)

    def test_other_target_recipe_changes_do_not_invalidate_unchanged_metal_receipt(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            receipt = self.fixture(root, 'macos-arm64')
            receipt['runtime']['targets']['windows-x64'] = {'backend': 'old-cpu-recipe'}
            self.write_receipt(root, receipt)
            p.package_receipt(root, 'macos-arm64')
            receipt['runtime']['targets']['macos-arm64']['backend'] = 'other'
            self.write_receipt(root, receipt)
            with self.assertRaises(ValueError):
                p.package_receipt(root, 'macos-arm64')

    def test_duplicate_receipt_and_traversal_are_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            receipt = self.fixture(root)
            receipt['files'].append(receipt['files'][0])
            self.write_receipt(root, receipt)
            with self.assertRaises(ValueError):
                p.package_receipt(root)
        for name in ('../bin/whisper-cli', '/bin/whisper-cli', 'package/../bin/whisper-cli',
                     'package/bin/../whisper-cli', 'package/bin\\whisper-cli',
                     'package/bin/./whisper-cli', 'package/bin/C:whisper-cli'):
            with self.subTest(name=name), self.assertRaises(ValueError):
                p.package_path(name, 'package')

    def test_corrupt_cached_archive_never_downloads_or_overwrites(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            archive = root / 'archive.zip'
            archive.write_bytes(b'corrupt')
            with patch.object(p.urllib.request, 'urlopen') as network:
                with self.assertRaises(ValueError):
                    p.download(root, archive.name, {'bytes': 7, 'sha256': '0' * 64}, 'https://example.invalid')
                network.assert_not_called()
            self.assertEqual(archive.read_bytes(), b'corrupt')

    def test_signing_transition_is_read_only_and_preserves_resource_hashes(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            before = self.fixture(root, 'macos-arm64')
            with (root / 'bin/whisper-cli').open('ab') as stream:
                stream.write(b'signed')
            with patch.object(p, 'verify_macos_links'), patch.object(p.subprocess, 'run') as mutation:
                transition = p.signed_transition(root, before)
                mutation.assert_not_called()
                p.verify_signed(root, transition)
                (root / 'share/licenses/whisper.cpp/LICENSE').write_bytes(b'changed license')
                with self.assertRaises(ValueError):
                    p.signed_transition(root, before)
                with self.assertRaises(ValueError):
                    p.verify_signed(root, transition)
                # A forged binary header must never make a license signable.
                (root / 'share/licenses/whisper.cpp/LICENSE').write_bytes(b'\xcf\xfa\xed\xfe forged')
                with self.assertRaises(ValueError):
                    p.signed_transition(root, before)

    def test_loader_checks_reject_external_or_missing_dependencies(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.fixture(root, 'macos-arm64')
            def rpath(path):
                return ['@loader_path' if path.suffix == '.dylib' else '@executable_path/../lib']
            def dependencies(path):
                return (['@rpath/' + path.name] if path.suffix == '.dylib' else []) + ['/usr/lib/libSystem.B.dylib']
            with patch.object(p, 'macos_rpaths', side_effect=rpath), patch.object(p, 'macos_dependencies', side_effect=dependencies):
                p.verify_macos_links(root)
            for bad in ('/tmp/build/libwhisper.dylib', '@rpath/missing.dylib', '@rpath/../libwhisper.1.dylib',
                        '/usr/lib/../../tmp/unpackaged.dylib', '/System/Library/../tmp/unpackaged.dylib'):
                with self.subTest(bad=bad), patch.object(p, 'macos_rpaths', side_effect=rpath), \
                        patch.object(p, 'macos_dependencies', return_value=[bad]):
                    with self.assertRaises(ValueError):
                        p.verify_macos_links(root)

    def test_relocation_rejects_system_path_traversal_before_mutating(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.fixture(root, 'macos-arm64')
            with patch.object(p, 'macos_dependencies', return_value=['/usr/lib/../../tmp/unpackaged.dylib']), \
                    patch.object(p.subprocess, 'run') as mutation:
                with self.assertRaises(ValueError):
                    p.relocate_macos(root)
                mutation.assert_not_called()


if __name__ == '__main__':
    unittest.main()
