#!/usr/bin/env python3
"""Windows-only JSON-lines bridge for KohdaLab PLE hardware.

The process owns the LightField automation instance.  Its stdout is a protocol
channel: diagnostics must go to stderr only.
"""
from __future__ import annotations

import csv
import hashlib
import importlib.util
import json
import logging
import math
import os
from pathlib import Path
import queue
import sys
import threading
import time
from contextlib import redirect_stdout
from dataclasses import dataclass
from typing import Any, Callable, Protocol

PROTOCOL_REVISION = 1
LOG = logging.getLogger("ple_bridge")


class BridgeError(RuntimeError):
    pass


def _finite(name: str, value: Any, positive: bool = False) -> float:
    try:
        number = float(value)
    except (TypeError, ValueError) as error:
        raise BridgeError(f"{name} must be numeric") from error
    if not math.isfinite(number) or (positive and number <= 0):
        raise BridgeError(f"{name} must be a finite{' positive' if positive else ''} number")
    return number


def _args(args: Any) -> dict[str, Any]:
    if not isinstance(args, dict):
        raise BridgeError("args must be an object")
    return args


def _output_paths(path_base: Path) -> tuple[Path, Path, Path]:
    """Append, rather than replace, terminal output suffixes.

    Position labels such as ``Y_18.6mm`` are valid basenames.
    """
    return (path_base.with_name(path_base.name + ".spe"),
            path_base.with_name(path_base.name + ".csv"),
            path_base.with_name(path_base.name + ".lightfield.csv"))


def _wait_stable(
    paths: list[Path], deadline: float, cancel: threading.Event,
    exclusively_readable: Callable[[list[Path]], bool] | None = None,
) -> None:
    """Wait for stable, complete output files, optionally with a Windows lock probe."""
    previous: tuple[int, ...] | None = None
    stable = 0
    while time.monotonic() < deadline:
        if cancel.is_set():
            raise BridgeError("acquisition was stopped; output is intentionally not consumed")
        if all(path.is_file() for path in paths):
            sizes = tuple(path.stat().st_size for path in paths)
            unlocked = exclusively_readable is None or exclusively_readable(paths)
            if sizes == previous and all(size > 0 for size in sizes) and unlocked:
                stable += 1
                if stable >= 3:
                    return
            else:
                stable = 0
            previous = sizes
        time.sleep(0.15)
    missing = ", ".join(str(path) for path in paths if not path.is_file())
    raise BridgeError(f"LightField export did not complete before timeout ({missing or 'file still changing'})")


def _wait_export_task(task: Any, deadline: float, cancel: threading.Event) -> None:
    """Wait for an ExportAsync Task only when this SDK exposes one."""
    if task is None or not hasattr(task, "IsCompleted"):
        return
    while not bool(task.IsCompleted):
        if cancel.is_set():
            raise BridgeError("acquisition was stopped; output is intentionally not consumed")
        if time.monotonic() >= deadline:
            raise BridgeError("LightField CSV export did not complete before timeout")
        time.sleep(0.05)
    if bool(getattr(task, "IsFaulted", False)) or bool(getattr(task, "IsCanceled", False)):
        raise BridgeError(f"LightField CSV export failed: {getattr(task, 'Exception', 'task faulted or canceled')}")


def _read_one_roi_csv(path: Path) -> tuple[list[float], list[float]]:
    """Read the LightField CSV layout used by the legacy Pylightfield wrapper."""
    with path.open(newline="", encoding="utf-8-sig") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        raise BridgeError("LightField CSV contains no rows")
    fields = {" ".join(field.strip().lower().split()): field for field in rows[0] if field}
    # R/F/Y/X are ROI, frame, row and pixel metadata in standard LightField
    # exports. They are never the calibrated spectral columns.
    x_key = next((original for normalized, original in fields.items()
                  if normalized == "wavelength_nm" or normalized.startswith("wavelength")), None)
    y_key = next((original for normalized, original in fields.items()
                  if normalized == "counts" or normalized.startswith("intensity")), None)
    if (x_key is None or y_key is None) and set(fields) == {"x", "y"}:
        x_key, y_key = fields["x"], fields["y"]
    if not x_key or not y_key:
        raise BridgeError("LightField CSV must include X/wavelength and Y/intensity columns")
    roi_key = fields.get("r") or fields.get("roi")
    frame_key = fields.get("f") or fields.get("frame")
    row_key = fields.get("y")
    if roi_key and len({row[roi_key] for row in rows}) != 1:
        raise BridgeError("multiple ROIs are not supported by PLE bridge revision 1")
    if frame_key and len({row[frame_key] for row in rows}) != 1:
        raise BridgeError("multiple exported frames are not supported; combine frames in LightField")
    if row_key and row_key != y_key and len({row[row_key] for row in rows}) != 1:
        raise BridgeError("multiple detector rows are not supported by PLE bridge revision 1")
    wavelength: list[float] = []
    counts: list[float] = []
    for row in rows:
        x, y = _finite(x_key, row[x_key]), _finite(y_key, row[y_key])
        if x <= 0:
            raise BridgeError("wavelength values must be positive")
        wavelength.append(x)
        counts.append(y)
    if len(wavelength) < 2:
        raise BridgeError("LightField CSV needs at least two spectral samples")
    if any(right <= left for left, right in zip(wavelength, wavelength[1:])):
        raise BridgeError("wavelength values must be strictly increasing; unsupported ROI/frame layout")
    return wavelength, counts


class Spectrometer(Protocol):
    def connect(self, args: dict[str, Any]) -> dict[str, Any]: ...
    def configure(self, args: dict[str, Any]) -> dict[str, Any]: ...
    def acquire(self, args: dict[str, Any], cancel: threading.Event) -> dict[str, Any]: ...
    def stop(self) -> None: ...
    def state(self) -> dict[str, Any]: ...
    def disconnect(self) -> None: ...


