#!/usr/bin/env python3
"""Non-motion installation/configuration checks for a KohdaLab PLE PC.

This program deliberately never constructs LightField ``Automation`` or
``ConexCC`` objects, opens COM/VISA/TCP resources, or discovers devices.
Vendor reflection is opt-in because it loads assemblies but remains non-motion.
"""
from __future__ import annotations

import argparse
import ast
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from typing import Any, Callable

REQUIRED_POWER_METHODS = {"listDevices", "connect", "disconnect", "setWaveLength", "getWaveLength", "setAverageTime", "getAverageTime", "setPowerAutoRange", "updatePowerReading"}
REQUIRED_CONEX_METHODS = {"OpenInstrument", "CloseInstrument", "SL_Get", "SR_Get", "TP", "TS", "PA_Set", "ST"}
REQUIRED_LIGHTFIELD_MEMBERS = {
    "Automation": (), "CameraSettings": ("ShutterTimingExposureTime",),
    "ExperimentSettings": ("FrameSettingsFramesCombined", "FileNameGenerationDirectory", "FileNameGenerationBaseFileName", "FileNameGenerationAttachIncrement", "FileNameGenerationAttachDate", "FileNameGenerationAttachTime", "OnlineCorrectionsBackgroundCorrectionEnabled"),
    "SpectrometerSettings": ("GratingCenterWavelength", "Grating"), "ExportFileType": ("Csv",),
}


def check(name: str, passed: bool, detail: str, *, severity: str = "error") -> dict[str, Any]:
    return {"name": name, "passed": bool(passed), "detail": detail, "severity": severity}


def required_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip()) and not value.startswith("enter_")


def load_config(path: Path) -> tuple[dict[str, Any] | None, str | None]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return None, f"cannot parse config: {error}"
    return (value, None) if isinstance(value, dict) else (None, "configuration root must be a JSON object")


def helper_methods(path: Path) -> set[str]:
    """Read legacy helper syntax only; never import or execute vendor code."""
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    for node in tree.body:
        if isinstance(node, ast.ClassDef) and node.name == "ThorlabsPowerMeter":
            return {item.name for item in node.body if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef))}
    return set()


def inspect_legacy_power(driver_path: str) -> list[dict[str, Any]]:
    if not required_string(driver_path):
        return [check("legacy power helper", False, "power.driver_path must name the reviewed helper directory")]
    root = Path(driver_path)
    helper, dll_dir = root / "ThorlabsPowerMeter.py", root / "Thorlabs_DotNet_dll"
    checks = [check("legacy power helper file", helper.is_file(), f"expected {helper}"),
              check("legacy power SDK directory", dll_dir.is_dir(), f"expected {dll_dir}")]
    for name in ("TLPM_64.dll", "Thorlabs.TLPM_64.Interop.dll"):
        dll = dll_dir / name
        checks.append(check(f"legacy power {name}", dll.is_file(),
                            f"expected {dll}; TLPMX/PM100D DLLs are not drop-in replacements"))
    if helper.is_file():
        try:
            missing = sorted(REQUIRED_POWER_METHODS - helper_methods(helper))
            checks.append(check("legacy power helper API", not missing,
                                "required methods present" if not missing else f"missing methods: {', '.join(missing)}"))
        except (OSError, SyntaxError) as error:
            checks.append(check("legacy power helper API", False, f"AST parse failed: {error}"))
    return checks


def probe_python(executable: str, module: str | None = None) -> tuple[bool, str]:
    if not required_string(executable) or shutil.which(executable) is None:
        return False, f"configured Python is not on PATH: {executable!r}"
    source = "import struct,sys; assert struct.calcsize('P') == 8, '64-bit Python required'; assert sys.platform == 'win32', 'Hardware LightField bridge requires Windows Python'"
    if module:
        source += f"; import {module}"
    try:
        result = subprocess.run([executable, "-c", source], text=True, capture_output=True, check=False, timeout=30)
    except subprocess.TimeoutExpired:
        return False, "Python/SDK import check exceeded 30 seconds; no device was opened"
    except OSError as error:
        return False, f"could not start configured Python: {error}"
    if result.returncode:
        return False, (result.stderr or result.stdout or f"exit status {result.returncode}").strip()
    return True, "64-bit Windows Python" + (f" and {module} import succeeded" if module else "")


