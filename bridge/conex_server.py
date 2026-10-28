#!/usr/bin/env python3
"""Conservative TCP adapter for a Newport CONEX-CC on a Windows lab PC.

It intentionally never homes (``OR``) or enables a positioner (``MM_Set``).
The controller must be made safe and ready by the operator before a move.
"""
from __future__ import annotations

import argparse
import hmac
import json
import math
import os
import sys
from pathlib import Path
import socketserver
import threading
import time
from dataclasses import dataclass
from typing import Any, Callable

REVISION = 1
MAX_LINE = 16 * 1024
DEVICE = 1


class ConexError(RuntimeError):
    pass


def _number(name: str, value: Any, *, positive: bool = False) -> float:
    try:
        result = float(value)
    except (TypeError, ValueError) as error:
        raise ConexError(f"{name} must be numeric") from error
    if not math.isfinite(result) or (positive and result <= 0):
        raise ConexError(f"{name} must be a finite{' positive' if positive else ''} number")
    return result


def _sdk_error(error: Any, operation: str) -> None:
    if error not in (None, "", 0):
        raise ConexError(f"CONEX {operation} failed: {error}")


class ConexController:
    """A single SDK instance with short lock-held calls only."""
    def __init__(self, driver_factory: Callable[[], Any] | None = None, sdk_directory: str | None = None) -> None:
        self._factory = driver_factory
        self._sdk_directory = sdk_directory or os.environ.get("CONEX_SDK_DIR")
        self._driver: Any = None
        self._port: str | None = None
        self._sdk_lock = threading.RLock()
        self._move_lock = threading.Lock()
        self._abort: threading.Event | None = None

    def _new_driver(self) -> Any:
        if self._factory is not None:
            return self._factory()
        if os.name != "nt":
            raise ConexError("CONEX .NET adapter requires Windows")
        try:  # pragma: no cover - vendor SDK
            import clr  # type: ignore
            if self._sdk_directory:
                directory=Path(self._sdk_directory).resolve()
                assembly=directory / "Newport.CONEXCC.CommandInterface.dll"
                if not assembly.is_file():raise ConexError(f"Newport SDK assembly not found: {assembly}")
                sys.path.insert(0,str(directory))
                clr.AddReference(str(assembly))
            else:
                clr.AddReference("Newport.CONEXCC.CommandInterface")
            import CommandInterfaceConexCC  # type: ignore
            return CommandInterfaceConexCC.ConexCC()
        except Exception as error:  # pragma: no cover - vendor SDK
            raise ConexError(f"could not load Newport CONEX-CC .NET SDK: {error}") from error

    def _require(self) -> Any:
        if self._driver is None:
            raise ConexError("CONEX is not connected")
        return self._driver

    def _readback(self) -> dict[str, Any]:
        """Read only: no implicit recovery, enable, or homing actions."""
        with self._sdk_lock:
            driver = self._require()
            _ret, minimum, error = driver.SL_Get(DEVICE, 0, "")
            _sdk_error(error, "SL_Get")
            _ret, maximum, error = driver.SR_Get(DEVICE, 0, "")
            _sdk_error(error, "SR_Get")
            _ret, position, error = driver.TP(DEVICE, 0, "")
            _sdk_error(error, "TP")
            _ret, positioner_error, state, error = driver.TS(DEVICE, "", "", "")
            _sdk_error(error, "TS")
        state_text, error_text = str(state), str(positioner_error)
        return {"position_mm": _number("position_mm", position),
                "min_mm": _number("min_mm", minimum), "max_mm": _number("max_mm", maximum),
                "state": state_text, "positioner_error": error_text,
                "ready": error_text == "" and state_text in {"32", "33", "34"}}

    def connect(self, args: dict[str, Any]) -> dict[str, Any]:
        if self._move_lock.locked():
            raise ConexError("cannot connect while a move is active")
        port = args.get("com_port")
        if not isinstance(port, str) or not port.strip():
            raise ConexError("com_port must be a non-empty string")
        port = port.strip()
        if self._driver is not None:
            if port != self._port:
                raise ConexError(f"CONEX is already open on {self._port}")
            return self._readback()
        driver = self._new_driver()
        with self._sdk_lock:
            result = driver.OpenInstrument(port)
            if result != 0:
                raise ConexError(f"could not open CONEX port {port} (code {result})")
            if "velocity_mm_s" in args and args["velocity_mm_s"] is not None:
                velocity = _number("velocity_mm_s", args["velocity_mm_s"], positive=True)
                _ret, error = driver.VA_Set(DEVICE, velocity, "")
                _sdk_error(error, "VA_Set")
        self._driver, self._port = driver, port
        try:
            return self._readback()
        except Exception:
            self.close()
            raise

    def status(self) -> dict[str, Any]:
        return self._readback()

    def move(self, args: dict[str, Any]) -> dict[str, Any]:
        if not self._move_lock.acquire(blocking=False):
            raise ConexError("a CONEX move is already active")
        abort = threading.Event()
        self._abort = abort
        try:
            position = _number("position_mm", args.get("position_mm"))
            tolerance = _number("tolerance_mm", args.get("tolerance_mm", 0.001), positive=True)
            timeout_s = _number("timeout_s", args.get("timeout_s", 60), positive=True)
            initial = self._readback()
            if not initial["ready"]:
                raise ConexError(f"CONEX is not ready for motion (state {initial['state']}, error {initial['positioner_error']!r})")
            if position < initial["min_mm"] or position > initial["max_mm"]:
                raise ConexError("requested position is outside the configured software limits")
            with self._sdk_lock:
                if abort.is_set():
                    raise ConexError("CONEX move was stopped before PA_Set")
                _ret, error = self._require().PA_Set(DEVICE, position, "")
                _sdk_error(error, "PA_Set")
            deadline = time.monotonic() + timeout_s
            while time.monotonic() < deadline:
                if abort.is_set():
                    raise ConexError("CONEX move was stopped")
                readback = self._readback()
                if readback["ready"] and abs(readback["position_mm"] - position) <= tolerance:
                    return readback
                time.sleep(0.05)
            raise ConexError("CONEX move timed out without confirmed ready position")
        finally:
            self._abort = None
            self._move_lock.release()

    def stop(self, args: dict[str, Any]) -> dict[str, Any]:
        timeout_s = _number("timeout_s", args.get("timeout_s", 5), positive=True)
        if timeout_s > 30:
            raise ConexError("stop timeout must be at most 30 seconds")
        abort = self._abort
        if abort is not None:
            abort.set()
        with self._sdk_lock:
            driver = self._require()
            # Newport's CommandInterfaceConexCC ST signature is (device, "").
            _ret, error = driver.ST(DEVICE, "")
            _sdk_error(error, "ST")
        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            readback = self._readback()
            if readback["ready"]:
                return readback
            time.sleep(0.05)
        raise ConexError("CONEX stop is unconfirmed: controller is not ready after ST")

    def close(self) -> None:
        if self._move_lock.locked():
            raise ConexError("cannot close while a move is active")
        with self._sdk_lock:
            driver = self._driver
            self._driver = self._port = None
            if driver is not None:
                try:
                    driver.CloseInstrument()
                except Exception as error:  # pragma: no cover - vendor SDK
                    raise ConexError(f"CONEX close failed: {error}") from error


