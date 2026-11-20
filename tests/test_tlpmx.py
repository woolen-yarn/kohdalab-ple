import ctypes
import math
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parents[1] / "bridge"))
import ple_bridge as bridge
import tlpmx


class Function:
    def __init__(self, sdk, name):
        self.sdk = sdk
        self.name = name

    def __call__(self, *args):
        self.sdk.calls.append((self.name, args))
        status = self.sdk.errors.get(self.name, 0)
        if status < 0:
            return status
        name = self.name
        if name == "init":
            args[3]._obj.value = 123
        elif name == "identificationQuery":
            for buffer, value in zip(args[1:], (b"Thorlabs", b"PM100D", b"meter-1", b"1.0")):
                buffer.value = value
        elif name == "revisionQuery":
            args[1].value = b"5.0"
            args[2].value = b"1.0"
        elif name == "getSensorInfo":
            for buffer, value in zip(args[1:4], (b"S130C", b"sensor-1", b"2024")):
                buffer.value = value
            for pointer, value in zip(args[4:7], (self.sdk.sensor_type, 2, 0)):
                pointer._obj.value = value
        elif name == "getWavelength":
            args[2]._obj.value = {0: self.sdk.wavelength, 1: 400., 2: 1100.}[args[1]]
        elif name == "setWavelength":
            self.sdk.wavelength = args[1]
        elif name == "getAvgTime":
            args[2]._obj.value = {0: self.sdk.average, 1: .001, 2: 10.}[args[1]]
        elif name == "setAvgTime":
            self.sdk.average = args[1] if not self.sdk.clamp_average else .2
        elif name == "getPowerUnit":
            args[1]._obj.value = self.sdk.unit
        elif name == "measPower":
            args[1]._obj.value = self.sdk.power
        return status


class FakeSDK:
    def __init__(self):
        self.calls = []
        self.errors = {}
        self.wavelength = 635.
        self.average = .1
        self.clamp_average = False
        self.sensor_type = 1
        self.unit = 0
        self.power = -1e-9
        for name in ("init", "close", "identificationQuery", "revisionQuery",
                     "getSensorInfo", "getWavelength", "setWavelength",
                     "getAvgTime", "setAvgTime", "setPowerAutoRange",
                     "getPowerUnit", "measPower"):
            setattr(self, "TLPMX_" + name, Function(self, name))


