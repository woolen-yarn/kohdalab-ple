import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("collect_licenses", ROOT / "scripts" / "collect_licenses.py")
assert SPEC and SPEC.loader
collector = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(collector)


class CollectLicensesTests(unittest.TestCase):
    def _metadata(self, workspace: Path, package: dict) -> dict:
        return {"workspace_root": str(workspace), "packages": [package]}

    def test_copies_declared_and_named_license_texts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            crate = root / "registry-crate"
            crate.mkdir()
            (crate / "Cargo.toml").write_text("[package]\nname='demo'\n")
            (crate / "COPYING.extra").write_text("copying text")
            (crate / "licenses").mkdir()
            (crate / "licenses" / "MIT.txt").write_text("MIT text")
            package = {"id": "demo 1.0.0", "name": "demo", "version": "1.0.0", "source": "registry+https://example",
                       "license": "MIT", "license_file": "licenses/MIT.txt", "manifest_path": str(crate / "Cargo.toml")}
            report = collector.collect(self._metadata(root, package), root / "out")
            copied = report["packages"][0]["copied_license_texts"]
            self.assertIn("licenses/MIT.txt", copied)
            self.assertIn("COPYING.extra", copied)
            self.assertTrue(report["ok"])

    def test_rejects_license_file_path_escape_and_records_missing_registry_text(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            crate = root / "registry-crate"
            crate.mkdir()
            (crate / "Cargo.toml").touch()
            (root / "outside.txt").write_text("do not copy")
            package = {"id": "bad 1.0.0", "name": "bad", "version": "1.0.0", "source": "registry+https://example",
                       "license": "MIT", "license_file": "../outside.txt", "manifest_path": str(crate / "Cargo.toml")}
            report = collector.collect(self._metadata(root, package), root / "out")
            item = report["packages"][0]
            self.assertFalse(report["ok"])
            self.assertFalse(item["copied_license_texts"])
            self.assertTrue(any("escapes" in text for text in item["problems"]))
            self.assertEqual(len(report["missing_registry_license_texts"]), 1)

    def test_supplemental_license_requires_matching_package_and_hash(self) -> None:
        import hashlib
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            crate = root / "registry-crate"
            crate.mkdir()
            (crate / "Cargo.toml").touch()
            supplemental = root / "third_party" / "rust-upstream"
            supplemental.mkdir(parents=True)
            license_text = supplemental / "LICENSE-MIT"
            license_text.write_text("preserved upstream text")
            (supplemental / "provenance.json").write_text(json.dumps([{
                "package": "demo", "version": "1.0.0", "file": "LICENSE-MIT",
                "sha256": hashlib.sha256(license_text.read_bytes()).hexdigest(),
            }]))
            package = {"id": "demo 1.0.0", "name": "demo", "version": "1.0.0", "source": "registry+https://example",
                       "license": "MIT", "manifest_path": str(crate / "Cargo.toml")}
            self.assertTrue(collector.collect(self._metadata(root, package), root / "out")["ok"])
            license_text.write_text("changed")
            with self.assertRaisesRegex(RuntimeError, "hash mismatch"):
                collector.collect(self._metadata(root, package), root / "changed-out")

    def test_missing_registry_license_is_reported_in_notice(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            crate = root / "registry-crate"
            crate.mkdir()
            (crate / "Cargo.toml").touch()
            package = {"id": "empty 1.0.0", "name": "empty", "version": "1.0.0", "source": "registry+https://example",
                       "license": "MIT", "manifest_path": str(crate / "Cargo.toml")}
            report = collector.collect(self._metadata(root, package), root / "out")
            notice = json.loads((root / "out" / "NOTICE.json").read_text())
            self.assertFalse(report["ok"])
            self.assertEqual(notice["missing_registry_license_texts"][0]["name"], "empty")


if __name__ == "__main__":
    unittest.main()
