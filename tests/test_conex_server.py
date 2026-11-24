import contextlib
import io
import os
import subprocess
import sys
import tempfile
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
                patch.dict(os.environ),
                patch.object(conex, "ConexRequestServer") as server,
                contextlib.redirect_stderr(io.StringIO()) as stderr,
            ):
                os.environ.pop("KOHDA_CONEX_TOKEN", None)
                os.environ.update(env)
                with self.assertRaises(SystemExit) as error:
                    conex.main()
                self.assertEqual(error.exception.code, 2)
                self.assertIn("requires --token or KOHDA_CONEX_TOKEN", stderr.getvalue())
                server.assert_not_called()

    def test_loopback_without_token_and_remote_with_token_are_allowed(self):
        for args, env, expected_host, expected_token in [
            ([], {}, "127.0.0.1", None),
            (["--host", "127.0.0.2"], {}, "127.0.0.2", None),
            (["--host", "0.0.0.0", "--token", "cli"], {"KOHDA_CONEX_TOKEN": "env"}, "0.0.0.0", "cli"),
            (["--host", "0.0.0.0"], {"KOHDA_CONEX_TOKEN": "env"}, "0.0.0.0", "env"),
        ]:
            with (
                self.subTest(args=args, env=env),
                patch.object(sys, "argv", ["conex_server.py", *args]),
                patch.dict(os.environ),
                patch.object(conex, "ConexRequestServer") as server,
            ):
                os.environ.pop("KOHDA_CONEX_TOKEN", None)
                os.environ.update(env)
                self.assertEqual(conex.main(), 0)
                self.assertEqual(server.call_args.args[0], (expected_host, 9000))
                self.assertEqual(server.call_args.args[2], expected_token)
                server.return_value.serve_forever.assert_called_once()


    def test_hostname_binds_to_the_resolved_loopback_address(self):
        with (
            patch.object(sys, "argv", ["conex_server.py", "--host", "localhost"]),
            patch.dict(os.environ, {"KOHDA_CONEX_TOKEN": ""}),
            patch.object(conex.socket, "gethostbyname", return_value="127.0.0.2") as resolve,
            patch.object(conex, "ConexRequestServer") as server,
        ):
            self.assertEqual(conex.main(), 0)
        resolve.assert_called_once_with("localhost")
        self.assertEqual(server.call_args.args[0], ("127.0.0.2", 9000))


# Child process holding/probing a lock with the raw OS call fs2 0.4
# try_lock_exclusive uses (TRKR Port::open), independent of conex_server.
_RAW_LOCK = r"""
import os, sys
path, mode = sys.argv[1], sys.argv[2]
fd = os.open(path, os.O_RDWR | os.O_CREAT | getattr(os, "O_BINARY", 0))
if os.name == "nt":
    import ctypes, msvcrt
    from ctypes import wintypes
    class OV(ctypes.Structure):
        _fields_ = [("a", ctypes.c_size_t), ("b", ctypes.c_size_t), ("c", wintypes.DWORD),
                    ("d", wintypes.DWORD), ("e", wintypes.HANDLE)]
    k = ctypes.WinDLL("kernel32", use_last_error=True)
    k.LockFileEx.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD,
                             wintypes.DWORD, ctypes.POINTER(OV)]
    ok = bool(k.LockFileEx(msvcrt.get_osfhandle(fd), 3, 0, 0xFFFFFFFF, 0xFFFFFFFF, ctypes.byref(OV())))
else:
    import fcntl
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB); ok = True
    except BlockingIOError:
        ok = False
print("locked" if ok else "busy", flush=True)
if mode == "hold":
    sys.stdin.readline()
"""


def _raw_lock(path, mode):
    return subprocess.Popen([sys.executable, "-I", "-c", _RAW_LOCK, path, mode],
                            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)


def _lock_is_free(path):
    child = _raw_lock(path, "probe")
    out, _ = child.communicate(timeout=30)
    return out.strip() == "locked"


class SerialPortLeaseTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = self.tmp.name
        self.factory_calls = 0
        self.driver = FakeDriver()

    def factory(self):
        self.factory_calls += 1
        return self.driver

    def controller(self):
        return conex.ConexController(self.factory, lock_directory=self.dir)

    def test_key_matches_trkr_normalization(self):
        # TRKR: name.trim().to_ascii_uppercase().replace("\\\\.\\", "") as lowercase hex bytes.
        self.assertEqual(conex.serial_lock_key("COM4"), "434f4d34")
        for alias in [" com4\t", "\\\\.\\COM4", chr(0x3000) + "\\\\.\\cOm4" + chr(0x2028)]:
            self.assertEqual(conex.serial_lock_key(alias), "434f4d34", repr(alias))
        self.assertEqual(conex.serial_lock_key("\x1fCOM4"), "1f434f4d34")  # not Rust whitespace
        self.assertEqual(conex.serial_lock_key("com" + chr(0xE9)), "434f4dc3a9")  # non-ASCII not uppercased
        self.assertEqual(conex.serial_lock_path("COM10", self.dir),
                         os.path.join(self.dir, "kohdalab-motion-434f4d3130.lock"))

    def test_default_location_is_rust_temp_dir(self):
        self.assertEqual(conex.serial_lock_path("COM4"),
                         os.path.join(conex.host_temp_dir(), "kohdalab-motion-434f4d34.lock"))
        if os.name != "nt":
            with patch.dict(os.environ, {"TMPDIR": self.dir}):
                self.assertEqual(conex.host_temp_dir(), self.dir)
            with patch.dict(os.environ):
                os.environ.pop("TMPDIR", None)
                self.assertEqual(conex.host_temp_dir(), "/tmp")

    def test_conflict_with_other_process_fails_before_sdk(self):
        path = conex.serial_lock_path("COM4", self.dir)
        holder = _raw_lock(path, "hold")
        controller = self.controller()
        try:
            self.assertEqual(holder.stdout.readline().strip(), "locked")
            with self.assertRaisesRegex(conex.ConexError, "already used by another measurement process"):
                controller.connect({"com_port": "\\\\.\\com4"})
            self.assertEqual(self.factory_calls, 0)
            self.assertEqual(self.driver.calls, [])
            self.assertIsNone(controller._lease)
        finally:
            holder.communicate("\n", timeout=30)
        # Once the other process releases, the port can be taken.
        controller.connect({"com_port": "COM4"})
        self.assertFalse(_lock_is_free(path))
        controller.close()

    def test_held_while_connected_and_released_on_successful_close(self):
        controller = self.controller()
        controller.connect({"com_port": "COM4"})
        path = conex.serial_lock_path("COM4", self.dir)
        self.assertFalse(_lock_is_free(path))
        controller.close()
        self.assertEqual(self.driver.calls[-1], ("close",))
        self.assertTrue(_lock_is_free(path))
        self.assertTrue(os.path.exists(path))  # lock file is never removed

    def test_close_failure_or_unknown_outcome_retains_lock(self):
        def raises():
            raise RuntimeError("io")
        for name, close in [("raises", raises), ("code", lambda: -1)]:
            with self.subTest(name):
                self.driver = FakeDriver()
                self.driver.CloseInstrument = close
                controller = self.controller()
                controller.connect({"com_port": "COM5"})
                with self.assertRaisesRegex(conex.ConexError, "lock retained"):
                    controller.close()
                path = conex.serial_lock_path("COM5", self.dir)
                self.assertFalse(_lock_is_free(path))
                controller.close()  # nothing confirmed closed: still retained
                self.assertFalse(_lock_is_free(path))
                calls = self.factory_calls
                with self.assertRaisesRegex(conex.ConexError, "did not close cleanly"):
                    controller.connect({"com_port": "COM5"})
                self.assertEqual(self.factory_calls, calls)
                controller._lease.release()  # stands in for process exit
                self.assertTrue(_lock_is_free(path))

    def test_open_failures(self):
        path = conex.serial_lock_path("COM6", self.dir)
        # A reported open failure and an SDK load failure release the lock.
        self.driver.OpenInstrument = lambda port: -1
        with self.assertRaisesRegex(conex.ConexError, "code -1"):
            self.controller().connect({"com_port": "COM6"})
        self.assertTrue(_lock_is_free(path))
        def broken_factory():
            raise conex.ConexError("no SDK")
        with self.assertRaisesRegex(conex.ConexError, "no SDK"):
            conex.ConexController(broken_factory, lock_directory=self.dir).connect({"com_port": "COM6"})
        self.assertTrue(_lock_is_free(path))
        # An exception from OpenInstrument leaves the outcome unknown.
        def throw(port):
            raise RuntimeError("SDK fault")
        self.driver.OpenInstrument = throw
        controller = self.controller()
        with self.assertRaisesRegex(RuntimeError, "SDK fault"):
            controller.connect({"com_port": "COM6"})
        self.assertFalse(_lock_is_free(path))
        controller._lease.release()

    def test_concurrent_connect_to_other_port_never_reaches_second_open(self):
        opened, release = threading.Event(), threading.Event()
        drivers = []

        class GatedDriver(FakeDriver):
            def OpenInstrument(self, port):
                super().OpenInstrument(port)
                opened.set()
                release.wait(5)
                return 0

        def factory():
            drivers.append(GatedDriver())
            return drivers[-1]

        controller = conex.ConexController(factory, lock_directory=self.dir)
        # Wrap the RLock to observe (not alter) when the second request
        # reaches it, so the interleaving is deterministic without sleeps.
        inner, contended = controller._sdk_lock, threading.Event()

        class ObservedLock:
            def __enter__(self):
                if threading.current_thread().name == "second":
                    contended.set()
                return inner.__enter__()

            def __exit__(self, *exc):
                return inner.__exit__(*exc)

        controller._sdk_lock = ObservedLock()
        results = {}

        def run(name, port):
            try:
                results[name] = controller.connect({"com_port": port})
            except Exception as error:
                results[name] = error

        first = threading.Thread(target=run, args=("first", "COM3"), name="first")
        first.start()
        self.assertTrue(opened.wait(5))
        second = threading.Thread(target=run, args=("second", "COM8"), name="second")
        second.start()
        self.assertTrue(contended.wait(5))
        release.set()
        first.join(5)
        second.join(5)
        self.assertFalse(first.is_alive() or second.is_alive())

        self.assertIsInstance(results["first"], dict)
        self.assertIsInstance(results["second"], conex.ConexError)
        self.assertIn("already open on COM3", str(results["second"]))
        self.assertEqual(len(drivers), 1)
        self.assertEqual([c for c in drivers[0].calls if c[0] == "open"], [("open", "COM3")])
        self.assertIs(controller._driver, drivers[0])
        self.assertEqual(controller._lease.port, "COM3")
        self.assertFalse(_lock_is_free(conex.serial_lock_path("COM3", self.dir)))
        self.assertTrue(_lock_is_free(conex.serial_lock_path("COM8", self.dir)))
        controller.close()
        self.assertTrue(_lock_is_free(conex.serial_lock_path("COM3", self.dir)))

    def test_failed_readback_after_open_closes_and_releases(self):
        self.driver.SL_Get = lambda *_: (0, 0.0, "fault")
        controller = self.controller()
        with self.assertRaisesRegex(conex.ConexError, "SL_Get failed"):
            controller.connect({"com_port": "COM7"})
        self.assertEqual(self.driver.calls[-1], ("close",))
        self.assertTrue(_lock_is_free(conex.serial_lock_path("COM7", self.dir)))


