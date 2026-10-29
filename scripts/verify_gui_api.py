"""Exercise the real Rust GUI API using only Demo instruments.

Usage: python scripts/verify_gui_api.py target/release/kohdalab-ple --output-dir PATH
Creates exactly one GUI process and records its PID/log/evidence. Never uses hardware.
"""
from __future__ import annotations
import argparse
import csv
import json
import math
import subprocess
import sys
import time
import uuid
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/"bindings/python"))
from kohdalab_ple import GuiPLE,GuiPLEError

def until(predicate,timeout=10):
    deadline=time.monotonic()+timeout
    while time.monotonic()<deadline:
        value=predicate()
        if value:return value
        time.sleep(.05)
    raise AssertionError("Timed out awaiting GUI state")

def main():
    parser=argparse.ArgumentParser();parser.add_argument("executable",type=Path);parser.add_argument("--output-dir",type=Path,required=True);args=parser.parse_args()
    output=args.output_dir.resolve();output.mkdir(parents=True,exist_ok=True);endpoint_dir=output/"endpoints";endpoint_dir.mkdir(exist_ok=True)
    profile=json.loads((ROOT/"examples/ple-settings.json").read_text());profile["mode"]="demo";profile["sample"]="api_validation";profile["output_dir"]=str(output/"runs");profile["scan"].update(stop_mm=18.04,step_mm=.02,repeats=2,settle_s=0.)
    profile["power"].update(wavelength_mode="position_calibration",wavelength_tolerance_nm=1.0);profile["background"]["mode"]="manual";profile_path=output/"profile.json";profile_path.write_text(json.dumps(profile,indent=2))
    # CLI validation must fail before opening a window or publishing an endpoint.
    for invalid_args in [["--check-config"], ["--config"], ["--screenshot"], ["--api-dir"], ["--unknown-option"]]:
        rejected_cli=subprocess.run([str(args.executable.resolve()),*invalid_args],capture_output=True,timeout=10)
        assert rejected_cli.returncode!=0, (invalid_args,rejected_cli.stdout,rejected_cli.stderr)
    version=subprocess.run([str(args.executable.resolve()),"--version"],capture_output=True,text=True,timeout=10,check=True)
    assert version.stdout.startswith("kohdalab-ple ")
    # Explicit invalid profiles must fail before publishing a GUI endpoint.
    invalid=output/"invalid.json";invalid.write_text('{"mode":"not_a_mode"}')
    rejected=subprocess.run([str(args.executable.resolve()),"--gui-api","--api-dir",str(endpoint_dir),"--config",str(invalid)],capture_output=True,timeout=10)
    assert rejected.returncode!=0 and not list(endpoint_dir.glob("ple-gui-*.json"))
    log=(output/"gui.log").open("wb");command=[str(args.executable.resolve()),"--api-dir",str(endpoint_dir),"--config",str(profile_path)]
    process=subprocess.Popen(command,stdout=log,stderr=subprocess.STDOUT);(output/"process.json").write_text(json.dumps({"pid":process.pid,"command":command,"log":str(output/"gui.log")}))
    client=None;observer=None;endpoint=None
    try:
        endpoint=until(lambda:next(iter(endpoint_dir.glob(f"ple-gui-{process.pid}-*.json")),None))
        observer=GuiPLE(endpoint);assert Path(observer.config()["config_path"])==profile_path;assert observer.capabilities()["api_version"]==1;assert not observer.status()["control_owned"]
        client=GuiPLE(endpoint);client.acquire();assert observer.status()["control_owned"]
        assert client.configure(profile)["configured"]
        connect=client.connect();assert client.wait(connect["id"],timeout=10)["status"]=="completed"
        apply_id=str(uuid.uuid4());applied=client.apply_spectrometer({"exposure_ms":100.,"frames_combined":2},request_id=apply_id)
        assert client.wait(applied["id"],timeout=5)["status"]=="completed"
        assert client.apply_spectrometer({"exposure_ms":100.,"frames_combined":2},request_id=apply_id)["status"]=="completed"
        for target in [0.,25.]:assert client.wait(client.move(target)["id"],timeout=5)["status"]=="completed"
        try:client.move(25.001)
        except GuiPLEError as error:assert error.code=="validation"
        else:raise AssertionError("Out-of-range movement was accepted")
        request_id=str(uuid.uuid4());job=client.start(request_id=request_id);assert client.start(request_id=request_id)["id"]==job["id"]
        until(lambda:client.status()["awaiting"]=="background_confirmation")
        confirmation=str(uuid.uuid4());ack=client.confirm_background(True,request_id=confirmation)
        until(lambda:client.status()["awaiting"]=="signal_confirmation");assert client.confirm_background(True,request_id=confirmation)==ack
        client.confirm_signal(True);until(lambda:client.status()["awaiting"]=="analysis_selection")
        preview=client.spectrum();assert preview["kind"]=="preview";assert len(preview["counts"])==641
        analysis={"peak_nm":780.,"peak_half_width_nm":15.,"integral_min_nm":765.,"integral_max_nm":795.}
        confirm_id=str(uuid.uuid4());analysis_ack=client.select_analysis(analysis,request_id=confirm_id);assert client.select_analysis(analysis,request_id=confirm_id)==analysis_ack;assert client.result(confirm_id)==analysis_ack
        receipt=client.wait(job["id"],timeout=10);assert receipt["rows"]==6;assert client.start(request_id=request_id)==receipt
        run=Path(receipt["output"]);assert json.loads((run/"automation.json").read_text())["request_id"]==request_id;metadata=[json.loads(p.read_text()) for p in (run/"metadata").glob("Y_*.json")];assert len(metadata)==6
        saved_preview=json.loads((run/"metadata/preview.json").read_text());first=next(s for s in metadata if s["point_index"]==0 and s["repeat_index"]==0);assert first["acquisition_started_unix_s"]==saved_preview["acquisition_started_unix_s"];assert first["spectrum"]["counts"]==saved_preview["spectrum"]["counts"]
        for sample in metadata:
            expected=700.+(sample["actual_mm"]-18.)*50.
            assert math.isclose(sample["power_sensor_wavelength_nm"],expected,abs_tol=1e-9)
            assert sample["power_timestamp_unix_s"]<=sample["acquisition_started_unix_s"]
            assert abs(sample["excitation_nm"]-sample["power_sensor_wavelength_nm"])<=1.
        for path in (run/"processed").glob("Y_*.csv"):
            rows=list(csv.DictReader(path.open()));assert len(rows)==641
            for r in rows:
                v={k:float(x) for k,x in r.items()};assert math.isclose(v["corrected_counts"],v["raw_counts"]-v["background_counts"],abs_tol=1e-10);assert math.isclose(v["emission_eV"]*v["emission_nm"],1239.8419843320026,rel_tol=1e-12)
        assert not list(run.rglob("*.spe"));assert json.loads((run/"accepted_settings.json").read_text())["config"]["analysis"]==analysis
        # Automatic BG uses the same GUI engine. Demo must never attach to the
        # external shutter descriptor (even when an explicit nonexistent path is set).
        automatic={"mode":"automatic","backend":"kohdalab_shutter","endpoint":"missing-demo-shutter.json","channel":0,"timeout_s":1.0}
        bg_id=str(uuid.uuid4());bg_ack=client.set_background(automatic,request_id=bg_id)
        assert client.set_background(automatic,request_id=bg_id)==bg_ack
        auto=client.start();until(lambda:client.status()["awaiting"]=="analysis_selection")
        assert client.preview()["kind"]=="preview"
        client.select_analysis(analysis)
        auto_receipt=client.wait(auto["id"],timeout=10)
        auto_run=Path(auto_receipt["output"])
        auto_events=[json.loads(line) for line in (auto_run/"events.jsonl").read_text().splitlines()]
        for transition in ("shutter_closed","shutter_opened","shutter_end_close"):
            matches=[e for e in auto_events if e["kind"]==transition]
            assert len(matches)==1, (transition,matches)
            assert matches[0]["data"]["controller_state_confirmed"]
            assert matches[0]["data"]["physical_optical_sensor_confirmed"] is False
        assert json.loads((auto_run/"metadata/shutter.json").read_text())["feedback"]=="simulated"
        assert client.shutter_status()["settings"]["backend"]=="kohdalab_shutter"
        client.set_background({"mode":"manual"})
        # Real lease loss: do not send Stop or heartbeat on behalf of this owner.
        stopped_job=client.start();until(lambda:client.status()["awaiting"]=="background_confirmation");expired_client_id=client.client_id;old_lease=client.lease;client._clear_lease()
        until(lambda:not observer.status()["control_owned"],timeout=13);until(lambda:not observer.status()["busy"],timeout=5)
        stopped=observer.result(stopped_job["id"]);assert stopped["status"]=="stopped"
        try:GuiPLE._wire(client.endpoint,{"action":"heartbeat","lease":old_lease},2)
        except GuiPLEError as error:assert error.code=="ownership_lost"
        else:raise AssertionError("Expired owner still has control")
        replacement=GuiPLE(endpoint);replacement.acquire();assert replacement.wait(replacement.disconnect()["id"],timeout=5)["status"]=="completed";replacement.close();client.close();observer.close()
        evidence={"api_version":1,"mode":"demo","completed_rows":len(metadata),"bins_per_spectrum":641,"checks":["default_gui_api_enabled","automatic_bg_controller_metadata","background_config_idempotency","demo_never_opens_shutter_service","invalid_cli_rejected_before_gui","explicit_invalid_profile_rejected","read_only_observer","exclusive_lease","staged_connect","connected_spectrometer_apply_readback","power_wavelength_tracking_before_exposure","manual_0_25mm","out_of_range_rejected","idempotent_start","idempotent_background_confirmation","idempotent_analysis_confirmation","retained_first_raw","raw_bg_csv_energy_consistency","no_fake_spe","real_heartbeat_expiry_stop","revoked_owner_rejected","disconnect_and_release"],"hardware_tested":False}
        (output/"evidence.json").write_text(json.dumps(evidence,indent=2)+"\n");print(json.dumps(evidence,indent=2))
    finally:
        # This process was launched with an explicit Demo-only profile and all
        # instruments are simulated. Do not terminate an arbitrary user's GUI.
        process.terminate()
        try:process.wait(timeout=10)
        except subprocess.TimeoutExpired:process.kill();process.wait()
        log.close()
        if endpoint and endpoint.exists():endpoint.unlink()

if __name__=="__main__":main()
