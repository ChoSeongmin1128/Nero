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
        self.receipt.write_text(json.dumps({**self.contract, "upstreamTree": self.tree}, indent=2) + "\n")
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
            executable.write_bytes(b'\xcf\xfa\xed\xfe' + contents)
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
        self.team = "ABCDE12345"
        self.cdhash = "c" * 40
        self.codesign_status = 0

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
        if arguments[0] == "codesign":
            if "-d" in arguments:
                identity = f"Identifier=MossHarness\nTeamIdentifier={self.team}\nCDHash={self.cdhash}\n"
                return subprocess.CompletedProcess(arguments, 0, "", identity)
            return subprocess.CompletedProcess(arguments, self.codesign_status, "", "")
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

    def verify_staged(self, execute_runtime=False):
        with patch.object(package.patching, "load_contract", return_value=(self.contract, None)):
            return package.verify_staged(self.staged, source_root=self.source_root,
                                         execute_runtime=execute_runtime, runner=self.runner)

    def sign(self):
        self.finalize()
        self.calls.clear()
        preflight = self.verify_staged(execute_runtime=True)
        # Simulate the permitted effects of the trusted codesign invocation.
        with (self.staged / "MossHarness").open("ab") as output:
            output.write(b"Developer ID signature fixture")
        signature = self.staged / (package.BUNDLES[0] + ".bundle/Contents/_CodeSignature/CodeResources")
        signature.parent.mkdir()
        signature.write_text("signed resource envelope fixture")
        return preflight


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

    def test_static_consumption_check_verifies_staged_contract_without_running_worker(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = Fixture(Path(temporary))
            fixture.finalize()
            fixture.calls.clear()
            observed = fixture.verify_staged()
            self.assertFalse(observed["runtimeExecuted"])
            self.assertEqual(observed["preSignExecutableSha256"], package.file_sha256(fixture.staged / "MossHarness"))
            self.assertEqual(fixture.calls, [])
            fixture.verify_staged(execute_runtime=True)
            self.assertEqual(len(fixture.calls), 1)
            self.assertEqual(fixture.calls[0][0][-1], "--runtime-info")

    def test_staged_check_rejects_stale_missing_or_tampered_inputs_before_runtime_execution(self):
        for invalid in ("missing provenance", "old format", "helper source", "binary", "canonical lock",
                        "staged lock", "patch manifest", "patch receipt", "receipt digest", "metal", "resource", "runtime"):
            with self.subTest(invalid=invalid), tempfile.TemporaryDirectory() as temporary:
                fixture = Fixture(Path(temporary))
                fixture.finalize()
                fixture.calls.clear()
                provenance_path = fixture.staged / "RuntimeProvenance.json"
                provenance = json.loads(provenance_path.read_text())
                if invalid == "missing provenance": provenance_path.rename(provenance_path.with_suffix(".saved"))
                if invalid == "old format": provenance.pop("workerSourceFilesSha256")
                if invalid == "helper source": fixture.worker_source.with_name("Parser.swift").write_text("new parser\n")
                if invalid == "binary": (fixture.staged / "MossHarness").write_bytes(b"old chunk worker")
                if invalid == "canonical lock": fixture.lock.write_text('{"pins":[]}')
                if invalid == "staged lock": (fixture.staged / "Package.resolved").write_text('{"pins":[]}')
                if invalid == "patch manifest": provenance["runtimePatchManifest"]["runtime"] = "old-runtime"
                if invalid == "patch receipt": provenance["patchProvenance"].pop("upstreamTree")
                if invalid == "receipt digest": provenance["patchReceiptSha256"] = "0" * 64
                if invalid == "metal": (fixture.staged / package.METAL_RESOURCE).write_bytes(b"other shader")
                if invalid == "resource": (fixture.staged / (package.BUNDLES[1] + ".bundle/Contents/Info.plist")).write_text("changed")
                if invalid == "runtime": provenance["runtimeInfo"]["defaultPolicy"] = "short-180-long-60-v1"
                if provenance_path.exists(): provenance_path.write_text(json.dumps(provenance))
                with self.assertRaises(ValueError): fixture.verify_staged(execute_runtime=True)
                self.assertEqual(fixture.calls, [])

    def test_staged_package_rejects_symlinks_and_hardlinks(self):
        for invalid in ("symlink", "hardlink"):
            with self.subTest(invalid=invalid), tempfile.TemporaryDirectory() as temporary:
                fixture = Fixture(Path(temporary))
                fixture.finalize()
                unexpected = fixture.staged / "unexpected"
                if invalid == "symlink": unexpected.symlink_to(fixture.staged / "MossHarness")
                else: unexpected.hardlink_to(fixture.staged / "MossHarness")
                with self.assertRaisesRegex(ValueError, "[Ll]inked|[Ll]ink"):
                    fixture.verify_staged()

    def test_runtime_metadata_requires_exact_json_types(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = Fixture(Path(temporary))
            fixture.information["metadataSchemaVersion"] = True
            with self.assertRaisesRegex(ValueError, "stale or has a different"):
                fixture.finalize()

    def test_signing_receipt_preserves_original_provenance_and_links_new_binary_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = Fixture(Path(temporary))
            preflight = fixture.sign()
            provenance = (fixture.staged / "RuntimeProvenance.json").read_bytes()
            observed = package.verify_signed_transition(fixture.staged, preflight, fixture.team, runner=fixture.runner)
            self.assertNotEqual(observed["preSignExecutableSha256"], observed["postSignExecutableSha256"])
            self.assertEqual(observed["preSignExecutableSha256"], preflight["preSignExecutableSha256"])
            self.assertEqual(observed["signature"]["CDHash"], fixture.cdhash)
            self.assertEqual(observed["signature"]["TeamIdentifier"], fixture.team)
            self.assertEqual((fixture.staged / "RuntimeProvenance.json").read_bytes(), provenance)
            # Repackaging a signed app must use its post-sign receipt, not fail on the old ad-hoc hash.
            self.assertIs(package.verify_signed_receipt(fixture.staged, observed, fixture.team, runner=fixture.runner), observed)
            with self.assertRaisesRegex(ValueError, "resource bundles|executable"):
                fixture.verify_staged()

    def test_signing_transition_rejects_changes_outside_signature_envelopes(self):
        for invalid in ("provenance", "metal", "extra file", "missing file", "executable format", "team", "signature", "runtime", "static preflight"):
            with self.subTest(invalid=invalid), tempfile.TemporaryDirectory() as temporary:
                fixture = Fixture(Path(temporary))
                preflight = fixture.sign()
                if invalid == "provenance": (fixture.staged / "RuntimeProvenance.json").write_text("{}")
                if invalid == "metal": (fixture.staged / package.METAL_RESOURCE).write_text("changed shader")
                if invalid == "extra file": (fixture.staged / "unexpected.txt").write_text("extra")
                if invalid == "missing file": (fixture.staged / "Package.resolved").rename(fixture.task / "saved-lock")
                if invalid == "executable format": (fixture.staged / "MossHarness").write_text("replaced executable")
                if invalid == "team": fixture.team = "OTHER12345"
                if invalid == "signature": fixture.codesign_status = 1
                if invalid == "runtime": fixture.information["runtime"] = "old-runtime"
                if invalid == "static preflight": preflight["runtimeExecuted"] = False
                with self.assertRaises(ValueError):
                    package.verify_signed_transition(fixture.staged, preflight, "ABCDE12345", runner=fixture.runner)

    def test_preserved_signed_receipt_rejects_package_or_signature_drift(self):
        for invalid in ("missing receipt", "provenance", "binary", "resource", "signature", "team", "runtime"):
            with self.subTest(invalid=invalid), tempfile.TemporaryDirectory() as temporary:
                fixture = Fixture(Path(temporary))
                preflight = fixture.sign()
                receipt = package.verify_signed_transition(fixture.staged, preflight, fixture.team, runner=fixture.runner)
                if invalid == "missing receipt": receipt = None
                if invalid == "provenance": (fixture.staged / "RuntimeProvenance.json").write_text("{}")
                if invalid == "binary": (fixture.staged / "MossHarness").write_text("different executable")
                if invalid == "resource": (fixture.staged / package.METAL_RESOURCE).write_text("different shader")
                if invalid == "signature": fixture.cdhash = "d" * 40
                if invalid == "team": fixture.team = "OTHER12345"
                if invalid == "runtime": fixture.information["runtime"] = "different-runtime"
                with self.assertRaises(ValueError):
                    package.verify_signed_receipt(fixture.staged, receipt, "ABCDE12345", runner=fixture.runner)

    def test_mac_build_and_signing_consumers_use_the_package_guard(self):
        root = package.HERE.parents[1]
        desktop = root / "apps/desktop"
        config = json.loads((desktop / "src-tauri/tauri.conf.json").read_text())
        self.assertIn("npm run verify:native", config["build"]["beforeBuildCommand"])
        native = (desktop / "scripts/verify-native.mjs").read_text()
        self.assertIn("process.platform === 'darwin'", native)
        self.assertIn("target.includes('apple-darwin')", native)
        self.assertIn("'verify-staged'", native)
        cargo = (desktop / "src-tauri/build.rs").read_text()
        self.assertIn('== Ok("macos")', cargo)
        self.assertIn('.arg("verify-staged")', cargo)
        signer = (desktop / "scripts/sign-macos.py").read_text()
        self.assertLess(signer.index('verify_staged(worker,execute_runtime=True)'), signer.index("for file in files:"))
        self.assertIn('verify_signed_transition(worker,worker_preflight', signer)
        self.assertIn('verify_signed_receipt(worker,receipt.get(\'moss_package\')', signer)
        self.assertIn("'app_tree_sha256'", signer)
        self.assertIn("path.open('x')", signer)


if __name__ == "__main__":
    unittest.main()
