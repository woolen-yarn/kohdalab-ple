"""Native 64-bit TLPMX adapter for the existing power-meter interface."""
from __future__ import annotations

import ctypes
import math
import os
from pathlib import Path


def _load_library(path: Path):
    if os.name != "nt" or ctypes.sizeof(ctypes.c_void_p) != 8:
        raise RuntimeError("TLPMX requires 64-bit Windows Python")
    return ctypes.WinDLL(str(path.resolve()))


class TLPMXMeter:
    """Use channel 1 and explicit VISA selection without resetting the meter."""

    def __init__(self, path: Path, resource: str) -> None:
        self._session = ctypes.c_uint32()
        self._dll = _load_library(path)
        self.deviceNET = self
        session = ctypes.c_uint32
        boolean = ctypes.c_uint16
        selector = ctypes.c_int16
        real = ctypes.c_double
        text = ctypes.c_char_p
        pointer = ctypes.POINTER
        signatures = {
            "init": [text, boolean, boolean, pointer(session)],
            "close": [session],
            "identificationQuery": [session, text, text, text, text],
            "revisionQuery": [session, text, text],
            "getSensorInfo": [session, text, text, text, pointer(selector),
                              pointer(selector), pointer(selector), boolean],
            "getWavelength": [session, selector, pointer(real), boolean],
            "setWavelength": [session, real, boolean],
            "getAvgTime": [session, selector, pointer(real), boolean],
            "setAvgTime": [session, real, boolean],
            "setPowerAutoRange": [session, boolean, boolean],
            "getPowerUnit": [session, pointer(selector), boolean],
            "measPower": [session, pointer(real), boolean],
        }
        # Resolve every required symbol before opening an instrument.
        for name, arguments in signatures.items():
            function = getattr(self._dll, "TLPMX_" + name)
            function.argtypes = arguments
            function.restype = ctypes.c_int32
        try:
            self._invoke("init", resource.encode("ascii"), 1, 0,
                         ctypes.byref(self._session))
            if not self._session.value:
                raise RuntimeError("TLPMX_init returned an empty session")
            manufacturer, model, serial, firmware = self._buffers(4)
            self._invoke("identificationQuery", self._session, manufacturer,
                         model, serial, firmware)
            self.manufacturer = self._text(manufacturer)
            self.modelName = self._text(model)
            self.serialNumber = self._text(serial)
            driver, firmware = self._buffers(2)
            self._invoke("revisionQuery", self._session, driver, firmware)
            self.driverVersion = self._text(driver)
        except Exception as error:
            try:
                self.disconnect()
            except Exception as cleanup:
                raise RuntimeError(f"{error}; TLPMX cleanup also failed: {cleanup}") from error
            raise

    @staticmethod
    def _buffers(count):
        return [ctypes.create_string_buffer(1024) for _ in range(count)]

    @staticmethod
    def _text(buffer):
        return buffer.value.decode("utf-8", errors="replace").strip()

    def _invoke(self, name, *arguments):
        status = getattr(self._dll, "TLPMX_" + name)(*arguments)
        if status < 0:
            raise RuntimeError(f"TLPMX_{name} failed with SDK status {status}")
        return status

    def _read_setting(self, name, selector):
        value = ctypes.c_double()
        status = self._invoke(name, self._session, selector,
                              ctypes.byref(value), 1)
        if not math.isfinite(value.value) or value.value <= 0:
            raise RuntimeError(f"TLPMX_{name} returned an invalid value")
        return status, value.value

    def getWavelength(self, selector):
        return self._read_setting("getWavelength", selector)

    def getWaveLength(self):
        self.wavelengthSet = self.getWavelength(0)[1]

    def setWaveLength(self, wavelength):
        self.wavelengthMin = self.getWavelength(1)[1]
        self.wavelengthMax = self.getWavelength(2)[1]
        if not self.wavelengthMin <= wavelength <= self.wavelengthMax:
            raise ValueError("requested wavelength is outside the TLPMX sensor range")
        self._invoke("setWavelength", self._session, wavelength, 1)

    def setAverageTime(self, seconds):
        minimum = self._read_setting("getAvgTime", 1)[1]
        maximum = self._read_setting("getAvgTime", 2)[1]
        if not minimum <= seconds <= maximum:
            raise ValueError("requested averaging time is outside the TLPMX range")
        self._invoke("setAvgTime", self._session, seconds, 1)

    def getAverageTime(self):
        self.averageTimeSet = self._read_setting("getAvgTime", 0)[1]

    def setPowerAutoRange(self, enabled):
        self._invoke("setPowerAutoRange", self._session, int(enabled), 1)

    def getSensorInfo(self):
        name, serial, message = self._buffers(3)
        sensor_type = ctypes.c_int16()
        subtype = ctypes.c_int16()
        flags = ctypes.c_int16()
        self._invoke("getSensorInfo", self._session, name, serial, message,
                     ctypes.byref(sensor_type), ctypes.byref(subtype),
                     ctypes.byref(flags), 1)
        self.sensorName = self._text(name)
        self.sensorSerialNumber = self._text(serial)
        self.sensorCalibrationMessage = self._text(message)
        self.sensorType = "No sensor" if sensor_type.value == 0 else str(sensor_type.value)
        self.sensorSubType = str(subtype.value)
        self.sensorFlags = str(flags.value)

    def updatePowerReading(self, _interval):
        unit = ctypes.c_int16()
        self._invoke("getPowerUnit", self._session, ctypes.byref(unit), 1)
        if unit.value not in (0, 1):
            raise RuntimeError(f"unsupported TLPMX power unit {unit.value}")
        value = ctypes.c_double()
        self._invoke("measPower", self._session, ctypes.byref(value), 1)
        self.meterPowerReading = value.value
        self.meterPowerUnit = {0: "W", 1: "dBm"}[unit.value]

    def disconnect(self):
        if self._session.value:
            # Keep ownership if close fails so cleanup can be retried.
            self._invoke("close", self._session)
            self._session = ctypes.c_uint32()
