import json
import socket
import tempfile
import threading
import unittest
import uuid
from pathlib import Path
from unittest.mock import patch

from kohdalab_control import (
    ControlClient,
    ControlError,
    Endpoint,
    discover,
    load_endpoint,
)
import kohdalab_control as control


class DirectoryLayoutTests(unittest.TestCase):
    def test_home_selection_prefers_native_nonempty_value_and_falls_back(self):
        # A rooted path without a drive is not absolute on Windows.
        fallback = Path(__file__).resolve().parent / "fallback"
        self.assertTrue(fallback.is_absolute())
        self.assertEqual(
            control._home_directory(
                "unix-home", "windows-home", False, lambda: fallback
            ),
            Path("unix-home"),
        )
        self.assertEqual(
            control._home_directory(
                "unix-home", "windows-home", True, lambda: fallback
            ),
            Path("windows-home"),
        )
        for windows in (False, True):
            self.assertEqual(
                control._home_directory("", "profile", windows, lambda: fallback),
                Path("profile"),
            )
            self.assertEqual(
                control._home_directory("home", "", windows, lambda: fallback),
                Path("home"),
            )
            self.assertEqual(
                control._home_directory("", "", windows, lambda: fallback), fallback
            )

    def test_home_fallback_is_lazy_and_never_allows_relative_discovery_root(self):
        def unavailable():
            raise AssertionError("fallback should not be evaluated")

        self.assertEqual(
            control._home_directory("", "profile", False, unavailable),
            Path("profile"),
        )
        with self.assertRaises(ControlError) as caught:
            control._home_directory("", "", False, lambda: Path("."))
        self.assertEqual(caught.exception.code, "validation")

        with (
            patch.dict(
                control.os.environ, {"HOME": "", "USERPROFILE": "/profile"}, clear=False
            ),
            patch.object(control.Path, "home", side_effect=unavailable),
        ):
            self.assertEqual(
                control._directories("kohdalab-iv"),
                [
                    Path("/profile/.kohdalab/iv/run"),
                    Path("/profile/.kohdalab/control"),
                    Path("/profile/.kohdalab/run"),
                ],
            )
        with (
            patch.dict(
                control.os.environ, {"HOME": "", "USERPROFILE": ""}, clear=False
            ),
            patch.object(control.Path, "home", return_value=Path(".")),
        ):
            with self.assertRaises(ControlError) as caught:
                control._directories("kohdalab-iv")
        self.assertEqual(caught.exception.code, "validation")

    def test_app_owned_run_discovery_and_explicit_override(self):
        import os
        import kohdalab_control as control

        with (
            tempfile.TemporaryDirectory() as tmp,
            patch.dict(os.environ, {"HOME": tmp, "USERPROFILE": tmp}),
        ):
            root = Path(tmp) / ".kohdalab"
            iv = root / "iv/run/iv.json"
            trkr = root / "trkr/run/trkr.json"
            explicit = Path(tmp) / "custom/other.json"
            for path in (iv, trkr, explicit):
                path.parent.mkdir(parents=True)
                path.write_text("{}")

            def endpoint(path):
                name = "trkr" if path == trkr else "iv"
                return Endpoint(
                    "kohdalab-" + name, "127.0.0.1", 1, "private", str(path), 1, path
                )

            def describe(endpoint, *args):
                return {"application": endpoint.application, "control_protocol": 1}

            with (
                patch.object(control, "load_endpoint", side_effect=endpoint),
                patch.object(control, "_exchange", side_effect=describe),
            ):
                self.assertEqual({e.path for e in discover()}, {iv, trkr})
                self.assertEqual([e.path for e in discover("kohdalab-iv")], [iv])
                self.assertEqual(
                    [e.path for e in discover(directory=explicit.parent)], [explicit]
                )
            self.assertFalse((root / "control").exists())
            self.assertFalse((root / "iv/config").exists())

    def test_slm_run_directory_is_discovered_read_only(self):
        import os

        with (
            tempfile.TemporaryDirectory() as tmp,
            patch.dict(os.environ, {"HOME": tmp, "USERPROFILE": tmp}),
        ):
            root = Path(tmp) / ".kohdalab"
            self.assertEqual(
                control._directories("kohdalab-slm"),
                [root / "slm/run", root / "control", root / "run"],
            )
            self.assertIn(root / "slm/run", control._directories())
            # Resolving directories performs no I/O.
            self.assertFalse(root.exists())
            slm = root / "slm/run/slm.json"
            slm.parent.mkdir(parents=True)
            slm.write_text("{}")
            before = sorted(p.relative_to(root) for p in root.rglob("*"))
            calls = []

            def endpoint(path):
                return Endpoint(
                    "kohdalab-slm", "127.0.0.1", 1, "private", str(path), 1, path
                )

            def describe(endpoint, method, params, timeout):
                calls.append(method)
                return {"application": endpoint.application, "control_protocol": 1}

            with (
                patch.object(control, "load_endpoint", side_effect=endpoint),
                patch.object(control, "_exchange", side_effect=describe),
            ):
                self.assertEqual([e.path for e in discover("kohdalab-slm")], [slm])
                self.assertEqual([e.path for e in discover()], [slm])
            # Only the read-only describe probe is sent; nothing is created.
            self.assertEqual(set(calls), {"describe"})
            self.assertIn("describe", control.READ_ONLY)
            self.assertEqual(
                sorted(p.relative_to(root) for p in root.rglob("*")), before
            )
            self.assertEqual(slm.read_text(), "{}")