def _load_lightfield_sdk(root: Path) -> dict[str, Any]:
    """Load type metadata only; do not instantiate ``Automation``."""
    import clr  # type: ignore
    for directory in (root, root / "AddInViews"):
        if str(directory) not in sys.path:
            sys.path.insert(0, str(directory))
    clr.AddReference("PrincetonInstruments.LightFieldViewV5")
    clr.AddReference("PrincetonInstruments.LightField.AutomationV5")
    clr.AddReference("PrincetonInstruments.LightFieldAddInSupportServices")
    from PrincetonInstruments.LightField.Automation import Automation  # type: ignore
    from PrincetonInstruments.LightField.AddIns import CameraSettings, ExperimentSettings, ExportFileType, SpectrometerSettings  # type: ignore
    return {"Automation": Automation, "CameraSettings": CameraSettings, "ExperimentSettings": ExperimentSettings,
            "SpectrometerSettings": SpectrometerSettings, "ExportFileType": ExportFileType}


def inspect_lightfield(root_text: str, loader: Callable[[Path], dict[str, Any]] = _load_lightfield_sdk) -> list[dict[str, Any]]:
    if not required_string(root_text):
        return [check("LightField root", False, "set LIGHTFIELD_ROOT explicitly")]
    root = Path(root_text)
    checks = [check("LightField root", root.is_dir(), f"expected installation directory {root}"),
              check("LightField AddInViews", (root / "AddInViews").is_dir(), f"expected {root / 'AddInViews'}")]
    if not all(item["passed"] for item in checks):
        return checks
    try:
        sdk = loader(root)
        missing = [f"{kind}.{member}" for kind, members in REQUIRED_LIGHTFIELD_MEMBERS.items()
                   for member in members if kind not in sdk or not hasattr(sdk[kind], member)]
        missing += [kind for kind in REQUIRED_LIGHTFIELD_MEMBERS if kind not in sdk]
        checks.append(check("LightField Automation type metadata", not missing,
                            "required type members are available" if not missing else f"missing: {', '.join(missing)}"))
    except Exception as error:
        checks.append(check("LightField Automation type metadata", False, f"assembly/type load failed: {error}"))
    return checks


def _load_conex_sdk(root: Path) -> Any:
    """Reflect the vendor type only; do not construct a controller."""
    import clr  # type: ignore
    clr.AddReference(str(root / "Newport.CONEXCC.CommandInterface.dll"))
    import CommandInterfaceConexCC  # type: ignore
    return CommandInterfaceConexCC.ConexCC


def inspect_conex_sdk(root_text: str | None, loader: Callable[[Path], Any] = _load_conex_sdk) -> list[dict[str, Any]]:
    if not required_string(root_text):
        return [check("Newport CONEX SDK", False, "provide --conex-sdk-dir or CONEX_SDK_DIR; no DLL location is assumed")]
    root = Path(root_text)
    assembly = root / "Newport.CONEXCC.CommandInterface.dll"
    checks = [check("Newport CONEX SDK directory", root.is_dir(), f"expected explicit directory {root}"),
              check("Newport CONEX assembly", assembly.is_file(), f"expected {assembly}; vendor DLL is not redistributed")]
    if not all(item["passed"] for item in checks):
        return checks
    try:
        method_names = {str(method.Name) for method in loader(root).GetMethods()}
        missing = sorted(REQUIRED_CONEX_METHODS - method_names)
        checks.append(check("Newport CONEX type reflection", not missing,
                            "required methods are available without constructing ConexCC" if not missing else f"missing methods: {', '.join(missing)}"))
    except Exception as error:
        checks.append(check("Newport CONEX type reflection", False, f"assembly/type reflection failed: {error}"))
    return checks


