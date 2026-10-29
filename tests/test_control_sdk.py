"""Loopback contract smoke test for the shared ControlClient."""
from __future__ import annotations

import json
import os
import socket
import threading
import uuid
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).parents[1] / "bindings" / "control"))
from kohdalab_control import ControlClient  # noqa: E402


class ControlSocket(unittest.TestCase):
    def test_observe_acquire_start_result_release(self):
        listener = socket.socket(); listener.bind(("127.0.0.1", 0)); listener.listen()
        port = listener.getsockname()[1]; token = "test-token-" + uuid.uuid4().hex; instance = str(uuid.uuid4()); stop = threading.Event()

        def serve():
            while not stop.is_set():
                listener.settimeout(0.2)
                try: peer, _ = listener.accept()
                except TimeoutError: continue
                with peer:
                    body = json.loads(peer.makefile("rb").readline()); method = body["method"]
                    if method == "control.describe": result = {"control_protocol": 1, "application": "kohdalab-ple", "methods": ["control.describe", "control.status", "control.acquire", "control.start", "control.result", "control.release"]}
                    elif method == "control.status": result = {"phase": "ready", "busy": False, "ownership": {"owned": False}, "cleanup": "not_required"}
                    elif method == "control.acquire": result = {"client_id": body["client_id"], "lease": "lease-" + uuid.uuid4().hex, "heartbeat_seconds": 1, "timeout_seconds": 10}
                    elif method == "control.heartbeat": result = {"alive": True}
                    elif method == "control.start": result = {"job_id": str(uuid.uuid4()), "status": "accepted", "terminal": False}
                    elif method == "control.result": result = {"job_id": body["job_id"], "status": "completed", "terminal": True, "cleanup": "verified"}
                    elif method == "control.release": result = {"released": True, "cleanup": "verified"}
                    else: result = {"alive": True}
                    peer.sendall((json.dumps({"control_protocol": 1, "ok": True, "request_id": body.get("request_id"), "result": result, "error": None}) + "\n").encode())

        thread = threading.Thread(target=serve, daemon=True); thread.start()
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            descriptor = directory / "ple-gui-test.json"
            descriptor.write_text(json.dumps({"control_protocol": 1, "discovery_version": 1, "transport": "tcp-jsonl", "application": "kohdalab-ple", "app": "kohdalab-ple", "host": "127.0.0.1", "port": port, "token": token, "instance_id": instance, "pid": 1}), encoding="utf-8")
            os.chmod(descriptor, 0o600)
            observer = ControlClient("kohdalab-ple", directory=directory); self.assertEqual(observer.status()["phase"], "ready")
            client = ControlClient("kohdalab-ple", directory=directory); acquired = client.acquire(); self.assertEqual(acquired["client_id"], client.client_id)
            job = client.invoke("start"); self.assertTrue(client.result(job["job_id"])["terminal"]); client.release()
        stop.set(); listener.close()


if __name__ == "__main__": unittest.main()
