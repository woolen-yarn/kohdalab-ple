"""Execute portable PowerShell checks when PLE_PWSH (or pwsh) is available."""
import json
import os
from pathlib import Path
import shutil
import shlex
import hashlib
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
PWSH = os.environ.get("PLE_PWSH") or shutil.which("pwsh")


@unittest.skipUnless(PWSH, "set PLE_PWSH to run PowerShell execution tests")
class PowerShellRuntimeTests(unittest.TestCase):
    @unittest.skipIf(os.name == "nt", "POSIX command fixtures; native PE inspection runs in release CI")
    def test_import_inspection_without_powershell_hash_cmdlet(self):
        with tempfile.TemporaryDirectory(prefix="ple import audit ") as temporary:
            root = Path(temporary)
            stage = root / "portable"; stage.mkdir()
            dumpbin = root / "dumpbin.exe"
            vswhere = root / "Program Files/Microsoft Visual Studio/Installer/vswhere.exe"
            vswhere.parent.mkdir(parents=True)
            binary = stage / "kohdalab-ple.exe"
            for path, source in ((vswhere, "#!/bin/sh\nprintf '%s\\n' " + shlex.quote(str(dumpbin)) + "\n"),
                                 (binary, "#!/bin/sh\necho 'kohdalab-ple fixture'\n")):
                path.write_text(source); path.chmod(0o755)
            launcher = root / "inspect.ps1"
            launcher.write_text("function Get-FileHash { throw 'hash cmdlet unavailable' }; & $args[0] -Directory $args[1]\n")
            environment = dict(os.environ)
            environment["ProgramFiles(x86)"] = str(root / "Program Files")
            for dependency, success in (("KERNEL32.dll", True), ("ucrtbase.dll", False)):
                dumpbin.write_text("#!/bin/sh\nprintf '%s\\n' " + shlex.quote("    " + dependency) + "\n")
                dumpbin.chmod(0o755)
                result = subprocess.run([PWSH,"-NoProfile","-File",str(launcher),str(ROOT / "scripts/inspect-windows-binary.ps1"),str(stage)],env=environment,capture_output=True,text=True,timeout=20)
                self.assertEqual(result.returncode == 0,success,result.stderr)
                if success:
                    metadata = json.loads((stage / "windows-metadata.json").read_text(encoding="utf-8-sig"))
                    self.assertEqual(metadata["executable_sha256"],hashlib.sha256(binary.read_bytes()).hexdigest())

    def test_all_powershell_sources_parse(self):
        script = '''$errorsFound = @(); Get-ChildItem -LiteralPath $args[0] -Filter *.ps1 | ForEach-Object {
          $tokens = $null; $parseErrors = $null
          $null = [System.Management.Automation.Language.Parser]::ParseFile($_.FullName, [ref]$tokens, [ref]$parseErrors)
          $errorsFound += $parseErrors
        }; if ($errorsFound.Count -gt 0) { $errorsFound | Out-String | Write-Error; exit 1 }'''
        with tempfile.TemporaryDirectory() as temporary:
            checker = Path(temporary) / "parse.ps1"
            checker.write_text(script)
            result = subprocess.run([PWSH, "-NoLogo", "-NoProfile", "-File", str(checker), str(ROOT / "scripts")], capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stderr)

    def _preflight(self, sdk_success=True, app_success=True, sdk_report=None, app_report=True, gui_native=False):
        with tempfile.TemporaryDirectory(prefix="ple preflight ") as temporary:
            root = Path(temporary)
            (root / "scripts").mkdir()
            (root / "bridge").mkdir()
            wrapper = root / "scripts" / "preflight.ps1"
            shutil.copyfile(ROOT / "scripts" / "preflight.ps1", wrapper)
            (root / "bridge" / "sdk_check.py").write_text(
                'import json,sys\nprint(json.dumps({"installation_config_pass":' + str(sdk_success if sdk_report is None else sdk_report) + '}))\nsys.exit(' + str(0 if sdk_success else 3) + ')\n')
            app = root / "fake app.ps1"
            app.write_text("$global:LASTEXITCODE = " + str(0 if app_success else 7) + "\n'" + json.dumps({"config_valid":app_report}) + "'\n")
            if gui_native:
                source = root / "gui_fixture.rs"
                source.write_text('#![windows_subsystem = "windows"]\nfn main() { std::thread::sleep(std::time::Duration::from_millis(100)); println!(r#"{{"config_valid":true}}"#); }\n')
                app = root / "fake gui app.exe"
                subprocess.run(["rustc", str(source), "--crate-name", "ple_gui_fixture", "-o", str(app)], check=True, capture_output=True, timeout=30)
            profile = root / "profile settings.json"
            profile.write_text(json.dumps({"spectrometer": {"python": sys.executable}}))
            return subprocess.run([PWSH, "-NoLogo", "-NoProfile", "-File", str(wrapper), "-ConfigPath", str(profile), "-AppPath", str(app)], capture_output=True, text=True, timeout=30, cwd=ROOT.parent)

    def test_preflight_emits_one_json_document_and_handles_paths_with_spaces(self):
        result = self._preflight()
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertTrue(report["installation_config_pass"])
        self.assertTrue(report["application_check_config"]["config_valid"])

    @unittest.skipUnless(sys.platform == "win32" and shutil.which("rustc"), "Windows GUI-subsystem fixture needs Windows rustc")
    def test_windows_gui_process_is_awaited_before_reading_its_report(self):
        result = self._preflight(gui_native=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)["application_check_config"]["config_valid"])

    def test_zero_exit_code_cannot_override_a_negative_json_result(self):
        for overrides in [{"sdk_report":False}, {"app_report":False}]:
            result = self._preflight(**overrides)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(json.loads(result.stdout)["installation_config_pass"])

    def test_failed_sdk_or_application_cannot_pass_preflight(self):
        for sdk_ok, app_ok in [(False, True), (True, False)]:
            result = self._preflight(sdk_ok, app_ok)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(json.loads(result.stdout)["installation_config_pass"])

    def test_windows_package_rebuild_replaces_stale_files_and_preserves_previous_on_failure(self):
        with tempfile.TemporaryDirectory(prefix="ple package ") as temporary:
            root = Path(temporary)
            (root / "scripts").mkdir()
            shutil.copyfile(ROOT / "scripts" / "build-windows.ps1", root / "scripts" / "build-windows.ps1")
            for folder in ("bridge", "bindings", "examples", "docs", "servers", "third_party"):
                (root / folder).mkdir()
                (root / folder / "fixture.txt").write_text("fixture")
            for name in ("preflight.ps1", "verify_run.py", "verify_gui_api.py", "collect_licenses.py"):
                (root / "scripts" / name).touch()
            for name in ("README.md", "START-HERE.md", "RELEASE-NOTES.md", "LICENSE", "CHANGELOG.md", "CODE_SIGNING_POLICY.md", "CONTRIBUTING.md", "PRIVACY.md", "REDISTRIBUTION.md", "ROADMAP.md", "SAFETY.md", "SECURITY.md"):
                (root / name).write_text("fixture")
            (root / "target" / "release").mkdir(parents=True)
            (root / "target" / "release" / "kohdalab-ple.exe").write_bytes(b"packaging-test-fixture; never executed")
            launcher = root / "verify-packaging.ps1"
            launcher.write_text('''$ErrorActionPreference = "Stop"
function cargo { $global:LASTEXITCODE = 0 }
function rustc { 'host: x86_64-pc-windows-msvc'; $global:LASTEXITCODE = 0 }
function python {
  if ($global:failLicense) { $global:LASTEXITCODE = 7; return }
  $index = [array]::IndexOf($args, '--output')
  $out = $args[$index + 1]
  if (Test-Path -LiteralPath $out) { throw 'license output already exists' }
  New-Item -ItemType Directory -Path $out | Out-Null
  Set-Content -LiteralPath (Join-Path $out 'NOTICE.json') -Value '{"ok":true}'
  $global:LASTEXITCODE = 0
}
$build = Join-Path $PSScriptRoot 'scripts/build-windows.ps1'
$dest = Join-Path $PSScriptRoot 'dist/kohdalab-ple-windows'
& $build
Set-Content -LiteralPath (Join-Path $dest 'stale.marker') -Value 'stale'
& $build
if (Test-Path -LiteralPath (Join-Path $dest 'stale.marker')) { throw 'stale file survived rebuild' }
Set-Content -LiteralPath (Join-Path $dest 'keep.marker') -Value 'previous good package'
$global:failLicense = $true
$failed = $false
try { & $build } catch { $failed = $true }
if (-not $failed) { throw 'license collection failure was ignored' }
if (-not (Test-Path -LiteralPath (Join-Path $dest 'keep.marker'))) { throw 'previous package was lost' }
if (@(Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot 'dist') -Filter '.kohdalab-ple-windows-staging-*').Count -ne 0) { throw 'staging leaked' }
'PACKAGING_TEST_PASS'
''')
            result = subprocess.run([PWSH, "-NoLogo", "-NoProfile", "-File", str(launcher)], capture_output=True, text=True, timeout=30, cwd=ROOT.parent)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("PACKAGING_TEST_PASS", result.stdout)