class LightFieldSpectrometer:
    REQUIRED_BACKGROUND_SETTINGS = frozenset({
        "selected_regions", "adc_analog_gain", "adc_speed", "adc_quality", "adc_bit_depth",
        "readout_control_mode", "sensor_temperature_setpoint", "online_cosmic_ray_enabled",
        "online_cosmic_ray_method", "online_cosmic_ray_kernel_size", "online_flatfield_enabled",
        "online_flatfield_reference_file", "online_blemish_enabled", "online_blemish_definition_file",
        "online_orientation_enabled", "online_orientation_flip_horizontal",
        "online_orientation_flip_vertical", "online_orientation_rotate_clockwise",
    })
    """Thin, owned-instance wrapper around LightField AutomationV5."""
    def __init__(self) -> None:
        self.auto = self.experiment = self.file_manager = None
        self.api: dict[str, Any] = {}
        self.completed = None
        self.settings: dict[str, Any] = {}

    def _load_api(self) -> None:
        if self.api:
            return
        if os.name != "nt":
            raise BridgeError("LightField bridge requires Windows")
        root = os.environ.get("LIGHTFIELD_ROOT")
        if not root:
            raise BridgeError("LIGHTFIELD_ROOT must point to the LightField installation")
        try:
            import clr  # type: ignore
            from System import Int32, String  # type: ignore
            from System.Collections.Generic import List  # type: ignore
            from System.IO import File, FileAccess, FileMode, FileShare  # type: ignore
            from System.Threading import AutoResetEvent  # type: ignore
            sys.path.extend([root, os.path.join(root, "AddInViews")])
            clr.AddReference("PrincetonInstruments.LightFieldViewV5")
            clr.AddReference("PrincetonInstruments.LightField.AutomationV5")
            clr.AddReference("PrincetonInstruments.LightFieldAddInSupportServices")
            from PrincetonInstruments.LightField.Automation import Automation  # type: ignore
            from PrincetonInstruments.LightField.AddIns import (  # type: ignore
                CameraSettings, ExperimentSettings, ExportFileType, SpectrometerSettings,
            )
        except Exception as error:  # pragma: no cover - Windows vendor SDK
            raise BridgeError(f"could not load LightField AutomationV5: {error}") from error
        self.api = locals()

    def _set(self, setting: Any, value: Any) -> None:
        if not self.experiment.Exists(setting):
            raise BridgeError(f"LightField experiment does not support setting {setting}")
        try:
            self.experiment.SetValue(setting, self.api["Int32"](value) if isinstance(value, int) and not isinstance(value, bool) else value)
        except Exception as error:
            raise BridgeError(f"LightField could not set {setting}: {error}") from error

    def _get(self, setting: Any) -> Any:
        if not self.experiment.Exists(setting):
            raise BridgeError(f"LightField experiment does not support setting {setting}")
        try:
            return self.experiment.GetValue(setting)
        except Exception as error:
            raise BridgeError(f"LightField could not read {setting}: {error}") from error

    def _observed_setting(self, group: str, member: str) -> dict[str, Any]:
        """Read one documented setting without mistaking absence for equality."""
        try:
            setting = getattr(self.api[group], member)
        except (KeyError, AttributeError) as error:
            return {"status": "unknown_sdk_member", "error": str(error)}
        try:
            if not self.experiment.Exists(setting):
                return {"status": "unsupported"}
            return {"status": "supported", "value": self._json_setting_value(self.experiment.GetValue(setting))}
        except Exception as error:
            return {"status": "unreadable", "error": str(error)}

    @staticmethod
    def _json_setting_value(value: Any) -> bool | int | float | str:
        """Convert documented SDK scalar/enum results before JSON transport."""
        if isinstance(value, bool): return value
        if isinstance(value, int): return value
        if isinstance(value, float):
            if math.isfinite(value): return value
            raise ValueError("non-finite numeric setting")
        if isinstance(value, str): return value
        to_string = getattr(value, "ToString", None)
        if callable(to_string) and isinstance((result := to_string()), str): return result
        raise TypeError(f"unsupported setting value type {type(value).__name__}")

    def _selected_regions(self) -> dict[str, Any]:
        try:
            regions = [
                {name: int(getattr(region, name)) for name in ("X", "Y", "Width", "Height", "XBinning", "YBinning")}
                for region in self.experiment.SelectedRegions
            ]
            if not regions or any(region["X"] < 0 or region["Y"] < 0 or region["Width"] <= 0 or region["Height"] <= 0 or region["XBinning"] <= 0 or region["YBinning"] <= 0 for region in regions):
                return {"status": "unqualified", "error": "SelectedRegions is empty or has invalid geometry/binning"}
            return {"status": "supported", "value": regions}
        except AttributeError as error:
            return {"status": "unknown_sdk_member", "error": str(error)}
        except Exception as error:
            return {"status": "unreadable", "error": str(error)}

    @staticmethod
    def _reference_identity(observation: dict[str, Any]) -> dict[str, Any]:
        if observation.get("status") != "supported" or not isinstance(observation.get("value"), str):
            return {"status": "unqualified", "error": "reference setting is unavailable"}
        path = Path(observation["value"])
        try:
            digest = hashlib.sha256()
            with path.open("rb") as handle:
                for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                    digest.update(chunk)
            return {"status": "supported", "path": str(path), "sha256": digest.hexdigest()}
        except OSError as error:
            return {"status": "unqualified", "path": str(path), "error": str(error)}

    def _read_settings(self) -> dict[str, Any]:
        a = self.api
        background_settings = {
            "selected_regions": self._selected_regions(),
            "adc_analog_gain": self._observed_setting("CameraSettings", "AdcAnalogGain"),
            "adc_speed": self._observed_setting("CameraSettings", "AdcSpeed"),
            "adc_quality": self._observed_setting("CameraSettings", "AdcQuality"),
            "adc_bit_depth": self._observed_setting("CameraSettings", "AdcBitDepth"),
            "readout_control_mode": self._observed_setting("CameraSettings", "ReadoutControlMode"),
            "sensor_temperature_setpoint": self._observed_setting("CameraSettings", "SensorTemperatureSetPoint"),
            "online_cosmic_ray_enabled": self._observed_setting("ExperimentSettings", "OnlineCorrectionsCosmicRayCorrectionEnabled"),
            "online_cosmic_ray_method": self._observed_setting("ExperimentSettings", "OnlineCorrectionsCosmicRayCorrectionMethod"),
            "online_cosmic_ray_kernel_size": self._observed_setting("ExperimentSettings", "OnlineCorrectionsCosmicRayCorrectionKernelSize"),
            "online_flatfield_enabled": self._observed_setting("ExperimentSettings", "OnlineCorrectionsFlatfieldCorrectionEnabled"),
            "online_flatfield_reference_file": self._observed_setting("ExperimentSettings", "OnlineCorrectionsFlatfieldCorrectionReferenceFile"),
            "online_blemish_enabled": self._observed_setting("ExperimentSettings", "OnlineCorrectionsBlemishCorrectionEnabled"),
            "online_blemish_definition_file": self._observed_setting("ExperimentSettings", "OnlineCorrectionsBlemishCorrectionDefinitionFile"),
            "online_orientation_enabled": self._observed_setting("ExperimentSettings", "OnlineCorrectionsOrientationCorrectionEnabled"),
            "online_orientation_flip_horizontal": self._observed_setting("ExperimentSettings", "OnlineCorrectionsOrientationCorrectionFlipHorizontally"),
            "online_orientation_flip_vertical": self._observed_setting("ExperimentSettings", "OnlineCorrectionsOrientationCorrectionFlipVertically"),
            "online_orientation_rotate_clockwise": self._observed_setting("ExperimentSettings", "OnlineCorrectionsOrientationCorrectionRotateClockwise"),
        }
        if background_settings["online_flatfield_enabled"].get("value") is True:
            background_settings["online_flatfield_reference_identity"] = self._reference_identity(background_settings["online_flatfield_reference_file"])
        if background_settings["online_blemish_enabled"].get("value") is True:
            background_settings["online_blemish_definition_identity"] = self._reference_identity(background_settings["online_blemish_definition_file"])
        return {
            "exposure_ms": float(self._get(a["CameraSettings"].ShutterTimingExposureTime)),
            "frames_combined": int(self._get(a["ExperimentSettings"].FrameSettingsFramesCombined)),
            "center_nm": float(self._get(a["SpectrometerSettings"].GratingCenterWavelength)),
            "grating": str(self._get(a["SpectrometerSettings"].Grating)),
            "online_background_correction": bool(self._get(a["ExperimentSettings"].OnlineCorrectionsBackgroundCorrectionEnabled)),
            "background_settings": background_settings,
            # Temperature evolves during a run; record it but do not use the
            # reading/status as a BG equality criterion.  The setpoint is kept
            # separately so a saved run shows the intended condition.
            "temperature": {
                "reading": self._observed_setting("CameraSettings", "SensorTemperatureReading"),
                "status": self._observed_setting("CameraSettings", "SensorTemperatureStatus"),
            },
            "hardware": {"lightfield": self._hardware_inventory()},
        }

    @staticmethod
    def _device_value(device: Any, *names: str) -> str | None:
        """Return one documented-or-exposed LightField property without guessing."""
        for name in names:
            try:
                value = getattr(device, name)
            except Exception:
                continue
            if value is not None and str(value).strip():
                return str(value).strip()
        return None

    def _hardware_inventory(self) -> dict[str, Any]:
        """Best-effort, read-only inventory; unsupported SDK members never block data."""
        result: dict[str, Any] = {"available": True, "devices": [], "grating_choices": [], "errors": []}
        try:
            devices = self.experiment.ExperimentDevices
        except Exception as error:
            result["available"] = False
            result["errors"].append(f"ExperimentDevices unavailable: {error}")
            devices = []
        try:
            for device in devices:
                result["devices"].append({
                    "type": self._device_value(device, "Type", "DeviceType"),
                    "model": self._device_value(device, "Model"),
                    "serial_number": self._device_value(device, "SerialNumber"),
                })
        except Exception as error:
            result["errors"].append(f"could not enumerate LightField devices: {error}")
        result["devices"].sort(key=lambda device: tuple(device.get(key) or "" for key in ("type", "model", "serial_number")))
        try:
            choices = self.experiment.GetCurrentCapabilities(self.api["SpectrometerSettings"].Grating)
            result["grating_choices"] = [str(choice) for choice in choices]
        except Exception as error:
            result["errors"].append(f"grating capability query unavailable: {error}")
        return result

    @staticmethod
    def _same_settings(expected: dict[str, Any], actual: dict[str, Any]) -> bool:
        basic = (expected.get("online_background_correction") == actual.get("online_background_correction")
                and expected["frames_combined"] == actual["frames_combined"]
                and expected["grating"] == actual["grating"]
                and math.isclose(expected["exposure_ms"], actual["exposure_ms"], rel_tol=0, abs_tol=1e-6)
                and math.isclose(expected["center_nm"], actual["center_nm"], rel_tol=0, abs_tol=1e-6))
        if not basic:
            return False
        expected_bg, actual_bg = expected.get("background_settings"), actual.get("background_settings")
        if expected_bg is None or actual_bg is None:  # legacy/basic requested-setting comparison
            return True
        if not isinstance(expected_bg, dict) or not isinstance(actual_bg, dict):
            return False
        for value in (*expected_bg.values(), *actual_bg.values()):
            if not isinstance(value, dict) or value.get("status") not in {"supported", "unsupported"}:
                return False
        return expected_bg == actual_bg

    @staticmethod
    def _same_runtime_settings(expected: dict[str, Any], actual: dict[str, Any]) -> bool:
        snapshots = (expected.get("background_settings"), actual.get("background_settings"))
        return all(isinstance(snapshot, dict) and LightFieldSpectrometer.REQUIRED_BACKGROUND_SETTINGS <= snapshot.keys() for snapshot in snapshots) and LightFieldSpectrometer._same_settings(expected, actual)

    def _assert_expected_settings(self) -> None:
        actual = self._read_settings()
        if not self._same_runtime_settings(self.settings, actual):
            raise BridgeError(f"LightField settings changed after configuration; expected {self.settings}, read {actual}")

    def _exclusively_readable(self, paths: list[Path]) -> bool:
        """A paused writer still holds the file on Windows; do not parse it."""
        file_type = self.api.get("File")
        if file_type is None:
            return True
        handles: list[Any] = []
        try:
            for path in paths:
                handles.append(file_type.Open(
                    str(path), self.api["FileMode"].Open, self.api["FileAccess"].Read,
                    getattr(self.api["FileShare"], "None"),
                ))
            return True
        except Exception:
            return False
        finally:
            for handle in handles:
                try:
                    handle.Dispose()
                except Exception:
                    pass

    def _apply(self, args: dict[str, Any]) -> dict[str, Any]:
        exposure = _finite("exposure_ms", args.get("exposure_ms"), True)
        center = _finite("center_nm", args.get("center_nm"), True)
        try:
            frames = int(args.get("frames_combined"))
        except (TypeError, ValueError) as error:
            raise BridgeError("frames_combined must be an integer") from error
        if frames < 1:
            raise BridgeError("frames_combined must be at least 1")
        grating = args.get("grating", "")
        if not isinstance(grating, str):
            raise BridgeError("grating must be an exact LightField grating name or an empty string")
        a = self.api
        self._set(a["CameraSettings"].ShutterTimingExposureTime, exposure)
        self._set(a["ExperimentSettings"].FrameSettingsFramesCombined, frames)
        if grating:
            self._set(a["SpectrometerSettings"].Grating, grating)
            self._wait_ready(20.)
        # Changing a turret/grating can reset the center or its allowed range.
        # Apply the requested wavelength to the selected grating, then read back.
        self._set(a["SpectrometerSettings"].GratingCenterWavelength, center)
        # An empty grating preserves the template grating, but the actual
        # readback is always what is recorded and checked before acquisition.
        self._set(a["ExperimentSettings"].OnlineCorrectionsBackgroundCorrectionEnabled, False)
        self._wait_ready(20.)
        self.settings = self._read_settings()
        expected = {"online_background_correction":False,"exposure_ms": exposure, "frames_combined": frames, "center_nm": center,
                    "grating": self.settings["grating"] if not grating else grating}
        if not self._same_settings(expected, self.settings):
            raise BridgeError(f"LightField rejected a requested setting; requested {expected}, read {self.settings}")
        return dict(self.settings)

    def connect(self, args: dict[str, Any]) -> dict[str, Any]:
        if self.auto is not None:
            raise BridgeError("spectrometer is already connected")
        experiment_name = args.get("experiment")
        if not isinstance(experiment_name, str) or not experiment_name:
            raise BridgeError("experiment must be a non-empty LightField experiment name")
        visible = bool(args.get("visible", True))
        self._load_api()
        try:  # pragma: no cover - Windows vendor SDK
            experiments = self.api["List"][self.api["String"]]()
            experiments.Add(experiment_name)
            self.auto = self.api["Automation"](visible, experiments)
            app = self.auto.LightFieldApplication
            self.experiment, self.file_manager = app.Experiment, app.FileManager
            self._validate_acquisition_api()
            self.experiment.Load(experiment_name)
            self.completed = self.api["AutoResetEvent"](False)
            self.experiment.ExperimentCompleted += self._on_completed
            self._apply(args)
        except BridgeError:
            self.disconnect()
            raise
        except Exception as error:
            self.disconnect()
            raise BridgeError(f"LightField connection failed: {error}") from error
        return self.state()

    def _validate_acquisition_api(self) -> None:
        # Instance methods cannot be established by the non-motion enum check.
        # Fail at connection before changing acquisition or grating settings.
        missing = [f"{label}.{name}"
                   for label, instance, names in (
                       ("Experiment", self.experiment, ("Load", "Exists", "SetValue", "GetValue", "Acquire", "Stop", "IsReadyToRun", "ExperimentCompleted")),
                       ("FileManager", self.file_manager, ("CreateExportSettings", "ExportAsync")))
                   for name in names if not hasattr(instance, name)]
        if missing:
            raise BridgeError(f"LightField acquisition API missing: {', '.join(missing)}")

    def _wait_ready(self, timeout_s: float) -> None:
        if self.experiment is None or not hasattr(self.experiment, "IsReadyToRun"):
            raise BridgeError("LightField does not expose IsReadyToRun; readiness cannot be confirmed")
        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            if bool(self.experiment.IsReadyToRun): return
            time.sleep(0.05)
        raise BridgeError("LightField did not confirm ready/idle before timeout")

    def _on_completed(self, _sender: Any, _event: Any) -> None:  # pragma: no cover - SDK callback
        self.completed.Set()

    def configure(self, args: dict[str, Any]) -> dict[str, Any]:
        if self.auto is None:
            raise BridgeError("spectrometer is not connected")
        return self._apply(args)

    def acquire(self, args: dict[str, Any], cancel: threading.Event) -> dict[str, Any]:
        if self.auto is None:
            raise BridgeError("spectrometer is not connected")
        if cancel.is_set():
            raise BridgeError("acquisition was canceled before LightField Acquire")
        base = args.get("path_base")
        if not isinstance(base, str) or not base:
            raise BridgeError("path_base must be a non-empty path base")
        path_base = Path(base)
        # ``Path.with_suffix`` considers the final part of a decimal sample
        # position (for example ``Y_18.6mm``) an extension.  The app owns the
        # basename, so only reject extensions that would collide with files we
        # create; append output suffixes to the complete basename below.
        if path_base.suffix.lower() in {".spe", ".csv"}:
            raise BridgeError("path_base must not include a .spe or .csv extension")
        timeout_s = _finite("timeout_s", args.get("timeout_s", 120), True)
        path_base.parent.mkdir(parents=True, exist_ok=True)
        spe, native_csv, csv_path = _output_paths(path_base)
        if spe.exists() or native_csv.exists() or csv_path.exists():
            raise BridgeError("refusing to overwrite existing .spe, temporary LightField CSV, or .lightfield.csv output")
        a = self.api
        self._set(a["ExperimentSettings"].FileNameGenerationDirectory, str(path_base.parent))
        self._set(a["ExperimentSettings"].FileNameGenerationBaseFileName, path_base.name)
        self._set(a["ExperimentSettings"].FileNameGenerationAttachIncrement, False)
        self._set(a["ExperimentSettings"].FileNameGenerationAttachDate, False)
        self._set(a["ExperimentSettings"].FileNameGenerationAttachTime, False)
        self._assert_expected_settings()
        self._wait_ready(10.)
        self.completed.Reset()
        try:  # pragma: no cover - SDK callback
            deadline = time.monotonic() + timeout_s
            self.experiment.Acquire()
            while time.monotonic() < deadline:
                if cancel.is_set():
                    self.stop()
                    raise BridgeError("Stop requested; LightField ready/idle confirmed; output is intentionally not consumed")
                if self.completed.WaitOne(100):
                    break
            else:
                self.stop()
                raise BridgeError("LightField acquisition did not complete before timeout")
            _wait_stable([spe], deadline, cancel, self._exclusively_readable)
            export = self.file_manager.CreateExportSettings(a["ExportFileType"].Csv)
            export_task = self.file_manager.ExportAsync(export, str(spe))
            _wait_export_task(export_task, deadline, cancel)
            _wait_stable([spe, native_csv], deadline, cancel, self._exclusively_readable)
            os.replace(native_csv, csv_path)
            wavelength, counts = _read_one_roi_csv(csv_path)
        except BridgeError:
            if cancel.is_set():
                self.stop()
                raise BridgeError("Stop requested; LightField ready/idle confirmed; raw output is intentionally not consumed")
            raise
        except Exception as error:
            raise BridgeError(f"LightField acquisition failed: {error}") from error
        return {"wavelength_nm": wavelength, "counts": counts, "spe_path": str(spe), "csv_path": str(csv_path),
                "background": bool(args.get("background", False)), "settings": self._read_settings()}

    def stop(self, timeout_s: float = 3.) -> None:
        if self.experiment is None:
            return
        try:  # Vendor API exposes Stop in supported LightField versions.
            self.experiment.Stop()
            self._wait_ready(timeout_s)
        except Exception as error:  # pragma: no cover - SDK dependent
            raise BridgeError(f"LightField stop request failed: {error}") from error

    def state(self) -> dict[str, Any]:
        return {"connected": self.auto is not None, "settings": dict(self.settings)}

    def disconnect(self) -> None:
        if self.auto is not None:
            try:
                self.auto.Dispose()
            except Exception as error:  # pragma: no cover - SDK dependent
                raise BridgeError(f"LightField dispose failed; disconnect unconfirmed: {error}") from error
        self.auto = self.experiment = self.file_manager = self.completed = None
        self.settings = {}


