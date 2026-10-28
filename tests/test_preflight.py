from pathlib import Path
import unittest


class PreflightScriptTests(unittest.TestCase):
    def test_preflight_requires_explicit_app_and_python_checker(self) -> None:
        source = (Path(__file__).resolve().parents[1] / "scripts" / "preflight.ps1").read_text()
        self.assertIn("[string]$AppPath", source)
        self.assertIn("sdk_check.py", source)
        self.assertIn("--check-config --config", source)
        self.assertIn("$sdkText | ConvertFrom-Json", source)

    def test_preflight_is_non_motion_and_reports_hardware_gap(self) -> None:
        source = (Path(__file__).resolve().parents[1] / "scripts" / "preflight.ps1").read_text()
        self.assertIn("hardware_not_tested", source)
        self.assertIn("Vendor assembly type loading is opt-in", source)
        self.assertNotIn("PA_Set", source)
        self.assertNotIn("OpenInstrument", source)
        self.assertNotIn("conex_server.py", source)

    def test_cross_package_script_fails_instead_of_faking_an_exe(self) -> None:
        source = (Path(__file__).resolve().parents[1] / "scripts" / "package-windows-cross.sh").read_text()
        self.assertIn('cargo build --offline --release --locked --target "$target" --bin kohdalab-ple', source)
        self.assertIn("cargo reported success but produced no executable", source)

    def test_release_packaging_is_offline_and_contains_api_validation(self) -> None:
        root = Path(__file__).resolve().parents[1]
        windows = (root / "scripts" / "package-windows-cross.sh").read_text()
        macos = (root / "scripts" / "package-macos.sh").read_text()
        native_windows = (root / "scripts" / "build-windows.ps1").read_text()
        self.assertIn('rustup target list --installed', windows)
        self.assertIn('cargo metadata --offline --locked', windows)
        self.assertIn('scripts/verify_gui_api.py', windows)
        self.assertIn('scripts/verify_gui_api.py', macos)
        self.assertIn('scripts/verify_gui_api.py', native_windows)
        self.assertIn('Move-Item -LiteralPath $stage -Destination $dest', native_windows)
        self.assertIn('Move-Item -LiteralPath $dest -Destination $backup', native_windows)


if __name__ == "__main__":
    unittest.main()
