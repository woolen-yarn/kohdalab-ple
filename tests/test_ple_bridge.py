import io
import json
import sys
import threading
import time
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

sys.path.insert(0, str(Path(__file__).parents[1] / "bridge"))
import ple_bridge as bridge


class FakeSpectrometer:
    def __init__(self): self.connected = False; self.stopped = False; self.disconnects = 0
    def connect(self, args): self.connected = True; return self.state()
    def configure(self, args): return {"center_nm": args["center_nm"]}
    def acquire(self, args, cancel):
        if cancel.is_set(): raise bridge.BridgeError("acquisition was stopped")
        return {"wavelength_nm": [700.0, 701.0], "counts": [1.0, 2.0], "spe_path": args["path_base"] + ".spe", "settings": {}}
    def stop(self): self.stopped = True
    def state(self): return {"connected": self.connected}
    def disconnect(self): self.connected = False; self.disconnects += 1


class FakePower:
    def __init__(self): self.connected = False; self.disconnects = 0; self.wavelength_nm = 720.0; self.wavelength_readback = None
    def connect(self, args):
        self.connected = True
        self.wavelength_nm = float(args.get("wavelength_nm", self.wavelength_nm))
        return self.state()
    def read(self):
        if not self.connected: raise bridge.BridgeError("power meter is not connected")
        return {"power_w": 1e-6, "wavelength_nm": self.wavelength_nm,
                "native_unit": "W", "native_value": 1e-6, "unit": "W"}
    def set_wavelength(self, wavelength_nm, tolerance_nm=1e-6):
        self.wavelength_nm = float(wavelength_nm if self.wavelength_readback is None else self.wavelength_readback)
        return {"wavelength_nm": self.wavelength_nm}
    def disconnect(self): self.connected = False; self.disconnects += 1
    def state(self): return {"connected": self.connected, "wavelength_nm": self.wavelength_nm if self.connected else None}


class FakeLegacyMeter:
    """Models the attributes and capitalized methods of ThorlabsPowerMeter.py."""
    def __init__(self, *, wavelength_nm=720.0, unit="W", value=1e-6, clamp_to=None,
                 minimum_nm=None, maximum_nm=None):
        self.wavelengthSet = wavelength_nm
        self.meterPowerUnit = unit
        self.meterPowerReading = value
        self.clamp_to = clamp_to
        self.wavelengthMin = minimum_nm
        self.wavelengthMax = maximum_nm
        self.calls = []

    def setWaveLength(self, wavelength_nm):
        self.calls.append(("setWaveLength", wavelength_nm))
        self.wavelengthSet = self.clamp_to if self.clamp_to is not None else wavelength_nm

    def getWaveLength(self): self.calls.append(("getWaveLength",))
    def updatePowerReading(self, period): self.calls.append(("updatePowerReading", period))


class FakeShutter:
    def __init__(self): self.connected = False; self.closed = None; self.calls = []; self.disconnects = 0
    def connect(self, args): self.connected = True; self.calls.append(("connect", args)); return self.state()
    def set_closed(self, closed, _timeout_s, cancel):
        if not closed and cancel.is_set(): raise bridge.BridgeError("shutter cannot open while stop is latched")
        self.closed = closed; self.calls.append(("set", closed)); return self.state()
    def disconnect(self): self.connected = False; self.disconnects += 1
    def state(self): return {"connected": self.connected, "closed": self.closed}