class PowerMeter(Protocol):
    def connect(self, args: dict[str, Any]) -> dict[str, Any]: ...
    def read(self) -> dict[str, Any]: ...
    def disconnect(self) -> None: ...
    def state(self) -> dict[str, Any]: ...


class Shutter(Protocol):
    def connect(self, args: dict[str, Any]) -> dict[str, Any]: ...
    def set_closed(self, closed: bool, timeout_s: float, cancel: threading.Event) -> dict[str, Any]: ...
    def disconnect(self) -> None: ...
    def state(self) -> dict[str, Any]: ...


class ThorlabsMffShutter:
    """Explicit-state MFF shutter; state meaning is supplied by the operator."""
    def __init__(self, device_factory: Callable[[str], Any] | None = None) -> None:
        self.device: Any = None
        self._device_factory = device_factory
        self.serial: str | None = None
        self.closed_state: int | None = None
        self.open_state: int | None = None

    @staticmethod
    def _state(name: str, value: Any) -> int:
        if isinstance(value, bool):
            raise BridgeError(f"{name} must be an integer device state")
        try:
            state = int(value)
        except (TypeError, ValueError) as error:
            raise BridgeError(f"{name} must be an integer device state") from error
        if str(value).strip() != str(state):
            raise BridgeError(f"{name} must be an integer device state")
        return state

    def connect(self, args: dict[str, Any]) -> dict[str, Any]:
        if self.device is not None:
            raise BridgeError("shutter is already connected")
        serial = args.get("serial")
        if not isinstance(serial, str) or not serial.strip():
            raise BridgeError("serial must be a non-empty shutter serial number")
        closed_state = self._state("closed_state", args.get("closed_state"))
        open_state = self._state("open_state", args.get("open_state"))
        if closed_state == open_state:
            raise BridgeError("closed_state and open_state must be distinct")
        _finite("timeout_s", args.get("timeout_s", 10), True)
        try:
            # Keep the vendor dependency lazy: manual-background users need not
            # install pylablib and a failed import must not move any hardware.
            if self._device_factory is None:
                from pylablib.devices import Thorlabs  # type: ignore
                device = Thorlabs.kinesis.MFF(serial.strip())
            else:
                device = self._device_factory(serial.strip())
            device.get_state()  # connectivity/readback only: never move on connect
        except Exception as error:  # pragma: no cover - hardware/vendor SDK
            raise BridgeError(f"could not connect shutter {serial}: {error}") from error
        self.device, self.serial = device, serial.strip()
        self.closed_state, self.open_state = closed_state, open_state
        return self.state()

    def set_closed(self, closed: bool, timeout_s: float, cancel: threading.Event) -> dict[str, Any]:
        if self.device is None or self.closed_state is None or self.open_state is None:
            raise BridgeError("shutter is not connected")
        if not closed and cancel.is_set():
            raise BridgeError("shutter cannot open while stop is latched")
        target = self.closed_state if closed else self.open_state
        try:
            self.device.move_to_state(target)
            deadline = time.monotonic() + timeout_s
            while time.monotonic() < deadline:
                if not closed and cancel.is_set():
                    raise BridgeError("shutter opening canceled")
                if int(self.device.get_state()) == target:
                    return self.state()
                time.sleep(0.05)
        except BridgeError:
            raise
        except Exception as error:  # pragma: no cover - hardware/vendor SDK
            raise BridgeError(f"shutter motion/readback failed: {error}") from error
        raise BridgeError(f"shutter did not reach {'closed' if closed else 'open'} state before timeout")

    def disconnect(self) -> None:
        device = self.device
        self.device = self.serial = self.closed_state = self.open_state = None
        if device is None:
            return
        close = getattr(device, "close", None)
        if callable(close):
            try:
                close()
            except Exception as error:  # pragma: no cover - hardware/vendor SDK
                raise BridgeError(f"shutter disconnect failed: {error}") from error

    def state(self) -> dict[str, Any]:
        result: dict[str, Any] = {"connected": self.device is not None, "serial": self.serial,
                                  "closed_state": self.closed_state, "open_state": self.open_state}
        if self.device is None:
            return result
        try:
            current = int(self.device.get_state())
        except Exception as error:  # pragma: no cover - hardware/vendor SDK
            result["read_error"] = str(error)
            return result
        result["state"] = current
        result["closed"] = current == self.closed_state
        return result