class TLPMXTests(unittest.TestCase):
    def setUp(self):
        self.directory = TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name)
        (self.path / "TLPMX_64.dll").write_bytes(b"synthetic DLL fixture")
        self.sdk = FakeSDK()
        self.loader = patch.object(tlpmx, "_load_library", return_value=self.sdk)
        self.loader.start()
        self.addCleanup(self.loader.stop)

    def connect(self):
        power = bridge.LegacyThorlabsPowerMeter()
        power.connect({"driver_path": str(self.path), "resource": "USB::meter-1",
                       "wavelength_nm": 800., "average_s": .5})
        self.addCleanup(power.disconnect)
        return power

    def test_normal_entry_connect_read_and_disconnect_without_legacy_helper(self):
        power = self.connect()
        reading = power.read()
        self.assertEqual(reading["native_unit"], "W")
        self.assertEqual(reading["native_value"], -1e-9)
        self.assertEqual(reading["power_w"], -1e-9)
        self.assertEqual(reading["wavelength_nm"], 800.)
        self.assertEqual(reading["hardware"]["model"], "PM100D")
        self.assertEqual(reading["hardware"]["driver_version"], "5.0")
        self.assertEqual(reading["hardware"]["sensor"]["name"], "S130C")
        init_args = next(args for name, args in self.sdk.calls if name == "init")
        self.assertEqual(init_args[:3], (b"USB::meter-1", 1, 0))
        self.assertEqual(self.sdk.TLPMX_init.argtypes[3], ctypes.POINTER(ctypes.c_uint32))
        channel_calls = {"getSensorInfo", "getWavelength", "setWavelength", "getAvgTime",
                         "setAvgTime", "setPowerAutoRange", "getPowerUnit", "measPower"}
        for name, args in self.sdk.calls:
            if name in channel_calls:
                self.assertEqual(args[-1], 1, name)
        power.disconnect()
        power.disconnect()
        self.assertEqual(sum(name == "close" for name, _ in self.sdk.calls), 1)

    def test_dbm_conversion_and_wavelength_refresh_use_shared_logic(self):
        power = self.connect()
        self.sdk.unit = 1
        self.sdk.power = -30.
        self.sdk.wavelength = 805.
        reading = power.read()
        self.assertAlmostEqual(reading["power_w"], 1e-6)
        self.assertEqual(reading["native_unit"], "dBm")
        self.assertEqual(reading["wavelength_nm"], 805.)
        power.set_wavelength(810.)
        self.assertEqual(power.state()["wavelength_nm"], 810.)

    def test_missing_export_fails_before_instrument_open(self):
        del self.sdk.TLPMX_measPower
        with self.assertRaises(AttributeError):
            tlpmx.TLPMXMeter(self.path / "TLPMX_64.dll", "USB::meter-1")
        self.assertFalse(self.sdk.calls)

    def test_setup_error_closes_native_session(self):
        self.sdk.errors["setAvgTime"] = -42
        with self.assertRaisesRegex(bridge.BridgeError, "SDK status -42"):
            self.connect()
        self.assertEqual(sum(name == "close" for name, _ in self.sdk.calls), 1)

    def test_close_failure_keeps_session_for_retry(self):
        meter = tlpmx.TLPMXMeter(self.path / "TLPMX_64.dll", "USB::meter-1")
        self.sdk.errors["close"] = -45
        with self.assertRaisesRegex(RuntimeError, "SDK status -45"):
            meter.disconnect()
        self.assertEqual(meter._session.value, 123)
        self.sdk.errors.clear()
        meter.disconnect()
        self.assertEqual(meter._session.value, 0)

    def test_partial_native_initialization_is_cleaned_up(self):
        self.sdk.errors["identificationQuery"] = -43
        with self.assertRaisesRegex(bridge.BridgeError, "SDK status -43"):
            self.connect()
        self.assertEqual(sum(name == "close" for name, _ in self.sdk.calls), 1)

    def test_averaging_clamp_is_rejected_and_session_closed(self):
        self.sdk.clamp_average = True
        with self.assertRaisesRegex(bridge.BridgeError, "averaging readback"):
            self.connect()
        self.assertEqual(sum(name == "close" for name, _ in self.sdk.calls), 1)

    def test_outside_sensor_range_is_rejected_before_setting(self):
        power = self.connect()
        before = sum(name == "setWavelength" for name, _ in self.sdk.calls)
        with self.assertRaisesRegex(bridge.BridgeError, "outside"):
            power.set_wavelength(1200.)
        self.assertEqual(sum(name == "setWavelength" for name, _ in self.sdk.calls), before)

    def test_fault_unit_and_nonfinite_power_are_rejected(self):
        power = self.connect()
        self.sdk.errors["measPower"] = -44
        with self.assertRaisesRegex(bridge.BridgeError, "SDK status -44"):
            power.read()
        self.sdk.errors.clear()
        self.sdk.unit = 7
        with self.assertRaisesRegex(bridge.BridgeError, "unsupported TLPMX power unit"):
            power.read()
        self.sdk.unit = 0
        self.sdk.power = math.nan
        with self.assertRaises(bridge.BridgeError):
            power.read()

    def test_sensor_removed_is_rejected(self):
        power = self.connect()
        self.sdk.sensor_type = 0
        with self.assertRaisesRegex(bridge.BridgeError, "no sensor"):
            power.read()

    def test_legacy_helper_has_priority_and_never_falls_back_on_failure(self):
        (self.path / "ThorlabsPowerMeter.py").write_text('raise RuntimeError("legacy unavailable")')
        (self.path / "Thorlabs_DotNet_dll").mkdir()
        with self.assertRaisesRegex(bridge.BridgeError, "legacy unavailable"):
            self.connect()
        self.assertFalse(self.sdk.calls)


if __name__ == "__main__":
    unittest.main()
