"""KohdaLab Control Protocol 1. No instrument handles, automatic retries, or dependencies."""

from __future__ import annotations

import json
import math
import os
import socket
import stat
import threading
import time
import uuid
import weakref
from dataclasses import dataclass, field
from pathlib import Path

VERSION = 1
MAX_DESCRIPTOR = 65536
MAX_RESPONSE = 64 * 1024 * 1024
READ_ONLY = frozenset(
    (
        "describe",
        "status",
        "config",
        "result",
        "plan",
        "output",
        "devices",
        "spectrum",
        "background",
        "preview",
        "shutter",
    )
)


class ControlError(RuntimeError):
    def __init__(
        self,
        message,
        *,
        code="application",
        application_code=None,
        request_id=None,
        outcome_unknown=False,
    ):
        super().__init__(message)
        self.code = code
        self.application_code = application_code or code
        self.request_id = request_id
        self.outcome_unknown = outcome_unknown


@dataclass(frozen=True)
class Endpoint:
    application: str
    host: str
    port: int
    token: str = field(repr=False)
    instance_id: str = ""
    pid: int = 0
    path: Path | None = None


def _positive(value, name):
    if (
        isinstance(value, bool)
        or not isinstance(value, (int, float))
        or not math.isfinite(value)
        or value <= 0
    ):
        raise ValueError(name + " must be a positive finite number")
    return float(value)


def _directories(application=None) -> list[Path]:
    home = os.environ.get("USERPROFILE") if os.name == "nt" else os.environ.get("HOME")
    root = (Path(home) if home else Path.home()) / ".kohdalab"
    applications = ("iv", "trkr", "camera", "shutter", "ple")
    if application is not None:
        name = application.removeprefix("kohdalab-")
        applications = (name,) if name in applications else ()
    # Legacy paths are read-only discovery fallbacks; new applications write
    # only their own run directory. Explicit directory= remains exact.
    return [root / name / "run" for name in applications] + [root / "control", root / "run"]


def load_endpoint(path) -> Endpoint:
    path = Path(path)
    with path.open("rb") as stream:
        metadata = os.fstat(stream.fileno())
        if os.name != "nt" and stat.S_IMODE(metadata.st_mode) & 0o077:
            raise ControlError(
                "Control descriptor must be private to its owner", code="authentication"
            )
        data = stream.read(MAX_DESCRIPTOR + 1)
    if len(data) > MAX_DESCRIPTOR:
        raise ControlError("Control descriptor is too large", code="validation")
    try:
        d = json.loads(data)
        if (
            not isinstance(d, dict)
            or type(d.get("control_protocol")) is not int
            or d["control_protocol"] != VERSION
        ):
            raise ValueError("unsupported protocol")
        if d.get("discovery_version") != 1 or d.get("transport") != "tcp-jsonl":
            raise ValueError("unsupported discovery transport")
        if d.get("host") not in ("127.0.0.1", "::1"):
            raise ValueError("endpoint is not loopback")
        if type(d.get("port")) is not int or not 0 < d["port"] < 65536:
            raise ValueError("invalid port")
        if type(d.get("pid")) is not int or d["pid"] <= 0:
            raise ValueError("invalid PID")
        if not isinstance(d.get("application"), str) or not d["application"].startswith(
            "kohdalab-"
        ):
            raise ValueError("invalid application")
        if not isinstance(d.get("token"), str) or not 16 <= len(d["token"]) <= 4096:
            raise ValueError("invalid token")
        uuid.UUID(d["instance_id"])
        return Endpoint(
            d["application"],
            d["host"],
            d["port"],
            d["token"],
            d["instance_id"],
            d["pid"],
            path,
        )
    except (ValueError, TypeError, KeyError) as error:
        raise ControlError(
            "Invalid KohdaLab control descriptor", code="validation"
        ) from error