class LegacyThorlabsPowerMeter:
    """Adapter for the laboratory's existing ThorlabsPowerMeter.py, supplied externally."""
    def __init__(self) -> None:
        self.meter = None
        self.resource: str | None = None
        self.wavelength_nm: float | None = None
        self.hardware: dict[str, Any] = {"available": False, "errors": ["power meter is not connected"]}

    @staticmethod
    def _optional_text(value: Any) -> str | None:
        return str(value).strip() if value is not None and str(value).strip() else None

    def _hardware_inventory(self, meter: Any) -> dict[str, Any]:
        """Read helper-provided meter/sensor identity without treating it as settings."""
        result: dict[str, Any] = {
            "available": True, "resource": self.resource,
            "model": self._optional_text(getattr(meter, "modelName", None)),
            "serial_number": self._optional_text(getattr(meter, "serialNumber", None)),
            "manufacturer": self._optional_text(getattr(meter, "manufacturer", None)),
            "driver_version": self._optional_text(getattr(type(meter), "driverVersion", None)),
            "wavelength_range_nm": {"min": None, "max": None},
            "sensor": {"available": False}, "errors": [],
        }
        try:
            # The original helper appends decoded flags each time this method
            # runs. Clear its display cache so a sensor hot-swap readback is
            # an identity, rather than a growing history of prior sensors.
            for name in ("sensorName", "sensorSerialNumber", "sensorCalibrationMessage", "sensorType", "sensorSubType", "sensorFlags"):
                if hasattr(meter, name):
                    setattr(meter, name, "" if name == "sensorFlags" else None)
            with redirect_stdout(sys.stderr):
                meter.getSensorInfo()
            sensor_type = self._optional_text(getattr(meter, "sensorType", None))
            sensor_name = self._optional_text(getattr(meter, "sensorName", None))
            no_sensor = sensor_type == "No sensor" or sensor_name in (None, "No sensor")
            result["sensor"] = {
                "available": not no_sensor,
                "name": None if no_sensor else sensor_name,
                "serial_number": None if no_sensor else self._optional_text(getattr(meter, "sensorSerialNumber", None)),
                "type": sensor_type,
                "subtype": self._optional_text(getattr(meter, "sensorSubType", None)),
                "calibration": None if no_sensor else self._optional_text(getattr(meter, "sensorCalibrationMessage", None)),
                "flags": self._optional_text(getattr(meter, "sensorFlags", None)),
            }
        except Exception as error:
            result["errors"].append(f"sensor inventory unavailable: {error}")
        for key, selector in (("min", 1), ("max", 2)):
            try:
                # Refresh limits from the current sensor, not the helper cache
                # left by a prior setWaveLength or a different sensor head.
                status, value = meter.deviceNET.getWavelength(selector)
                if int(status) < 0:
                    raise BridgeError(f"SDK status {status}")
                value = float(value)
                if not math.isfinite(value) or value <= 0:
                    raise BridgeError("non-positive or non-finite wavelength limit")
                result["wavelength_range_nm"][key] = value
            except Exception as error:
                result["errors"].append(f"sensor wavelength {key} unavailable: {error}")
        return result

    @staticmethod
    def _read_actual_wavelength(meter: Any) -> float:
        """Refresh the legacy helper's wavelength cache and validate its readback."""
        meter.getWaveLength()
        return _finite("power meter wavelength readback", getattr(meter, "wavelengthSet", None), True)

    @staticmethod
    def _assert_requested_wavelength_in_range(meter: Any, wavelength_nm: float) -> None:
        """Reject legacy-helper clamping when it exposes physical limits."""
        minimum = getattr(meter, "wavelengthMin", None)
        maximum = getattr(meter, "wavelengthMax", None)
        if minimum is None or maximum is None:
            return
        minimum = _finite("power meter minimum wavelength", minimum, True)
        maximum = _finite("power meter maximum wavelength", maximum, True)
        if minimum > maximum:
            raise BridgeError("power meter reported an invalid wavelength range")
        if wavelength_nm < minimum or wavelength_nm > maximum:
            raise BridgeError(
                f"requested wavelength {wavelength_nm:.9g} nm is outside the power meter range "
                f"{minimum:.9g}–{maximum:.9g} nm"
            )

    @classmethod
    def _set_meter_wavelength(cls, meter: Any, wavelength_nm: float,
                              tolerance_nm: float = 1e-6) -> float:
        tolerance_nm = _finite("wavelength_tolerance_nm", tolerance_nm, True)
        meter.setWaveLength(wavelength_nm)
        actual = cls._read_actual_wavelength(meter)
        cls._assert_requested_wavelength_in_range(meter, wavelength_nm)
        if not math.isclose(actual, wavelength_nm, rel_tol=0, abs_tol=tolerance_nm):
            raise BridgeError(
                f"power meter wavelength readback {actual:.9g} nm differs from requested {wavelength_nm:.9g} nm"
            )
        return actual

    @staticmethod
    def _set_meter_average(meter: Any, seconds: float) -> float:
        meter.setAverageTime(seconds)
        meter.getAverageTime()
        actual = getattr(meter, "averageTimeSet", None)
        # The original ZIP helper stores the complete .NET (status, seconds)
        # return from getAvgTime(0), unlike its other scalar readback caches.
        if isinstance(actual, (tuple, list)):
            if len(actual) != 2 or not isinstance(actual[0], int) or actual[0] < 0:
                raise BridgeError(f"power meter averaging readback failed: {actual!r}")
            actual = actual[1]
        actual = _finite("power meter averaging readback", actual, True)
        if not math.isclose(actual, seconds, rel_tol=0, abs_tol=1e-6):
            raise BridgeError(f"power meter averaging readback {actual} differs from requested {seconds}")
        return actual

    def connect(self, args: dict[str, Any]) -> dict[str, Any]:
        driver_path = args.get("driver_path")
        resource = args.get("resource")
        if not isinstance(driver_path, str) or not Path(driver_path).is_dir():
            raise BridgeError("driver_path must be the folder containing ThorlabsPowerMeter.py and its official DLL")
        if not isinstance(resource, str) or not resource:
            raise BridgeError("resource must be a VISA resource name")
        wavelength = _finite("wavelength_nm", args.get("wavelength_nm"), True)
        tolerance = _finite("wavelength_tolerance_nm", args.get("wavelength_tolerance_nm", 1.0), True)
        average = _finite("average_s", args.get("average_s", 0.5), True)
        module_file = Path(driver_path) / "ThorlabsPowerMeter.py"
        dll_dir = Path(driver_path) / "Thorlabs_DotNet_dll"
        if not module_file.is_file():
            raise BridgeError(f"no ThorlabsPowerMeter.py at {module_file}")
        if not dll_dir.is_dir():
            raise BridgeError(f"no Thorlabs_DotNet_dll folder at {dll_dir}")
        spec = importlib.util.spec_from_file_location("kohdalab_thorlabs_power", module_file)
        if spec is None or spec.loader is None:
            raise BridgeError("could not load ThorlabsPowerMeter.py")
        module = importlib.util.module_from_spec(spec)
        meter = None
        try:
            with redirect_stdout(sys.stderr):
                spec.loader.exec_module(module)
                # The legacy helper's listDevices() incorrectly uses extend on
                # a string.  Add the DLL directory correctly first, then pass
                # it for its version lookup and AddReference resolution.
                if str(dll_dir) not in sys.path:
                    sys.path.append(str(dll_dir))
                devices = module.ThorlabsPowerMeter.listDevices(str(dll_dir))
                meter = devices.connect(resource)
                if meter is None:
                    raise BridgeError(f"could not connect power meter {resource}")
                actual_wavelength = self._set_meter_wavelength(meter, wavelength, tolerance)
                self._set_meter_average(meter, average)
                meter.setPowerAutoRange(True)
        except BridgeError:
            self._disconnect_local_meter(meter)
            raise
        except Exception as error:
            self._disconnect_local_meter(meter)
            raise BridgeError(f"Thorlabs power meter setup failed: {error}") from error
        self.meter, self.resource, self.wavelength_nm = meter, resource, actual_wavelength
        self.hardware = self._hardware_inventory(meter)
        return self.state()

    @staticmethod
    def _disconnect_local_meter(meter: Any) -> None:
        disconnect = getattr(meter, "disconnect", None)
        if callable(disconnect):
            try:
                with redirect_stdout(sys.stderr):
                    disconnect()
            except Exception:
                pass

    def set_wavelength(self, wavelength_nm: float, tolerance_nm: float = 1e-6) -> dict[str, Any]:
        if self.meter is None:
            raise BridgeError("power meter is not connected")
        requested = _finite("wavelength_nm", wavelength_nm, True)
        tolerance = _finite("wavelength_tolerance_nm", tolerance_nm, True)
        try:
            with redirect_stdout(sys.stderr):
                actual = self._set_meter_wavelength(self.meter, requested, tolerance)
        except BridgeError:
            raise
        except Exception as error:
            raise BridgeError(f"power meter wavelength set/readback failed: {error}") from error
        self.wavelength_nm = actual
        return {"wavelength_nm": actual}

    def read(self) -> dict[str, Any]:
        if self.meter is None:
            raise BridgeError("power meter is not connected")
        try:
            with redirect_stdout(sys.stderr):
                self.meter.updatePowerReading(0)
                wavelength_nm = self._read_actual_wavelength(self.meter)
            native_value = _finite("native power reading", getattr(self.meter, "meterPowerReading", None))
            native_unit = str(getattr(self.meter, "meterPowerUnit", "")).strip()
            if native_unit == "W":
                power_w = native_value
            elif native_unit == "dBm":
                try:
                    power_w = 1e-3 * 10 ** (native_value / 10)
                except OverflowError as error:
                    raise BridgeError("power meter dBm conversion overflowed") from error
                if not math.isfinite(power_w):
                    raise BridgeError("power meter dBm conversion is not finite")
            else:
                raise BridgeError(f"unsupported power meter unit {native_unit!r}; expected 'W' or 'dBm'")
        except BridgeError:
            raise
        except Exception as error:
            raise BridgeError(f"power meter read failed: {error}") from error
        self.wavelength_nm = wavelength_nm
        # A sensor can be hot-swapped while the console remains connected.
        # Refresh identity for the point metadata without making a failed
        # optional inventory query reject an otherwise valid power sample.
        self.hardware = self._hardware_inventory(self.meter)
        if self.hardware["sensor"].get("type") == "No sensor":
            raise BridgeError("power meter reports no sensor head attached")
        return {
            "power_w": power_w,
            "wavelength_nm": wavelength_nm,
            "native_unit": native_unit,
            "native_value": native_value,
            "unit": "W",
            "hardware": self.hardware,
        }

    def disconnect(self) -> None:
        if self.meter is not None:
            try:
                with redirect_stdout(sys.stderr):
                    self.meter.disconnect()
            finally:
                self.meter, self.resource, self.wavelength_nm = None, None, None
                self.hardware = {"available": False, "errors": ["power meter is not connected"]}

    def state(self) -> dict[str, Any]:
        return {"connected": self.meter is not None, "resource": self.resource,
                "wavelength_nm": self.wavelength_nm, "hardware": self.hardware}


