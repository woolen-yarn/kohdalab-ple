#!/usr/bin/env python3
"""Conservative TCP adapter for a Newport CONEX-CC on a Windows lab PC.

It intentionally never homes (``OR``) or enables a positioner (``MM_Set``).
The controller must be made safe and ready by the operator before a move.
"""
from __future__ import annotations

import argparse
import hmac
import ipaddress
import json
import math
import os
import sys
from pathlib import Path
import socket
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


# Cross-process serial-port lease shared with TRKR ``Port::open``
# (trkr-motion: ``kohdalab-motion-<hex>.lock`` in Rust ``std::env::temp_dir()``,
# locked with fs2 ``try_lock_exclusive``). Every detail below mirrors that
# implementation so that both applications exclude each other on one host.

# Rust ``str::trim`` removes Unicode White_Space; Python ``str.strip`` would
# also remove U+001C..U+001F, so the exact set is spelled out.
_RUST_WHITESPACE = "".join(map(chr, [*range(0x09, 0x0E), 0x20, 0x85, 0xA0, 0x1680, *range(0x2000, 0x200B),
                                      0x2028, 0x2029, 0x202F, 0x205F, 0x3000]))
_ASCII_UPPER = str.maketrans("abcdefghijklmnopqrstuvwxyz", "ABCDEFGHIJKLMNOPQRSTUVWXYZ")


def serial_lock_key(port: str) -> str:
    """Hex key equal to TRKR's ``name.trim().to_ascii_uppercase().replace("\\\\.\\", "")``."""
    normalized = port.strip(_RUST_WHITESPACE).translate(_ASCII_UPPER).replace("\\\\.\\", "")
    try:
        return normalized.encode("utf-8").hex()
    except UnicodeEncodeError as error:
        raise ConexError("com_port must be valid Unicode text") from error


def host_temp_dir() -> str:
    """The directory Rust ``std::env::temp_dir()`` returns for this process."""
    if os.name == "nt":
        return _windows_temp_dir()
    return os.environ.get("TMPDIR", "/tmp")


def serial_lock_path(port: str, directory: str | None = None) -> str:
    return os.path.join(host_temp_dir() if directory is None else directory,
                        f"kohdalab-motion-{serial_lock_key(port)}.lock")


if os.name == "nt":  # pragma: no cover - exercised on Windows only
    import ctypes
    import msvcrt
    from ctypes import wintypes

    _LOCKFILE_FAIL_IMMEDIATELY, _LOCKFILE_EXCLUSIVE_LOCK = 0x1, 0x2
    _ERROR_LOCK_VIOLATION = 33

    class _OVERLAPPED(ctypes.Structure):
        _fields_ = [("Internal", ctypes.c_size_t), ("InternalHigh", ctypes.c_size_t),
                    ("Offset", wintypes.DWORD), ("OffsetHigh", wintypes.DWORD),
                    ("hEvent", wintypes.HANDLE)]

    _kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _kernel32.LockFileEx.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD,
                                     wintypes.DWORD, ctypes.POINTER(_OVERLAPPED)]
    _kernel32.LockFileEx.restype = wintypes.BOOL
    _kernel32.UnlockFileEx.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD,
                                       ctypes.POINTER(_OVERLAPPED)]
    _kernel32.UnlockFileEx.restype = wintypes.BOOL

    def _windows_temp_dir() -> str:
        # Rust std uses GetTempPath2W and falls back to GetTempPathW.
        get = getattr(_kernel32, "GetTempPath2W", None) or _kernel32.GetTempPathW
        get.argtypes, get.restype = [wintypes.DWORD, wintypes.LPWSTR], wintypes.DWORD
        size = 261
        while True:
            buffer = ctypes.create_unicode_buffer(size)
            length = get(size, buffer)
            if length == 0:
                raise ConexError(f"could not resolve the temporary directory: {ctypes.WinError(ctypes.get_last_error())}")
            if length < size:
                return buffer[:length]
            size = length + 1

    def _try_lock(fd: int) -> bool:
        # Same call as fs2 try_lock_exclusive: whole range from offset 0.
        handle = msvcrt.get_osfhandle(fd)
        if _kernel32.LockFileEx(handle, _LOCKFILE_EXCLUSIVE_LOCK | _LOCKFILE_FAIL_IMMEDIATELY,
                                0, 0xFFFFFFFF, 0xFFFFFFFF, ctypes.byref(_OVERLAPPED())):
            return True
        code = ctypes.get_last_error()
        if code == _ERROR_LOCK_VIOLATION:
            return False
        raise OSError(code, ctypes.FormatError(code))

    def _unlock(fd: int) -> None:
        _kernel32.UnlockFileEx(msvcrt.get_osfhandle(fd), 0, 0xFFFFFFFF, 0xFFFFFFFF, ctypes.byref(_OVERLAPPED()))
else:
    import errno
    import fcntl

    def _windows_temp_dir() -> str:  # pragma: no cover - not reachable off Windows
        raise ConexError("Windows temporary directory requested on a non-Windows host")

    def _try_lock(fd: int) -> bool:
        # Same call as fs2 try_lock_exclusive on Unix.
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return True
        except OSError as error:
            if error.errno in (errno.EWOULDBLOCK, errno.EAGAIN):
                return False
            raise

    def _unlock(fd: int) -> None:
        fcntl.flock(fd, fcntl.LOCK_UN)


