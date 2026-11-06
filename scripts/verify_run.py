#!/usr/bin/env python3
"""Verify arithmetic and required files in a saved KohdaLab PLE run.

The verifier is read-only.  It does not connect to instruments, parse SPE
contents, or decide whether a physical calibration was valid.
"""
from __future__ import annotations

import argparse
import csv
import json
import math
from pathlib import Path
import sys
from typing import Any

HC_EV_NM = 1239.8419843320026


def finite(value: str, label: str) -> float:
    if isinstance(value, bool):
        raise ValueError(f"{label} must be numeric, not boolean")
    try:
        number = float(value)
    except (TypeError, ValueError) as error:
        raise ValueError(f"{label} is not numeric: {value!r}") from error
    if not math.isfinite(number):
        raise ValueError(f"{label} is not finite")
    return number


def read_csv(path: Path, required: set[str]) -> list[dict[str, str]]:
    try:
        with path.open(newline="", encoding="utf-8") as handle:
            reader = csv.DictReader(handle)
            fields = set(reader.fieldnames or [])
            missing = required - fields
            if missing:
                raise ValueError(f"{path}: missing columns {', '.join(sorted(missing))}")
            return list(reader)
    except OSError as error:
        raise ValueError(f"cannot read {path}: {error}") from error


def same_observation(left: float, right: float) -> bool:
    """Serialized observations must round-trip to the same IEEE-754 value."""
    return left == right


def close_arithmetic(left: float, right: float, tolerance: float) -> bool:
    """Allow rounding in derived arithmetic, never an absolute physical floor."""
    return math.isclose(left, right, rel_tol=tolerance, abs_tol=0.0) or math.isclose(
        left, right, rel_tol=0.0, abs_tol=max(math.ulp(left), math.ulp(right)) * 8
    )


def nonnegative_integer(value: Any, label: str) -> int:
    if isinstance(value, bool):
        raise ValueError(f"{label} must be a non-negative integer")
    number = finite(value, label)
    if number < 0 or not number.is_integer():
        raise ValueError(f"{label} must be a non-negative integer")
    return int(number)


def safe_spectrum_base(value: Any, identity: tuple[str, str]) -> str:
    if not isinstance(value, str) or not value or value in {".", ".."}:
        raise ValueError(f"point {identity}: invalid spectrum_base")
    if any(character in value for character in '/\\:<>"|?*') or any(ord(character) < 32 for character in value) or value.endswith((".", " ")):
        raise ValueError(f"point {identity}: spectrum_base must be a basename")
    return value


def optional_timestamp(value: dict[str, Any], key: str, label: str) -> float | None:
    timestamp = value.get(key)
    return None if timestamp is None else finite(timestamp, f"{label} {key}")


def nonempty_point_value(point: dict[str, str], key: str) -> str | None:
    value = point.get(key)
    return value if value is not None and value.strip() else None


def require_nonempty_spe(path: Path, label: str) -> None:
    try:
        if not path.is_file() or path.stat().st_size <= 0:
            raise ValueError(f"hardware run is missing a non-empty {label}")
    except OSError as error:
        raise ValueError(f"cannot inspect {label}: {error}") from error


def raw_spectrum(path: Path) -> tuple[list[float], list[float]]:
    rows = read_csv(path, {"wavelength_nm", "counts"})
    wavelength = [finite(row["wavelength_nm"], f"{path} wavelength") for row in rows]
    counts = [finite(row["counts"], f"{path} counts") for row in rows]
    if len(wavelength) < 2 or any(right <= left for left, right in zip(wavelength, wavelength[1:])):
        raise ValueError(f"{path}: wavelength axis must be strictly increasing with at least two bins")
    return wavelength, counts