class BoundaryTests(unittest.TestCase):
    def endpoint(self, port=1):
        return Endpoint(
            "kohdalab-test", "127.0.0.1", port, str(uuid.uuid4()), str(uuid.uuid4()), 1
        )

    def test_descriptor_rejects_nonlocal_invalid_future_and_never_prints_token(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "test.json"
            d = {
                "application": "kohdalab-test",
                "control_protocol": 1,
                "discovery_version": 1,
                "transport": "tcp-jsonl",
                "host": "127.0.0.1",
                "port": 123,
                "pid": 1,
                "token": str(uuid.uuid4()),
                "instance_id": str(uuid.uuid4()),
            }
            p.write_text(json.dumps(d))
            p.chmod(0o600)
            e = load_endpoint(p)
            self.assertNotIn(d["token"], repr(e))
            for key, value in [
                ("host", "192.168.1.1"),
                ("control_protocol", 2),
                ("port", True),
                ("pid", -1),
                ("instance_id", "bad"),
            ]:
                bad = dict(d)
                bad[key] = value
                p.write_text(json.dumps(bad))
                with self.assertRaises(ControlError):
                    load_endpoint(p)

    def test_socket_contract_identity_envelope_and_error_preserved(self):
        server = socket.socket()
        server.bind(("127.0.0.1", 0))
        server.listen(1)
        endpoint = self.endpoint(server.getsockname()[1])
        captured = []

        def serve():
            try:
                peer, _ = server.accept()
                with peer:
                    request = json.loads(peer.makefile("rb").readline())
                    captured.append(request)
                    peer.sendall(
                        json.dumps(
                            {
                                "control_protocol": 1,
                                "ok": False,
                                "request_id": request.get("request_id"),
                                "result": None,
                                "error": {
                                    "code": "busy",
                                    "application_code": "busy",
                                    "message": "Operation pending",
                                    "retryable": False,
                                },
                            }
                        ).encode()
                        + b"\n"
                    )
            finally:
                server.close()

        thread = threading.Thread(target=serve)
        thread.start()
        client = ControlClient(endpoint=endpoint)
        with self.assertRaises(ControlError) as cm:
            client.status()
        thread.join(2)
        self.assertFalse(thread.is_alive())
        self.assertEqual(cm.exception.code, "busy")
        self.assertEqual(captured[0]["method"], "control.status")
        self.assertEqual(captured[0]["control_protocol"], 1)

    def test_exchange_accepts_a_response_fragmented_before_its_newline(self):
        server = socket.socket()
        server.bind(("127.0.0.1", 0))
        server.listen(1)
        endpoint = self.endpoint(server.getsockname()[1])

        def serve():
            try:
                peer, _ = server.accept()
                with peer:
                    request = json.loads(peer.makefile("rb").readline())
                    response = json.dumps(
                        {
                            "control_protocol": 1,
                            "ok": True,
                            "request_id": request.get("request_id"),
                            "result": {"received": True},
                        }
                    ).encode()
                    peer.sendall(response[:7])
                    peer.sendall(response[7:])
                    peer.sendall(b"\n")
            finally:
                server.close()

        thread = threading.Thread(target=serve)
        thread.start()
        self.assertEqual(
            control._exchange(endpoint, "status", {"request_id": "fragmented"}, 1),
            {"received": True},
        )
        thread.join(2)
        self.assertFalse(thread.is_alive())

    def test_exchange_preserves_unterminated_and_response_limit_errors(self):
        class Peer:
            def settimeout(self, _timeout):
                pass

            def sendall(self, _data):
                pass

            def recv(self, size):
                return b"x" * size

            def __enter__(self):
                return self

            def __exit__(self, *_):
                pass

        with patch("kohdalab_control.socket.create_connection", return_value=Peer()):
            with self.assertRaises(ControlError) as caught:
                control._exchange(self.endpoint(), "status", {"request_id": "limit"}, 1)
        self.assertEqual(caught.exception.code, "validation")

        class IncompletePeer(Peer):
            def recv(self, _size):
                self.calls = getattr(self, "calls", 0) + 1
                return b"{}" if self.calls == 1 else b""

        with patch(
            "kohdalab_control.socket.create_connection", return_value=IncompletePeer()
        ):
            with self.assertRaises(ControlError) as caught:
                control._exchange(
                    self.endpoint(), "status", {"request_id": "incomplete"}, 1
                )
        self.assertEqual(caught.exception.code, "transport")

    def test_invalid_wait_or_stop_timeout_has_no_side_effect(self):
        client = ControlClient(endpoint=self.endpoint())
        with (
            patch.object(client, "_call") as call,
            patch.object(client, "stop") as stop,
        ):
            for invalid in [-1, 0, float("nan"), float("inf"), True]:
                with self.assertRaises(ValueError):
                    client.stop_and_wait(timeout=invalid)
                with self.assertRaises(ValueError):
                    client.wait("job", timeout=invalid)
                with self.assertRaises(ValueError):
                    client.release(timeout=invalid)
            call.assert_not_called()
            stop.assert_not_called()

    def test_wait_follows_explicit_link_not_latest_and_ack_is_not_run_completion(self):
        client = ControlClient(endpoint=self.endpoint())
        with patch.object(
            client,
            "result",
            side_effect=[
                {
                    "job_id": "ack",
                    "terminal": True,
                    "status": "completed",
                    "linked_job_id": "run",
                },
                {"job_id": "run", "terminal": False, "status": "running"},
                {"job_id": "run", "terminal": True, "status": "completed"},
            ],
        ) as result:
            done = client.wait("ack", poll_interval=0.001)
            self.assertEqual(done["job_id"], "run")
            self.assertEqual(
                [c.args[0] for c in result.call_args_list], ["ack", "run", "run"]
            )

    def test_stop_ack_does_not_prove_cleanup(self):
        client = ControlClient(endpoint=self.endpoint())
        states = [
            {"busy": False, "phase": "stopping", "cleanup": "pending"},
            {"busy": False, "phase": "ready", "cleanup": "verified"},
        ]
        with (
            patch.object(client, "stop", return_value={"requested": True}),
            patch.object(client, "status", side_effect=states),
        ):
            self.assertEqual(client.stop_and_wait()["cleanup"], "verified")

    def test_stop_and_wait_without_job_keeps_global_stop_for_existing_callers(self):
        client = ControlClient(endpoint=self.endpoint())
        with (
            patch.object(client, "stop", return_value={"requested": True}) as stop,
            patch.object(client, "result") as result,
            patch.object(
                client,
                "status",
                return_value={"busy": False, "phase": "ready", "cleanup": "verified"},
            ),
        ):
            client.stop_and_wait(timeout=1)
        stop.assert_called_once_with()
        result.assert_not_called()

    def test_stop_and_wait_targets_explicit_job_receipt_not_global_status(self):
        client = ControlClient(endpoint=self.endpoint())
        receipts = [
            {"job_id": "run", "terminal": False, "status": "running",
             "cleanup": "pending"},
            {"job_id": "run", "terminal": True, "status": "stopped",
             "cleanup": "verified"},
        ]
        with (
            patch.object(client, "stop", return_value={"requested": True}) as stop,
            patch.object(client, "result", side_effect=receipts) as result,
            patch.object(client, "status") as status,
        ):
            done = client.stop_and_wait("run", poll_interval=0.001)
        self.assertEqual(done["status"], "stopped")
        stop.assert_called_once_with(job_id="run")
        self.assertEqual([c.args[0] for c in result.call_args_list], ["run", "run"])
        status.assert_not_called()

    def test_stop_and_wait_sends_job_id_on_the_wire(self):
        client = ControlClient(endpoint=self.endpoint())
        client._description = {
            "methods": ["control.stop", "control.result"],
            "safety": {"stop_requires_lease": True},
        }
        client._lease = "opaque-test-lease"
        sent = []

        def exchange(_endpoint, method, payload, _timeout):
            sent.append((method, payload))
            if method == "stop":
                return {"requested": False, "job_id": "run", "cleanup": "verified"}
            return {"job_id": "run", "terminal": True, "status": "completed",
                    "cleanup": "verified"}

        with patch("kohdalab_control._exchange", side_effect=exchange):
            self.assertTrue(client.stop_and_wait("run")["terminal"])
        self.assertEqual([m for m, _ in sent], ["stop", "result"])
        self.assertEqual(sent[0][1]["job_id"], "run")
        self.assertEqual(sent[0][1]["lease"], "opaque-test-lease")
        uuid.UUID(sent[0][1]["request_id"])
        self.assertEqual(sent[1][1]["job_id"], "run")

    def test_stop_and_wait_job_rejects_unconfirmed_mismatched_or_late_receipts(self):
        client = ControlClient(endpoint=self.endpoint())
        cases = [
            ({"job_id": "run", "terminal": True, "status": "error",
              "cleanup": "unknown"}, "cleanup_unconfirmed"),
            ({"job_id": "run", "terminal": True, "status": "stopped",
              "cleanup": "pending"}, "cleanup_unconfirmed"),
            ({"job_id": "other", "terminal": True, "status": "stopped",
              "cleanup": "verified"}, "validation"),
            ({"job_id": "run", "terminal": False, "status": "running",
              "cleanup": "pending"}, "timeout"),
        ]
        for receipt, code in cases:
            with (
                self.subTest(code=code, receipt=receipt),
                patch.object(client, "stop", return_value={"requested": True}),
                patch.object(client, "result", return_value=receipt),
            ):
                with self.assertRaises(ControlError) as caught:
                    client.stop_and_wait("run", timeout=0.01, poll_interval=0.001)
                self.assertEqual(caught.exception.code, code)
                self.assertEqual(caught.exception.request_id, "run")

    def test_stop_and_wait_completed_start_ack_follows_link_to_acquisition(self):
        client = ControlClient(endpoint=self.endpoint())
        ack = {"job_id": "start", "terminal": True, "status": "completed",
               "cleanup": "verified", "linked_job_id": "acq"}
        receipts = [
            ack,
            {"job_id": "acq", "terminal": False, "status": "running",
             "cleanup": "pending"},
            {"job_id": "acq", "terminal": True, "status": "stopped",
             "cleanup": "verified"},
        ]
        with (
            patch.object(client, "stop", return_value={"requested": True}) as stop,
            patch.object(client, "result", side_effect=receipts) as result,
            patch.object(client, "status") as status,
            patch.object(client, "_call") as call,
        ):
            done = client.stop_and_wait("start", poll_interval=0.001)
        self.assertEqual(done["job_id"], "acq")
        self.assertEqual(done["status"], "stopped")
        stop.assert_called_once_with(job_id="start")
        self.assertEqual(
            [c.args[0] for c in result.call_args_list], ["start", "acq", "acq"]
        )
        status.assert_not_called()
        call.assert_not_called()

    def test_stop_and_wait_start_ack_alone_never_proves_cleanup(self):
        client = ControlClient(endpoint=self.endpoint())
        ack = {"job_id": "start", "terminal": True, "status": "completed",
               "cleanup": "verified", "linked_job_id": "acq"}
        pending = {"job_id": "acq", "terminal": False, "status": "running",
                   "cleanup": "pending"}
        cases = [
            ([ack], pending, "timeout", "acq"),
            ([ack], {"job_id": "other", "terminal": True, "status": "stopped",
                     "cleanup": "verified"}, "validation", "acq"),
            ([ack], {"terminal": True, "status": "stopped",
                     "cleanup": "verified"}, "validation", "acq"),
            ([ack], {"job_id": "acq", "terminal": True, "status": "stopped",
                     "cleanup": "pending"}, "cleanup_unconfirmed", "acq"),
            ([ack], {"job_id": "acq", "terminal": True, "status": "completed",
                     "cleanup": "verified", "linked_job_id": "start"},
             "validation", "acq"),
            ([dict(ack, linked_job_id=7)], pending, "validation", "start"),
        ]
        for first, then, code, rid in cases:
            receipts = iter(first)
            with (
                self.subTest(code=code, then=then),
                patch.object(client, "stop", return_value={"requested": True}) as stop,
                patch.object(client, "result",
                             side_effect=lambda _j: next(receipts, then)),
                patch.object(client, "_call") as call,
            ):
                with self.assertRaises(ControlError) as caught:
                    client.stop_and_wait("start", timeout=0.01, poll_interval=0.001)
                self.assertEqual(caught.exception.code, code)
                self.assertEqual(caught.exception.request_id, rid)
                stop.assert_called_once_with(job_id="start")
                call.assert_not_called()

    def test_stop_and_wait_link_chain_is_bounded(self):
        client = ControlClient(endpoint=self.endpoint())
        with (
            patch.object(client, "stop", return_value={"requested": True}) as stop,
            patch.object(
                client, "result",
                side_effect=lambda j: {"job_id": j, "terminal": True,
                                       "status": "completed", "cleanup": "verified",
                                       "linked_job_id": j + "x"},
            ) as result,
        ):
            with self.assertRaises(ControlError) as caught:
                client.stop_and_wait("j", timeout=5, poll_interval=0.001)
        self.assertEqual(caught.exception.code, "validation")
        self.assertLessEqual(result.call_count, 32)
        stop.assert_called_once_with(job_id="j")

    def test_stop_and_wait_invalid_job_or_poll_has_no_side_effect(self):
        client = ControlClient(endpoint=self.endpoint())
        with (
            patch.object(client, "_call") as call,
            patch.object(client, "stop") as stop,
        ):
            for kwargs in ({"job_id": ""}, {"job_id": 7}, {"poll_interval": 0}):
                with self.assertRaises(ValueError):
                    client.stop_and_wait(**kwargs)
            call.assert_not_called()
            stop.assert_not_called()

    def test_release_ack_with_pending_cleanup_is_not_completion(self):
        client = ControlClient(endpoint=self.endpoint())
        client._lease = "opaque-test-lease"
        states = [
            {"ownership": {"owned": False}, "phase": "stopping", "cleanup": "pending"},
            {"ownership": {"owned": False}, "phase": "ready", "cleanup": "verified"},
        ]
        with (
            patch.object(
                client, "_call", return_value={"released": True, "cleanup": "pending"}
            ),
            patch.object(client, "status", side_effect=states) as status,
        ):
            self.assertEqual(client.release()["cleanup"], "verified")
            self.assertEqual(status.call_count, 2)
            self.assertIsNone(client._lease)

    def test_release_of_intentionally_continuing_job_returns_without_waiting(self):
        client = ControlClient(endpoint=self.endpoint())
        client._lease = "opaque-test-lease"
        job = {"job_id": "run", "terminal": False, "status": "running",
               "cleanup": "pending"}
        for cleanup in ("not_required", "pending"):
            ack = {"released": True, "cleanup": cleanup, "continuing": True,
                   "job_id": "run", "job": job}
            client._lease = "opaque-test-lease"
            with (
                self.subTest(cleanup=cleanup),
                patch.object(client, "_call", return_value=ack) as call,
                patch.object(client, "status") as status,
                patch.object(client, "result") as result,
                patch.object(client, "stop") as stop,
            ):
                released = client.release(timeout=0.01)
                # Returned unchanged: the job's pending cleanup is preserved.
                self.assertIs(released, ack)
                self.assertEqual(released["job"]["cleanup"], "pending")
                call.assert_called_once_with("release")
                status.assert_not_called()
                result.assert_not_called()
                stop.assert_not_called()
                self.assertIsNone(client._lease)
                self.assertTrue(client._heartbeat_stop.is_set())

    def test_continuing_flag_without_release_confirmation_is_not_success(self):
        client = ControlClient(endpoint=self.endpoint())
        client._lease = "opaque-test-lease"
        with (
            patch.object(client, "_call",
                         return_value={"released": False, "continuing": True}),
            patch.object(client, "status", return_value={
                "ownership": {"owned": True}, "phase": "running",
                "cleanup": "not_required"}),
        ):
            with self.assertRaises(ControlError) as caught:
                client.release(timeout=0.01)
        self.assertEqual(caught.exception.code, "timeout")
        self.assertEqual(client._lease, "opaque-test-lease")

    def test_release_that_stops_a_job_checks_that_jobs_receipt(self):
        ack = {"released": True, "cleanup": "pending", "continuing": False,
               "stop_requested": True, "job_id": "run"}
        released_state = {"ownership": {"owned": False}, "phase": "ready",
                          "cleanup": "not_required"}
        cases = [
            ({"job_id": "run", "terminal": True, "status": "stopped",
              "cleanup": "verified"}, None),
            ({"job_id": "run", "terminal": True, "status": "error",
              "cleanup": "unknown"}, "cleanup_unconfirmed"),
            ({"job_id": "other", "terminal": True, "status": "stopped",
              "cleanup": "verified"}, "cleanup_unconfirmed"),
        ]
        for receipt, code in cases:
            client = ControlClient(endpoint=self.endpoint())
            client._lease = "opaque-test-lease"
            with (
                self.subTest(receipt=receipt),
                patch.object(client, "_call", return_value=ack),
                patch.object(client, "status", return_value=released_state),
                patch.object(client, "result", return_value=receipt) as result,
            ):
                if code is None:
                    released = client.release()
                    self.assertEqual(released["job"], receipt)
                    self.assertEqual(released["job_id"], "run")
                    self.assertIs(released["release"], ack)
                else:
                    with self.assertRaises(ControlError) as caught:
                        client.release()
                    self.assertEqual(caught.exception.code, code)
                    self.assertEqual(caught.exception.request_id, "run")
                result.assert_called_once_with("run")
                # Ownership was confirmed returned before the receipt check.
                self.assertIsNone(client._lease)

    def test_release_pending_without_job_keeps_original_status_wait(self):
        client = ControlClient(endpoint=self.endpoint())
        client._lease = "opaque-test-lease"
        ack = {"released": True, "cleanup": "pending"}
        with (
            patch.object(client, "_call", return_value=ack),
            patch.object(client, "status", return_value={
                "ownership": {"owned": False}, "phase": "ready",
                "cleanup": "verified"}),
            patch.object(client, "result") as result,
        ):
            released = client.release()
        self.assertEqual(released["cleanup"], "verified")
        self.assertIs(released["release"], ack)
        result.assert_not_called()

    def test_release_still_reports_unconfirmed_cleanup_and_timeout(self):
        for state, code in (
            ({"ownership": {"owned": True}, "phase": "error", "cleanup": "unknown"},
             "cleanup_unconfirmed"),
            ({"ownership": {"owned": True}, "phase": "stopping",
              "cleanup": "pending"}, "timeout"),
        ):
            client = ControlClient(endpoint=self.endpoint())
            client._lease = "opaque-test-lease"
            with (
                self.subTest(code=code),
                patch.object(client, "_call",
                             return_value={"released": True, "cleanup": "pending"}),
                patch.object(client, "status", return_value=state),
            ):
                with self.assertRaises(ControlError) as caught:
                    client.release(timeout=0.01)
            self.assertEqual(caught.exception.code, code)

    def test_queued_retired_heartbeat_cannot_poison_reacquire(self):
        class QueuedStop:
            def __init__(self):
                self.entered = threading.Event()
                self.stopped = False

            def wait(self, _interval):
                self.entered.set()
                return False

            def set(self):
                self.stopped = True

            def is_set(self):
                return self.stopped

        client = ControlClient(endpoint=self.endpoint())
        retired = QueuedStop()
        client._lease = "retiring-lease"
        client._heartbeat_stop = retired
        queued = threading.Thread(
            target=control._heartbeat,
            args=(
                control.weakref.ref(client),
                retired,
                client._heartbeat_generation,
                0.1,
            ),
        )
        with client._lock:
            queued.start()
            self.assertTrue(retired.entered.wait(1))
            with patch.object(
                client, "_call", return_value={"released": True, "cleanup": "verified"}
            ):
                self.assertTrue(client.release()["released"])
        queued.join(1)
        self.assertFalse(queued.is_alive())
        self.assertIsNone(client._heartbeat_error)
        with patch.object(
            client,
            "_call",
            return_value={"lease": "reacquired-lease", "heartbeat_seconds": 2.0},
        ):
            self.assertEqual(client.acquire()["lease"], "reacquired-lease")
        client._heartbeat_stop.set()

    def test_active_heartbeat_revocation_remains_reported(self):
        class OneTickStop:
            def wait(self, _interval):
                return False

            def is_set(self):
                return False

        client = ControlClient(endpoint=self.endpoint())
        stop = OneTickStop()
        client._lease = "active-lease"
        client._heartbeat_stop = stop
        revoked = ControlError(
            "GUI control lease is not held or was revoked", code="busy"
        )
        with patch.object(client, "_call", side_effect=revoked):
            control._heartbeat(control.weakref.ref(client), stop, 0, 0.1)
        self.assertIs(client._heartbeat_error, revoked)
        with self.assertRaises(ControlError) as error:
            client.acquire()
        self.assertIs(error.exception, revoked)

    def test_advertised_read_view_never_claims_ownership(self):
        client = ControlClient(endpoint=self.endpoint())
        client._description = {"methods": ["control.plan"]}
        with (
            patch("kohdalab_control._exchange", return_value={"plan": {}}) as exchange,
            patch.object(client, "acquire") as acquire,
        ):
            self.assertEqual(client.invoke("plan"), {"plan": {}})
            acquire.assert_not_called()
            self.assertNotIn("lease", exchange.call_args.args[2])
            self.assertNotIn("request_id", exchange.call_args.args[2])

    def test_context_does_not_hide_cleanup_failure_behind_another_error(self):
        client = ControlClient(endpoint=self.endpoint())
        original = ValueError("Experiment failed")
        cleanup = ControlError(
            "Output cleanup was not confirmed", code="cleanup_unconfirmed"
        )
        with patch.object(client, "close", side_effect=cleanup):
            with self.assertRaises(ControlError) as caught:
                client.__exit__(ValueError, original, None)
        self.assertIs(caught.exception.__cause__, original)
        self.assertEqual(caught.exception.code, "cleanup_unconfirmed")

    def test_discovery_never_selects_by_recency(self):
        with tempfile.TemporaryDirectory() as tmp:
            paths = []
            for i in range(2):
                p = Path(tmp) / f"test-{i}.json"
                p.write_text(
                    json.dumps(
                        {
                            "application": "kohdalab-test",
                            "control_protocol": 1,
                            "discovery_version": 1,
                            "transport": "tcp-jsonl",
                            "host": "127.0.0.1",
                            "port": 100 + i,
                            "pid": i + 1,
                            "token": str(uuid.uuid4()),
                            "instance_id": str(uuid.uuid4()),
                        }
                    )
                )
                p.chmod(0o600)
                paths.append(p)
            with patch(
                "kohdalab_control._exchange",
                return_value={"application": "kohdalab-test", "control_protocol": 1},
            ):
                self.assertEqual(len(discover(directory=tmp)), 2)
                with self.assertRaises(ControlError):
                    ControlClient("kohdalab-test", directory=tmp)


if __name__ == "__main__":
    unittest.main()
