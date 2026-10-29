"""Run the real PLE GUI through the shared ControlClient in Demo mode."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bindings" / "control"))
from kohdalab_control import ControlClient


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("executable", type=Path)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    profile = json.loads((ROOT / "examples/ple-settings.json").read_text())
    profile.update(
        mode="demo",
        sample="common_control_demo",
        output_dir=str((a.output / "runs").resolve()),
    )
    profile["scan"].update(stop_mm=18.04, step_mm=0.02, repeats=1, settle_s=0.0)
    with tempfile.TemporaryDirectory(prefix="kohdalab-ple-control-") as root:
        endpoint_dir = Path(root) / "endpoints"
        endpoint_dir.mkdir()
        config = Path(root) / "profile.json"
        config.write_text(json.dumps(profile))
        log = (a.output / "gui.log").open("w")
        process = subprocess.Popen(
            [
                str(a.executable.resolve()),
                "--api-dir",
                str(endpoint_dir),
                "--config",
                str(config),
            ],
            stdout=log,
            stderr=subprocess.STDOUT,
        )
        try:
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline and not list(endpoint_dir.glob("*.json")):
                time.sleep(0.05)
            client = ControlClient("kohdalab-ple", directory=endpoint_dir)
            client.describe()
            client.acquire()
            client.invoke("configure", config=profile)
            connected = client.invoke(
                "connect", devices=["spectrometer", "stage", "power"]
            )
            client.wait(connected["job_id"], timeout=10)
            run = client.invoke("start")
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                awaiting = client.status().get("awaiting")
                if awaiting == "background_confirmation":
                    client.invoke("confirm_background", confirmed=True)
                elif awaiting == "signal_confirmation":
                    client.invoke("confirm_signal", confirmed=True)
                elif awaiting == "analysis_selection":
                    client.invoke("preview")
                    analysis = {
                        "peak_nm": 780.0,
                        "peak_half_width_nm": 15.0,
                        "integral_min_nm": 765.0,
                        "integral_max_nm": 795.0,
                    }
                    client.invoke("select_analysis", analysis=analysis)
                    break
                elif awaiting == "none":
                    time.sleep(0.05)
            result = client.wait(run["job_id"], timeout=30)
            released = client.release()
            (a.output / "evidence.json").write_text(
                json.dumps(
                    {
                        "describe": True,
                        "configure": "demo",
                        "connect": connected,
                        "start": run,
                        "result": result,
                        "release": released,
                    },
                    indent=2,
                )
                + "\n"
            )
            print(
                json.dumps(
                    {"ok": True, "job_id": run["job_id"], "status": result["status"]}
                )
            )
        finally:
            process.terminate()
            process.wait(timeout=10)
            log.close()


if __name__ == "__main__":
    main()
