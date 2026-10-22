from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


HERE = Path(__file__).resolve().parent


class OutputValidationTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("swiftc"), "The standalone Foundation parser check requires swiftc.")
    def test_raw_output_and_returned_segments_have_complete_correspondence(self):
        with tempfile.TemporaryDirectory() as temporary:
            executable = Path(temporary) / "moss-output-validation"
            compiled = subprocess.run([
                shutil.which("swiftc"),
                "-parse-as-library",
                str(HERE / "Sources/MossHarness/MossOutputValidation.swift"),
                str(HERE / "tests/MossOutputValidationTests.swift"),
                "-o", str(executable),
            ], capture_output=True, text=True, timeout=60)
            self.assertEqual(compiled.returncode, 0, compiled.stderr)
            result = subprocess.run([str(executable)], capture_output=True, text=True, timeout=15)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("MOSS output validation checks passed", result.stdout)


if __name__ == "__main__":
    unittest.main()