class BridgeTests(unittest.TestCase):
    def test_dark_negative_watts_remain_monitorable_but_not_fake_zero(self):
        p=bridge.LegacyThorlabsPowerMeter();p.meter=FakeLegacyMeter(unit="W",value=-1e-12);self.assertEqual(p.read()["power_w"],-1e-12)

    def test_average_readback_cannot_silently_clamp(self):
        class Meter:
            def setAverageTime(self,value):self.averageTimeSet=max(.5,value)
            def getAverageTime(self):pass
        self.assertEqual(bridge.LegacyThorlabsPowerMeter._set_meter_average(Meter(),.5),.5)
        with self.assertRaisesRegex(bridge.BridgeError,"averaging readback"):bridge.LegacyThorlabsPowerMeter._set_meter_average(Meter(),.2)

    def test_lightfield_stop_requires_ready_readback(self):
        class Experiment:
            IsReadyToRun=False
            def Stop(self):self.IsReadyToRun=True
        spec=bridge.LightFieldSpectrometer();spec.experiment=Experiment();spec.stop(.01);self.assertTrue(spec.experiment.IsReadyToRun)
        spec.experiment.Stop=lambda:None;spec.experiment.IsReadyToRun=False
        with self.assertRaisesRegex(bridge.BridgeError,"ready/idle"):spec.stop(.01)

    def test_ensure_idle_confirms_sdk_stop_and_closes_attached_shutter(self):
        spec,power,shutter=FakeSpectrometer(),FakePower(),FakeShutter();spec.connected=True;shutter.connected=True;shutter.closed=False
        h=bridge.HardwareBridge(spec,power,threading.Event(),shutter);result=h.dispatch("ensure_idle",{});self.assertTrue(result["idle_confirmed"]);self.assertTrue(spec.stopped);self.assertTrue(shutter.closed);self.assertTrue(h.cancel.is_set())

    def test_lightfield_dispose_error_is_not_reported_as_disconnected(self):
        class FailingAutomation:
            def Dispose(self): raise RuntimeError("SDK failed")
        spec=bridge.LightFieldSpectrometer();spec.auto=FailingAutomation()
        with self.assertRaisesRegex(bridge.BridgeError,"disconnect unconfirmed"):spec.disconnect()
        self.assertTrue(spec.state()["connected"])

    def test_blank_grating_retains_and_records_template_readback(self):
        class Settings:
            ShutterTimingExposureTime = "exposure"
            FrameSettingsFramesCombined = "frames"
            GratingCenterWavelength = "center"
            Grating = "grating"
            OnlineCorrectionsBackgroundCorrectionEnabled = "online_bg"
        class Experiment:
            IsReadyToRun = True
            values = {"online_bg":True,"exposure": 1.0, "frames": 1, "center": 700.0, "grating": "template grating"}
            def Exists(self, _setting): return True
            def SetValue(self, setting, value): self.values[setting] = value
            def GetValue(self, setting): return self.values[setting]
        spectrometer = bridge.LightFieldSpectrometer()
        spectrometer.experiment = Experiment()
        spectrometer.api = {"CameraSettings": Settings, "ExperimentSettings": Settings,
                             "SpectrometerSettings": Settings, "Int32": int}
        applied = spectrometer._apply({"exposure_ms": 5, "frames_combined": 2, "center_nm": 710, "grating": ""})
        self.assertEqual(applied["grating"], "template grating")
        self.assertFalse(applied["online_background_correction"])
        spectrometer.experiment.values["online_bg"]=True
        with self.assertRaisesRegex(bridge.BridgeError,"settings changed"):spectrometer._assert_expected_settings()
        spectrometer.experiment.values["online_bg"]=False
        spectrometer.experiment.values["center"] = 711.0
        with self.assertRaisesRegex(bridge.BridgeError, "changed after configuration"):
            spectrometer._assert_expected_settings()

    def test_set_does_not_coerce_bool_to_int32(self):
        received = []
        class Experiment:
            def Exists(self, _setting): return True
            def SetValue(self, _setting, value): received.append(value)
        class Int32:
            def __init__(self, value): self.value = value
        spectrometer = bridge.LightFieldSpectrometer()
        spectrometer.experiment = Experiment()
        spectrometer.api = {"Int32": Int32}
        spectrometer._set("flag", False)
        self.assertIs(received[0], False)

    def test_csv_rejects_multiple_frames(self):
        with TemporaryDirectory() as tmp:
            p = Path(tmp) / "a.csv"; p.write_text("Wavelength,Intensity,F,R\n700,1,0,0\n701,2,1,0\n")
            with self.assertRaisesRegex(bridge.BridgeError, "multiple exported frames"):
                bridge._read_one_roi_csv(p)

    def test_csv_prefers_named_calibrated_columns_over_r_f_y_x(self):
        with TemporaryDirectory() as tmp:
            p = Path(tmp) / "legacy.csv"
            p.write_text("R,F,Y,X,Wavelength (nm),Intensity\n0,0,10,100,700.0,3\n0,0,10,101,701.0,4\n")
            self.assertEqual(bridge._read_one_roi_csv(p), ([700.0, 701.0], [3.0, 4.0]))

    def test_csv_rejects_uncalibrated_metadata_only_export(self):
        with TemporaryDirectory() as tmp:
            p = Path(tmp) / "metadata.csv"; p.write_text("R,F,Y,X\n0,0,1,700\n0,0,2,701\n")
            with self.assertRaisesRegex(bridge.BridgeError, "X/wavelength"):
                bridge._read_one_roi_csv(p)

    def test_csv_rejects_multiple_detector_rows(self):
        with TemporaryDirectory() as tmp:
            p = Path(tmp) / "rows.csv"
            p.write_text("R,F,Y,X,Wavelength,Intensity\n0,0,1,100,700,1\n0,0,2,101,701,2\n")
            with self.assertRaisesRegex(bridge.BridgeError, "multiple detector rows"):
                bridge._read_one_roi_csv(p)

    def test_csv_requires_strictly_increasing_positive_wavelength(self):
        with TemporaryDirectory() as tmp:
            p = Path(tmp) / "bad.csv"; p.write_text("Wavelength,Intensity\n701,1\n700,2\n")
            with self.assertRaisesRegex(bridge.BridgeError, "strictly increasing"):
                bridge._read_one_roi_csv(p)

    def test_stable_wait_cancels(self):
        with TemporaryDirectory() as tmp:
            cancelled = threading.Event(); cancelled.set()
            with self.assertRaisesRegex(bridge.BridgeError, "stopped"):
                bridge._wait_stable([Path(tmp) / "nope"], time.monotonic() + 1, cancelled)

    def test_export_task_completion_and_cancel(self):
        class Completed:
            IsCompleted = True
            IsFaulted = False
            IsCanceled = False
        bridge._wait_export_task(Completed(), time.monotonic() + 1, threading.Event())
        class Pending:
            IsCompleted = False
        cancelled = threading.Event(); cancelled.set()
        with self.assertRaisesRegex(bridge.BridgeError, "stopped"):
            bridge._wait_export_task(Pending(), time.monotonic() + 1, cancelled)

    def test_stable_wait_requires_exclusive_probe(self):
        with TemporaryDirectory() as tmp:
            path = Path(tmp) / "done.csv"; path.write_text("ready")
            calls = []
            def probe(_paths):
                calls.append(True)
                return len(calls) > 1
            bridge._wait_stable([path], time.monotonic() + 2, threading.Event(), probe)
            self.assertGreaterEqual(len(calls), 4)

    def test_server_framing_and_stop(self):
        spec, power = FakeSpectrometer(), FakePower()
        input_lines = "\n".join([
            json.dumps({"revision": 1, "id": "a", "op": "protocol", "args": {}}),
            json.dumps({"revision": 1, "id": "b", "op": "power_read", "args": {}}),
            json.dumps({"revision": 1, "id": "c", "op": "stop", "args": {}}),
            "",
        ])
        output = io.StringIO()
        server = bridge.RpcServer(bridge.HardwareBridge(spec, power, threading.Event()), io.StringIO(input_lines), output)
        server.serve()
        replies = [json.loads(line) for line in output.getvalue().splitlines()]
        by_id = {reply["id"]: reply for reply in replies}
        self.assertEqual(set(by_id), {"a", "b", "c"})
        self.assertTrue(by_id["c"]["ok"])
        self.assertEqual(by_id["a"]["result"]["revision"], 1)
        self.assertFalse(by_id["b"]["ok"])
        self.assertTrue(server.bridge.cancel.is_set())
        self.assertEqual(spec.disconnects, 1)
        self.assertEqual(power.disconnects, 1)

    def test_dispatch_returns_real_power_shape(self):
        h = bridge.HardwareBridge(FakeSpectrometer(), FakePower(), threading.Event())
        h.dispatch("connect_power", {"resource": "fake"})
        self.assertEqual(h.dispatch("power_read", {}), {
            "power_w": 1e-6, "wavelength_nm": 720.0,
            "native_unit": "W", "native_value": 1e-6, "unit": "W",
        })

    def test_power_set_wavelength_requires_readback_and_legacy_detects_silent_clamp(self):
        power = bridge.LegacyThorlabsPowerMeter()
        power.meter = FakeLegacyMeter(clamp_to=700.0)
        power.resource = "USB::fake"
        with self.assertRaisesRegex(bridge.BridgeError, "differs from requested"):
            power.set_wavelength(720.0)

        fake = FakePower()
        h = bridge.HardwareBridge(FakeSpectrometer(), fake, threading.Event())
        h.dispatch("connect_power", {"resource": "fake"})
        self.assertEqual(h.dispatch("power_set_wavelength", {"wavelength_nm": 735.5}), {"wavelength_nm": 735.5})
        fake.wavelength_readback = 720.0
        self.assertEqual(
            h.dispatch("power_set_wavelength", {"wavelength_nm": 720.25, "wavelength_tolerance_nm": 0.5}),
            {"wavelength_nm": 720.0},
        )
        self.assertEqual(fake.wavelength_nm, 720.0)
        with self.assertRaisesRegex(bridge.BridgeError, "differs from requested"):
            h.dispatch("power_set_wavelength", {"wavelength_nm": 720.25})
        fake.wavelength_readback = 700.0
        with self.assertRaisesRegex(bridge.BridgeError, "differs from requested"):
            h.dispatch("power_set_wavelength", {"wavelength_nm": 720.0, "wavelength_tolerance_nm": 1.0})
        with self.assertRaisesRegex(bridge.BridgeError, "finite positive"):
            h.dispatch("power_set_wavelength", {"wavelength_nm": 0})

    def test_legacy_wavelength_range_rejects_request_even_with_tolerance(self):
        power = bridge.LegacyThorlabsPowerMeter()
        power.meter = FakeLegacyMeter(clamp_to=700.0, minimum_nm=700.0, maximum_nm=900.0)
        with self.assertRaisesRegex(bridge.BridgeError, "outside the power meter range"):
            power.set_wavelength(650.0, tolerance_nm=100.0)

    def test_legacy_power_read_reports_actual_wavelength_and_converts_dbm(self):
        power = bridge.LegacyThorlabsPowerMeter()
        power.meter = FakeLegacyMeter(wavelength_nm=785.25, unit="dBm", value=-30.0)
        power.resource = "USB::fake"
        result = power.read()
        self.assertAlmostEqual(result["power_w"], 1e-6)
        self.assertEqual(result["wavelength_nm"], 785.25)
        self.assertEqual(result["native_unit"], "dBm")
        self.assertEqual(result["native_value"], -30.0)
        self.assertEqual(result["unit"], "W")
        self.assertIn(("getWaveLength",), power.meter.calls)

    def test_legacy_power_read_rejects_unknown_native_unit(self):
        power = bridge.LegacyThorlabsPowerMeter()
        power.meter = FakeLegacyMeter(unit="mW", value=1.0)
        with self.assertRaisesRegex(bridge.BridgeError, "unsupported power meter unit"):
            power.read()

    def test_legacy_power_read_rejects_nonfinite_and_overflow_dbm(self):
        power = bridge.LegacyThorlabsPowerMeter()
        power.meter = FakeLegacyMeter(unit="W", value=float("nan"))
        with self.assertRaisesRegex(bridge.BridgeError, "finite"):
            power.read()
        power.meter = FakeLegacyMeter(unit="dBm", value=4000.0)
        with self.assertRaisesRegex(bridge.BridgeError, "overflowed"):
            power.read()

    def test_legacy_connect_rejects_silent_wavelength_clamp(self):
        with TemporaryDirectory() as tmp:
            driver = Path(tmp)
            cleanup_marker = driver / "disconnected"
            (driver / "Thorlabs_DotNet_dll").mkdir()
            (driver / "ThorlabsPowerMeter.py").write_text(f'''
class Meter:
    wavelengthSet = 700.0
    def setWaveLength(self, value): self.wavelengthSet = 700.0
    def getWaveLength(self): return None
    def setAverageTime(self, value): return None
    def setPowerAutoRange(self, value): return None
    def disconnect(self): open({str(cleanup_marker)!r}, "w").close()
class Devices:
    def connect(self, resource): return Meter()
class ThorlabsPowerMeter:
    @staticmethod
    def listDevices(path): return Devices()
''')
            with self.assertRaisesRegex(bridge.BridgeError, "differs from requested"):
                bridge.LegacyThorlabsPowerMeter().connect({
                    "driver_path": str(driver), "resource": "USB::fake",
                    "wavelength_nm": 720.0, "average_s": 0.5,
                })
            self.assertTrue(cleanup_marker.is_file())

    def test_stop_latches_until_configuration(self):
        h = bridge.HardwareBridge(FakeSpectrometer(), FakePower(), threading.Event())
        h.request_stop()
        with self.assertRaisesRegex(bridge.BridgeError, "stopped"):
            h.dispatch("acquire", {"path_base": "ignored"})
        h.dispatch("configure_spectrometer", {"center_nm": 700})
        self.assertFalse(h.cancel.is_set())
        self.assertEqual(h.dispatch("acquire", {"path_base": "ignored"})["counts"], [1.0, 2.0])

    def test_shutter_close_before_background_and_reopen(self):
        shutter = FakeShutter()
        h = bridge.HardwareBridge(FakeSpectrometer(), FakePower(), threading.Event(), shutter)
        h.dispatch("shutter_connect", {"serial": "123", "closed_state": 7, "open_state": 3, "timeout_s": 1})
        h.dispatch("shutter_set", {"closed": True, "timeout_s": 1})
        self.assertTrue(h.dispatch("state", {})["shutter"]["closed"])
        h.dispatch("shutter_set", {"closed": False, "timeout_s": 1})
        self.assertEqual([call for call in shutter.calls if call[0] == "set"], [("set", True), ("set", False)])

    def test_stop_allows_fail_closed_but_rejects_open(self):
        shutter = FakeShutter()
        h = bridge.HardwareBridge(FakeSpectrometer(), FakePower(), threading.Event(), shutter)
        h.dispatch("shutter_connect", {"serial": "123", "closed_state": 1, "open_state": 0})
        h.request_stop()
        h.dispatch("shutter_set", {"closed": True})
        with self.assertRaisesRegex(bridge.BridgeError, "cannot open"):
            h.dispatch("shutter_set", {"closed": False})

    def test_shutdown_closes_attached_shutter_before_disconnect(self):
        shutter = FakeShutter()
        h = bridge.HardwareBridge(FakeSpectrometer(), FakePower(), threading.Event(), shutter)
        h.dispatch("shutter_connect", {"serial": "123", "closed_state": 1, "open_state": 0})
        h.shutdown()
        self.assertEqual(shutter.calls[-1], ("set", True))
        self.assertEqual(shutter.disconnects, 1)

    def test_mff_state_mapping_requires_distinct_integer_states(self):
        shutter = bridge.ThorlabsMffShutter()
        self.assertEqual(shutter._state("closed_state", 2), 2)
        with self.assertRaisesRegex(bridge.BridgeError, "integer"):
            shutter._state("closed_state", True)

    def test_shutter_connect_reads_but_does_not_move(self):
        class Device:
            def __init__(self): self.calls = []
            def get_state(self): self.calls.append("get"); return 3
            def move_to_state(self, state): self.calls.append(("move", state))
        device = Device()
        shutter = bridge.ThorlabsMffShutter(lambda _serial: device)
        state = shutter.connect({"serial": "123", "closed_state": 7, "open_state": 3, "timeout_s": 1})
        self.assertFalse(any(isinstance(call, tuple) for call in device.calls))
        self.assertFalse(state["closed"])
        with self.assertRaisesRegex(bridge.BridgeError, "distinct"):
            bridge.ThorlabsMffShutter(lambda _serial: Device()).connect(
                {"serial": "123", "closed_state": 3, "open_state": 3})

    def test_decimal_position_basename_is_preserved(self):
        paths = bridge._output_paths(Path("Y_18.6mm_40uW_p00000_r000"))
        self.assertEqual([str(path) for path in paths], [
            "Y_18.6mm_40uW_p00000_r000.spe",
            "Y_18.6mm_40uW_p00000_r000.csv",
            "Y_18.6mm_40uW_p00000_r000.lightfield.csv",
        ])


if __name__ == "__main__": unittest.main()
