import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("moss_worker_package", Path(__file__).with_name("verify_worker_package.py"))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class Fixture:
    def __init__(self, root):
        self.work = root / ".work"
        self.task = self.work / "packaging-task"
        self.checkouts = self.task / "checkouts"
        self.checkout = self.checkouts / "mlx-audio-swift"
        self.products = self.task / "products"
        self.staged = self.task / "staged"
        self.source_root = self.task / "worker-source"
        self.source = self.checkout / package.patching.SOURCE_FILE
        self.source.parent.mkdir(parents=True)
        self.source.write_text("patched upstream fixture\n")
        self.contract = json.loads((package.HERE / "patches/manifest.json").read_text())
        self.contract["patchedSha256"] = package.file_sha256(self.source)
        self.tree = "b" * 40
        self.receipt = self.task / "patch-receipt.json"
        self.receipt.write_text(json.dumps({**self.contract, "upstreamTree": self.tree}))
        self.worker_source = self.source_root / "Sources/MossHarness/MossHarness.swift"
        self.worker_source.parent.mkdir(parents=True)
        self.worker_source.write_text("current source recorded at packaging\n")
        self.package_swift = self.source_root / "Package.swift"
        self.package_swift.write_text("canonical package definition\n")
        self.lock = self.source_root / "Package.resolved"
        self.lock.write_text(json.dumps({"pins": [{"identity": "mlx-audio-swift", "state": {
            "revision": self.contract["upstreamCommit"], "version": self.contract["upstreamVersion"],
        }}]}))
        self.products.mkdir()
        self.staged.mkdir()
        for folder, contents in ((self.products, b"original build executable"), (self.staged, b"ad-hoc signed executable")):
            executable = folder / "MossHarness"
            executable.write_bytes(contents)
            executable.chmod(0o755)
        for name in package.BUNDLES:
            resource = self.products / (name + ".bundle") / "Contents/Info.plist"
            resource.parent.mkdir(parents=True)
            resource.write_text("resource fixture")
        metal = self.products / package.METAL_RESOURCE
        metal.parent.mkdir(parents=True)
        metal.write_bytes(b"matching metal fixture")
        for name in package.BUNDLES:
            shutil.copytree(self.products / (name + ".bundle"), self.staged / (name + ".bundle"))
        shutil.copyfile(self.lock, self.staged / "Package.resolved")
        self.commit = self.contract["upstreamCommit"]
        self.status = " M " + self.contract["sourceFile"] + "\n"
        self.calls = []
        self.information = dict(package.EXPECTED_RUNTIME_INFO)

    def git(self, checkout, *arguments):
        assert checkout == self.checkout
        return {
            ("rev-parse", "--show-toplevel"): str(self.checkout) + "\n",
            ("rev-parse", "HEAD"): self.commit + "\n",
            ("rev-parse", "HEAD^{tree}"): self.tree + "\n",
            ("status", "--porcelain=v1", "--untracked-files=all", "--ignore-submodules=none"): self.status,
        }[arguments]

    def runner(self, arguments, **options):
        self.calls.append((arguments, options))
        return subprocess.CompletedProcess(arguments, 0, json.dumps(self.information), "")

    def verify(self):
        with patch.object(package.patching, "load_contract", return_value=(self.contract, None)):
            return package.verify_inputs(
                self.products, self.checkouts, self.receipt, source_root=self.source_root,
                work_root=self.work, git_command=self.git,
            )

    def finalize(self):
        with patch.object(package.patching, "load_contract", return_value=(self.contract, None)):
            return package.finalize(
                self.products, self.checkouts, self.staged, self.receipt,
                source_root=self.source_root, work_root=self.work,
                git_command=self.git, runner=self.runner,
            )


