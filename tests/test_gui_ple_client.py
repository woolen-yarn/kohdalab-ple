import json
import socket
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parents[1] / "bindings" / "python"))
from kohdalab_ple import GuiPLE, GuiPLEError


class Server:
    def __init__(self):
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen()
        self.port = self.listener.getsockname()[1]
        self.requests = []
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while True:
            try:
                stream, _ = self.listener.accept()
            except OSError:
                return
            with stream:
                request = json.loads(stream.recv(65536).split(b"\n", 1)[0])
                self.requests.append(request)
                if request["token"] != "secret":
                    response = {"ok": False, "error": "bad token", "error_code": "authentication"}
                elif request["action"] == "acquire":
                    response = {"ok": True, "result": {"lease": "lease"}}
                elif request["action"] == "status":
                    response = {"ok": True, "result": {"busy": False, "cleanup_pending": False, "state": "stopped"}}
                else:
                    response = {"ok": True, "result": {"action": request["action"]}}
                stream.sendall((json.dumps(response) + "\n").encode())

    def close(self):
        self.listener.close()


class GuiPLETests(unittest.TestCase):
    def setUp(self):
        self.server = Server()
        self.directory = tempfile.TemporaryDirectory()
        self.endpoint = Path(self.directory.name) / "ple-gui-1-test.json"
        self.endpoint.write_text(json.dumps({"app": "kohdalab-ple", "api_version": 1,
                                             "host": "127.0.0.1", "port": self.server.port,
                                             "token": "secret", "pid": 1}))

    def tearDown(self):
        self.server.close(); self.directory.cleanup()

    def test_common_client_alias_discovery_and_background_control(self):
        from kohdalab_ple import GuiClient, find_binary
        self.assertIs(GuiClient, GuiPLE)
        client = GuiClient(self.endpoint)
        client.lease = "lease"
        settings = {"mode":"automatic","backend":"kohdalab_shutter","endpoint":"","channel":0}
        client.set_background(settings,request_id="00000000-0000-4000-8000-000000000007")
        self.assertEqual(self.server.requests[-1]["background"],settings)
        self.assertEqual(self.server.requests[-1]["action"],"set_background")
        client.background(); self.assertEqual(self.server.requests[-1]["action"],"background")
        client.preview(); self.assertEqual(self.server.requests[-1]["action"],"preview")
        client._clear_lease()
        with tempfile.TemporaryDirectory() as directory:
            executable = Path(directory) / "gui.exe"
            executable.touch()
            self.assertEqual(find_binary(executable),executable.resolve())
            with patch.dict("os.environ",{"KOHDALAB_PLE_EXE":str(executable)}):
                self.assertEqual(find_binary(),executable.resolve())
            with self.assertRaises(GuiPLEError): find_binary(Path(directory)/"missing")

    def test_observer_and_leased_motion_envelope(self):
        client = GuiPLE(self.endpoint)
        self.assertFalse(client.status()["busy"])
        with self.assertRaises(GuiPLEError) as error:
            client.move(1.0)
        self.assertEqual(error.exception.code, "ownership")
        client.acquire()
        receipt = client.move(1.0, request_id="00000000-0000-4000-8000-000000000000")
        self.assertEqual(receipt["action"], "move")
        move = self.server.requests[-1]
        self.assertEqual(move["lease"], "lease")
        self.assertEqual(move["request_id"], "00000000-0000-4000-8000-000000000000")

    def test_uncertain_move_requests_stop_without_retry_and_retains_uuid(self):
        client=GuiPLE(self.endpoint);client.lease="lease";seen=[]
        def wire(_endpoint,payload,_timeout):
            seen.append(payload["action"])
            if payload["action"]=="move":raise GuiPLEError("Lost acknowledgement","transport")
            return {"stop_requested":True}
        request_id="00000000-0000-4000-8000-000000000001"
        with patch.object(GuiPLE,"_wire",side_effect=wire):
            with self.assertRaises(GuiPLEError) as error:client.move(1.,request_id=request_id)
        self.assertEqual(seen,["move","stop"]);self.assertEqual(error.exception.request_id,request_id);self.assertEqual(client.last_request_id,request_id);client._clear_lease()

    def test_apply_spectrometer_submits_only_settings_patch_and_uuid(self):
        client = GuiPLE(self.endpoint)
        client.lease = "lease"
        request_id = "00000000-0000-4000-8000-000000000002"
        job = client.apply_spectrometer({"exposure_ms": 25.0, "center_nm": 775.0}, request_id=request_id)
        self.assertEqual(job["action"], "apply_spectrometer")
        request = self.server.requests[-1]
        self.assertEqual(request["settings"], {"exposure_ms": 25.0, "center_nm": 775.0})
        self.assertEqual(request["request_id"], request_id)
        self.assertEqual(request["lease"], "lease")
        client._clear_lease()

    def test_output_none_fields_are_omitted_and_cleanup_errors_are_not_success(self):
        client=GuiPLE(self.endpoint);client.lease="lease";seen=[]
        def wire(_endpoint,payload,_timeout):
            seen.append(payload)
            if payload["action"]=="status":return {"busy":False,"cleanup_pending":False,"cleanup_error":"shutter unconfirmed"}
            return {}
        with patch.object(GuiPLE,"_wire",side_effect=wire):
            client.set_output(sample="sample")
            self.assertNotIn("directory",seen[0])
            with self.assertRaises(GuiPLEError) as error:client.stop_and_wait()
        self.assertEqual(error.exception.code,"interrupted_unconfirmed");client._clear_lease()

    def test_authentication_error_is_typed(self):
        value = json.loads(self.endpoint.read_text()); value["token"] = "wrong"; self.endpoint.write_text(json.dumps(value))
        with self.assertRaises(GuiPLEError) as error:
            GuiPLE(self.endpoint).status()
        self.assertEqual(error.exception.code, "authentication")

