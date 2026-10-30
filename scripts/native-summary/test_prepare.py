import copy
import importlib.util
import json
import errno
from pathlib import Path
import struct
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("summary_prepare", Path(__file__).with_name("prepare.py"))
prepare = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(prepare)


def versioned_dll(path, version):
    major, minor, build, revision = version
    info = [0xFEEF04BD, 0x00010000, major << 16 | minor, build << 16 | revision] + [0] * 9
    path.write_bytes(b"MZ" + bytes(62) + struct.pack("<13I", *info))


class SummaryPackageTests(unittest.TestCase):
    def test_old_crt_fails_before_any_package_is_created(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for name in ["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"]:
                versioned_dll(root / name, (14, 29, 30133, 0))
            output = root / "package"
            with self.assertRaisesRegex(ValueError, "older than the qualified"):
                prepare.prepare("windows-x64", root / "cache", output, root)
            self.assertFalse(output.exists())
            self.assertFalse((root / "cache").exists())

    def test_crt_discovery_never_copies_an_unrelated_runtime_dll(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            names = {"msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"}
            for name in names:
                versioned_dll(root / name, prepare.MIN_WINDOWS_CRT)
            (root / "ggml.dll").write_bytes(b"must not overwrite llama's runtime")
            self.assertEqual(set(prepare.windows_crt(root)), names)

    def test_missing_or_ambiguous_dll_metadata_is_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "runtime.dll"
            path.write_bytes(b"MZ no version resource")
            with self.assertRaises(ValueError):
                prepare.crt_version(path)
            versioned_dll(path, prepare.MIN_WINDOWS_CRT)
            path.write_bytes(path.read_bytes() * 2)
            with self.assertRaises(ValueError):
                prepare.crt_version(path)

    def package(self, root):
        for backend in ["cpu", "gpu"]:
            directory = root / backend
            directory.mkdir()
            (directory / "llama-server.exe").write_bytes(b"synthetic package, never executed")
            for name in ["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"]:
                versioned_dll(directory / name, prepare.MIN_WINDOWS_CRT)
        receipt = {
            "target": "windows-x64", "runtime": copy.deepcopy(prepare.SPEC),
            "source_archive_hashes_verified": True,
            "files": [{"path": str(p.relative_to(root)), "bytes": p.stat().st_size,
                       "sha256": prepare.sha(p)} for p in sorted(root.rglob("*")) if p.is_file()],
        }
        (root / "runtime.json").write_text(json.dumps(receipt))
        return receipt

    def test_unchanged_receipted_files_pass_without_executing_synthetic_binaries(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.package(root)
            prepare.verify_staged(root, "windows-x64")

    def test_changed_and_unlisted_files_fail_verification(self):
        for change in ["changed", "extra"]:
            with self.subTest(change=change), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                self.package(root)
                path = root / "cpu" / ("llama-server.exe" if change == "changed" else "extra.dll")
                path.write_bytes(b"unexpected file")
                with self.assertRaises(ValueError):
                    prepare.verify_staged(root, "windows-x64")

    def test_receipt_and_unlisted_directory_links_are_rejected(self):
        for change in ["receipt", "directory"]:
            with self.subTest(change=change), tempfile.TemporaryDirectory() as folder:
                outer = Path(folder)
                root = outer / "package"
                root.mkdir()
                self.package(root)
                try:
                    if change == "receipt":
                        receipt = root / "runtime.json"
                        saved = outer / "receipt.json"
                        receipt.rename(saved)
                        receipt.symlink_to(saved)
                    else:
                        linked = outer / "external"
                        linked.mkdir()
                        (root / "extra").symlink_to(linked, target_is_directory=True)
                except OSError as error:
                    if getattr(error, "winerror", None) == 1314 or error.errno in (errno.ENOSYS, errno.ENOTSUP):
                        self.skipTest("Symbolic link creation unavailable on this host")
                    raise
                with self.assertRaisesRegex(ValueError, "receipt|Linked"):
                    prepare.verify_staged(root, "windows-x64")

    def test_foreign_recipe_and_escaping_paths_fail_verification(self):
        for change in ["recipe", "path"]:
            with self.subTest(change=change), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                receipt = self.package(root)
                if change == "recipe":
                    receipt["runtime"]["commit"] = "foreign-runtime"
                else:
                    receipt["files"][0]["path"] = "../outside.dll"
                (root / "runtime.json").write_text(json.dumps(receipt))
                with self.assertRaises(ValueError):
                    prepare.verify_staged(root, "windows-x64")


if __name__ == "__main__":
    unittest.main()
