"""Check normal GUI startup in an isolated user home, without acquiring devices."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bindings/control"))
from kohdalab_control import ControlClient, discover


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("application", choices=("iv", "trkr", "camera", "shutter", "ple"))
    parser.add_argument("binary", type=Path)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    results = []
    with tempfile.TemporaryDirectory(prefix="kohdalab-user-layout-") as temporary:
        home = Path(temporary)
        app = home / ".kohdalab" / args.application
        config = app / "config/config.json"
        run = app / "run"
        app.mkdir(parents=True)
        legacy = app / "config.json"
        legacy_bytes = b'{"operator":"preserve old direct-child configuration"}'
        legacy.write_bytes(legacy_bytes)
        old_appdata = home / "AppData/Local/KohdaLab" / args.application / "config/default.json"
        old_appdata.parent.mkdir(parents=True)
        old_appdata.write_bytes(legacy_bytes)
        env = dict(os.environ, HOME=str(home), USERPROFILE=str(home), LOCALAPPDATA=str(home / "AppData/Local"))
        first = None
        for case in ("new", "restart", "invalid"):
            if case == "invalid":
                config.write_bytes(b"{ invalid operator profile")
            before = config.read_bytes() if config.exists() else None
            with args.report.with_name(args.report.stem + "-" + case + ".log").open("w") as log:
                process = subprocess.Popen([str(binary), "--gui-api"], env=env, stdout=log, stderr=log)
                started = time.monotonic()
                try:
                    while time.monotonic() - started < 35:
                        if process.poll() is not None:
                            raise AssertionError(f"{case}: GUI exited ({process.returncode})")
                        if config.is_file() and time.monotonic() - started > 2:
                            endpoints = discover("kohdalab-" + args.application, directory=run, probe_timeout=.5)
                            if endpoints:
                                status = ControlClient(endpoint=endpoints[0]).status()
                                assert not status["busy"], status
                                if args.application == "shutter":
                                    assert status["owner"] is None, status
                                    assert not status["connected"], status
                                else:
                                    assert not status["ownership"]["owned"], status
                                break
                        time.sleep(.15)
                    else:
                        raise AssertionError(f"{case}: canonical config/run not ready")
                    data = config.read_bytes()
                    if case == "new":
                        value = json.loads(data)
                        assert value["application"] == "kohdalab-" + args.application
                        if args.application in ("iv", "ple"):
                            assert Path(value["output_dir"]) == app / "results", value["output_dir"]
                        first = data
                    elif case == "restart":
                        assert data == first == before
                    else:
                        assert data == before
                    assert legacy.read_bytes() == legacy_bytes
                    assert old_appdata.read_bytes() == legacy_bytes
                    assert not (home / ".kohdalab/control").exists()
                    assert not (home / ".kohdalab/run").exists()
                    results.append({"case": case, "config_sha256": hashlib.sha256(data).hexdigest(), "owned": False, "busy": False})
                finally:
                    process.terminate()
                    try:
                        process.wait(timeout=8)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=8)
    args.report.write_text(json.dumps({"application": args.application, "binary": str(binary), "layout": f".kohdalab/{args.application}/config/config.json", "runtime": f".kohdalab/{args.application}/run", "cases": results, "legacy_profiles": "preserved byte-for-byte; not imported", "device_commands": "none; describe/status only"}, indent=2) + "\n")
    print(f"{args.application}: actual normal GUI config/run, restart and invalid-profile checks passed")


if __name__ == "__main__":
    main()
