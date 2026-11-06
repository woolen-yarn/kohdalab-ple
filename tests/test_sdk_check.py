import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import subprocess


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("sdk_check", ROOT / "bridge" / "sdk_check.py")
assert SPEC and SPEC.loader
sdk_check = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(sdk_check)


class SdkCheckTests(unittest.TestCase):
    def test_python_probe_is_bounded_and_reports_process_failures(self) -> None:
        with patch.object(sdk_check.shutil, "which", return_value="python"):
            with patch.object(sdk_check.subprocess, "run", side_effect=subprocess.TimeoutExpired("python", 30)):
                ok, reason = sdk_check.probe_python("python", "clr")
                self.assertFalse(ok)
                self.assertIn("30 seconds", reason)
            with patch.object(sdk_check.subprocess, "run", side_effect=OSError("cannot execute")):
                ok, reason = sdk_check.probe_python("python")
                self.assertFalse(ok)
                self.assertIn("cannot execute", reason)
            with patch.object(sdk_check.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, "", "")) as invoked:
                self.assertTrue(sdk_check.probe_python("python")[0])
                self.assertEqual(invoked.call_args.kwargs["timeout"], 30)
                self.assertIn("win32", invoked.call_args.args[0][2])

    def test_checker_has_no_device_construction_or_transport_imports(self) -> None:
        source = (ROOT / "bridge" / "sdk_check.py").read_text()
        self.assertNotIn("Automation(", source)
        self.assertNotIn("ConexCC(", source)
        self.assertNotIn("import socket", source)
        self.assertNotIn("import serial", source)

    def test_legacy_helper_is_ast_checked_without_importing(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Thorlabs_DotNet_dll").mkdir()
            for name in ("TLPM_64.dll", "Thorlabs.TLPM_64.Interop.dll"):
                (root / "Thorlabs_DotNet_dll" / name).touch()
            source = "raise RuntimeError('must not execute')\nclass ThorlabsPowerMeter:\n"
            source += "".join(f" def {name}(self): pass\n" for name in sdk_check.REQUIRED_POWER_METHODS)
            (root / "ThorlabsPowerMeter.py").write_text(source)
            checks = sdk_check.inspect_legacy_power(str(root))
        self.assertTrue(all(item["passed"] for item in checks), checks)

    def test_empty_or_tlpmx_only_directory_does_not_pass_legacy_check(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            dll_dir = root / "Thorlabs_DotNet_dll"
            dll_dir.mkdir()
            (dll_dir / "TLPMX_64.dll").touch()
            checks = sdk_check.inspect_legacy_power(str(root))
        missing = [item["name"] for item in checks if not item["passed"]]
        self.assertIn("legacy power TLPM_64.dll", missing)
        self.assertIn("legacy power Thorlabs.TLPM_64.Interop.dll", missing)

    def test_lightfield_metadata_loader_is_injected_and_never_constructs_automation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "AddInViews").mkdir()
            requested = []
            class Static:
                pass
            for members in sdk_check.REQUIRED_LIGHTFIELD_MEMBERS.values():
                for member in members:
                    setattr(Static, member, "setting")
            def loader(path: Path):
                requested.append(path)
                return {name: Static for name in sdk_check.REQUIRED_LIGHTFIELD_MEMBERS}
            checks = sdk_check.inspect_lightfield(str(root), loader)
        self.assertEqual(requested, [root])
        self.assertTrue(all(item["passed"] for item in checks), checks)

    def test_conex_reflection_loader_uses_type_method_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Newport.CONEXCC.CommandInterface.dll").touch()
            class Method:
                def __init__(self, name: str): self.Name = name
            class TypeOnly:
                @staticmethod
                def GetMethods(): return [Method(name) for name in sdk_check.REQUIRED_CONEX_METHODS]
            checks = sdk_check.inspect_conex_sdk(str(root), lambda _root: TypeOnly)
        self.assertTrue(all(item["passed"] for item in checks), checks)

    def test_legacy_protocol_is_supported_with_stop_warning(self) -> None:
        config = {"mode": "hardware", "spectrometer": {"experiment": "x"}, "power": {"resource": "USB::x"},
                  "stage": {"protocol": "legacy_tcp", "host": "host", "com_port": "COM1", "min_mm": 0, "max_mm": 25},
                  "background": {"mode": "manual"}}
        checks, warnings = sdk_check.configuration_checks(config)
        protocol = next(item for item in checks if item["name"] == "CONEX protocol")
        self.assertTrue(protocol["passed"])
        self.assertEqual(protocol["severity"], "warning")
        self.assertTrue(warnings)


if __name__ == "__main__":
    unittest.main()
