"""Compile/run isolated NSIS lifecycle checks without touching the user library."""
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


def nsis_compiler():
    candidates = [os.environ.get("NERO_NSIS_COMPILER"), shutil.which("makensis"),
                  str(Path(os.environ.get("LOCALAPPDATA", ".")) / "tauri/NSIS/makensis.exe")]
    compiler = next((Path(p) for p in candidates if p and Path(p).is_file()), None)
    if compiler is None:
        if os.environ.get("NERO_REQUIRE_NSIS_TESTS") == "1":
            raise RuntimeError("A Windows installer build must provide its NSIS compiler")
        raise unittest.SkipTest("NSIS compiler unavailable; set NERO_NSIS_COMPILER")
    return compiler

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

    def test_setup_and_uninstaller_use_the_application_icon(self):
        config = json.loads((HOOK.parents[1] / "tauri.conf.json").read_text())
        nsis = config["bundle"]["windows"]["nsis"]
        for field in ("installerIcon", "uninstallerIcon"):
            self.assertEqual(nsis[field], "icons/icon.ico")
            self.assertTrue((HOOK.parents[1] / nsis[field]).is_file())


@unittest.skipUnless(sys.platform == "win32", "Windows shell-link acceptance")
class ShortcutIconTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.compiler = nsis_compiler()

    def powershell(self, directory, body):
        script = directory / "fixture.ps1"
        script.write_text("$ErrorActionPreference='Stop'\n" + body, encoding="utf-8-sig")
        result = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive",
                                 "-ExecutionPolicy", "Bypass", "-File", str(script)],
                                capture_output=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        return result.stdout.decode("utf-8-sig")

    @staticmethod
    def ps(value):
        return "'" + str(value).replace("'", "''") + "'"

    def inspect(self, directory, link):
        output = self.powershell(directory,
            "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)\n"
            f"$p={self.ps(link)}\n"
            "$w=New-Object -ComObject WScript.Shell\n$s=$w.CreateShortcut($p)\n"
            "$shell=New-Object -ComObject Shell.Application\n"
            "$item=$shell.NameSpace((Split-Path -LiteralPath $p)).ParseName([IO.Path]::GetFileName($p))\n"
            "@{target=$s.TargetPath;icon=$s.IconLocation;arguments=$s.Arguments;"
            "description=$s.Description;workingDirectory=$s.WorkingDirectory;"
            "appId=$item.ExtendedProperty('System.AppUserModel.ID')} | ConvertTo-Json -Compress\n")
        return json.loads(output)

    def run_installer(self, directory, body):
        script = directory / "shortcut.nsi"
        output = directory / "shortcut.exe"
        # Match Tauri's include order: the upstream macro exists before our hooks.
        script.write_text(
            'Unicode true\nRequestExecutionLevel user\nSilentInstall silent\n'
            '!include "LogicLib.nsh"\n!include "FileFunc.nsh"\n'
            '!include "Win\\COM.nsh"\n!include "Win\\Propkey.nsh"\n'
            '!include "Win\\RestartManager.nsh"\n'
            '!macro CheckIfAppIsRunning executablePath productName\n!macroend\n'
            '!macro SetLnkAppUserModelId shortcut\n!error "Upstream shortcut helper survived"\n!macroend\n'
            f'!include "{HOOK}"\n'
            '!define BUNDLEID "io.github.nero.shortcut-test"\n'
            '!define PRODUCTNAME "Nero"\n!define STARTMENUFOLDER ""\n'
            f'OutFile "{output}"\nSection\n'
            f'StrCpy $INSTDIR "{directory}"\n' + body + '\nSectionEnd\n',
            encoding="utf-8-sig")
        built = subprocess.run([str(self.compiler), "/V2", "/WX", str(script)],
                               capture_output=True, timeout=30)
        self.assertEqual(built.returncode, 0, built.stdout.decode(errors="replace") +
                         built.stderr.decode(errors="replace"))
        result = subprocess.run([str(output), "/S"], capture_output=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))

    def test_fresh_finish_page_shortcut_has_explicit_icon_and_app_id(self):
        with tempfile.TemporaryDirectory(prefix="nero-shortcut-") as folder:
            directory = Path(folder) / "한글 경로"
            directory.mkdir()
            link = directory / "Nero.lnk"
            self.run_installer(directory,
                f'CreateShortcut "{link}" "$INSTDIR\\nero-desktop.exe"\n'
                f'!insertmacro SetLnkAppUserModelId "{link}"')
            actual = self.inspect(directory, link)
            self.assertEqual(actual["icon"], str(directory / "nero-desktop.exe") + ",0")
            self.assertEqual(actual["appId"], "io.github.nero.shortcut-test")

    def test_update_repairs_icon_and_preserves_other_shortcut_properties(self):
        with tempfile.TemporaryDirectory(prefix="nero-shortcut-") as folder:
            directory = Path(folder)
            link = directory / "Nero.lnk"
            self.run_installer(directory,
                f'CreateShortcut "{link}" "$INSTDIR\\nero-desktop.exe"\n'
                f'!insertmacro SetLnkAppUserModelId "{link}"')
            self.powershell(directory,
                "$w=New-Object -ComObject WScript.Shell\n"
                f"$s=$w.CreateShortcut({self.ps(link)})\n"
                "$s.IconLocation='missing-old-icon.ico,3'\n"
                "$s.Arguments='--retained-argument'\n$s.Description='Keep this description'\n"
                f"$s.WorkingDirectory={self.ps(directory)}\n$s.Save()\n")
            before = self.inspect(directory, link)
            self.run_installer(directory, f'!insertmacro NeroRefreshShortcutIcon "{link}"')
            actual = self.inspect(directory, link)
            self.assertEqual(actual.pop("icon"), str(directory / "nero-desktop.exe") + ",0")
            before.pop("icon")
            self.assertEqual(actual, before)
            self.assertEqual(actual["appId"], "io.github.nero.shortcut-test")

    def test_missing_and_foreign_shortcuts_are_untouched(self):
        with tempfile.TemporaryDirectory(prefix="nero-shortcut-") as folder:
            directory = Path(folder)
            foreign = directory / "Foreign.lnk"
            absent = directory / "Deleted.lnk"
            self.powershell(directory,
                "$w=New-Object -ComObject WScript.Shell\n"
                f"$s=$w.CreateShortcut({self.ps(foreign)})\n"
                f"$s.TargetPath={self.ps(directory / 'another.exe')}\n"
                "$s.IconLocation='foreign-icon.ico,2'\n$s.Save()\n")
            before = foreign.read_bytes()
            self.run_installer(directory,
                f'!insertmacro NeroRefreshShortcutIcon "{foreign}"\n'
                f'!insertmacro NeroRefreshShortcutIcon "{absent}"')
            self.assertEqual(foreign.read_bytes(), before)
            self.assertFalse(absent.exists())

    def test_update_refreshes_surviving_links_without_recreating_deleted_links(self):
        with tempfile.TemporaryDirectory(prefix="nero-shortcut-") as folder:
            directory = Path(folder)
            desktop = directory / "desktop"
            programs = directory / "programs"
            desktop.mkdir()
            programs.mkdir()
            # The desktop link was deleted by the user. A start-menu link survives.
            link = programs / "Nero.lnk"
            self.powershell(directory,
                "$w=New-Object -ComObject WScript.Shell\n"
                f"$s=$w.CreateShortcut({self.ps(link)})\n"
                f"$s.TargetPath={self.ps(directory / 'nero-desktop.exe')}\n$s.Save()\n")
            self.run_installer(directory,
                f'!insertmacro NeroRefreshInstalledShortcutIcons "{desktop}" "{programs}"')
            self.assertEqual(self.inspect(directory, link)["icon"],
                             str(directory / "nero-desktop.exe") + ",0")
            self.assertFalse((desktop / "Nero.lnk").exists())


@unittest.skipUnless(sys.platform == "win32", "Windows Restart Manager acceptance")
class RunningAppGuardTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.compiler = nsis_compiler()

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
                '${If} ${Errors}\nSetErrorLevel 3\nQuit\n${EndIf}\n'
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
