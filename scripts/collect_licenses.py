#!/usr/bin/env python3
"""Collect upstream license texts for the locked, target-filtered Rust graph.

This is a file-preservation utility, not a license policy engine.  It copies
only declared/license-named text files from source directories reported by
Cargo, never runs vendor SDK code, and reports missing texts rather than
silently treating an SPDX expression as a substitute for license text.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
from typing import Any

LICENSE_NAME = re.compile(r"^(?:license|copying|ofl|unlicense)(?:[._-].*)?$", re.IGNORECASE)


def _inside(path: Path, root: Path) -> Path | None:
    """Return a resolved relative path only when ``path`` cannot escape root."""
    try:
        return path.resolve().relative_to(root.resolve())
    except (OSError, ValueError):
        return None


def _safe_component(value: str) -> str:
    return re.sub(r"[^A-Za-z0-9._-]+", "_", value).strip("._") or "package"


def _metadata_from_cargo(cargo: str, target: str) -> dict[str, Any]:
    command = [cargo, "metadata", "--format-version", "1", "--locked", "--offline", "--filter-platform", target]
    completed = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if completed.returncode:
        raise RuntimeError(f"cargo metadata failed ({completed.returncode}): {completed.stderr.strip()}")
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"cargo metadata did not return JSON: {error}") from error
    if not isinstance(value, dict):
        raise RuntimeError("cargo metadata root is not an object")
    return value


def _license_candidates(package: dict[str, Any], workspace_root: Path) -> tuple[list[Path], list[str]]:
    manifest = package.get("manifest_path")
    if not isinstance(manifest, str):
        return [], ["manifest_path missing"]
    manifest_dir = Path(manifest).parent
    candidates: list[Path] = []
    problems: list[str] = []
    declared = package.get("license_file")
    if isinstance(declared, str) and declared:
        declared_path = Path(declared)
        candidate = declared_path if declared_path.is_absolute() else manifest_dir / declared_path
        if _inside(candidate, manifest_dir) is None:
            problems.append(f"license_file escapes manifest directory: {declared}")
        elif candidate.is_file():
            candidates.append(candidate)
        else:
            problems.append(f"declared license_file missing: {declared}")
    if manifest_dir.is_dir():
        for entry in manifest_dir.rglob("*"):
            if not entry.is_file() or not LICENSE_NAME.match(entry.name):
                continue
            if _inside(entry, manifest_dir) is not None:
                candidates.append(entry)
    # Workspace crates often designate only an SPDX expression.  Preserve the
    # repository MIT text without guessing a registry location.
    if package.get("source") is None and not candidates:
        fallback = workspace_root / "LICENSE"
        if fallback.is_file() and _inside(fallback, workspace_root) is not None:
            candidates.append(fallback)
        else:
            problems.append("workspace LICENSE fallback missing")
    unique: dict[Path, None] = {}
    for candidate in candidates:
        unique[candidate.resolve()] = None
    return list(unique), problems


def _copy_candidate(source: Path, source_root: Path, destination_root: Path) -> str | None:
    relative = _inside(source, source_root)
    if relative is None:
        return None
    destination = destination_root / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    return relative.as_posix()


def collect(metadata: dict[str, Any], output: Path) -> dict[str, Any]:
    """Collect text files and return a JSON-safe report.  ``output`` must not exist."""
    if output.exists():
        raise RuntimeError(f"refusing to overwrite existing output directory: {output}")
    packages = metadata.get("packages")
    workspace_root_text = metadata.get("workspace_root")
    if not isinstance(packages, list) or not isinstance(workspace_root_text, str):
        raise RuntimeError("metadata needs packages[] and workspace_root")
    workspace_root = Path(workspace_root_text)
    output.mkdir(parents=True)
    package_reports: list[dict[str, Any]] = []
    missing_registry: list[dict[str, Any]] = []
    for package in packages:
        if not isinstance(package, dict):
            continue
        name, version = package.get("name"), package.get("version")
        if not isinstance(name, str) or not isinstance(version, str):
            continue
        manifest = package.get("manifest_path")
        source_root = Path(manifest).parent if isinstance(manifest, str) else workspace_root
        package_id = str(package.get("id", f"{name} {version}"))
        suffix = hashlib.sha256(package_id.encode("utf-8")).hexdigest()[:12]
        package_root = output / f"{_safe_component(name)}-{_safe_component(version)}-{suffix}"
        candidates, problems = _license_candidates(package, workspace_root)
        supplemental_root = workspace_root / "third_party" / "rust-upstream"
        provenance_file = supplemental_root / "provenance.json"
        if not candidates and provenance_file.is_file():
            for entry in json.loads(provenance_file.read_text(encoding="utf-8")):
                if entry.get("package") != name or entry.get("version") != version:
                    continue
                candidate = supplemental_root / entry["file"]
                if _inside(candidate, supplemental_root) is None:
                    raise RuntimeError("supplemental license escapes provenance directory")
                if hashlib.sha256(candidate.read_bytes()).hexdigest() != entry["sha256"]:
                    raise RuntimeError(f"supplemental license hash mismatch: {candidate}")
                candidates.append(candidate)
        copied: list[str] = []
        for candidate in candidates:
            # A workspace fallback lives outside a member manifest, but is
            # copied from workspace root with an explicit stable prefix.
            root = workspace_root if _inside(candidate, source_root) is None else source_root
            relative = _copy_candidate(candidate, root, package_root)
            if relative is None:
                problems.append(f"candidate escapes source root: {candidate}")
            else:
                copied.append(relative)
        report = {
            "name": name, "version": version, "package_id": package_id,
            "source": package.get("source"), "spdx_expression": package.get("license"),
            "manifest_path": manifest, "copied_license_texts": sorted(copied), "problems": problems,
        }
        (package_root / "package.json").parent.mkdir(parents=True, exist_ok=True)
        (package_root / "package.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        package_reports.append(report)
        if package.get("source") is not None and not copied:
            missing_registry.append(report)
    result = {
        "tool": "kohdalab-ple collect_licenses", "purpose": "preserve upstream copyright/license texts; not a legal review",
        "package_count": len(package_reports), "packages": package_reports,
        "missing_registry_license_texts": missing_registry,
        "ok": not missing_registry,
    }
    (output / "NOTICE.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path, help="new directory for copied license texts")
    parser.add_argument("--target", default="x86_64-pc-windows-gnu", help="Cargo target platform")
    parser.add_argument("--cargo", default="cargo", help="explicit cargo executable")
    parser.add_argument("--metadata", type=Path, help="saved cargo metadata JSON; avoids running cargo")
    args = parser.parse_args(argv)
    try:
        metadata = json.loads(args.metadata.read_text(encoding="utf-8")) if args.metadata else _metadata_from_cargo(args.cargo, args.target)
        if not isinstance(metadata, dict):
            raise RuntimeError("metadata JSON root is not an object")
        report = collect(metadata, args.output)
    except (OSError, RuntimeError, json.JSONDecodeError) as error:
        print(f"collect_licenses: {error}", file=sys.stderr)
        return 2
    print(json.dumps({"output": str(args.output), "ok": report["ok"], "missing_registry_license_texts": len(report["missing_registry_license_texts"])}, sort_keys=True))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