def configuration_checks(config: dict[str, Any]) -> tuple[list[dict[str, Any]], list[str]]:
    spectrometer = config.get("spectrometer") if isinstance(config.get("spectrometer"), dict) else {}
    stage = config.get("stage") if isinstance(config.get("stage"), dict) else {}
    power = config.get("power") if isinstance(config.get("power"), dict) else {}
    background = config.get("background") if isinstance(config.get("background"), dict) else {}
    checks = [check("hardware mode", config.get("mode") == "hardware", "config.mode must be hardware"),
              check("LightField experiment", required_string(spectrometer.get("experiment")), "enter a reviewed LightField experiment name"),
              check("power VISA resource", required_string(power.get("resource")), "enter the reviewed VISA resource; it is not opened here"),
              check("CONEX host", required_string(stage.get("host")), "enter the controller-PC hostname/address"),
              check("CONEX COM port", required_string(stage.get("com_port")), "enter the controller-PC COM port")]
    minimum, maximum = stage.get("min_mm"), stage.get("max_mm")
    checks.append(check("CONEX soft range", isinstance(minimum, (int, float)) and isinstance(maximum, (int, float)) and 0 <= minimum < maximum <= 25, "soft range must remain inside 0 to 25 mm"))
    warnings: list[str] = []
    if not spectrometer.get("grating"):
        warnings.append("Blank grating preserves the LightField experiment grating; verify its exact readback and wavelength coverage before acquisition")
    if power.get("wavelength_mode", "fixed") == "fixed":
        warnings.append("Fixed power-sensor wavelength does not follow a PLE scan; use measured position calibration for wavelength-dependent correction")
    warnings.append("Camera model/ROI, power sensor identity/range, CONEX axis mapping and laser optics require physical qualification; apparatus model labels do not identify attached devices")
    if stage.get("protocol") == "remote_json":
        checks.append(check("CONEX protocol", True, "remote_json selected; monitored independent Stop is recommended"))
    elif stage.get("protocol") == "legacy_tcp":
        checks.append(check("CONEX protocol", True, "legacy_tcp is supported, but monitored independent Stop is unavailable", severity="warning"))
        warnings.append("legacy_tcp is supported but does not provide monitored independent Stop; remote_json is recommended")
    else:
        checks.append(check("CONEX protocol", False, "protocol must be remote_json or legacy_tcp"))
    if background.get("mode") == "automatic" and background.get("backend", "mff") == "mff":
        checks.append(check("automatic shutter serial", required_string(background.get("shutter_serial")), "enter the MFF shutter serial; state mapping remains a physical qualification"))
    if background.get("mode") == "automatic" and background.get("backend") == "kohdalab_shutter":
        endpoint = background.get("endpoint", "")
        checks.append(check("kohdalab-shutter endpoint", isinstance(endpoint,str) and (not endpoint.strip() or Path(endpoint).is_file()), "blank uses GUI discovery; otherwise specify the private endpoint file (no token is printed)"))
        warnings.append("kohdalab-shutter must be running on this PC with Local control API enabled and the physical Pico connected; settled controller states are not optical sensor confirmation")
    return checks, warnings


def report_for(config_path: Path, *, sdk_imports: bool, conex_sdk_dir: str | None) -> dict[str, Any]:
    config, error = load_config(config_path)
    checks: list[dict[str, Any]] = []
    warnings: list[str] = []
    if error or config is None:
        checks.append(check("configuration file", False, error or "unknown config error"))
    else:
        logical, warnings = configuration_checks(config)
        checks.extend(logical)
        spectrometer = config.get("spectrometer", {})
        background = config.get("background", {})
        python = spectrometer.get("python", "") if isinstance(spectrometer, dict) else ""
        for name, module in (("configured 64-bit Python", None), ("pythonnet import", "clr"), ("legacy helper pywin32 import", "win32api")):
            ok, detail = probe_python(python, module)
            checks.append(check(name, ok, detail))
        if isinstance(background, dict) and background.get("mode") == "automatic" and background.get("backend", "mff") == "mff":
            ok, detail = probe_python(python, "pylablib")
            checks.append(check("automatic shutter pylablib import", ok, detail))
        power = config.get("power", {})
        checks.extend(inspect_legacy_power(str(power.get("driver_path", "")) if isinstance(power, dict) else ""))
        if sdk_imports:
            checks.extend(inspect_lightfield(os.environ.get("LIGHTFIELD_ROOT", "")))
            local_conex_sdk = conex_sdk_dir or os.environ.get("CONEX_SDK_DIR")
            if local_conex_sdk:
                checks.extend(inspect_conex_sdk(local_conex_sdk))
            else:
                checks.append(check("remote CONEX SDK", True,
                    "SDK reflection skipped on acquisition PC; check Newport SDK on the remote controller PC", severity="warning"))
        else:
            checks.append(check("vendor SDK reflection", True, "not run; pass --sdk-imports to load type metadata without creating devices", severity="warning"))
    failed = [item for item in checks if not item["passed"] and item["severity"] == "error"]
    return {"check_kind": "installation/configuration only; no physical hardware operation", "config_path": str(config_path),
            "sdk_imports_requested": sdk_imports, "configuration_and_installation_checks": checks, "warnings": warnings,
            "installation_config_pass": not failed,
            "hardware_not_tested": ["No LightField Automation instance, camera acquisition, VISA resource, COM port, TCP endpoint, or device discovery was opened.",
                                    "Physically qualify LightField experiment/grating/export/Stop, CONEX readback/move/Stop within 0–25 mm, power calibration, and shutter fail-closed behavior."]}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True, type=Path, help="explicit PLE JSON configuration")
    parser.add_argument("--sdk-imports", action="store_true", help="load vendor assembly type metadata only; never construct devices")
    parser.add_argument("--conex-sdk-dir", help="explicit directory containing Newport.CONEXCC.CommandInterface.dll")
    args = parser.parse_args(argv)
    report = report_for(args.config, sdk_imports=args.sdk_imports, conex_sdk_dir=args.conex_sdk_dir)
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["installation_config_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