class ConexControllerTests(unittest.TestCase):
    def setUp(self):
        self.driver = FakeDriver()
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.controller = conex.ConexController(lambda: self.driver, lock_directory=tmp.name)
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

    def test_close_admission_never_overlaps_a_move(self):
        # Gate close at the _sdk_lock (after its admission check) and start a
        # move there: the move must be refused, never commanded, while close
        # proceeds to release the port.
        inner = self.controller._sdk_lock
        admitted, move_done = threading.Event(), threading.Event()
        active = []  # move-slot state observed at CloseInstrument

        class GatedLock:
            def __enter__(self):
                if threading.current_thread().name == "closer" and not admitted.is_set():
                    admitted.set()
                    move_done.wait(5)
                return inner.__enter__()

            def __exit__(self, *exc):
                return inner.__exit__(*exc)

        def close_instrument():
            active.append(self.driver.state)
            self.driver.calls.append(("close",))

        self.driver.CloseInstrument = close_instrument
        self.controller._sdk_lock = GatedLock()
        results = {}

        def closer():
            try: results["close"] = self.controller.close()
            except Exception as error: results["close"] = error

        def mover():
            try: results["move"] = self.controller.move({"position_mm": 2, "timeout_s": 0.2})
            except Exception as error: results["move"] = error
            finally: move_done.set()

        close_thread = threading.Thread(target=closer, name="closer")
        close_thread.start()
        self.assertTrue(admitted.wait(5))
        move_thread = threading.Thread(target=mover, name="mover")
        move_thread.start()
        move_thread.join(5)
        close_thread.join(5)
        self.assertFalse(close_thread.is_alive() or move_thread.is_alive())

        self.assertIsInstance(results["move"], conex.ConexError)
        self.assertIn("already active", str(results["move"]))
        self.assertIsNone(results["close"])
        self.assertFalse(any(call[0] == "PA_Set" for call in self.driver.calls))
        self.assertEqual(active, ["32"])  # stage idle when the port closed
        self.assertIsNone(self.controller._driver)
        self.assertFalse(self.controller._move_lock.locked())

    def test_close_is_refused_while_a_move_is_commanded(self):
        commanded, release = threading.Event(), threading.Event()
        original = self.driver.PA_Set

        def gated_pa_set(*args):
            result = original(*args)
            commanded.set()
            release.wait(5)
            return result

        self.driver.PA_Set = gated_pa_set
        results = []

        def mover():
            try: self.controller.move({"position_mm": 2, "timeout_s": 0.2})
            except Exception as error: results.append(error)

        thread = threading.Thread(target=mover)
        thread.start()
        self.assertTrue(commanded.wait(5))
        try:
            with self.assertRaisesRegex(conex.ConexError, "cannot close while a move is active"):
                self.controller.close()
            self.assertNotIn(("close",), self.driver.calls)
            self.assertIsNotNone(self.controller._driver)
        finally:
            release.set()
            thread.join(5)
        self.assertFalse(thread.is_alive())

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
