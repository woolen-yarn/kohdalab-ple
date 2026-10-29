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
