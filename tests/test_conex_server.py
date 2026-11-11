import contextlib
import io
import os
import sys
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parents[1] / "bridge"))
import conex_server as conex


class FakeDriver:
    def __init__(self):
        self.position = 1.0
        self.state = "32"
        self.calls = []

    def OpenInstrument(self, port): self.calls.append(("open", port)); return 0
    def CloseInstrument(self): self.calls.append(("close",))
    def SL_Get(self, *_): self.calls.append(("SL_Get",)); return 0, 0.0, ""
    def SR_Get(self, *_): self.calls.append(("SR_Get",)); return 0, 10.0, ""
    def TP(self, *_): self.calls.append(("TP",)); return 0, self.position, ""
    def TS(self, *_): self.calls.append(("TS",)); return 0, "", self.state, ""
    def VA_Set(self, *_): self.calls.append(("VA_Set",)); return 0, ""
    def PA_Set(self, _dev, position, _empty):
        self.calls.append(("PA_Set", position)); self.state = "28"; return 0, ""
    def ST(self, *_): self.calls.append(("ST",)); self.state = "32"; return 0, ""


class ConexServerStartupTests(unittest.TestCase):
    def test_non_loopback_without_token_is_rejected_before_bind(self):
        for args, env in [
            ([], {}),
            ([], {"KOHDA_CONEX_TOKEN": ""}),
            (["--token", " "], {}),
        ]:
            with (
                self.subTest(args=args, env=env),
                patch.object(sys, "argv", ["conex_server.py", "--host", "0.0.0.0", *args]),
                patch.dict(os.environ, env, clear=True),
                patch.object(conex, "ConexRequestServer") as server,
                contextlib.redirect_stderr(io.StringIO()) as stderr,
            ):
                with self.assertRaises(SystemExit) as error:
                    conex.main()
                self.assertEqual(error.exception.code, 2)
                self.assertIn("requires --token or KOHDA_CONEX_TOKEN", stderr.getvalue())
                server.assert_not_called()

    def test_loopback_without_token_and_remote_with_token_are_allowed(self):
        for args, env, expected_host, expected_token in [
            ([], {}, "127.0.0.1", None),
            (["--host", "localhost"], {}, "127.0.0.1", None),
            (["--host", "127.0.0.2"], {}, "127.0.0.2", None),
            (["--host", "0.0.0.0", "--token", "cli"], {"KOHDA_CONEX_TOKEN": "env"}, "0.0.0.0", "cli"),
            (["--host", "0.0.0.0"], {"KOHDA_CONEX_TOKEN": "env"}, "0.0.0.0", "env"),
        ]:
            with (
                self.subTest(args=args, env=env),
                patch.object(sys, "argv", ["conex_server.py", *args]),
                patch.dict(os.environ, env, clear=True),
                patch.object(conex, "ConexRequestServer") as server,
            ):
                self.assertEqual(conex.main(), 0)
                self.assertEqual(server.call_args.args[0], (expected_host, 9000))
                self.assertEqual(server.call_args.args[2], expected_token)
                server.return_value.serve_forever.assert_called_once()


class ConexControllerTests(unittest.TestCase):
    def setUp(self):
        self.driver = FakeDriver()
        self.controller = conex.ConexController(lambda: self.driver)
        self.controller.connect({"com_port": "COM4"})

    def tearDown(self):
        if not self.controller._move_lock.locked(): self.controller.close()

    def test_connect_is_read_only_no_home_or_enable(self):
        names = [call[0] for call in self.driver.calls]
        self.assertIn("SL_Get", names)
        self.assertNotIn("OR", names)
        self.assertNotIn("MM_Set", names)

    def test_status_remains_available_while_move_is_active(self):
        done = []
        def run():
            try: self.controller.move({"position_mm": 2, "timeout_s": 1})
            except Exception as error: done.append(error)
        thread = threading.Thread(target=run)
        thread.start()
        for _ in range(50):
            if self.controller._move_lock.locked(): break
            time.sleep(0.01)
        status = self.controller.status()
        self.assertEqual(status["state"], "28")
        self.controller.stop({})
        thread.join(1)
        self.assertFalse(thread.is_alive())
        self.assertIsInstance(done[0], Exception)

    def test_concurrent_stop_aborts_move_and_uses_st(self):
        result = []
        def run():
            try: self.controller.move({"position_mm": 2, "timeout_s": 1})
            except Exception as error: result.append(error)
        thread = threading.Thread(target=run)
        thread.start()
        for _ in range(50):
            if self.controller._move_lock.locked(): break
            time.sleep(0.01)
        stopped = self.controller.stop({})
        thread.join(1)
        self.assertTrue(stopped["ready"])
        self.assertTrue(any(call[0] == "ST" for call in self.driver.calls))
        self.assertEqual(len(result), 1)
        self.assertIn("stopped", str(result[0]))

    def test_stop_between_initial_readback_and_command_prevents_motion(self):
        original = self.controller._readback
        def read_then_stop():
            value = original()
            self.controller._readback = original
            self.controller.stop({})
            return value
        self.controller._readback = read_then_stop
        with self.assertRaisesRegex(conex.ConexError, "stopped before PA_Set"):
            self.controller.move({"position_mm": 2, "timeout_s": 1})
        self.assertFalse(any(call[0] == "PA_Set" for call in self.driver.calls))

    def test_stop_waits_for_ready_and_has_a_bounded_timeout(self):
        self.driver.ST = lambda *_: (0, "")
        self.driver.state = "28"
        with self.assertRaisesRegex(conex.ConexError, "stop is unconfirmed"):
            self.controller.stop({"timeout_s": 0.01})
        with self.assertRaisesRegex(conex.ConexError, "at most 30"):
            self.controller.stop({"timeout_s": 31})

    def test_protocol_hello_is_revisioned_and_no_home(self):
        server = object.__new__(conex.ConexRequestServer)
        server.controller, server.token = self.controller, None
        self.assertEqual(server.dispatch({"revision": 1, "id": "x", "op": "hello", "args": {}}),
                         {"protocol": "kohdalab-conex", "revision": 1, "can_stop": True, "can_home": False})


if __name__ == "__main__":
    unittest.main()