def _exchange(endpoint, method, payload, timeout):
    request = dict(
        payload,
        method="control." + method,
        control_protocol=VERSION,
        token=endpoint.token,
    )
    request_id = request.get("request_id")
    encoded = (
        json.dumps(request, allow_nan=False, separators=(",", ":")).encode() + b"\n"
    )
    if len(encoded) > 1024 * 1024:
        raise ControlError(
            "Request exceeds the common protocol limit",
            code="validation",
            request_id=request_id,
        )
    try:
        deadline = time.monotonic() + timeout
        with socket.create_connection(
            (endpoint.host, endpoint.port), timeout=timeout
        ) as peer:
            peer.settimeout(max(0.001, deadline - time.monotonic()))
            peer.sendall(encoded)
            chunks = bytearray()
            while b"\n" not in chunks:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("Control response timed out")
                peer.settimeout(remaining)
                part = peer.recv(min(65536, MAX_RESPONSE + 1 - len(chunks)))
                if not part:
                    raise ConnectionError("Incomplete control response")
                chunks.extend(part)
                if len(chunks) > MAX_RESPONSE:
                    raise ControlError(
                        "Response exceeds the common protocol limit",
                        code="validation",
                        request_id=request_id,
                    )
        response = json.loads(chunks.split(b"\n", 1)[0])
    except (OSError, TimeoutError, ValueError) as error:
        raise ControlError(
            "Control response was not confirmed; inspect status/request ID before retrying",
            code="timeout" if isinstance(error, TimeoutError) else "transport",
            request_id=request_id,
            outcome_unknown=method not in READ_ONLY,
        ) from error
    if (
        not isinstance(response, dict)
        or type(response.get("control_protocol")) is not int
        or response["control_protocol"] != VERSION
        or type(response.get("ok")) is not bool
    ):
        raise ControlError(
            "Invalid common control response",
            code="validation",
            request_id=request_id,
            outcome_unknown=method not in READ_ONLY,
        )
    if response.get("request_id") != request_id:
        raise ControlError(
            "Control response request ID mismatch",
            code="validation",
            request_id=request_id,
            outcome_unknown=method not in READ_ONLY,
        )
    if not response["ok"]:
        error = response.get("error")
        if not isinstance(error, dict):
            raise ControlError(
                "Malformed control error", code="validation", request_id=request_id
            )
        raise ControlError(
            str(error.get("message", "Control request failed")),
            code=str(error.get("code", "application")),
            application_code=error.get("application_code"),
            request_id=request_id,
        )
    return response.get("result")


def discover(application=None, *, directory=None, probe_timeout=0.25):
    """Find responsive instances; never select one by process/file recency."""
    probe_timeout = _positive(probe_timeout, "probe_timeout")
    endpoints = {}
    directories = [Path(directory)] if directory is not None else _directories(application)
    paths = sorted({path for folder in directories for path in folder.glob("*.json")})
    if len(paths) > 256:
        raise ControlError(
            "Too many discovery files; select an explicit endpoint", code="validation"
        )
    for path in paths:
        try:
            endpoint = load_endpoint(path)
            if application is not None and endpoint.application != application:
                continue
            result = _exchange(endpoint, "describe", {}, probe_timeout)
            if (
                result.get("application") != endpoint.application
                or result.get("control_protocol") != VERSION
            ):
                continue
            endpoints[(endpoint.application, endpoint.instance_id)] = endpoint
        except (OSError, ControlError, ValueError, TypeError, AttributeError):
            continue
    return list(endpoints.values())


def _heartbeat(reference, stop, interval):
    while not stop.wait(interval):
        client = reference()
        if client is None:
            return
        try:
            with client._lock:
                if client._lease is None:
                    return
                client._call("heartbeat")
        except ControlError as error:
            client._heartbeat_error = error
            return
        finally:
            del client