def json_object(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read {path}: {error}") from error
    if not isinstance(value, dict):
        raise ValueError(f"{path}: expected JSON object")
    return value


def verify(run: Path, tolerance: float) -> dict[str, Any]:
    errors: list[str] = []
    warnings: list[str] = []
    try:
        settings = json_object(run / "settings.json")
        config = settings["config"]
        if not isinstance(config, dict):
            raise ValueError("settings.json: config must be an object")
        hardware = config.get("mode") == "hardware" or bool(settings.get("raw_spe_available"))
    except (KeyError, ValueError) as error:
        return {"run": str(run), "ok": False, "errors": [str(error)], "warnings": warnings, "verified_acquisitions": 0}
    try:
        bg_nm, bg_counts = raw_spectrum(run / "raw" / "background.csv")
        background_meta = json_object(run / "metadata" / "background.json")
        meta_spectrum = background_meta.get("wavelength_nm"), background_meta.get("counts")
        if not all(isinstance(values, list) for values in meta_spectrum):
            raise ValueError("background metadata must contain wavelength_nm and counts arrays")
        if meta_spectrum[0] != bg_nm or meta_spectrum[1] != bg_counts:
            errors.append("background metadata does not exactly match raw/background.csv")
        if hardware:
            require_nonempty_spe(run / "raw" / "background.spe", "raw/background.spe")
    except ValueError as error:
        errors.append(str(error))
        bg_nm, bg_counts = [], []

    try:
        points = read_csv(run / "points.csv", {"point_index", "repeat_index", "actual_mm", "sample_power_W", "spectrum_base"})
    except ValueError as error:
        return {"run": str(run), "ok": False, "errors": errors + [str(error)], "warnings": warnings, "verified_acquisitions": 0}
    seen: set[tuple[int, int]] = set()
    verified = 0
    for point in points:
        base = point.get("spectrum_base", "")
        identity = (point.get("point_index", ""), point.get("repeat_index", ""))
        try:
            base = safe_spectrum_base(base, identity)
            point_index = nonnegative_integer(point["point_index"], f"point {identity} point_index")
            repeat_index = nonnegative_integer(point["repeat_index"], f"point {identity} repeat_index")
            canonical_identity = (point_index, repeat_index)
            if canonical_identity in seen:
                raise ValueError(f"duplicate point/repeat row {identity}")
            seen.add(canonical_identity)
            actual = finite(point["actual_mm"], f"point {identity} actual_mm")
            power = finite(point["sample_power_W"], f"point {identity} sample_power_W")
            if not 0 <= actual <= 25:
                raise ValueError(f"point {identity}: actual_mm is outside 0–25 mm")
            if power <= 0:
                raise ValueError(f"point {identity}: sample_power_W must be positive")
            raw_nm, raw_counts = raw_spectrum(run / "raw" / f"{base}.csv")
            acquisition = json_object(run / "metadata" / f"{base}.json")
            spectrum = acquisition.get("spectrum")
            corrected = acquisition.get("corrected_counts")
            if not isinstance(spectrum, dict) or not isinstance(corrected, list):
                raise ValueError(f"{base}: metadata lacks spectrum/corrected_counts")
            metadata_identity = (
                nonnegative_integer(acquisition.get("point_index"), f"{base} metadata point_index"),
                nonnegative_integer(acquisition.get("repeat_index"), f"{base} metadata repeat_index"),
            )
            if metadata_identity != (point_index, repeat_index):
                raise ValueError(f"{base}: metadata point/repeat differs from points.csv")
            metadata_actual = finite(acquisition.get("actual_mm"), f"{base} metadata actual_mm")
            metadata_power = finite(acquisition.get("sample_power_w"), f"{base} metadata sample_power_w")
            raw_power = finite(acquisition.get("power_w"), f"{base} metadata power_w")
            if metadata_power <= 0 or raw_power <= 0:
                raise ValueError(f"{base}: metadata powers must be positive")
            point_raw_power = nonempty_point_value(point, "power_W")
            if point_raw_power is not None and not same_observation(
                finite(point_raw_power, f"point {identity} power_W"), raw_power
            ):
                raise ValueError(f"{base}: metadata power_w differs from points.csv")
            if not same_observation(metadata_actual, actual):
                raise ValueError(f"{base}: metadata actual_mm differs from points.csv")
            if not same_observation(metadata_power, power):
                raise ValueError(f"{base}: metadata sample_power_w differs from points.csv")
            correction = config.get("power", {}).get("correction_factor") if isinstance(config.get("power"), dict) else None
            if correction is not None:
                factor = finite(correction, "settings power correction_factor")
                if factor <= 0 or not close_arithmetic(metadata_power, raw_power * factor, tolerance):
                    raise ValueError(f"{base}: sample_power_w does not match power_w × correction_factor")
            timestamps = [
                optional_timestamp(acquisition, "power_timestamp_unix_s", base),
                optional_timestamp(acquisition, "acquisition_started_unix_s", base),
                optional_timestamp(acquisition, "timestamp_unix_s", base),
            ]
            for earlier, later in zip(timestamps, timestamps[1:]):
                if earlier is not None and later is not None and earlier > later:
                    raise ValueError(f"{base}: power/read/acquisition timestamps are out of order")
            for key, metadata_time in zip(
                ("power_timestamp_unix_s", "acquisition_started_unix_s", "acquisition_completed_unix_s"),
                timestamps,
            ):
                point_time = nonempty_point_value(point, key)
                if point_time is not None and metadata_time is not None and not same_observation(
                    finite(point_time, f"point {identity} {key}"), metadata_time
                ):
                    raise ValueError(f"{base}: {key} differs from points.csv")
            if spectrum.get("wavelength_nm") != raw_nm or spectrum.get("counts") != raw_counts:
                raise ValueError(f"{base}: metadata spectrum does not exactly match raw CSV")
            if len(raw_nm) != len(bg_nm) or raw_nm != bg_nm or len(corrected) != len(raw_nm):
                raise ValueError(f"{base}: raw/background/corrected bin axes do not match")
            processed = read_csv(run / "processed" / f"{base}.csv", {
                "emission_nm", "emission_eV", "raw_counts", "background_counts", "corrected_counts", "corrected_counts_per_W",
            })
            if len(processed) != len(raw_nm):
                raise ValueError(f"{base}: processed CSV bin count differs from raw")
            for index, row in enumerate(processed):
                values = {key: finite(row[key], f"{base} row {index} {key}") for key in row if key in {
                    "emission_nm", "emission_eV", "raw_counts", "background_counts", "corrected_counts", "corrected_counts_per_W",
                }}
                if not same_observation(values["emission_nm"], raw_nm[index]):
                    raise ValueError(f"{base}: processed wavelength differs at bin {index}")
                if not close_arithmetic(values["emission_eV"] * values["emission_nm"], HC_EV_NM, tolerance):
                    raise ValueError(f"{base}: energy conversion differs at bin {index}")
                if not same_observation(values["raw_counts"], raw_counts[index]) or not same_observation(values["background_counts"], bg_counts[index]):
                    raise ValueError(f"{base}: raw/background differs at bin {index}")
                expected = raw_counts[index] - bg_counts[index]
                if not close_arithmetic(values["corrected_counts"], expected, tolerance) or not close_arithmetic(float(corrected[index]), expected, tolerance):
                    raise ValueError(f"{base}: BG subtraction differs at bin {index}")
                if not close_arithmetic(values["corrected_counts_per_W"], expected / power, tolerance):
                    raise ValueError(f"{base}: power normalization differs at bin {index}")
            if hardware:
                require_nonempty_spe(run / "raw" / f"{base}.spe", f"raw/{base}.spe")
            verified += 1
        except (KeyError, ValueError) as error:
            errors.append(str(error))
    status_path = run / "status.json"
    if status_path.is_file():
        try:
            status = json_object(status_path)
            state = status.get("state")
            completed = status.get("completed_acquisitions")
            planned = status.get("planned_acquisitions")
            if state == "completed":
                if nonnegative_integer(completed, "status completed_acquisitions") != verified:
                    errors.append("completed run status completed_acquisitions differs from verified point rows")
                if nonnegative_integer(planned, "status planned_acquisitions") != verified:
                    errors.append("completed run status planned_acquisitions differs from verified point rows")
            elif completed != verified:
                warnings.append("status completed_acquisitions differs from verified point rows (interrupted runs may retain partial data)")
        except ValueError as error:
            errors.append(str(error))
    else:
        warnings.append("status.json is absent; run may be incomplete")
    return {"run": str(run), "ok": not errors, "errors": errors, "warnings": warnings,
            "verified_acquisitions": verified, "hardware_spe_required": hardware, "tolerance": tolerance}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path, help="saved run directory containing settings.json")
    parser.add_argument("--tolerance", type=float, default=1e-8, help="relative tolerance for derived arithmetic (default: 1e-8)")
    parser.add_argument("--json", action="store_true", help="emit the full JSON report")
    args = parser.parse_args(argv)
    if not math.isfinite(args.tolerance) or args.tolerance < 0:
        parser.error("--tolerance must be a finite non-negative number")
    report = verify(args.run, args.tolerance)
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print(f"{'PASS' if report['ok'] else 'FAIL'}: {report['verified_acquisitions']} acquisition(s) verified")
        for message in report["errors"]:
            print(f"ERROR: {message}")
        for message in report["warnings"]:
            print(f"WARNING: {message}")
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