def _reply(request_id: Any, ok: bool, result: Any = None, error: str | None = None) -> bytes:
    return (json.dumps({"id": request_id, "ok": ok, "result": result if ok else None, "error": error},
                       allow_nan=False) + "\n").encode("utf-8")


@dataclass
class ConexRequestServer(socketserver.ThreadingTCPServer):
    controller: ConexController
    token: str | None = None
    allow_reuse_address = True
    daemon_threads = True

    def __init__(self, address: tuple[str, int], controller: ConexController, token: str | None = None):
        self.controller, self.token = controller, token
        super().__init__(address, _RequestHandler)

    def dispatch(self, request: Any) -> Any:
        if not isinstance(request, dict):
            raise ConexError("request must be an object")
        if self.token is not None:
            supplied = request.get("token")
            if not isinstance(supplied, str) or not hmac.compare_digest(supplied, self.token):
                raise ConexError("unauthorized")
        if request.get("revision") != REVISION:
            raise ConexError(f"request revision must be {REVISION}")
        op, args = request.get("op"), request.get("args", {})
        if not isinstance(op, str) or not isinstance(args, dict):
            raise ConexError("op must be a string and args must be an object")
        if op == "hello":
            return {"protocol": "kohdalab-conex", "revision": REVISION, "can_stop": True, "can_home": False}
        if op == "connect": return self.controller.connect(args)
        if op == "status": return self.controller.status()
        if op == "move": return self.controller.move(args)
        if op == "stop": return self.controller.stop(args)
        raise ConexError(f"unknown operation {op!r}")


class _RequestHandler(socketserver.BaseRequestHandler):
    def handle(self) -> None:
        self.request.settimeout(5)
        data = b""
        while b"\n" not in data and len(data) <= MAX_LINE:
            piece = self.request.recv(min(4096, MAX_LINE + 1 - len(data)))
            if not piece:
                break
            data += piece
        request_id = None
        try:
            if not data or b"\n" not in data or len(data) > MAX_LINE:
                raise ConexError("request must be one newline-terminated line of at most 16384 bytes")
            line, rest = data.split(b"\n", 1)
            if rest:
                raise ConexError("exactly one request is allowed per connection")
            request = json.loads(line.decode("utf-8"))
            request_id = request.get("id") if isinstance(request, dict) else None
            result = self.server.dispatch(request)  # type: ignore[attr-defined]
            self.request.sendall(_reply(request_id, True, result))
        except Exception as error:
            self.request.sendall(_reply(request_id, False, error=str(error)))


def main() -> int:
    parser = argparse.ArgumentParser(description="KohdaLab CONEX-CC JSON-line adapter")
    parser.add_argument("--host", default="127.0.0.1", help="bind host; use 0.0.0.0 only intentionally on a lab PC")
    parser.add_argument("--sdk-dir",default=os.environ.get("CONEX_SDK_DIR"),help="folder containing Newport.CONEXCC.CommandInterface.dll on the controller PC")
    parser.add_argument("--port", type=int, default=9000)
    parser.add_argument("--token", default=os.environ.get("KOHDA_CONEX_TOKEN"), help="optional shared token (or KOHDA_CONEX_TOKEN)")
    parsed = parser.parse_args()
    server = ConexRequestServer((parsed.host, parsed.port), ConexController(sdk_directory=parsed.sdk_dir), parsed.token)
    try:
        server.serve_forever()
    finally:
        # Do not call BaseServer.shutdown from this serving thread: Python's
        # socketserver documents that as a deadlock. server_close stops new
        # accepts, then the SDK connection is explicitly released below.
        server.server_close()
        try:
            server.controller.close()
        except ConexError as error:
            print(f"CONEX shutdown warning: {error}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
