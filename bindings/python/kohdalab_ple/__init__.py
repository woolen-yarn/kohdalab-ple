"""Client for the local KohdaLab PLE GUI API.

The GUI retains all device ownership.  This module only attaches to its private
loopback endpoint; it never opens LightField, CONEX, or a power meter itself.
"""
from __future__ import annotations

import json
import os
import socket
import shutil
import subprocess
import threading
import time
import uuid
import weakref
from pathlib import Path
from typing import Any, Iterable

API_VERSION = 1
LEASE_TIMEOUT_S = 10.0


class GuiPLEError(RuntimeError):
    """An API error with a stable diagnostic ``code``."""
    def __init__(self, message: str, code: str = "transport") -> None:
        super().__init__(message)
        self.code = code
        self.request_id: str | None = None


class GuiPLE:
    """Attach to a running GUI, optionally acquiring its exclusive lease.

    Read-only operations work without a lease.  Mutations require :meth:`acquire`
    except ``stop``, which stays available as an emergency token-authenticated
    path.  Lost acknowledgements are never retried automatically.
    """
    def __init__(self, endpoint: str | Path | None = None, *, timeout: float = 2.0,
                 stop_on_timeout: bool = True) -> None:
        self.timeout = timeout
        self.stop_on_timeout = stop_on_timeout
        self.endpoint = self._load_endpoint(endpoint)
        self.lease: str | None = None
        self.client_id = str(uuid.uuid4())
        self.last_request_id: str | None = None
        self._closed = False
        self._heartbeat_stop = threading.Event()
        self._heartbeat_thread: threading.Thread | None = None

    @staticmethod
    def _run_directory() -> Path:
        return Path(os.environ.get("USERPROFILE") or os.environ.get("HOME") or ".") / ".kohdalab" / "run"

    @classmethod
    def _load_endpoint(cls, endpoint: str | Path | None) -> dict[str, Any]:
        if endpoint is not None:
            return cls._read_endpoint(Path(endpoint))
        candidates = []
        for path in sorted(cls._run_directory().glob("ple-gui-*.json")):
            try:
                candidate = cls._read_endpoint(path)
                # A stale descriptor is ignored only after a read-only probe;
                # never delete another process's file from a client library.
                cls._probe(candidate)
                candidates.append(candidate)
            except GuiPLEError:
                continue
        if not candidates:
            raise GuiPLEError("No running KohdaLab PLE GUI API was discovered", "discovery")
        if len(candidates) != 1:
            raise GuiPLEError("Multiple PLE GUIs found; pass endpoint= explicitly", "ambiguous_endpoint")
        return candidates[0]

    @staticmethod
    def _read_endpoint(path: Path) -> dict[str, Any]:
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError) as error:
            raise GuiPLEError(f"Cannot read PLE endpoint {path}: {error}", "discovery") from error
        required = {"host", "port", "token", "api_version", "app", "pid"}
        if not required.issubset(value) or value.get("app") != "kohdalab-ple" or value.get("api_version") != API_VERSION:
            raise GuiPLEError("Endpoint is not a compatible KohdaLab PLE GUI", "discovery")
        if value["host"] != "127.0.0.1" or not isinstance(value["port"], int) or not isinstance(value["token"], str):
            raise GuiPLEError("PLE endpoint is malformed", "discovery")
        return value

    @staticmethod
    def _probe(endpoint: dict[str, Any]) -> None:
        GuiPLE._wire(endpoint, {"action": "status"}, 0.5)

    @staticmethod
    def _wire(endpoint: dict[str, Any], payload: dict[str, Any], timeout: float) -> dict[str, Any]:
        request = {"revision": 1, "api_version": API_VERSION, "token": endpoint["token"], **payload}
        try:
            encoded=(json.dumps(request,separators=(",",":"),allow_nan=False)+"\n").encode()
        except (ValueError,TypeError) as error:
            raise GuiPLEError(f"Invalid PLE request: {error}","validation") from error
        try:
            with socket.create_connection((endpoint["host"], endpoint["port"]), timeout=timeout) as stream:
                stream.settimeout(timeout)
                stream.sendall(encoded)
                received = bytearray()
                while len(received) <= 128 * 1024 * 1024:
                    chunk = stream.recv(4096)
                    if not chunk:
                        break
                    received.extend(chunk)
                    if b"\n" in chunk:
                        break
        except OSError as error:
            raise GuiPLEError(f"PLE GUI transport: {error}", "transport") from error
        if b"\n" not in received or len(received)>128*1024*1024:
            raise GuiPLEError("Incomplete or oversized PLE reply", "transport")
        try:
            response = json.loads(bytes(received).split(b"\n", 1)[0])
        except (ValueError, IndexError) as error:
            raise GuiPLEError("Invalid response from PLE GUI", "transport") from error
        if response.get("ok") is not True:
            raise GuiPLEError(str(response.get("error", "PLE GUI request failed")), str(response.get("error_code", "unknown")))
        return response.get("result", response)

    def _call(self, action: str, *, lease: bool = False, **payload: Any) -> dict[str, Any]:
        if lease:
            if self.lease is None:
                raise GuiPLEError("Acquire exclusive PLE control before mutating it", "ownership")
            payload["lease"] = self.lease
        request_id = payload.get("request_id")
        if request_id is not None: self.last_request_id = str(request_id)
        try:
            return self._wire(self.endpoint, {"action": action, **payload}, self.timeout)
        except GuiPLEError as error:
            error.request_id = str(request_id) if request_id is not None else None
            if lease and self.stop_on_timeout and error.code in {"transport", "timeout"}:
                # Never retry uncertain motion/acquisition. Stop uses the same
                # GUI/engine and needs no lease, even after ownership loss.
                try: self._wire(self.endpoint, {"action":"stop"}, self.timeout)
                except GuiPLEError: pass
            raise

    # Read-only inspection
    def capabilities(self) -> dict[str, Any]: return self._call("capabilities")
    def status(self) -> dict[str, Any]: return self._call("status")
    def config(self) -> dict[str, Any]: return self._call("config")
    def plan(self) -> dict[str, Any]: return self._call("plan")
    def output(self) -> dict[str, Any]: return self._call("output")
    def spectrum(self) -> dict[str, Any]: return self._call("spectrum")
    def devices(self) -> dict[str, Any]: return self._call("devices")
    def background(self) -> dict[str, Any]: return self._call("background")
    def preview(self) -> dict[str, Any]: return self._call("preview")
    def shutter_status(self) -> dict[str, Any]: return self._call("shutter")
    def result(self, request_id: str) -> dict[str, Any]: return self._call("result", request_id=request_id)

    def acquire(self) -> dict[str, Any]:
        result = self._call("acquire", client_id=self.client_id)
        self.lease = str(result["lease"])
        self._start_heartbeat()
        return result

    def heartbeat(self) -> dict[str, Any]: return self._call("heartbeat", lease=True)
    def release(self) -> dict[str, Any]:
        if self.lease is None: return {"released": False}
        self.stop_and_wait()
        result = self._call("release", lease=True)
        if result.get("released") is not True:
            raise GuiPLEError("PLE cleanup is still pending; control was not released", "busy")
        self._clear_lease()
        return result

    def configure(self, config: dict[str, Any], *, request_id: str | None = None) -> dict[str, Any]: return self._call("configure", lease=True, config=config, request_id=request_id or str(uuid.uuid4()))
    def set_background(self, background: dict[str, Any], *, request_id: str | None = None) -> dict[str, Any]:
        return self._call("set_background", lease=True, background=background, request_id=request_id or str(uuid.uuid4()))
    def set_plan(self, scan: dict[str, Any], *, request_id: str | None = None) -> dict[str, Any]: return self._call("set_plan", lease=True, scan=scan, request_id=request_id or str(uuid.uuid4()))
    def set_output(self, *, directory: str | None = None, sample: str | None = None, request_id: str | None = None) -> dict[str, Any]:
        return self._call("set_output", lease=True, request_id=request_id or str(uuid.uuid4()), **{k:v for k,v in {"directory":directory,"sample":sample}.items() if v is not None})
    def apply_spectrometer(self, settings: dict[str, Any], *, request_id: str | None = None) -> dict[str, Any]:
        """Apply an allowed connected-spectrometer patch as an asynchronous job."""
        if not isinstance(settings, dict):
            raise GuiPLEError("settings must be a dictionary", "validation")
        return self._call("apply_spectrometer", lease=True, settings=settings,
                          request_id=request_id or str(uuid.uuid4()))
    def connect(self, devices: Iterable[str] = ("spectrometer", "stage", "power"), *, request_id: str | None = None) -> dict[str, Any]:
        return self._call("connect", lease=True, devices=list(devices), request_id=request_id or str(uuid.uuid4()))
    def disconnect(self, devices: Iterable[str] = ("spectrometer", "stage", "power"), *, request_id: str | None = None) -> dict[str, Any]:
        return self._call("disconnect", lease=True, devices=list(devices), request_id=request_id or str(uuid.uuid4()))
    def start(self, *, request_id: str | None = None) -> dict[str, Any]:
        return self._call("start", lease=True, request_id=request_id or str(uuid.uuid4()))
    def move(self, position_mm: float, *, request_id: str | None = None) -> dict[str, Any]:
        return self._call("move", lease=True, position_mm=position_mm, request_id=request_id or str(uuid.uuid4()))
    def confirm_background(self, confirmed: bool, *, request_id: str | None = None) -> dict[str, Any]: return self._call("confirm_background", lease=True, confirmed=confirmed, request_id=request_id or str(uuid.uuid4()))
    def confirm_signal(self, confirmed: bool, *, request_id: str | None = None) -> dict[str, Any]: return self._call("confirm_signal", lease=True, confirmed=confirmed, request_id=request_id or str(uuid.uuid4()))
    def select_analysis(self, analysis: dict[str, Any], *, excitation: dict[str, Any] | None = None, request_id: str | None = None) -> dict[str, Any]:
        allowed={"excitation_method","excitation_min_nm","excitation_max_nm","excitation_min_snr","excitation_peaks"}
        if excitation and not set(excitation).issubset(allowed):
            raise GuiPLEError("Use Config excitation field names", "validation")
        return self._call("select_analysis", lease=True, analysis=analysis, request_id=request_id or str(uuid.uuid4()), **(excitation or {}))
    def stop(self) -> dict[str, Any]: return self._call("stop")

    def wait(self, request_id: str, *, timeout: float | None = None, poll_s: float = 0.1) -> dict[str, Any]:
        deadline = None if timeout is None else time.monotonic() + timeout
        while True:
            receipt = self.result(request_id)
            state = receipt.get("status")
            if state is None:
                raise GuiPLEError("This is a command acknowledgement; wait for its run_job instead", "validation")
            if state == "completed": return receipt
            if state in {"stopped", "error", "interrupted_unconfirmed"}:
                original = GuiPLEError(str(receipt.get("error") or state), str(receipt.get("error_code") or state))
                try: self.stop_and_wait()
                except GuiPLEError as cleanup:
                    raise GuiPLEError(f"{original}; cleanup failed: {cleanup}", cleanup.code) from original
                raise original
            if deadline is not None and time.monotonic() >= deadline:
                if self.stop_on_timeout:
                    self.stop_and_wait()
                raise GuiPLEError("PLE job wait timed out", "timeout")
            time.sleep(poll_s)

    def stop_and_wait(self, *, timeout: float = 15.0, poll_s: float = 0.1) -> dict[str, Any]:
        requested = self.stop()
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            snapshot = self.status()
            if not snapshot.get("busy", False) and not snapshot.get("cleanup_pending", False):
                # Hardware state must be explicit: interrupted/unconfirmed is
                # never rewritten as a safe stopped result by this client.
                if snapshot.get("cleanup_error"):
                    raise GuiPLEError(str(snapshot["cleanup_error"]), "interrupted_unconfirmed")
                return {"requested": requested, "snapshot": snapshot}
            time.sleep(poll_s)
        raise GuiPLEError("PLE stop cleanup timed out; inspect hardware state", "timeout")

    def _start_heartbeat(self) -> None:
        if self._heartbeat_thread and self._heartbeat_thread.is_alive(): return
        owner = weakref.ref(self)
        stop = self._heartbeat_stop
        stop.clear()
        def run() -> None:
            while not stop.wait(LEASE_TIMEOUT_S / 3):
                client = owner()
                if client is None or client.lease is None: return
                try: client.heartbeat()
                except GuiPLEError: return
                finally: del client
        self._heartbeat_thread = threading.Thread(target=run, name="ple-gui-heartbeat", daemon=True)
        self._heartbeat_thread.start()

    def _clear_lease(self) -> None:
        self.lease = None; self._heartbeat_stop.set()

    def close(self) -> None:
        if self._closed: return
        self._closed = True
        try:
            if self.lease is not None: self.release()
        finally:
            self._clear_lease()

    def __enter__(self) -> "GuiPLE": self.acquire(); return self
    def __exit__(self, *_: Any) -> None: self.close()

    @classmethod
    def launch(cls, executable: str | Path | None = None, *, profile: str | Path | None = None,
               endpoint_directory: str | Path | None = None, timeout: float = 10.0) -> "GuiPLE":
        executable = find_binary(executable)
        directory = Path(endpoint_directory or cls._run_directory())
        directory.mkdir(parents=True, exist_ok=True)
        command = [str(executable), "--gui-api", "--api-dir", str(directory)]
        if profile is not None: command += ["--config", str(profile)]
        process = subprocess.Popen(command)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for path in directory.glob(f"ple-gui-{process.pid}-*.json"):
                try:
                    client = cls(path)
                    cls._probe(client.endpoint)
                    return client
                except GuiPLEError: pass
            if process.poll() is not None: raise GuiPLEError("PLE GUI failed to launch", "launch")
            time.sleep(0.05)
        process.terminate()
        raise GuiPLEError("Timed out waiting for PLE GUI API", "launch")

    def __del__(self) -> None:
        if hasattr(self,"_heartbeat_stop"): self._clear_lease()



