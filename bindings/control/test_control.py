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
        fallback = Path("/fallback")
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
                [Path("/profile/.kohdalab/iv/run"), Path("/profile/.kohdalab/control"), Path("/profile/.kohdalab/run")],
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
        with tempfile.TemporaryDirectory() as tmp, patch.dict(os.environ, {"HOME": tmp, "USERPROFILE": tmp}):
            root = Path(tmp) / ".kohdalab"
            iv = root / "iv/run/iv.json"
            trkr = root / "trkr/run/trkr.json"
            explicit = Path(tmp) / "custom/other.json"
            for path in (iv, trkr, explicit):
                path.parent.mkdir(parents=True)
                path.write_text("{}")
            def endpoint(path):
                name = "trkr" if path == trkr else "iv"
                return Endpoint("kohdalab-" + name, "127.0.0.1", 1, "private", str(path), 1, path)
            def describe(endpoint, *args):
                return {"application": endpoint.application, "control_protocol": 1}
            with patch.object(control, "load_endpoint", side_effect=endpoint), patch.object(control, "_exchange", side_effect=describe):
                self.assertEqual({e.path for e in discover()}, {iv, trkr})
                self.assertEqual([e.path for e in discover("kohdalab-iv")], [iv])
                self.assertEqual([e.path for e in discover(directory=explicit.parent)], [explicit])
            self.assertFalse((root / "control").exists())
            self.assertFalse((root / "iv/config").exists())


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
                control._exchange(
                    self.endpoint(), "status", {"request_id": "limit"}, 1
                )
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
