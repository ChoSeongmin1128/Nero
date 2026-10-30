"""Compile/run the NSIS guard without installing Nero or touching user data."""
import os
import json
from pathlib import Path
import shutil
import re
import subprocess
import sys
import tempfile
import time
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

    def run_guard(self, target, uninstall=False, update=False, before_run=None):
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
            if before_run:
                before_run()
            completed = subprocess.run([str(output), "/S"] + (["/UPDATE"] if update else []), capture_output=True, timeout=15)
            if uninstall:
                self.assertEqual(completed.returncode, 0)
                completed = subprocess.run([str(uninstaller), "/S", f"_?={directory}"],
                                           capture_output=True, timeout=15)
            return completed.returncode

    def test_update_waits_for_natural_exit_without_terminating_a_process(self):
        with tempfile.TemporaryDirectory(prefix="nero-updater-exit-") as folder:
            executable = Path(folder) / "synthetic-helper.exe"
            shutil.copy2(sys.executable, executable)
            environment = dict(os.environ, PYTHONHOME=sys.prefix)
            environment["PATH"] = str(Path(sys.executable).parent) + os.pathsep + environment.get("PATH", "")
            child = None
            def start_child():
                nonlocal child
                child = subprocess.Popen([str(executable), "-c", "import time; time.sleep(1)"], env=environment)
                time.sleep(0.1)
                self.assertIsNone(child.poll())
            try:
                self.assertEqual(self.run_guard(executable, update=True, before_run=start_child), 0)
                self.assertEqual(child.wait(timeout=5), 0)
            finally:
                if child is not None and child.poll() is None:
                    child.terminate()
                    child.wait(timeout=5)

    def test_update_timeout_keeps_the_running_process_alive(self):
        started = time.monotonic()
        self.assertEqual(self.run_guard(Path(sys.executable), update=True), 2)
        self.assertGreater(time.monotonic() - started, 4)

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
