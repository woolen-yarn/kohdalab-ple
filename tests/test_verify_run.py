import csv
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("verify_run", ROOT / "scripts" / "verify_run.py")
assert SPEC and SPEC.loader
verify_run = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verify_run)


class VerifyRunTests(unittest.TestCase):
    def _write_csv(self, path: Path, fields: list[str], rows: list[dict[str, float | str]]) -> None:
        with path.open("w", newline="", encoding="utf-8") as handle:
            writer = csv.DictWriter(handle, fieldnames=fields)
            writer.writeheader()
            writer.writerows(rows)

    def _run_fixture(self, hardware: bool = False) -> Path:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        run = Path(temporary.name) / "sample_run"
        for name in ("raw", "processed", "metadata"):
            (run / name).mkdir(parents=True, exist_ok=True)
        settings = {"config": {"mode": "hardware" if hardware else "demo"}, "raw_spe_available": hardware}
        (run / "settings.json").write_text(json.dumps(settings))
        wavelength, bg, raw = [700.0, 701.0], [2.0, 3.0], [12.0, 23.0]
        self._write_csv(run / "raw" / "background.csv", ["wavelength_nm", "counts"],
                        [{"wavelength_nm": x, "counts": y} for x, y in zip(wavelength, bg)])
        (run / "metadata" / "background.json").write_text(json.dumps({"wavelength_nm": wavelength, "counts": bg}))
        base = "point_00000_repeat_000"
        self._write_csv(run / "raw" / f"{base}.csv", ["wavelength_nm", "counts"],
                        [{"wavelength_nm": x, "counts": y} for x, y in zip(wavelength, raw)])
        power = 2.0
        corrected = [x - y for x, y in zip(raw, bg)]
        processed = []
        for x, r, b, c in zip(wavelength, raw, bg, corrected):
            processed.append({"emission_nm": x, "emission_eV": verify_run.HC_EV_NM / x, "raw_counts": r,
                              "background_counts": b, "corrected_counts": c, "corrected_counts_per_W": c / power})
        fields = ["emission_nm", "emission_eV", "raw_counts", "background_counts", "corrected_counts", "corrected_counts_per_W"]
        self._write_csv(run / "processed" / f"{base}.csv", fields, processed)
        acquisition = {
            "point_index": 0,
            "repeat_index": 0,
            "actual_mm": 18.0,
            "power_w": power,
            "sample_power_w": power,
            "power_timestamp_unix_s": 10.0,
            "acquisition_started_unix_s": 11.0,
            "timestamp_unix_s": 12.0,
            "spectrum": {"wavelength_nm": wavelength, "counts": raw},
            "corrected_counts": corrected,
        }
        (run / "metadata" / f"{base}.json").write_text(json.dumps(acquisition))
        self._write_csv(run / "points.csv", ["point_index", "repeat_index", "actual_mm", "power_W", "sample_power_W", "spectrum_base", "power_timestamp_unix_s", "acquisition_started_unix_s", "acquisition_completed_unix_s"],
                        [{"point_index": 0, "repeat_index": 0, "actual_mm": 18.0, "power_W": power, "sample_power_W": power, "spectrum_base": base,
                          "power_timestamp_unix_s": 10.0, "acquisition_started_unix_s": 11.0, "acquisition_completed_unix_s": 12.0}])
        (run / "status.json").write_text(json.dumps({"state": "completed", "completed_acquisitions": 1, "planned_acquisitions": 1}))
        return run

    def test_malformed_background_metadata_returns_report(self) -> None:
        run = self._run_fixture()
        path = run / "metadata" / "background.json"
        path.write_text(json.dumps({"wavelength_nm": 700.0, "counts": [2.0, 3.0]}))
        report = verify_run.verify(run, 1e-8)
        self.assertFalse(report["ok"])
        self.assertTrue(any("arrays" in error for error in report["errors"]), report)

    def test_windows_drive_relative_base_is_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "basename"):
            verify_run.safe_spectrum_base("C:outside", ("0", "0"))

    def test_numeric_identity_alias_is_still_a_duplicate(self) -> None:
        run = self._run_fixture()
        path = run / "points.csv"
        with path.open(newline="", encoding="utf-8") as handle:
            rows = list(csv.DictReader(handle))
        alias = dict(rows[0]); alias["point_index"] = "00"
        self._write_csv(path, list(rows[0]), rows + [alias])
        report = verify_run.verify(run, 1e-8)
        self.assertFalse(report["ok"])
        self.assertTrue(any("duplicate" in error for error in report["errors"]), report)

    def test_valid_demo_fixture_passes(self) -> None:
        report = verify_run.verify(self._run_fixture(), 1e-8)
        self.assertTrue(report["ok"], report)
        self.assertEqual(report["verified_acquisitions"], 1)
        self.assertFalse(report["hardware_spe_required"])

    def test_processed_arithmetic_mutation_fails(self) -> None:
        run = self._run_fixture()
        path = run / "processed" / "point_00000_repeat_000.csv"
        with path.open(newline="", encoding="utf-8") as handle:
            rows = list(csv.DictReader(handle))
        rows[0]["corrected_counts"] = "999"
        self._write_csv(path, list(rows[0]), rows)
        report = verify_run.verify(run, 1e-8)
        self.assertFalse(report["ok"])
        self.assertTrue(any("BG subtraction" in error for error in report["errors"]), report)

    def test_small_power_uses_relative_arithmetic_without_absolute_watt_floor(self) -> None:
        run = self._run_fixture()
        base = "point_00000_repeat_000"
        power = 1e-12
        acquisition = json.loads((run / "metadata" / f"{base}.json").read_text())
        acquisition["power_w"] = power
        acquisition["sample_power_w"] = power
        (run / "metadata" / f"{base}.json").write_text(json.dumps(acquisition))
        with (run / "points.csv").open(newline="", encoding="utf-8") as handle:
            points = list(csv.DictReader(handle))
        points[0]["power_W"] = str(power)
        points[0]["sample_power_W"] = str(power)
        self._write_csv(run / "points.csv", list(points[0]), points)
        processed = run / "processed" / f"{base}.csv"
        with processed.open(newline="", encoding="utf-8") as handle:
            rows = list(csv.DictReader(handle))
        for row in rows:
            row["corrected_counts_per_W"] = str(float(row["corrected_counts"]) / power)
        self._write_csv(processed, list(rows[0]), rows)
        report = verify_run.verify(run, 1e-8)
        self.assertTrue(report["ok"], report)

    def test_epoch_timestamp_identity_rejects_ten_second_difference(self) -> None:
        run = self._run_fixture()
        with (run / "points.csv").open(newline="", encoding="utf-8") as handle:
            points = list(csv.DictReader(handle))
        points[0]["acquisition_completed_unix_s"] = "22.0"
        self._write_csv(run / "points.csv", list(points[0]), points)
        report = verify_run.verify(run, 1e-8)
        self.assertFalse(report["ok"])
        self.assertTrue(any("acquisition_completed_unix_s differs" in error for error in report["errors"]), report)

    def test_raw_power_observation_identity_rejects_two_picowatt_substitution(self) -> None:
        run = self._run_fixture()
        with (run / "points.csv").open(newline="", encoding="utf-8") as handle:
            points = list(csv.DictReader(handle))
        points[0]["power_W"] = "2e-12"
        self._write_csv(run / "points.csv", list(points[0]), points)
        report = verify_run.verify(run, 1e-8)
        self.assertFalse(report["ok"])
        self.assertTrue(any("metadata power_w differs" in error for error in report["errors"]), report)

    def test_hardware_fixture_requires_background_and_signal_spe(self) -> None:
        report = verify_run.verify(self._run_fixture(hardware=True), 1e-8)
        self.assertFalse(report["ok"])
        self.assertTrue(report["hardware_spe_required"])
        self.assertTrue(any(".spe" in error for error in report["errors"]), report)

    def test_point_metadata_and_path_traversal_are_rejected(self) -> None:
        run = self._run_fixture()
        points = run / "points.csv"
        with points.open(newline="", encoding="utf-8") as handle:
            rows = list(csv.DictReader(handle))
        rows[0]["spectrum_base"] = "../outside"
        self._write_csv(points, list(rows[0]), rows)
        report = verify_run.verify(run, 1e-8)
        self.assertFalse(report["ok"])
        self.assertTrue(any("basename" in error for error in report["errors"]), report)

    def test_completed_status_and_metadata_identity_must_match(self) -> None:
        run = self._run_fixture()
        metadata_path = run / "metadata" / "point_00000_repeat_000.json"
        metadata = json.loads(metadata_path.read_text())
        metadata["actual_mm"] = 19.0
        metadata_path.write_text(json.dumps(metadata))
        (run / "status.json").write_text(json.dumps({"state": "completed", "completed_acquisitions": 0, "planned_acquisitions": 2}))
        report = verify_run.verify(run, 1e-8)
        self.assertFalse(report["ok"])
        self.assertTrue(any("actual_mm differs" in error for error in report["errors"]), report)
        self.assertTrue(any("completed run status" in error for error in report["errors"]), report)

    def test_hardware_zero_byte_spe_is_rejected(self) -> None:
        run = self._run_fixture(hardware=True)
        (run / "raw" / "background.spe").touch()
        (run / "raw" / "point_00000_repeat_000.spe").touch()
        report = verify_run.verify(run, 1e-8)
        self.assertFalse(report["ok"])
        self.assertTrue(any("non-empty" in error for error in report["errors"]), report)


if __name__ == "__main__":
    unittest.main()