class SerialPortLease:
    """Exclusive OS lock on the shared KohdaLab lock file for one serial port.

    The lock file is never removed; only the lock is released.
    """
    def __init__(self, port: str, path: str, fd: int) -> None:
        self.port, self.path, self._fd = port, path, fd

    @classmethod
    def acquire(cls, port: str, directory: str | None = None) -> "SerialPortLease":
        path = serial_lock_path(port, directory)
        try:
            # Like Rust OpenOptions read+write+create without truncation.
            # Python descriptors are non-inheritable by default.
            fd = os.open(path, os.O_RDWR | os.O_CREAT | getattr(os, "O_BINARY", 0))
        except OSError as error:
            raise ConexError(f"could not open serial-port lock {path}: {error}") from error
        try:
            locked = _try_lock(fd)
        except OSError as error:
            os.close(fd)
            raise ConexError(f"could not lock serial port {port}: {error}") from error
        if not locked:
            os.close(fd)
            raise ConexError(f"Serial port {port} is already used by another measurement process")
        return cls(port, path, fd)

    def release(self) -> None:
        fd, self._fd = self._fd, None
        if fd is None:
            return
        try:
            _unlock(fd)
        finally:
            os.close(fd)


class ConexController:
    """A single SDK instance with short lock-held calls only."""
    def __init__(self, driver_factory: Callable[[], Any] | None = None, sdk_directory: str | None = None,
                 lock_directory: str | None = None) -> None:
        self._factory = driver_factory
        self._sdk_directory = sdk_directory or os.environ.get("CONEX_SDK_DIR")
        self._lock_directory = lock_directory
        self._driver: Any = None
        self._port: str | None = None
        # Held from before the SDK opens the port until CloseInstrument is
        # confirmed; retained (until process exit) when that is unknown.
        self._lease: SerialPortLease | None = None
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
        port = args.get("com_port")
        if not isinstance(port, str) or not port.strip():
            raise ConexError("com_port must be a non-empty string")
        port = port.strip()
        velocity = None
        if "velocity_mm_s" in args and args["velocity_mm_s"] is not None:
            velocity = _number("velocity_mm_s", args["velocity_mm_s"], positive=True)
        # Admission is serialized from the driver/lease checks through state
        # publication and the initial readback, so concurrent requests
        # (ThreadingTCPServer) for different ports cannot both pass the checks
        # and overwrite each other's driver and lease. The RLock lets
        # _readback re-enter; no other lock is acquired while it is held.
        with self._sdk_lock:
            if self._move_lock.locked():
                raise ConexError("cannot connect while a move or close is active")
            if self._driver is not None:
                if port != self._port:
                    raise ConexError(f"CONEX is already open on {self._port}")
                return self._readback()
            if self._lease is not None:
                raise ConexError(f"the previous CONEX session on {self._lease.port} did not close cleanly; "
                                 "its serial-port lock is kept until this bridge process exits")
            # Fail closed before the vendor SDK is loaded or touches the port.
            lease = SerialPortLease.acquire(port, self._lock_directory)
            try:
                driver = self._new_driver()
            except BaseException:
                lease.release()
                raise
            try:
                result = driver.OpenInstrument(port)
            except BaseException:
                self._lease = lease  # open outcome unknown: keep the port locked
                raise
            if result != 0:
                lease.release()
                raise ConexError(f"could not open CONEX port {port} (code {result})")
            self._driver, self._port, self._lease = driver, port, lease
            try:
                if velocity is not None:
                    _ret, error = driver.VA_Set(DEVICE, velocity, "")
                    _sdk_error(error, "VA_Set")
                return self._readback()
            except Exception as error:
                # Still under the lock: a move that started meanwhile is
                # blocked before its initial readback completes and cannot
                # have commanded motion, so closing here is safe.
                try:
                    self._close_locked()
                except ConexError as close_error:
                    raise ConexError(f"{error}; {close_error}") from error
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
        # Reserve the move slot for the whole close (same order as move:
        # _move_lock, then _sdk_lock) so a move cannot be admitted between
        # the check and the port release.
        if not self._move_lock.acquire(blocking=False):
            raise ConexError("cannot close while a move is active")
        try:
            with self._sdk_lock:
                self._close_locked()
        finally:
            self._move_lock.release()

    def _close_locked(self) -> None:
        """Close the published session; the caller holds ``_sdk_lock``."""
        driver = self._driver
        self._driver = self._port = None
        if driver is None:
            # Nothing confirmed closed; an orphaned lease stays held.
            return
        # On any close failure the lease is retained: the port may still
        # be open, so another process must not take it.
        try:
            result = driver.CloseInstrument()
        except Exception as error:
            raise ConexError(f"CONEX close failed: {error}; serial-port lock retained") from error
        if result not in (None, 0):
            raise ConexError(f"CONEX close failed (code {result}); serial-port lock retained")
        lease, self._lease = self._lease, None
        if lease is not None:
            lease.release()


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
            if not isinstance(supplied, str) or not hmac.compare_digest(supplied.encode("utf-8"), self.token.encode("utf-8")):
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
    parser.add_argument("--token", default=os.environ.get("KOHDA_CONEX_TOKEN"), help="shared token required for non-loopback binds (or KOHDA_CONEX_TOKEN)")
    parsed = parser.parse_args()
    host = parsed.host
    token = parsed.token if parsed.token and parsed.token.strip() else None
    if token is None:
        # Bind to the exact IPv4 address checked here so a hostname is not
        # resolved a second time by the server.
        try:
            host = socket.gethostbyname(host)
        except OSError as error:
            parser.error(f"could not resolve --host {host!r}: {error}")
        if not ipaddress.IPv4Address(host).is_loopback:
            parser.error("non-loopback --host requires --token or KOHDA_CONEX_TOKEN")
    server = ConexRequestServer((host, parsed.port), ConexController(sdk_directory=parsed.sdk_dir), token)
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
