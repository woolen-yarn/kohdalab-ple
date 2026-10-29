import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch
import zipfile

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("windows_installer", ROOT / "scripts/build_windows_installer.py")
installer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(installer)


class WindowsInstallerTests(unittest.TestCase):
    def payload(self, root):
        payload = root / "portable payload"
        for name in installer.REQUIRED + ("docs/reference/guide.md",):
            path = payload / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture")
        return payload

    def test_source_installs_complete_payload_for_all_users_and_preserves_data(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            payload = self.payload(root)
            source = installer.nsis_source(payload, root / "Setup.exe", "1.2.3")
            self.assertIn('RequestExecutionLevel admin', source)
            self.assertIn('!define VERSION_FILE "1.2.3.0"', source)
            self.assertIn('StrCpy $INSTDIR "$PROGRAMFILES64\\Kohdalab\\KohdaLab PLE"', source)
            self.assertIn("HKLM", source)
            self.assertNotIn("rmdir /r", source.lower())
            self.assertNotIn("File /r", source)
            self.assertIn('SetOutPath "$INSTDIR"', source)
            self.assertIn('CreateShortcut "$SMPROGRAMS\\KohdaLab\\PLE\\Shutter (automatic BG).lnk"', source)
            self.assertIn('"--gui-api"', source)
            self.assertIn('Delete "$INSTDIR\\docs\\reference\\guide.md"', source)
            self.assertIn('RMDir "$INSTDIR\\docs"', source)
            self.assertNotIn('Delete "$DOCUMENTS', source)
            self.assertIn('MUI_LANGUAGE "Japanese"', source)

    def test_rejects_version_and_windows_path_injection(self):
        for version in ('1.2.3"\n!system calc', "65536.0.0", "1.2", "01.2.3", "1.2.3.4"):
            with self.assertRaises(ValueError): installer.version_words(version)
        for name in ("../outside", "C:/outside", "/outside", "foo\\bar", "foo/CON.txt", "foo/end.", "foo/bar:stream", "foo\nbar", "foo//bar"):
            with self.assertRaises(ValueError, msg=name): installer.relative_path(name)
        self.assertEqual(installer.relative_path("foo/$operator.txt"), "foo\\$$operator.txt")

    def test_rejects_incomplete_payload_symlink_and_output_overwrite(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            payload = self.payload(root)
            output = root / "Setup.exe"
            output.touch()
            with self.assertRaises(ValueError): installer.build(payload, output, "1.2.3", source_only=True)
            with self.assertRaises(ValueError): installer.build(payload, payload / "Setup.exe", "1.2.3", source_only=True)
            (payload / "kohdalab-ple.exe").unlink()
            with self.assertRaisesRegex(ValueError, "incomplete"): installer.payload_files(payload)
            (payload / "kohdalab-ple.exe").write_text("fixture")
            try:
                (payload / "outside-link").symlink_to(root, target_is_directory=True)
            except OSError:
                return  # Windows runners may not grant link creation.
            with self.assertRaisesRegex(ValueError, "link"): installer.payload_files(payload)

    def test_compiler_cache_is_rebuilt_from_hash_verified_archive(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            archive = root / "fixture.zip"
            with zipfile.ZipFile(archive, "w") as z:
                z.writestr("nsis-3.12/", "")
                z.writestr("nsis-3.12/Bin/makensis.exe", b"verified compiler fixture")
            checksum = installer.digest(archive)
            def download(_url, path): shutil.copyfile(archive, path)
            with patch.object(installer, "ROOT", root), patch.object(installer, "NSIS_SHA256", checksum), patch.object(installer, "urlretrieve", side_effect=download):
                compiler = installer.pinned_compiler()
                compiler.write_bytes(b"modified cache")
                self.assertEqual(installer.pinned_compiler().read_bytes(), b"verified compiler fixture")

    def test_nsis_license_bytes_match_pinned_provenance(self):
        folder = ROOT / "third_party/nsis-3.12"
        provenance = json.loads((folder / "provenance.json").read_text())
        self.assertEqual(hashlib.sha256((folder / "COPYING").read_bytes()).hexdigest(), provenance["copying_sha256"])
        self.assertEqual(provenance["archive_sha256"], installer.NSIS_SHA256)


if __name__ == "__main__":
    unittest.main()