@dataclass
class HardwareBridge:
    spectrometer: Spectrometer
    power: PowerMeter
    cancel: threading.Event
    shutter: Shutter | None = None

    @classmethod
    def production(cls) -> "HardwareBridge":
        return cls(LightFieldSpectrometer(), LegacyThorlabsPowerMeter(), threading.Event(), ThorlabsMffShutter())

    def request_stop(self) -> None:
        """Signal cancellation without calling a vendor SDK from the reader thread."""
        self.cancel.set()

    def dispatch(self, op: str, raw_args: Any) -> Any:
        args = _args(raw_args)
        if op == "protocol":
            return {"revision": PROTOCOL_REVISION}
        if op == "connect_spectrometer":
            self.cancel.clear()
            return self.spectrometer.connect(args)
        if op == "configure_spectrometer":
            # A reviewed configuration begins a new run and is the only normal
            # operation allowed to release a previously acknowledged stop.
            self.cancel.clear()
            return self.spectrometer.configure(args)
        if op == "acquire":
            return self.spectrometer.acquire(args, self.cancel)
        if op == "ensure_idle":
            self.cancel.set()
            self.spectrometer.stop()
            if self.shutter is not None and self.shutter.state().get("connected"):
                self.shutter.set_closed(True, 10., self.cancel)
            return {"idle_confirmed":True,"spectrometer":self.spectrometer.state(),"shutter":self.shutter.state() if self.shutter else None}
        if op == "disconnect_spectrometer":
            self.spectrometer.disconnect(); return self.spectrometer.state()
        if op == "connect_power":
            return self.power.connect(args)
        if op == "power_read":
            return self.power.read()
        if op == "power_set_wavelength":
            requested = _finite("wavelength_nm", args.get("wavelength_nm"), True)
            tolerance = _finite("wavelength_tolerance_nm", args.get("wavelength_tolerance_nm", 1e-6), True)
            setter = getattr(self.power, "set_wavelength", None)
            if not callable(setter):
                raise BridgeError("connected power meter does not support wavelength control")
            result = setter(requested, tolerance)
            if not isinstance(result, dict):
                raise BridgeError("power meter wavelength setter returned an invalid response")
            actual = _finite("power meter wavelength readback", result.get("wavelength_nm"), True)
            if not math.isclose(actual, requested, rel_tol=0, abs_tol=tolerance):
                raise BridgeError(
                    f"power meter wavelength readback {actual:.9g} nm differs from requested {requested:.9g} nm"
                )
            return result
        if op == "disconnect_power":
            self.power.disconnect(); return self.power.state()
        if op == "shutter_connect":
            if self.shutter is None:
                raise BridgeError("automatic shutter support is unavailable")
            return self.shutter.connect(args)
        if op == "shutter_set":
            if self.shutter is None:
                raise BridgeError("automatic shutter support is unavailable")
            closed = args.get("closed")
            if not isinstance(closed, bool):
                raise BridgeError("closed must be a boolean")
            return self.shutter.set_closed(closed, _finite("timeout_s", args.get("timeout_s", 10), True), self.cancel)
        if op == "disconnect_shutter":
            if self.shutter is None:
                raise BridgeError("automatic shutter support is unavailable")
            self.shutter.disconnect(); return self.shutter.state()
        if op == "state":
            return {"revision": PROTOCOL_REVISION, "spectrometer": self.spectrometer.state(),
                    "power": self.power.state(), "shutter": self.shutter.state() if self.shutter else {"connected": False}}
        if op == "shutdown":
            self.shutdown()
            return {"shutdown": True}
        raise BridgeError(f"unknown operation {op!r}")

    def shutdown(self) -> None:
        """Best-effort resource cleanup, with fail-closed shutter semantics."""
        self.cancel.set()
        errors: list[str] = []
        try:
            self.spectrometer.stop()
        except Exception as error:
            errors.append(f"spectrometer stop: {error}")
        if self.shutter is not None and self.shutter.state().get("connected"):
            try:
                # Closing deliberately remains legal after cancellation latches.
                self.shutter.set_closed(True, 10, self.cancel)
            except Exception as error:
                errors.append(f"shutter close: {error}")
            try:
                self.shutter.disconnect()
            except Exception as error:
                errors.append(f"shutter disconnect: {error}")
        try:
            self.power.disconnect()
        except Exception as error:
            errors.append(f"power disconnect: {error}")
        try:
            self.spectrometer.disconnect()
        except Exception as error:
            errors.append(f"spectrometer disconnect: {error}")
        if errors:
            raise BridgeError("shutdown incomplete: " + "; ".join(errors))