class ControlClient:
    def __init__(self, application=None, *, endpoint=None, directory=None, timeout=3.0):
        timeout = _positive(timeout, "timeout")
        if endpoint is None:
            matches = discover(application, directory=directory)
            if len(matches) != 1:
                raise ControlError(
                    "Select an explicit endpoint: expected exactly one responsive application",
                    code="validation",
                )
            endpoint = matches[0]
        self.endpoint = (
            endpoint if isinstance(endpoint, Endpoint) else load_endpoint(endpoint)
        )
        if application is not None and self.endpoint.application != application:
            raise ControlError(
                "Endpoint belongs to another application", code="validation"
            )
        self.timeout = float(timeout)
        self.client_id = str(uuid.uuid4())
        self._lease = None
        self._lock = threading.RLock()
        self._heartbeat_stop = threading.Event()
        self._heartbeat_error = None
        self._description = None

    def _call(self, method, *, request_id=None, **params):
        if any(
            key in params
            for key in (
                "token",
                "method",
                "action",
                "client",
                "client_id",
                "lease",
                "control_protocol",
                "_control_method",
            )
        ):
            raise ControlError("Reserved control field", code="validation")
        method = method.removeprefix("control.")
        if not method or any(c not in "abcdefghijklmnopqrstuvwxyz_" for c in method):
            raise ControlError("Invalid control method", code="validation")
        if method not in READ_ONLY:
            request_id = (
                str(uuid.UUID(str(request_id)))
                if request_id is not None
                else str(uuid.uuid4())
            )
        with self._lock:
            payload = dict(params, client_id=self.client_id)
            if self._lease is not None:
                payload["lease"] = self._lease
            if request_id is not None:
                payload["request_id"] = request_id
            return _exchange(self.endpoint, method, payload, self.timeout)

    def describe(self):
        result = self._call("describe")
        if not isinstance(result, dict):
            raise ControlError("Invalid contract description", code="validation")
        if (
            result.get("application") != self.endpoint.application
            or result.get("control_protocol") != VERSION
        ):
            raise ControlError(
                "Application/contract identity mismatch", code="validation"
            )
        self._description = result
        return result

    def status(self):
        return self._call("status")

    def config(self):
        return self._call("config")

    def acquire(self, **policy):
        if self._heartbeat_error:
            raise self._heartbeat_error
        result = self._call("acquire", **policy)
        lease = result.get("lease")
        if not isinstance(lease, str) or not lease:
            raise ControlError("Acquire did not return a lease", code="validation")
        self._heartbeat_stop.set()
        self._heartbeat_stop = threading.Event()
        self._lease = lease
        interval = result.get("heartbeat_seconds", 1.0)
        if (
            isinstance(interval, bool)
            or not isinstance(interval, (float, int))
            or not math.isfinite(interval)
            or not 0.1 <= interval <= 2.0
        ):
            raise ControlError("Invalid heartbeat interval", code="validation")
        self._heartbeat_stop.clear()
        thread = threading.Thread(
            target=_heartbeat,
            args=(weakref.ref(self), self._heartbeat_stop, float(interval)),
            daemon=True,
        )
        thread.start()
        return result

    def invoke(self, method, *, request_id=None, **params):
        if self._heartbeat_error:
            raise self._heartbeat_error
        name = method.removeprefix("control.")
        description = self._description or self.describe()
        if "control." + name not in description["methods"]:
            raise ControlError(
                "Operation is not supported by this application", code="unsupported"
            )
        if name not in READ_ONLY and self._lease is None:
            self.acquire()
        return self._call(name, request_id=request_id, **params)

    def result(self, job_id):
        return self._call("result", job_id=str(job_id))

    def wait(self, job_id, *, timeout=60.0, poll_interval=0.05):
        timeout = _positive(timeout, "timeout")
        poll_interval = _positive(poll_interval, "poll_interval")
        deadline = time.monotonic() + timeout
        visited = set()
        while True:
            receipt = self.result(job_id)
            linked = receipt.get("linked_job_id")
            if linked and linked != job_id:
                if linked in visited or len(visited) >= 32:
                    raise ControlError("Receipt link cycle", code="validation")
                visited.add(str(job_id))
                job_id = str(linked)
                continue
            if receipt.get("terminal") is True:
                if receipt.get("status") == "error":
                    raise ControlError(
                        "Application job failed; inspect receipt.native for details",
                        code="application",
                        request_id=str(job_id),
                    )
                return receipt
            if time.monotonic() >= deadline:
                raise ControlError(
                    "Job wait timed out; job may still run. Use explicit stop/status.",
                    code="timeout",
                    request_id=str(job_id),
                    outcome_unknown=True,
                )
            time.sleep(min(poll_interval, max(0, deadline - time.monotonic())))

    def stop(self, **params):
        description = self._description or self.describe()
        if "control.stop" not in description["methods"]:
            raise ControlError(
                "This application has no stop operation", code="unsupported"
            )
        if (
            description["safety"].get("stop_requires_lease", True)
            and self._lease is None
        ):
            self.acquire()
        return self._call("stop", **params)

    def stop_and_wait(self, *, timeout=10.0):
        timeout = _positive(timeout, "timeout")
        self.stop()
        deadline = time.monotonic() + timeout
        while True:
            state = self.status()
            if not state["busy"] and state["cleanup"] in ("verified", "not_required"):
                return state
            if state["phase"] == "error":
                raise ControlError(
                    "Stop cleanup was not confirmed", code="cleanup_unconfirmed"
                )
            if time.monotonic() >= deadline:
                raise ControlError(
                    "Stop acknowledgement is not cleanup confirmation",
                    code="timeout",
                    outcome_unknown=True,
                )
            time.sleep(0.05)

    def release(self, *, timeout=10.0):
        timeout = _positive(timeout, "timeout")
        if self._lease is None:
            return {"released": True, "cleanup": "not_required"}
        result = self._call("release")
        # Accepted release may revoke the lease while cleanup remains pending.
        # Do not let a heartbeat to that retiring lease poison future voluntary reacquisition.
        self._heartbeat_stop.set()
        deadline = time.monotonic() + timeout
        while not (
            result.get("released") is True
            and result.get("cleanup") in ("verified", "not_required")
        ):
            state = self.status()
            if state["ownership"].get("owned") is False and state["cleanup"] in (
                "verified",
                "not_required",
            ):
                result = {
                    "released": True,
                    "cleanup": state["cleanup"],
                    "native": state,
                }
                break
            if state["phase"] == "error":
                raise ControlError(
                    "Release cleanup was not confirmed", code="cleanup_unconfirmed"
                )
            if time.monotonic() >= deadline:
                raise ControlError(
                    "Release is still pending", code="timeout", outcome_unknown=True
                )
            time.sleep(0.05)
        self._lease = None
        self._heartbeat_stop.set()
        return result

    def close(self):
        try:
            return self.release()
        finally:
            self._heartbeat_stop.set()

    def __enter__(self):
        self.acquire()
        return self

    def __exit__(self, exc_type, exc, tb):
        try:
            self.close()
        except ControlError as cleanup_error:
            if exc_type is None:
                raise
            raise cleanup_error from exc
        return False
