#!/usr/bin/env python3
"""Create reproducible KohdaLab PLE release archives.

This script produces portable ZIPs and optional Windows Setup executables, and
makes no hardware qualification claim. Use ``--no-build`` when a CI job has
already created the platform stage.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import stat
import subprocess
import sys
import tomllib
import zipfile
from pathlib import Path
from typing import Iterable


ROOT = Path(__file__).resolve().parents[1]
PROJECT_NAME = "KohdaLab-PLE"
PLATFORMS = ("source", "windows-x64", "macos-arm64")

# Explicitly listing distributable roots makes accidental inclusion of output,
# editor state, virtual environments, and hardware data much less likely.
SOURCE_PATHS = (
    ".github",
    "bindings",
    "packaging",
    "bridge",
    "crates",
    "docs",
    "examples",
    "scripts",
    "servers",
    "tests",
    "third_party",
    "Cargo.lock",
    "Cargo.toml",
    ".gitattributes",
    ".gitignore",
    "LICENSE",
    "README.md",
    "START-HERE.md",
    "RELEASE-NOTES.md",
    "CHANGELOG.md",
    "CODE_SIGNING_POLICY.md",
    "CONTRIBUTING.md",
    "PRIVACY.md",
    "REDISTRIBUTION.md",
    "ROADMAP.md",
    "SAFETY.md",
    "SECURITY.md",
)
EXCLUDED_PARTS = {".git", "__pycache__", "target", "dist", "output", ".venv"}
EXCLUDED_NAMES = {".DS_Store"}
SOURCE_EXCLUDED_SUFFIXES = (".local.json", ".dll", ".spe")
SOURCE_EXCLUDED_NAMES = {".env"}


def project_version(root: Path = ROOT) -> str:
    """Read the workspace version without depending on Cargo at packaging time."""
    with (root / "Cargo.toml").open("rb") as handle:
        manifest = tomllib.load(handle)
    try:
        return str(manifest["workspace"]["package"]["version"])
    except KeyError as error:
        raise ValueError("Cargo.toml must define workspace.package.version") from error


def validate_tag(tag: str | None, version: str) -> None:
    """Reject an accidental release tag for a different source version."""
    if tag and tag != f"v{version}":
        raise ValueError(f"release tag {tag!r} does not match Cargo version v{version}")


def archive_members(root: Path, allowed: Iterable[str], *, source_archive: bool = False) -> list[Path]:
    """Return a sorted, safe list of source files under the allowlist."""
    files: list[Path] = []
    for relative in allowed:
        candidate = root / relative
        if not candidate.exists():
            continue
        if candidate.is_file() and not is_excluded(candidate, root, source_archive=source_archive):
            files.append(candidate)
            continue
        for item in candidate.rglob("*"):
            if item.is_file() and not is_excluded(item, root, source_archive=source_archive):
                files.append(item)
    return sorted(files, key=lambda item: item.relative_to(root).as_posix())


def is_excluded(path: Path, root: Path, *, source_archive: bool) -> bool:
    relative = path.relative_to(root)
    if any(part in EXCLUDED_PARTS for part in relative.parts):
        return True
    if path.name in EXCLUDED_NAMES or path.name.endswith((".pyc", ".pyo")):
        return True
    return source_archive and (
        path.name in SOURCE_EXCLUDED_NAMES or path.name.endswith(SOURCE_EXCLUDED_SUFFIXES)
    )


def zip_timestamp() -> tuple[int, int, int, int, int, int]:
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "315532800"))
    # ZIP cannot represent dates before 1980-01-01.
    epoch = max(epoch, 315532800)
    return tuple(__import__("time").gmtime(epoch)[:6])


def write_zip(destination: Path, files: Iterable[tuple[Path, str]]) -> None:
    """Write a deterministic deflated ZIP, preserving executable bits."""
    timestamp = zip_timestamp()
    with zipfile.ZipFile(destination, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for source, member in sorted(files, key=lambda entry: entry[1]):
            info = zipfile.ZipInfo(member, date_time=timestamp)
            info.create_system = 3
            info.compress_type = zipfile.ZIP_DEFLATED
            # ditto/Finder require the regular-file type as well as permission
            # bits; omitting S_IFREG silently extracts macOS executables as 644.
            info.external_attr = (stat.S_IFREG | stat.S_IMODE(source.stat().st_mode)) << 16
            archive.writestr(info, source.read_bytes())


def write_checksum(output: Path, artifact: Path, platform: str) -> Path:
    checksum = hashlib.sha256(artifact.read_bytes()).hexdigest()
    destination = output / f"SHA256SUMS-{platform}.txt"
    destination.write_text(f"{checksum}  {artifact.name}\n", encoding="utf-8", newline="\n")
    return destination


def run_stage_builder(platform: str, shutter_source: Path | None = None) -> None:
    if platform == "windows-x64":
        subprocess.run(
            ["powershell", "-ExecutionPolicy", "Bypass", "-File", "scripts/build-windows.ps1"],
            cwd=ROOT,
            check=True,
        )
        subprocess.run(
            ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "scripts/inspect-windows-binary.ps1", "-Directory", str(ROOT / "dist" / "kohdalab-ple-windows")],
            cwd=ROOT, check=True,
        )
        if shutter_source is not None:
            subprocess.run(
                ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "scripts/build-shutter-companion.ps1",
                 "-SourceDirectory", str(shutter_source.resolve()), "-Destination", str(ROOT / "dist/kohdalab-ple-windows/companions/kohdalab-shutter")],
                cwd=ROOT, check=True,
            )
    elif platform == "macos-arm64":
        subprocess.run(["sh", "scripts/package-macos.sh"], cwd=ROOT, check=True)


def platform_source(platform: str, portable_dir: Path | None, macos_app: Path | None) -> tuple[Path, str]:
    if platform == "windows-x64":
        source = portable_dir or ROOT / "dist" / "kohdalab-ple-windows"
        executable = source / "kohdalab-ple.exe"
        if not executable.is_file():
            raise ValueError(f"Windows portable stage is missing {executable}")
        return source, source.name
    if platform == "macos-arm64":
        source = macos_app or ROOT / "dist" / "KohdaLab PLE.app"
        executable = source / "Contents" / "MacOS" / "kohdalab-ple"
        if not executable.is_file():
            raise ValueError(f"macOS application stage is missing {executable}")
        return source, source.name
    raise ValueError(f"unsupported platform {platform!r}")


def package_source(output: Path, version: str) -> tuple[Path, Path]:
    artifact = output / f"{PROJECT_NAME}-v{version}-source.zip"
    prefix = f"kohdalab-ple-{version}"
    files = [
        (item, f"{prefix}/{item.relative_to(ROOT).as_posix()}")
        for item in archive_members(ROOT, SOURCE_PATHS, source_archive=True)
    ]
    if not any(member.endswith("Cargo.toml") for _, member in files):
        raise ValueError("source archive has no Cargo.toml")
    write_zip(artifact, files)
    return artifact, write_checksum(output, artifact, "source")


def package_platform(
    output: Path, version: str, platform: str, portable_dir: Path | None, macos_app: Path | None
) -> tuple[Path, Path]:
    stage, stage_name = platform_source(platform, portable_dir, macos_app)
    artifact = output / f"{PROJECT_NAME}-v{version}-{platform}.zip"
    prefix = f"{PROJECT_NAME}-v{version}-{platform}/{stage_name}"
    files = [(item, f"{prefix}/{item.relative_to(stage).as_posix()}") for item in archive_members(stage, ["."])]
    if not files:
        raise ValueError(f"portable stage {stage} has no files")
    write_zip(artifact, files)
    return artifact, write_checksum(output, artifact, platform)


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=PLATFORMS, required=True)
    parser.add_argument("--output", type=Path, required=True, help="directory for ZIP and checksum")
    parser.add_argument("--portable-dir", type=Path, help="pre-staged Windows portable directory")
    parser.add_argument("--macos-app", type=Path, help="pre-staged .app bundle")
    parser.add_argument("--tag", help="tag to validate, normally GITHUB_REF_NAME")
    parser.add_argument("--shutter-source", type=Path, help="pinned kohdalab-shutter Git checkout to include in Windows portable")
    parser.add_argument("--with-installer", action="store_true", help="also compile Windows Setup from the same prepared payload")
    parser.add_argument("--makensis", type=Path, help="explicit NSIS compiler for Windows Setup")
    parser.add_argument("--no-build", action="store_true", help="do not invoke the platform staging helper")
    return parser.parse_args()


def main() -> int:
    args = parse_arguments()
    if args.with_installer and args.platform != "windows-x64":
        raise ValueError("--with-installer requires --platform windows-x64")
    version = project_version()
    environment_tag = os.environ.get("GITHUB_REF_NAME") if os.environ.get("GITHUB_REF_TYPE") == "tag" else None
    validate_tag(args.tag if args.tag is not None else environment_tag, version)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)

    if args.platform == "source":
        artifact, checksum = package_source(output, version)
    else:
        if not args.no_build:
            run_stage_builder(args.platform, args.shutter_source)
        artifact, checksum = package_platform(
            output, version, args.platform, args.portable_dir, args.macos_app
        )
        if args.with_installer:
            stage, _ = platform_source(args.platform, args.portable_dir, args.macos_app)
            setup = output / f"{PROJECT_NAME}-{version}-Setup.exe"
            command = [sys.executable, str(ROOT / "scripts/build_windows_installer.py"),
                       "--payload-dir", str(stage), "--output", str(setup), "--version", version]
            if args.makensis:
                command += ["--makensis", str(args.makensis)]
            subprocess.run(command, check=True, cwd=ROOT)
            print(setup)
            print(write_checksum(output, setup, "windows-setup"))
    print(artifact)
    print(checksum)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"release packaging failed: {error}", file=sys.stderr)
        raise SystemExit(1)
