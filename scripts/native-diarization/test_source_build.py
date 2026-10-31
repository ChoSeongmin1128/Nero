import importlib.util
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("speech_source", Path(__file__).with_name("source_build.py"))
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)


class SourceBuildTests(unittest.TestCase):
    def test_verified_cache_is_used_without_network(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "owner-repo-commit.tar.gz"
            path.write_bytes(b"source")
            item = {"repository": "owner/repo", "commit": "commit", "bytes": 6, "sha256": build.digest(path)}
            with patch.object(build.urllib.request, "urlopen", side_effect=AssertionError("network")):
                self.assertEqual(build.archive(item, root), path)

    def test_wrong_download_hash_never_becomes_a_cache_entry(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            item = {"repository": "owner/repo", "commit": "commit", "bytes": 3, "sha256": "0" * 64}
            with patch.object(build.urllib.request, "urlopen", return_value=io.BytesIO(b"bad")):
                with self.assertRaises(ValueError):
                    build.archive(item, root)
            self.assertFalse((root / "owner-repo-commit.tar.gz").exists())

    def test_extract_rejects_escape_and_external_aliases(self):
        for name, link in (("../outside", None), ("source/link", "../../outside")):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                archive = root / "source.tar.gz"
                with tarfile.open(archive, "w:gz") as bundle:
                    item = tarfile.TarInfo(name)
                    if link:
                        item.type = tarfile.SYMTYPE
                        item.linkname = link
                        bundle.addfile(item)
                    else:
                        item.size = 1
                        bundle.addfile(item, io.BytesIO(b"x"))
                with self.assertRaises((ValueError, tarfile.FilterError)):
                    build.unpack(archive, root / "out")
                self.assertFalse((root / "outside").exists())

    def test_patch_hash_is_checked_before_any_source_mutation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "source").mkdir()
            (root / "patch").write_bytes(b"bad")
            profile = {"patch": {"file": "patch", "bytes": 3, "sha256": "0" * 64}}
            with patch.object(build.subprocess, "run", side_effect=AssertionError("patch invoked")):
                with self.assertRaises(ValueError):
                    build.patch_source(root / "source", profile, root, {})

    def test_non_mac_source_build_fails_before_download(self):
        with patch.object(build.platform, "system", return_value="Windows"), patch.object(build, "archive", side_effect=AssertionError("download")):
            with self.assertRaises(ValueError):
                build.build({}, Path("unused"), Path("unused"), "cmake", None)

    def test_windows_cpu_recipe_rejects_other_hosts_before_download(self):
        for system, machine in [("Darwin", "arm64"), ("Windows", "ARM64")]:
            with self.subTest(system=system, machine=machine), patch.object(build.platform, "system", return_value=system), patch.object(build.platform, "machine", return_value=machine), patch.object(build, "archive", side_effect=AssertionError("download")):
                with self.assertRaises(ValueError):
                    build.build({"sourceBackends": ["cpu"]}, Path("unused"), Path("unused"), ["cmake"], None)

    def test_patch_application_overrides_host_checkout_line_endings(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "patch").write_bytes(b"patch")
            profile = {"patch": {"file": "patch", "bytes": 5, "sha256": build.digest(root / "patch"), "files": []}}
            with patch.object(build.subprocess, "run") as run:
                build.patch_source(root, profile, root, {})
            self.assertEqual(run.call_count, 2)
            for call in run.call_args_list:
                self.assertEqual(call.args[0][:4], ["git", "-c", "core.autocrlf=false", "apply"])


    def test_mac_recipe_requires_explicit_portable_cpu_and_deployment(self):
        import json
        spec = json.loads(Path(__file__).with_name("runtime.json").read_text())
        profile = spec["sourceBuilds"]["macos-arm64"]
        settings = build.macos_settings(profile)
        self.assertEqual(settings["GGML_NATIVE:BOOL"], "OFF")
        self.assertEqual(settings["CMAKE_OSX_DEPLOYMENT_TARGET:STRING"], "13.0")
        self.assertEqual(settings["GGML_CPU_ARM_ARCH:STRING"], "armv8.2-a+dotprod+fp16")
        for bad in ({}, {"cmakeFlags": [flag.replace("GGML_NATIVE:BOOL=OFF", "GGML_NATIVE:BOOL=ON") for flag in profile["cmakeFlags"]]}):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                build.macos_settings(bad)

    def test_actual_cache_must_match_the_declared_portable_settings(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "CMakeCache.txt").write_text("GGML_NATIVE:BOOL=ON\nCMAKE_OSX_DEPLOYMENT_TARGET:STRING=27.0\n")
            with self.assertRaises(ValueError):
                build.verify_cmake_settings(root, {"GGML_NATIVE:BOOL": "OFF"})
            with self.assertRaises(ValueError):
                build.verify_cmake_settings(root, {"CMAKE_OSX_DEPLOYMENT_TARGET:STRING": "13.0"})
            (root / "CMakeCache.txt").write_text("GGML_NATIVE:BOOL=OFF\nCMAKE_OSX_DEPLOYMENT_TARGET:STRING=13.0\n")
            build.verify_cmake_settings(root, {"GGML_NATIVE:BOOL": "OFF", "CMAKE_OSX_DEPLOYMENT_TARGET:STRING": "13.0"})


if __name__ == "__main__":
    unittest.main()