class WorkerPackageTests(unittest.TestCase):
    def test_shell_contract_requires_receipt_and_validates_before_copy_and_after_signing(self):
        script = (package.HERE / "package_worker.sh").read_text()
        self.assertIn('[[ $# -ne 4 ]]', script)
        self.assertIn('PATCH_RECEIPT', script)
        self.assertLess(script.index('verify_worker_package.py" preflight'), script.index('mkdir -p "$destination/Licenses"'))
        self.assertLess(script.index('codesign --verify'), script.index('verify_worker_package.py" finalize'))

    def test_final_provenance_records_hashes_and_limitations_without_absolute_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = Fixture(Path(temporary))
            observed = fixture.finalize()
            encoded = (fixture.staged / "RuntimeProvenance.json").read_text()
            self.assertNotIn(temporary, encoded)
            self.assertEqual(observed["runtimeInfo"], package.EXPECTED_RUNTIME_INFO)
            self.assertEqual(observed["workerSourceSha256"], package.file_sha256(fixture.worker_source))
            self.assertEqual(observed["workerSourceFilesSha256"], {
                "Package.swift": package.file_sha256(fixture.package_swift),
                "Sources/MossHarness/MossHarness.swift": package.file_sha256(fixture.worker_source),
            })
            self.assertEqual(observed["packageResolvedSha256"], package.file_sha256(fixture.lock))
            self.assertEqual(observed["inputExecutableSha256"], package.file_sha256(fixture.products / "MossHarness"))
            self.assertEqual(observed["executableSha256"], package.file_sha256(fixture.staged / "MossHarness"))
            self.assertNotEqual(observed["inputExecutableSha256"], observed["executableSha256"])
            self.assertEqual(observed["mlxMetalResource"]["sha256"], package.file_sha256(fixture.products / package.METAL_RESOURCE))
            self.assertEqual(observed["signatureStage"], "workerAdHocBeforeDeveloperId")
            self.assertEqual(observed["sourceBinding"], "recordedAtPackagingNotEmbeddedInExecutable")
            self.assertEqual(len(fixture.calls), 1)
            arguments, options = fixture.calls[0]
            self.assertEqual(arguments, [str((fixture.staged / "MossHarness").resolve()), "--runtime-info"])
            self.assertEqual(options["cwd"], str(fixture.staged.resolve()))
            self.assertEqual(options["timeout"], 15)
            with self.assertRaisesRegex(ValueError, "existing runtime provenance"):
                fixture.finalize()

    def test_provenance_includes_all_swift_helpers_and_package_in_deterministic_order(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = Fixture(Path(temporary))
            nested = fixture.worker_source.parent / "Parsing/ZTimestamp.swift"
            nested.parent.mkdir()
            nested.write_text("nested parser helper\n")
            helper = fixture.worker_source.with_name("ATimestamp.swift")
            helper.write_text("foundation parser helper\n")
            fixture.worker_source.with_name("ignored.txt").write_text("not a Swift source\n")
            observed = fixture.finalize()["workerSourceFilesSha256"]
            self.assertEqual(list(observed), [
                "Package.swift",
                "Sources/MossHarness/ATimestamp.swift",
                "Sources/MossHarness/MossHarness.swift",
                "Sources/MossHarness/Parsing/ZTimestamp.swift",
            ])
            self.assertEqual(observed["Sources/MossHarness/ATimestamp.swift"], package.file_sha256(helper))
            self.assertEqual(observed["Sources/MossHarness/Parsing/ZTimestamp.swift"], package.file_sha256(nested))
            self.assertEqual(observed["Package.swift"], package.file_sha256(fixture.package_swift))

    def test_source_hash_recording_rejects_missing_package_or_symlinked_sources(self):
        for invalid in ("package", "helper", "directory"):
            with self.subTest(invalid=invalid), tempfile.TemporaryDirectory() as temporary:
                fixture = Fixture(Path(temporary))
                if invalid == "package":
                    fixture.package_swift.rename(fixture.package_swift.with_suffix(".missing"))
                elif invalid == "helper":
                    fixture.worker_source.with_name("Parser.swift").symlink_to(fixture.worker_source)
                else:
                    fixture.worker_source.with_name("Parsing").symlink_to(fixture.worker_source.parent, target_is_directory=True)
                with self.assertRaises(ValueError): fixture.verify()
                self.assertEqual(fixture.calls, [])

    def test_preflight_rejects_wrong_commit_patch_receipt_dirty_checkout_and_lock(self):
        for invalid in ("commit", "source", "receipt", "dirty", "lock"):
            with self.subTest(invalid=invalid), tempfile.TemporaryDirectory() as temporary:
                fixture = Fixture(Path(temporary))
                if invalid == "commit": fixture.commit = "c" * 40
                if invalid == "source": fixture.source.write_text("unpatched source")
                if invalid == "receipt": fixture.receipt.write_text("{}")
                if invalid == "dirty": fixture.status += "?? other-file\n"
                if invalid == "lock": fixture.lock.write_text('{"pins":[]}')
                with self.assertRaises(ValueError): fixture.verify()
                self.assertEqual(fixture.calls, [])
                self.assertFalse((fixture.staged / "RuntimeProvenance.json").exists())

    def test_stale_or_unexpected_runtime_info_never_produces_provenance(self):
        for changed in (
            {"runtime": "mlx-audio-swift-v0.1.3"},
            {"defaultPolicy": "short-180-long-60-v1"},
            {"maxInputSeconds": 180},
            {"maxRequestedTokens": 8192},
            {"unexpectedPath": "/private/example"},
        ):
            with self.subTest(changed=changed), tempfile.TemporaryDirectory() as temporary:
                fixture = Fixture(Path(temporary))
                fixture.information.update(changed)
                with self.assertRaisesRegex(ValueError, "stale or has a different"):
                    fixture.finalize()
                self.assertFalse((fixture.staged / "RuntimeProvenance.json").exists())

    def test_resource_or_staged_lock_mismatch_is_rejected_before_executable_runs(self):
        for invalid in ("metal", "lock"):
            with self.subTest(invalid=invalid), tempfile.TemporaryDirectory() as temporary:
                fixture = Fixture(Path(temporary))
                target = fixture.staged / (package.METAL_RESOURCE if invalid == "metal" else "Package.resolved")
                target.write_text("mismatching content")
                with self.assertRaises(ValueError): fixture.finalize()
                self.assertEqual(fixture.calls, [])
                self.assertFalse((fixture.staged / "RuntimeProvenance.json").exists())

    def test_runtime_info_failure_timeout_and_invalid_json_are_bounded_errors(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = Fixture(Path(temporary))
            for returncode, output in ((1, "private log"), (0, "not JSON"), (0, "x" * 65537)):
                def runner(arguments, **options):
                    return subprocess.CompletedProcess(arguments, returncode, output, "private log")
                with self.assertRaises(ValueError): package.runtime_info(fixture.staged, runner=runner)
            def timeout(arguments, **options):
                raise subprocess.TimeoutExpired(arguments, options["timeout"])
            with self.assertRaisesRegex(ValueError, "did not complete"):
                package.runtime_info(fixture.staged, runner=timeout)


if __name__ == "__main__":
    unittest.main()
