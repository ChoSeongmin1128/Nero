import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("moss_runtime_patch", Path(__file__).with_name("apply-runtime-patch.py"))
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)


class Fixture:
    def __init__(self, root):
        self.work = root / ".work"
        self.task = self.work / "task"
        self.checkout = self.task / "checkouts/mlx-audio-swift"
        self.source = self.checkout / build.SOURCE_FILE
        self.source.parent.mkdir(parents=True)
        self.before = b"pinned source fixture\n"
        self.after = b"patched source fixture\n"
        self.source.write_bytes(self.before)
        self.patch = self.task / "fixture.patch"
        self.patch.write_bytes(b"validated patch fixture\n")
        self.receipt = self.task / "patch-receipt.json"
        self.contract = {
            "schemaVersion": 1, "runtime": "fixture", "upstreamCommit": build.PINNED_COMMIT,
            "sourceFile": build.SOURCE_FILE, "unpatchedSha256": build.sha256(self.before),
            "patchedSha256": build.sha256(self.after), "patchSha256": build.sha256(self.patch.read_bytes()),
        }
        self.commit = build.PINNED_COMMIT
        self.status = None
        self.apply_calls = 0
        self.wrong_patch = False

    def git(self, checkout, *arguments):
        assert checkout == self.checkout
        if arguments == ("rev-parse", "--show-toplevel"):
            return str(self.checkout) + "\n"
        if arguments == ("rev-parse", "HEAD"):
            return self.commit + "\n"
        if arguments == ("rev-parse", "HEAD^{tree}"):
            return "f" * 40 + "\n"
        if arguments[0] == "status":
            if self.status is not None:
                return self.status
            return "" if self.source.read_bytes() == self.before else " M " + build.SOURCE_FILE + "\n"
        if arguments[:2] == ("apply", "--check"):
            assert self.source.read_bytes() == self.before
            return ""
        if arguments[0] == "apply":
            self.apply_calls += 1
            self.source.write_bytes(b"unexpected" if self.wrong_patch else self.after)
            return ""
        raise AssertionError(arguments)

    def apply(self):
        return build.apply_runtime_patch(
            self.checkout, self.task, self.receipt, work_root=self.work,
            contract=self.contract, patch=self.patch, git_command=self.git,
        )


class RuntimePatchTests(unittest.TestCase):
    def test_pinned_artifact_records_effective_budget_and_distinct_stops_without_changing_guard(self):
        contract, path = build.load_contract()
        self.assertEqual(contract["runtime"], "mlx-audio-swift-v0.1.3+nero-moss-whole-v1")
        self.assertEqual(contract["terminationReasons"], ["eos", "length", "repetition"])
        text = path.read_text()
        added = "\n".join(line[1:] for line in text.splitlines() if line.startswith("+") and not line.startswith("+++"))
        removed = "\n".join(line[1:] for line in text.splitlines() if line.startswith("-") and not line.startswith("---"))
        for field in ("promptTokens", "requestedMaxTokens", "effectiveMaxTokens", "contextSize", "generatedTokens", "terminationReason"):
            self.assertIn("public let " + field + ":", added)
        self.assertIn("prepared.promptTokenCount < contextSize - 1", added)
        self.assertIn("min(maxTokens, contextSize - prepared.promptTokenCount - 1)", added)
        self.assertIn("maxTokens > 0", added)
        self.assertEqual(added.count("generationMetadata.removeAll(keepingCapacity: true)"), 2)
        self.assertEqual(added.count("generationMetadata.append(GenerationMetadata("), 1)
        for reason in ('terminationReason = "eos"', 'terminationReason = "length"', 'terminationReason = "repetition"'):
            self.assertIn(reason, added)
        for unchanged in ("repetitionPenalty == 1.0", "generated.count >= 24", "Set(tail).count <= 3", "temperature"):
            self.assertNotIn(unchanged, removed)

    def test_clean_apply_is_idempotent_and_receipt_is_bound_to_source_hashes(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = Fixture(Path(temporary))
            first = fixture.apply()
            receipt_bytes = fixture.receipt.read_bytes()
            self.assertEqual(first["action"], "applied")
            self.assertEqual(fixture.source.read_bytes(), fixture.after)
            self.assertEqual(fixture.apply()["action"], "alreadyApplied")
            self.assertEqual(fixture.apply_calls, 1)
            self.assertEqual(fixture.receipt.read_bytes(), receipt_bytes)
            receipt = json.loads(receipt_bytes)
            self.assertEqual(receipt["unpatchedSha256"], build.sha256(fixture.before))
            self.assertEqual(receipt["patchedSha256"], build.sha256(fixture.after))

    def test_unknown_commit_dirty_tree_source_patch_or_receipt_is_rejected_before_mutation(self):
        for condition in ("commit", "dirty", "source", "patch", "receipt"):
            with self.subTest(condition=condition), tempfile.TemporaryDirectory() as temporary:
                fixture = Fixture(Path(temporary))
                if condition == "commit": fixture.commit = "a" * 40
                if condition == "dirty": fixture.status = "?? unknown.txt\n"
                if condition == "source": fixture.source.write_bytes(b"unknown edit")
                if condition == "patch": fixture.patch.write_bytes(b"unknown patch")
                if condition == "receipt": fixture.receipt.write_text("{}")
                before = fixture.source.read_bytes()
                with self.assertRaises(ValueError): fixture.apply()
                self.assertEqual(fixture.apply_calls, 0)
                self.assertEqual(fixture.source.read_bytes(), before)

    def test_post_patch_hash_mismatch_never_creates_a_success_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = Fixture(Path(temporary))
            fixture.wrong_patch = True
            with self.assertRaisesRegex(ValueError, "digest mismatch"):
                fixture.apply()
            self.assertFalse(fixture.receipt.exists())

    def test_canonical_cache_other_task_and_symlink_paths_are_not_patch_targets(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixture = Fixture(root)
            outside = root / "canonical-cache"
            outside.mkdir()
            with self.assertRaises(ValueError): build.owned_path(outside, fixture.work)
            other_task = fixture.work / "other-task"
            other_task.mkdir()
            with self.assertRaises(ValueError):
                build.apply_runtime_patch(
                    fixture.checkout, other_task, fixture.receipt, work_root=fixture.work,
                    contract=fixture.contract, patch=fixture.patch, git_command=fixture.git,
                )
            alias = fixture.task / "alias"
            alias.symlink_to(fixture.checkout, target_is_directory=True)
            with self.assertRaises(ValueError): build.owned_path(alias / build.SOURCE_FILE, fixture.work)
            fixture.source.chmod(0o755)
            with self.assertRaises(ValueError): fixture.apply()


if __name__ == "__main__":
    unittest.main()
