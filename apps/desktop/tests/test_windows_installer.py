"""Compile/run the NSIS guard without installing Nero or touching user data."""
import os
import json
from pathlib import Path
import shutil
import re
import subprocess
import sys
import tempfile
import unittest


HOOK = Path(__file__).resolve().parents[1] / "src-tauri/windows/hooks.nsh"

class InstallerConfigurationTests(unittest.TestCase):
    def test_early_guard_uses_the_configured_main_executable(self):
        tauri = HOOK.parents[1]
        config = json.loads((tauri / "tauri.conf.json").read_text())
        package = re.search(r'^name\s*=\s*"([^"]+)"',
                            (tauri / "Cargo.toml").read_text(), re.MULTILINE).group(1)
        expected = config.get("mainBinaryName", package) + ".exe"
        declared = re.search(r'!define NERO_APP_EXECUTABLE "([^"]+)"',
                             HOOK.read_text(encoding="utf-8")).group(1)
        self.assertEqual(declared, expected)


@unittest.skipUnless(sys.platform == "win32", "Windows Restart Manager acceptance")
class RunningAppGuardTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        candidates = [
            os.environ.get("NERO_NSIS_COMPILER"),
            shutil.which("makensis"),
            str(Path(os.environ.get("LOCALAPPDATA", ".")) / "tauri/NSIS/makensis.exe"),
        ]
        cls.compiler = next((Path(p) for p in candidates if p and Path(p).is_file()), None)
        if cls.compiler is None:
            if os.environ.get("NERO_REQUIRE_NSIS_TESTS") == "1":
                raise RuntimeError("A Windows installer build must provide its NSIS compiler")
            raise unittest.SkipTest("NSIS compiler unavailable; set NERO_NSIS_COMPILER")

    def run_guard(self, target, uninstall=False):
        with tempfile.TemporaryDirectory(prefix="nero-nsis-guard-") as folder:
            directory = Path(folder)
            output = directory / "guard.exe"
            uninstaller = directory / "uninstall.exe"
            script = directory / "guard.nsi"
            # The default helper would fail compilation if it survives the override.
            script.write_text(
                'Unicode true\nRequestExecutionLevel user\nSilentInstall silent\n'
                'AutoCloseWindow true\n!include "LogicLib.nsh"\n'
                '!include "FileFunc.nsh"\n!include "Win\\RestartManager.nsh"\n'
                '!macro CheckIfAppIsRunning executablePath productName\n'
                '!error "Unsafe upstream helper was not overridden"\n!macroend\n'
                f'!include "{HOOK}"\nOutFile "{output}"\n'
                'Function .onGUIInit\nCall NeroCheckBeforeMaintenance\nFunctionEnd\nSection\n' +
                (f'WriteUninstaller "{uninstaller}"\nSetErrorLevel 0\nSectionEnd\n'
                 'Section "Uninstall"\n' if uninstall else '') +
                f'!insertmacro CheckIfAppIsRunning "{target}" "Nero"\n'
                'SetErrorLevel 0\nSectionEnd\n', encoding="utf-8-sig")
            built = subprocess.run([str(self.compiler), "/V2", "/WX", str(script)],
                                   capture_output=True, timeout=30)
            self.assertEqual(built.returncode, 0, built.stdout.decode(errors="replace") +
                             built.stderr.decode(errors="replace"))
            completed = subprocess.run([str(output), "/S"], capture_output=True, timeout=15)
            if uninstall:
                self.assertEqual(completed.returncode, 0)
                completed = subprocess.run([str(uninstaller), "/S", f"_?={directory}"],
                                           capture_output=True, timeout=15)
            return completed.returncode

    def test_running_executable_is_rejected_without_killing_its_process(self):
        # This test process owns the real Python executable; it must survive.
        for uninstall in [False, True]:
            with self.subTest(uninstall=uninstall):
                self.assertEqual(self.run_guard(Path(sys.executable), uninstall), 2)

    def test_existing_unopened_file_does_not_block_installation(self):
        with tempfile.TemporaryDirectory(prefix="nero-nsis-idle-") as folder:
            target = Path(folder) / "inactive.exe"
            target.write_bytes(b"synthetic inactive file")
            for uninstall in [False, True]:
                with self.subTest(uninstall=uninstall):
                    self.assertEqual(self.run_guard(target, uninstall), 0)

    def test_first_install_without_an_existing_executable_is_allowed(self):
        with tempfile.TemporaryDirectory(prefix="nero-nsis-new-") as folder:
            for uninstall in [False, True]:
                with self.subTest(uninstall=uninstall):
                    self.assertEqual(self.run_guard(Path(folder) / "absent.exe", uninstall), 0)


if __name__ == "__main__":
    unittest.main()
