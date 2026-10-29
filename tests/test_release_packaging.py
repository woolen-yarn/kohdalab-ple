import hashlib
import json
import importlib.util
import os
import stat
import shutil
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("build_release", ROOT / "scripts" / "build_release.py")
assert SPEC and SPEC.loader
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ReleasePackagingTests(unittest.TestCase):
    def test_release_evidence_checks_archive_and_binary_hashes(self):
        spec = importlib.util.spec_from_file_location("release_metadata", ROOT / "scripts/release_metadata.py")
        metadata = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(metadata)
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            binary = b"test-only-binary"
            for platform in ("source", "windows-x64", "macos-arm64"):
                archive = output / f"KohdaLab-PLE-v1.2.3-{platform}.zip"
                with zipfile.ZipFile(archive, "w") as contents:
                    contents.writestr("package/README.md", "fixture")
                    if platform == "windows-x64":
                        contents.writestr("package/kohdalab-ple.exe", binary)
                        contents.writestr("package/windows-metadata.json", json.dumps({"executable_sha256":hashlib.sha256(binary).hexdigest(),"external_vc_runtime_required":False}))
                release.write_checksum(output, archive, platform)
            metadata.write_metadata(output,"1.2.3","fixture-commit","https://example.test/run")
            validation = json.loads((output / "VALIDATION.json").read_text())
            self.assertFalse(validation["physical_hardware_tested"])
            self.assertFalse(validation["native_gui_window_tested"])
            self.assertEqual(json.loads((output / "BUILD-ORIGIN.json").read_text())["commit"],"fixture-commit")
            (output / "SHA256SUMS-source.txt").write_text("wrong checksum")
            with self.assertRaisesRegex(ValueError,"checksum mismatch"):
                metadata.write_metadata(output,"1.2.3","fixture-commit","https://example.test/run")

    def test_tag_must_match_workspace_version(self):
        release.validate_tag("v1.2.3", "1.2.3")
        with self.assertRaisesRegex(ValueError, "does not match"):
            release.validate_tag("v1.2.4", "1.2.3")

    def test_source_allowlist_excludes_build_and_cache_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
            (root / "bridge").mkdir()
            (root / "bridge" / "ple_bridge.py").write_text("ok\n", encoding="utf-8")
            (root / "bridge" / ".env").write_text("secret\n", encoding="utf-8")
            (root / "bridge" / "endpoint.local.json").write_text("secret\n", encoding="utf-8")
            (root / "bridge" / "vendor.dll").write_bytes(b"vendor")
            (root / "servers").mkdir()
            (root / "servers" / "README.md").write_text("future server\n", encoding="utf-8")
            (root / "target").mkdir()
            (root / "target" / "secret.bin").write_bytes(b"not source")
            (root / "docs").mkdir()
            (root / "docs" / "__pycache__").mkdir()
            (root / "docs" / "__pycache__" / "cache.pyc").write_bytes(b"cache")

            files = release.archive_members(
                root,
                ("Cargo.toml", "bridge", "servers", "target", "docs"),
                source_archive=True,
            )
            names = [item.relative_to(root).as_posix() for item in files]

            self.assertEqual(names, ["Cargo.toml", "bridge/ple_bridge.py", "servers/README.md"])

    def test_zip_preserves_regular_file_and_executable_mode_after_extraction(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            executable = root / "fixture"
            executable.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            executable.chmod(0o755)
            archive = root / "fixture.zip"
            release.write_zip(archive, [(executable, "Example.app/Contents/MacOS/fixture")])
            with zipfile.ZipFile(archive) as contents:
                item = contents.getinfo("Example.app/Contents/MacOS/fixture")
                self.assertEqual(item.create_system, 3)
                self.assertEqual(stat.S_IFMT(item.external_attr >> 16), stat.S_IFREG)
                self.assertEqual(stat.S_IMODE(item.external_attr >> 16), stat.S_IMODE(executable.stat().st_mode))
            if os.name == "posix" and shutil.which("ditto"):
                destination = root / "extracted"
                subprocess.run(["ditto", "-x", "-k", str(archive), str(destination)], check=True, capture_output=True)
                self.assertEqual(stat.S_IMODE((destination / "Example.app/Contents/MacOS/fixture").stat().st_mode), 0o755)

    def test_source_manifest_requires_reproducibility_and_policy_material(self):
        manifest = set(release.SOURCE_PATHS)
        self.assertTrue(
            {
                "Cargo.toml",
                "Cargo.lock",
                "README.md",
                "LICENSE",
                "bridge",
                "servers",
                "docs",
                "examples",
                "third_party",
                "CHANGELOG.md",
                "CODE_SIGNING_POLICY.md",
                "CONTRIBUTING.md",
                "PRIVACY.md",
                "REDISTRIBUTION.md",
                "ROADMAP.md",
                "SAFETY.md",
                "SECURITY.md",
            }.issubset(manifest)
        )

    def test_source_zip_is_deterministic_and_has_checksum(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary) / "workspace"
            output = Path(temporary) / "out"
            workspace.mkdir()
            (workspace / "Cargo.toml").write_text(
                "[workspace]\n[workspace.package]\nversion = '1.2.3'\n", encoding="utf-8"
            )
            (workspace / "README.md").write_text("readme\n", encoding="utf-8")
            (workspace / "LICENSE").write_text("MIT\n", encoding="utf-8")
            (workspace / "servers").mkdir()
            (workspace / "servers" / "README.md").write_text("reserved\n", encoding="utf-8")

            original_root = release.ROOT
            try:
                release.ROOT = workspace
                output.mkdir()
                old_epoch = os.environ.get("SOURCE_DATE_EPOCH")
                os.environ["SOURCE_DATE_EPOCH"] = "315532800"
                artifact, checksum = release.package_source(output, "1.2.3")
                first = artifact.read_bytes()
                self.assertTrue(any(name.endswith("servers/README.md") for name in zipfile.ZipFile(artifact).namelist()))
                self.assertEqual(checksum.read_text(encoding="utf-8"), f"{hashlib.sha256(first).hexdigest()}  {artifact.name}\n")
                artifact.unlink()
                artifact, _ = release.package_source(output, "1.2.3")
                self.assertEqual(first, artifact.read_bytes())
            finally:
                release.ROOT = original_root
                if old_epoch is None:
                    os.environ.pop("SOURCE_DATE_EPOCH", None)
                else:
                    os.environ["SOURCE_DATE_EPOCH"] = old_epoch


if __name__ == "__main__":
    unittest.main()
