#!/usr/bin/env python3
"""Build the suite-standard machine-wide NSIS Setup from the inspected PLE payload.

NSIS 3.12 is downloaded to build/tools only when no compiler is supplied.
The archive is accepted only with the same pinned SHA-256 used by IV.
No system PATH, manufacturer software, device settings or firmware is changed.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath, PureWindowsPath
import re
import stat
import subprocess
from urllib.request import urlretrieve
import zipfile

ROOT = Path(__file__).resolve().parents[1]
NSIS_SHA256 = "56581f90db321581c5381193d796fffcf2d24b2f8fed2160a6c6a3baa67f2c4f"
NSIS_URLS = (
    "https://prdownloads.sourceforge.net/nsis/nsis-3.12.zip",
    "https://mirrors.mit.edu/macports/distfiles/nsis/nsis-3.12.zip",
)
REQUIRED = ("kohdalab-ple.exe", "START-HERE.md", "LICENSE", "bridge/ple_bridge.py",
            "examples/ple-settings.json", "bindings/python/kohdalab_ple/__init__.py",
            "companions/kohdalab-shutter/kohdalab-shutter-gui.exe",
            "companions/kohdalab-shutter/companion-origin.json")


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def version_words(version: str) -> str:
    if not re.fullmatch(r"(?:0|[1-9][0-9]{0,4})\.(?:0|[1-9][0-9]{0,4})\.(?:0|[1-9][0-9]{0,4})", version):
        raise ValueError("installer version must be numeric MAJOR.MINOR.PATCH")
    if any(int(v) > 65535 for v in version.split(".")):
        raise ValueError("Windows version component exceeds 65535")
    return version + ".0"


def quoted_path(path: Path) -> str:
    value = str(path.resolve())
    if any(c in value for c in ('"', '\r', '\n')):
        raise ValueError("unsafe NSIS source/output path")
    return value.replace("$", "$$").replace("/", "\\")


def relative_path(value: str) -> str:
    path = PurePosixPath(value)
    windows = PureWindowsPath(value)
    if (not value or path.is_absolute() or windows.drive or "\\" in value
            or any(p in ("", ".", "..") for p in value.split("/"))
            or any(c in value for c in '<>:"|?*')
            or any(ord(c) < 32 for c in value)
            or any(p.endswith((".", " ")) or PureWindowsPath(p).is_reserved() for p in path.parts)):
        raise ValueError(f"unsafe Windows payload path: {value!r}")
    return value.replace("$", "$$").replace("/", "\\")


def payload_files(payload: Path) -> dict[str, str]:
    if not payload.is_dir() or payload.is_symlink():
        raise ValueError("prepared portable directory is required")
    files = {}
    folded = set()
    for item in sorted(payload.rglob("*")):
        if item.is_symlink() or (hasattr(item, "is_junction") and item.is_junction()):
            raise ValueError(f"payload contains a link: {item}")
        if not item.resolve().is_relative_to(payload.resolve()):
            raise ValueError("payload path escapes its root")
        name = item.relative_to(payload).as_posix()
        relative_path(name)
        if name.casefold() in folded:
            raise ValueError(f"case-insensitive Windows path collision: {name}")
        folded.add(name.casefold())
        if item.is_file():
            files[name] = digest(item)
    if any(name not in files for name in REQUIRED):
        raise ValueError("prepared portable payload is incomplete")
    if any(name.casefold() == "uninstall.exe" for name in files):
        raise ValueError("payload must not supply its own uninstaller")
    return files


def payload_digest(files: dict[str, str]) -> str:
    return hashlib.sha256(json.dumps(files, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def payload_include(payload: Path) -> str:
    files = payload_files(payload)
    copies = []
    directories = set()
    previous = None
    for name in files:
        parent = PurePosixPath(name).parent
        directory = "" if str(parent) == "." else "\\" + relative_path(str(parent))
        if directory != previous:
            copies.append(f'  SetOutPath "$INSTDIR{directory}"')
            previous = directory
        copies.append(f'  File "{quoted_path(payload / name)}"')
        directories.update(p for p in PurePosixPath(name).parents if str(p) != ".")
    deletes = [f'  Delete "$INSTDIR\\{relative_path(name)}"' for name in files]
    deletes += [f'  RMDir "$INSTDIR\\{relative_path(str(p))}"'
                for p in sorted(directories, key=lambda p: (len(p.parts), str(p)), reverse=True)]
    return ("!macro InstallPayload\n" + "\n".join(copies) + "\n!macroend\n"
            + "!macro RemovePayload\n" + "\n".join(deletes) + "\n!macroend\n")


def nsis_source(payload: Path, output: Path, version: str) -> str:
    words = version_words(version)
    template = (ROOT / "packaging/windows/KohdaLab-PLE.nsi").read_text()
    template = template.replace('!include "${PAYLOAD_INCLUDE}"', payload_include(payload))
    return (f'!define PAYLOAD_INCLUDE "inline"\n!define OUTPUT_FILE "{quoted_path(output)}"\n'
            f'!define VERSION "{version}"\n!define VERSION_FILE "{words}"\n' + template)


def pinned_compiler() -> Path:
    tools = ROOT / "build/tools"
    archive = tools / "nsis-3.12.zip"
    tools.mkdir(parents=True, exist_ok=True)
    if not archive.is_file() or digest(archive) != NSIS_SHA256:
        errors = []
        for url in NSIS_URLS:
            try:
                urlretrieve(url, archive)
                if digest(archive) == NSIS_SHA256:
                    break
                errors.append(f"{url}: hash mismatch")
            except OSError as error:
                errors.append(f"{url}: {error}")
            archive.unlink(missing_ok=True)
        else:
            raise RuntimeError("Could not download pinned NSIS: " + "; ".join(errors))
    # Re-extract the hash-verified compiler instead of trusting cached tools.
    with zipfile.ZipFile(archive) as bundle:
        for member in bundle.infolist():
            name = member.filename.rstrip("/")
            relative_path(name)
            if not member.filename.startswith("nsis-3.12/") or stat.S_IFMT(member.external_attr >> 16) == stat.S_IFLNK:
                raise ValueError("unexpected NSIS archive layout/link")
        bundle.extractall(tools)
    result = tools / "nsis-3.12/Bin/makensis.exe"
    if not result.is_file():
        raise RuntimeError("NSIS compiler archive is incomplete")
    return result


def build(payload: Path, output: Path, version: str, makensis: Path | None = None, *, source_only: bool = False) -> Path:
    version_words(version)
    payload = payload.resolve()
    output = output.resolve()
    if output.is_relative_to(payload) or output.exists():
        raise ValueError("installer output must be new and outside the payload")
    files = payload_files(payload)
    script = ROOT / "build/windows-installer" / f"KohdaLab-PLE-{version}-Setup.nsi"
    script.parent.mkdir(parents=True, exist_ok=True)
    script.write_text(nsis_source(payload, output, version), encoding="utf-8")
    if source_only:
        return script
    tool = makensis or pinned_compiler()
    if not tool.is_file():
        raise ValueError("NSIS compiler does not exist")
    output.parent.mkdir(parents=True, exist_ok=True)
    verbosity = "/V3" if os.name == "nt" else "-V3"
    subprocess.run([str(tool.resolve()), verbosity, str(script)], check=True)
    if not output.is_file() or output.stat().st_size == 0:
        raise RuntimeError("NSIS did not produce Setup.exe")
    metadata = {"version": version, "setup_sha256": digest(output), "payload_sha256": payload_digest(files),
                "payload_files": len(files), "application_sha256": files["kohdalab-ple.exe"],
                "per_user": False, "physical_hardware_tested": False}
    (output.parent / "windows-installer-metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    return output


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--payload-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--makensis", type=Path)
    parser.add_argument("--source-only", action="store_true")
    args = parser.parse_args()
    print(build(args.payload_dir, args.output, args.version, args.makensis, source_only=args.source_only))
