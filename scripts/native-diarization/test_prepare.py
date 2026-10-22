import importlib.util
import io
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
import zipfile

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("diarization_build", Path(__file__).with_name("prepare.py"))
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)


class PackagingTests(unittest.TestCase):
    def test_archive_paths_reject_traversal_and_other_bundle_roots(self):
        for name in ("/worker/bin/nemo-speech", "worker/../bin/nemo-speech", "other/bin/nemo-speech"):
            with self.assertRaises(ValueError):
                build.archive_path(name, "worker")
        self.assertEqual(str(build.archive_path("worker/lib/libggml.dylib", "worker")), "lib/libggml.dylib")
        self.assertIsNone(build.archive_path("worker/bin/other-tool", "worker"))
        self.assertIsNone(build.archive_path("worker/share/nemo-speech/config/default.yaml", "worker"))

    def test_mac_layout_materializes_aliases_and_preserves_licenses(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "worker.tar.gz"
            with tarfile.open(archive, "w:gz") as bundle:
                for name, data in {
                    "worker/bin/nemo-speech": b"binary",
                    "worker/lib/libggml.0.dylib": b"library",
                    "worker/share/licenses/nemo-speech/LICENSE": b"license",
                }.items():
                    item = tarfile.TarInfo(name)
                    item.size = len(data)
                    bundle.addfile(item, io.BytesIO(data))
                link = tarfile.TarInfo("worker/lib/libggml.dylib")
                link.type = tarfile.SYMTYPE
                link.linkname = "libggml.0.dylib"
                bundle.addfile(link)
            output = root / "staged"
            build.extract(archive, output, {"name": archive.name})
            alias = output / "lib/libggml.dylib"
            self.assertFalse(alias.is_symlink())
            self.assertEqual(alias.read_bytes(), b"library")
            self.assertEqual((output / "share/licenses/nemo-speech/LICENSE").read_bytes(), b"license")
            self.assertTrue((output / "bin/nemo-speech").stat().st_mode & 0o100)

    def test_windows_layout_keeps_dlls_beside_executable_and_rejects_duplicates(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "worker.zip"
            with zipfile.ZipFile(archive, "w") as bundle:
                bundle.writestr("worker/bin/nemo-speech.exe", b"binary")
                bundle.writestr("worker/bin/runtime.dll", b"dll")
            output = root / "staged"
            build.extract(archive, output, {"name": archive.name})
            self.assertEqual((output / "bin/runtime.dll").read_bytes(), b"dll")
            with self.assertRaises(FileExistsError):
                build.extract(archive, output, {"name": archive.name})


if __name__ == "__main__":
    unittest.main()
