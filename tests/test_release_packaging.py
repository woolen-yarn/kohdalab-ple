import hashlib
import importlib.util
import os
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