def find_binary(executable: str | Path | None = None) -> Path:
    """Resolve an installed/portable GUI without starting it or probing devices."""
    explicit = executable if executable is not None else os.environ.get("KOHDALAB_PLE_EXE")
    def normalize(path: Path) -> Path:
        path = path.expanduser()
        if path.suffix == ".app": return path / "Contents/MacOS/kohdalab-ple"
        if path.is_dir(): return path / ("kohdalab-ple.exe" if os.name == "nt" else "kohdalab-ple")
        return path
    if explicit is not None:
        candidate = normalize(Path(explicit)).resolve()
        if not candidate.is_file(): raise GuiPLEError(f"PLE executable not found: {candidate}", "discovery")
        return candidate
    candidates = []
    found = shutil.which("kohdalab-ple")
    if found: candidates.append(Path(found))
    program_files = os.environ.get("ProgramW6432") or os.environ.get("ProgramFiles")
    if program_files:
        candidates.append(Path(program_files) / "Kohdalab/KohdaLab PLE/kohdalab-ple.exe")
    roots = [Path.cwd(), Path(__file__).resolve().parents[3]]
    for root in roots:
        candidates.extend(root / name for name in ("kohdalab-ple.exe", "kohdalab-ple", "target/release/kohdalab-ple.exe", "target/release/kohdalab-ple", "dist/kohdalab-ple-windows/kohdalab-ple.exe", "dist/KohdaLab PLE.app/Contents/MacOS/kohdalab-ple"))
    candidates.extend(Path.home() / folder / "KohdaLab PLE.app/Contents/MacOS/kohdalab-ple" for folder in ("Applications",))
    candidates.append(Path("/Applications/KohdaLab PLE.app/Contents/MacOS/kohdalab-ple"))
    for candidate in candidates:
        if candidate.is_file(): return candidate.resolve()
    raise GuiPLEError("No PLE executable found; pass executable= or set KOHDALAB_PLE_EXE", "discovery")


# Common GUI-client naming used across KohdaLab applications.
GuiClient = GuiPLE
