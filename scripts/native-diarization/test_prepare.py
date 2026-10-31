import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("diarization_build", Path(__file__).with_name("prepare.py"))
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)


class PackagingTests(unittest.TestCase):
    def fixture(self, root):
        for name, data in {
            "gpu/bin/nemo-speech": b"\xcf\xfa\xed\xfebinary",
            "gpu/lib/libggml.dylib": b"\xcf\xfa\xed\xfelibrary",
            "gpu/share/licenses/nemo-speech/LICENSE": b"license",
        }.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        receipt = {"target": "macos-arm64", "runtime": build.SPEC,
                   "sourceArchiveHashesVerified": True, "sourceBuild": build.SPEC.get("sourceBuilds", {}).get("macos-arm64"), "fileHashScope": "before Developer ID code signing",
                   "files": build.package_files(root)}
        (root / "runtime.json").write_text(json.dumps(receipt))
        return receipt

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
            if os.name != "nt":
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

    def test_staged_package_requires_unchanged_files_and_matching_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            expected = self.fixture(root)
            self.assertEqual(build.verify_staged(root, "macos-arm64"), expected)
            with self.assertRaises(ValueError):
                build.verify_staged(root, "windows-x64")
            (root / "gpu/lib/libggml.dylib").write_bytes(b"changed")
            with self.assertRaises(ValueError):
                build.verify_staged(root)

    def test_package_rejects_changed_source_pin(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            receipt = self.fixture(root)
            receipt["sourceArchiveHashesVerified"] = False
            (root / "runtime.json").write_text(json.dumps(receipt))
            with self.assertRaises(ValueError):
                build.verify_staged(root)

    def test_package_rejects_links(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            try:
                (root / "gpu/lib/alias.dylib").symlink_to(root / "gpu/lib/libggml.dylib")
            except OSError as error:
                if os.name == "nt" and getattr(error, "winerror", None) == 1314:
                    self.skipTest("Symbolic-link privilege is unavailable on this Windows host")
                raise
            with self.assertRaises(ValueError):
                build.verify_staged(root)

    def test_package_rejects_extra_model_even_with_matching_file_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            receipt = self.fixture(root)
            (root / "gpu/lib/model.gguf").write_bytes(b"not a runtime")
            receipt["files"] = build.package_files(root)
            (root / "runtime.json").write_text(json.dumps(receipt))
            with self.assertRaises(ValueError):
                build.verify_staged(root)

    def test_signing_allows_only_binary_changes_and_preserves_signed_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            before = self.fixture(root)
            with (root / "gpu/bin/nemo-speech").open("ab") as output:
                output.write(b"signed")
            signed = build.signed_transition(root, before)
            build.verify_signed(root, signed)
            (root / "gpu/share/licenses/nemo-speech/LICENSE").write_bytes(b"changed")
            with self.assertRaises(ValueError):
                build.signed_transition(root, before)
            with self.assertRaises(ValueError):
                build.verify_signed(root, signed)

    def test_runtime_version_check_has_no_inference_or_download(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            with patch.object(build.subprocess, "run") as run:
                run.return_value.stdout = "nemo-speech 0.2.0\n"
                build.verify_staged(root, execute_runtime=True)
            self.assertEqual(run.call_args.args[0], [str(root / "gpu/bin/nemo-speech"), "--version"])
            self.assertEqual(run.call_args.kwargs["timeout"], 15)



class SourceProvenanceTests(unittest.TestCase):
    def test_missing_or_changed_source_recipe_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            tests = PackagingTests()
            receipt = tests.fixture(root)
            for invalid in ({"pin": "unknown"}, {"patch": "unverified"}):
                receipt["sourceBuild"] = invalid
                (root / "runtime.json").write_text(json.dumps(receipt))
                with self.assertRaises(ValueError):
                    build.verify_staged(root)

class WindowsCrtTests(unittest.TestCase):
    def fixture(self, root, missing=None):
        files = {"bin/nemo-speech.exe":b"binary", "share/licenses/nemo-speech/LICENSE":b"license"}
        for name in ("msvcp140.dll","vcruntime140.dll","vcruntime140_1.dll"):
            if name != missing: files["bin/"+name] = ("pinned-"+name).encode()
        cache = root/"cache";cache.mkdir()
        archive = cache/"worker.zip"
        with zipfile.ZipFile(archive,"w") as bundle:
            for name,data in files.items():bundle.writestr("worker/"+name,data)
        spec = {"repository":"test/worker","version":"v1","commit":"pinned", "assets":{"windows-x64":[{
            "backend":"cpu","name":archive.name,"bytes":archive.stat().st_size,"sha256":build.sha(archive)}]}}
        return spec,cache,files

    def test_pinned_crt_is_not_replaced_by_an_older_external_toolchain(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            spec,cache,files = self.fixture(root)
            old = root/"older-crt";old.mkdir()
            for name in ("msvcp140.dll","vcruntime140.dll","vcruntime140_1.dll"):(old/name).write_bytes(b"old")
            with patch.object(build,"SPEC",spec), patch.object(build.importlib.util,"spec_from_file_location",side_effect=AssertionError("external CRT discovery")):
                build.prepare("windows-x64",cache,root/"out",old)
                build.verify_staged(root/"out","windows-x64")
                for name,data in files.items():self.assertEqual((root/"out/cpu"/name).read_bytes(),data)

    def test_incomplete_official_crt_fails_instead_of_mixing_libraries(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            spec,cache,_ = self.fixture(root,missing="vcruntime140_1.dll")
            with patch.object(build,"SPEC",spec):
                with self.assertRaises(ValueError):build.prepare("windows-x64",cache,root/"out")
            self.assertFalse((root/"out").exists())
    def test_windows_composite_recipe_requires_both_backends_and_pinned_crt(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for backend in ("cpu", "gpu"):
                for name in ("bin/nemo-speech.exe", "bin/msvcp140.dll", "bin/vcruntime140.dll", "bin/vcruntime140_1.dll", "share/licenses/nemo-speech/LICENSE"):
                    file = root / backend / name
                    file.parent.mkdir(parents=True, exist_ok=True)
                    file.write_bytes(b"fixture")
            receipt = {"target": "windows-x64", "runtime": build.SPEC,
                       "sourceArchiveHashesVerified": True,
                       "sourceBuild": build.SPEC["sourceBuilds"]["windows-x64"],
                       "files": build.package_files(root)}
            (root / "runtime.json").write_text(json.dumps(receipt))
            self.assertEqual(build.verify_staged(root, "windows-x64"), receipt)
            receipt["sourceBuild"] = None
            (root / "runtime.json").write_text(json.dumps(receipt))
            with self.assertRaises(ValueError):
                build.verify_staged(root, "windows-x64")
            receipt["sourceBuild"] = build.SPEC["sourceBuilds"]["windows-x64"]
            (root / "cpu/bin/vcruntime140.dll").unlink()
            receipt["files"] = build.package_files(root)
            (root / "runtime.json").write_text(json.dumps(receipt))
            with self.assertRaises(ValueError):
                build.verify_staged(root, "windows-x64")


if __name__ == "__main__":
    unittest.main()