def response(request_id: Any, ok: bool, result: Any = None, error: str | None = None) -> str:
    return json.dumps({"id": request_id, "ok": ok, "result": result if ok else None, "error": error}, allow_nan=False)


class RpcServer:
    def __init__(self, bridge: HardwareBridge, stdin: Any = sys.stdin, stdout: Any = sys.stdout) -> None:
        self.bridge, self.stdin, self.stdout = bridge, stdin, stdout
        self.requests: queue.Queue[dict[str, Any] | None] = queue.Queue()
        self.output_lock = threading.Lock()

    def _read(self) -> None:
        for line in self.stdin:
            try:
                request = json.loads(line)
                if not isinstance(request, dict):
                    raise BridgeError("request must be an object")
                if request.get("op") == "stop":
                    if request.get("revision") != PROTOCOL_REVISION:
                        raise BridgeError(f"request revision must be {PROTOCOL_REVISION}")
                    self.bridge.request_stop()
                    self._write(response(request.get("id"), True, {"stop_requested": True}))
                else:
                    self.requests.put(request)
            except Exception as error:
                self._write(response(None, False, error=str(error)))
        self.bridge.request_stop()
        self.requests.put(None)

    def _write(self, line: str) -> None:
        # The reader thread can acknowledge stop while the worker is replying.
        # Hold one lock so the JSON-lines transport never interleaves bytes.
        with self.output_lock:
            self.stdout.write(line + "\n")
            self.stdout.flush()

    def serve(self) -> None:
        threading.Thread(target=self._read, name="ple-rpc-reader", daemon=True).start()
        try:
            while (request := self.requests.get()) is not None:
                request_id = request.get("id")
                try:
                    if request.get("revision") != PROTOCOL_REVISION:
                        raise BridgeError(f"request revision must be {PROTOCOL_REVISION}")
                    op = request.get("op")
                    if not isinstance(op, str):
                        raise BridgeError("op must be a string")
                    self._write(response(request_id, True, self.bridge.dispatch(op, request.get("args", {}))))
                except Exception as error:
                    LOG.exception("operation failed")
                    self._write(response(request_id, False, error=str(error)))
        finally:
            try:
                self.bridge.shutdown()
            except Exception as error:
                LOG.error("bridge shutdown incomplete: %s", error)


def main() -> int:
    logging.basicConfig(level=logging.INFO, stream=sys.stderr, format="%(asctime)s %(levelname)s %(message)s")
    RpcServer(HardwareBridge.production()).serve()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
