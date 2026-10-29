#!/usr/bin/env python3
"""Collect archive hashes after all required release CI jobs succeeded."""
import argparse
import hashlib
import json
from pathlib import Path
import zipfile


def write_metadata(directory: Path, version: str, commit: str, run_url: str, *, require_shutter: bool = False) -> None:
    archives = [directory / f"KohdaLab-PLE-v{version}-{platform}.zip"
                for platform in ("source", "windows-x64", "macos-arm64")]
    hashes = {}
    for archive in archives:
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        platform = archive.stem.split(f"v{version}-", 1)[1]
        expected = (directory / f"SHA256SUMS-{platform}.txt").read_text().split()
        if expected != [digest, archive.name]:
            raise ValueError(f"checksum mismatch for {archive.name}")
        with zipfile.ZipFile(archive) as contents:
            if contents.testzip() is not None:
                raise ValueError(f"corrupt archive: {archive.name}")
        hashes[archive.name] = digest
    companion = None
    with zipfile.ZipFile(archives[1]) as windows:
        names = windows.namelist()
        metadata_name = next(n for n in names if n.endswith("/windows-metadata.json"))
        binary = next(n for n in names if n.endswith("/kohdalab-ple.exe"))
        metadata = json.loads(windows.read(metadata_name).decode("utf-8-sig"))
        if metadata["executable_sha256"] != hashlib.sha256(windows.read(binary)).hexdigest():
            raise ValueError("Windows binary hash differs from inspected binary")
        if metadata["external_vc_runtime_required"] is not False:
            raise ValueError("Unexpected external VC runtime requirement")
        companion_names = [n for n in names if n.endswith("/companions/kohdalab-shutter/companion-origin.json")]
        if require_shutter and len(companion_names) != 1:
            raise ValueError("Required shutter companion is missing")
        if companion_names:
            companion_name = companion_names[0]
            companion = json.loads(windows.read(companion_name).decode("utf-8-sig"))
            if companion.get("commit") != "8d22245899c36206622341ed5957919508aac0a1" or companion.get("protocol") != 2:
                raise ValueError("Shutter companion revision/protocol mismatch")
            prefix = companion_name.rsplit("/", 1)[0] + "/"
            for filename, key in (("kohdalab-shutter-gui.exe", "gui_sha256"), ("kohdalab-shutter.exe", "cli_sha256")):
                if hashlib.sha256(windows.read(prefix + filename)).hexdigest() != companion[key]:
                    raise ValueError("Shutter companion binary hash mismatch")
                inspection_name = "shutter-gui-metadata.json" if key == "gui_sha256" else "shutter-cli-metadata.json"
                inspection = json.loads(windows.read(prefix + inspection_name).decode("utf-8-sig"))
                if inspection.get("executable_sha256") != companion[key] or inspection.get("external_vc_runtime_required") is not False:
                    raise ValueError("Shutter binary inspection does not match companion")
            for required in ("source.zip", "LICENSE", "Start-Shutter.cmd", "third_party/rust/NOTICE.json", "shutter-gui-imports.txt", "shutter-cli-imports.txt", "shutter-gui-metadata.json", "shutter-cli-metadata.json"):
                windows.getinfo(prefix + required)
    origin = {"schema_version": 1, "version": version, "commit": commit,
              "workflow_url": run_url, "artifacts_sha256": hashes,
              "windows_executable_sha256": metadata["executable_sha256"], "shutter_companion": companion}
    validation = {"schema_version": 1, "version": version, "commit": commit,
                  "workflow_url": run_url, "software_checks": {
                      "rust_tests_fmt_clippy": "passed", "python_tests": "passed",
                      "windows_extracted_version_and_config": "passed",
                      "windows_import_inspection": "passed", "macos_extracted_app": "passed"},
                  "physical_hardware_tested": False, "native_gui_window_tested": False,
                  "vendor_sdk_included": False,
                  "required_next_step": "Follow START-HERE.md and the physical qualification checklist on the instrument PCs."}
    for name, value in (("BUILD-ORIGIN.json", origin), ("VALIDATION.json", validation)):
        (directory / name).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    checksums = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                 for p in directory.iterdir() if p.is_file() and not p.name.startswith("SHA256SUMS")}
    (directory / "SHA256SUMS.txt").write_text(
        "".join(f"{digest}  {name}\n" for name, digest in sorted(checksums.items())), encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--require-shutter", action="store_true")
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--run-url", required=True)
    args = parser.parse_args()
    write_metadata(args.directory, args.version, args.commit, args.run_url, require_shutter=args.require_shutter)
