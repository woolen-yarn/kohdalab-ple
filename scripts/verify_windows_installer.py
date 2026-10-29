#!/usr/bin/env python3
"""Run actual Setup install/update/uninstall in a disposable Windows CI user.

--fixture compiles and exercises the same NSIS shell before expensive builds.
The real release check compares every installed file with the portable stage.
No GUI or instrument connection is started.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import uuid

import build_windows_installer as installer


def special_folder(identifier: int) -> Path:
    buffer = ctypes.create_unicode_buffer(32768)
    result = ctypes.windll.shell32.SHGetFolderPathW(None, identifier, None, 0, buffer)
    if result != 0:
        raise RuntimeError(f"Windows folder lookup failed: {identifier}")
    return Path(buffer.value)


def run_setup(setup: Path, *, succeeds: bool = True) -> None:
    result = subprocess.run([str(setup), "/S"], timeout=90)
    if (result.returncode == 0) != succeeds:
        raise RuntimeError(f"Unexpected Setup exit code {result.returncode}; expected success={succeeds}")


def verify(setup: Path, payload: Path, version: str, *, fixture: bool = False) -> dict:
    if os.name != "nt" or os.environ.get("GITHUB_ACTIONS") != "true":
        raise RuntimeError("Installer lifecycle checks require a disposable GitHub Actions Windows user")
    import winreg
    app = special_folder(38) / "Kohdalab/KohdaLab PLE"
    workspace = Path.home() / ".kohdalab/ple"
    programs = special_folder(23) / "KohdaLab/PLE"
    desktop = special_folder(25) / "KohdaLab PLE.lnk"
    key = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\KohdaLabPLE"
    try:
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, key, 0, winreg.KEY_READ | winreg.KEY_WOW64_64KEY):
            raise RuntimeError("A real PLE installer registration already exists")
    except FileNotFoundError:
        pass
    if app.exists() or programs.exists() or desktop.exists():
        raise RuntimeError("Refusing to test over an existing PLE installation/shortcut")
    files = installer.payload_files(payload)
    protected = []
    extra = app / "operator-notes.csv"
    try:
        run_setup(setup)
        for name, expected in files.items():
            assert installer.digest(app / name) == expected, f"Installed payload differs: {name}"
        assert (app / "Uninstall.exe").is_file()
        assert desktop.is_file()
        assert (programs / "Shutter (automatic BG).lnk").is_file()
        shell = r'''$w=New-Object -ComObject WScript.Shell; $p=$w.CreateShortcut($env:PLE_TEST_LINK); @{target=$p.TargetPath;working=$p.WorkingDirectory} | ConvertTo-Json -Compress'''
        env = dict(os.environ, PLE_TEST_LINK=str(programs / "KohdaLab PLE.lnk"))
        shortcut = json.loads(subprocess.check_output(["powershell", "-NoProfile", "-Command", shell], env=env, text=True))
        assert Path(shortcut["target"]) == app / "kohdalab-ple.exe"
        assert Path(shortcut["working"]) == app
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, key, 0, winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as reg:
            assert winreg.QueryValueEx(reg, "DisplayVersion")[0] == version
        workspace.mkdir(parents=True, exist_ok=True)
        if not fixture:
            reported = subprocess.check_output([str(app / "kohdalab-ple.exe"), "--version"], text=True, cwd=workspace)
            assert version in reported
            result = json.loads(subprocess.check_output([
                str(app / "kohdalab-ple.exe"), "--check-config", "--config", str(app / "examples/ple-settings.json")
            ], text=True, cwd=workspace))
            assert result["config_valid"] is True
            env = dict(os.environ, PYTHONPATH=str(app / "bindings/python"))
            # Known machine-wide install discovery must work outside the app folder.
            discovered = subprocess.check_output(["python", "-c", "from kohdalab_ple import find_binary; print(find_binary())"],
                                                  text=True, cwd=workspace, env=env).strip()
            assert Path(discovered) == app / "kohdalab-ple.exe"
        workspace.mkdir(parents=True, exist_ok=True)
        marker = uuid.uuid4().hex
        for name in (f"installer-test-{marker}.json", f"installer-test-{marker}.csv"):
            path = workspace / name
            path.write_bytes(b"operator data: preserve exactly\r\n")
            protected.append((path, installer.digest(path)))
        extra.write_text("unlisted operator file")
        # A repeat/update replaces packaged files but retains operator data.
        (app / "START-HERE.md").write_text("old packaged contents")
        run_setup(setup)
        for name, expected in files.items():
            assert installer.digest(app / name) == expected, name
        for path, expected in protected:
            assert installer.digest(path) == expected
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, r"Software\KohdaLab\KohdaLab PLE", 0, winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as reg:
            backup = Path(winreg.QueryValueEx(reg, "PreviousLocation")[0])
        assert (backup / extra.name).read_text() == "unlisted operator file"
        assert not extra.exists()
        # Reject an update before any copying if either executable is locked.
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.CreateFileW.argtypes = [ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32,
                                      ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p]
        kernel.CreateFileW.restype = ctypes.c_void_p
        kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        for binary in ("kohdalab-ple.exe", "companions/kohdalab-shutter/kohdalab-shutter-gui.exe"):
            handle = kernel.CreateFileW(str(app / binary), 0x80000000, 0, None, 3, 0, None)
            if handle == ctypes.c_void_p(-1).value:
                raise ctypes.WinError(ctypes.get_last_error())
            try:
                run_setup(setup, succeeds=False)
            finally:
                kernel.CloseHandle(handle)
            assert installer.digest(app / "START-HERE.md") == files["START-HERE.md"]
        # NSIS normally copies the uninstaller to TEMP and returns through a
        # small launcher, so wait for registry/payload removal as well as exit.
        subprocess.run([str(app / "Uninstall.exe"), "/S"], check=True, timeout=90)
        deadline = time.monotonic() + 45
        while any((app / name).exists() for name in files) and time.monotonic() < deadline:
            time.sleep(0.2)
        for name in files:
            assert not (app / name).exists(), f"Uninstall retained packaged file: {name}"
        assert not desktop.exists()
        assert not (programs / "KohdaLab PLE.lnk").exists()
        try:
            winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, key, 0, winreg.KEY_READ | winreg.KEY_WOW64_64KEY)
        except FileNotFoundError:
            pass
        else:
            raise AssertionError("Uninstall registry entry was retained")
        for path, expected in protected:
            assert installer.digest(path) == expected
        assert (backup / extra.name).read_text() == "unlisted operator file"
        return {"version": version, "setup_sha256": installer.digest(setup),
                "payload_sha256": installer.payload_digest(files), "result": "passed",
                "checks": ["fresh_install", "all_payload_hashes", "shortcuts_and_working_directory",
                           "repeat_update", "user_data_preserved", "locked_app_and_shutter_refused",
                           "uninstall_and_registration_removal", "unlisted_local_file_preserved_in_backup"]
                          + ([] if fixture else ["installed_version_and_config", "installed_python_discovery"]),
                "physical_hardware_tested": False}
    finally:
        for path, _ in protected:
            path.unlink(missing_ok=True)
        extra.unlink(missing_ok=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--setup", type=Path)
    parser.add_argument("--payload-dir", type=Path)
    parser.add_argument("--version")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--fixture", action="store_true")
    args = parser.parse_args()
    if args.fixture:
        with tempfile.TemporaryDirectory(prefix="ple installer fixture ") as temp:
            root = Path(temp)
            payload = root / "payload with spaces"
            for name in installer.REQUIRED:
                path = payload / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"fixture only; never executed")
            setup = root / "fixture-Setup.exe"
            installer.build(payload, setup, "0.0.1")
            result = verify(setup, payload, "0.0.1", fixture=True)
    else:
        if not all((args.setup, args.payload_dir, args.version, args.output)):
            parser.error("--setup, --payload-dir, --version and --output are required")
        result = verify(args.setup.resolve(), args.payload_dir.resolve(), args.version)
        args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
