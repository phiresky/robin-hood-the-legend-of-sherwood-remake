"""Failure-path evidence tests; no game data, graphics stack or Rust build."""
import contextlib
import io
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

import briefing_x11
import client_x11
import frame_steps_live as frame
import input_worker
import namespace_x11
import runtime_evidence as evidence


class RuntimeEvidenceTests(unittest.TestCase):
    def test_client_key_and_click_use_confined_adapter_and_last_robin_window(self):
        for arguments in (["key", "F5"], ["click", "100", "200"]):
            with self.subTest(arguments=arguments):
                windows = [Mock(id=11), Mock(id=33)]
                for window in windows:
                    window.get_wm_name.return_value = "Robin"
                connection = Mock()
                connection.screen.return_value.root.query_tree.return_value.children = windows
                xlib = SimpleNamespace(X=SimpleNamespace(RevertToPointerRoot=1, CurrentTime=0,
                    NONE=0, KeyPressMask=1, KeyReleaseMask=2, ButtonPressMask=4, ButtonReleaseMask=8),
                    XK=Mock(), protocol=Mock())
                with patch.dict(sys.modules, {"Xlib": xlib, "Xlib.ext": SimpleNamespace(xtest=Mock())}), \
                     patch.object(client_x11, "open_display", return_value=connection) as opened, \
                     patch.object(client_x11.time, "sleep"), contextlib.redirect_stdout(io.StringIO()):
                    client_x11.send_input(arguments)
                opened.assert_called_once_with()
                windows[0].send_event.assert_not_called()
                self.assertEqual(windows[1].send_event.call_count, 2)
                connection.close.assert_called_once()

    def test_briefing_targets_every_robin_window_through_confined_adapter(self):
        windows = [Mock(id=11), Mock(id=22), Mock(id=33)]
        for window, title in zip(windows, ("Robin host", "not the game", "Robin peer")):
            window.get_wm_name.return_value = title
        connection = Mock()
        connection.screen.return_value.root.query_tree.return_value.children = windows
        xlib = SimpleNamespace(X=SimpleNamespace(RevertToPointerRoot=1, CurrentTime=0,
            NONE=0, KeyPressMask=1, KeyReleaseMask=2), XK=Mock(), protocol=Mock())
        with patch.dict(sys.modules, {"Xlib": xlib}), \
             patch.object(briefing_x11, "open_display", return_value=connection) as opened, \
             patch.object(briefing_x11.time, "sleep"):
            self.assertEqual(briefing_x11.dismiss_briefings(":77"), [11, 33])
        opened.assert_called_once_with(":77")
        self.assertEqual(windows[0].send_event.call_count, 2)
        windows[1].send_event.assert_not_called()
        self.assertEqual(windows[2].send_event.call_count, 2)
        self.assertEqual(connection.sync.call_count, 4)
        connection.close.assert_called_once()

    @unittest.skipUnless(sys.platform == "linux", "Linux abstract sockets")
    def test_confined_transport_connects_real_abstract_socket_only(self):
        with tempfile.TemporaryDirectory() as temporary:
            number = Path(temporary).name + "-diagnostic"
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
                server.bind("\0/tmp/.X11-unix/X" + number)
                server.listen(1)
                connection = namespace_x11._namespace_socket(":" + number, None, "", number)
                accepted, _ = server.accept()
                with connection, accepted:
                    connection.sendall(b"confined")
                    self.assertEqual(accepted.recv(8), b"confined")
            with self.assertRaises(OSError):
                namespace_x11._namespace_socket(":" + number, None, "", number)

    def test_retained_client_and_helper_do_not_follow_rebuilt_input(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "robin"
            helper = root / "robin-replay-admission"
            source.write_bytes(b"build one")
            helper.write_bytes(b"decoder one")
            retained = root / "evidence"
            retained.mkdir()
            summary = {}
            binary = evidence.snapshot_client(source, retained, summary)
            source.write_bytes(b"build two")
            helper.write_bytes(b"decoder two")
            evidence.verify_client(binary, summary)
            self.assertEqual(binary.read_bytes(), b"build one")
            binary.chmod(0o755)
            binary.write_bytes(b"corrupt retained output")
            with self.assertRaisesRegex(RuntimeError, "changed before launch"):
                evidence.verify_client(binary, summary)

    def test_input_subprocess_keeps_all_window_ids_and_detects_changed_worker(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            def successful(argv, **kwargs):
                kwargs["log"].write_text("[101, 202]\n")
            with patch.object(input_worker, "run", successful):
                self.assertEqual(input_worker.dismiss_briefings(":1", root), [101, 202])
                worker = root / "briefing_x11.py"
                worker.chmod(0o755)
                worker.write_text("changed input")
                with self.assertRaisesRegex(RuntimeError, "input executable changed"):
                    input_worker.dismiss_briefings(":1", root)

    def test_fatal_native_input_exit_is_confined_and_log_retained(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            def snapshot(_source, target):
                target.write_text("import os,sys\nprint('BadWindow: window disappeared', file=sys.stderr, flush=True)\nos._exit(17)\n")
                return evidence.digest(target)
            with patch.object(input_worker, "snapshot_executable", snapshot), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaisesRegex(RuntimeError, "BadWindow"):
                    input_worker.dismiss_briefings(":1", root)
            self.assertIn("BadWindow", next(root.glob("briefing-input-*.log")).read_text())

    def test_input_timeout_reaps_worker_and_retains_log(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            pidfile = root / "worker.pid"
            def snapshot(_source, target):
                target.write_text(f"import os,pathlib,time\npathlib.Path({str(pidfile)!r}).write_text(str(os.getpid()))\ntime.sleep(60)\n")
                return evidence.digest(target)
            with patch.object(input_worker, "snapshot_executable", snapshot), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(subprocess.TimeoutExpired):
                    input_worker.dismiss_briefings(":1", root, timeout=0.3)
            with self.assertRaises(ProcessLookupError):
                os.kill(int(pidfile.read_text()), 0)
            self.assertTrue(list(root.glob("briefing-input-*.log")))

    def test_driver_failure_paths_write_reports_and_reap_children(self):
        cases = [RuntimeError("BadWindow: window disappeared"),
                 RuntimeError("no real Robin windows for briefing dismissal"),
                 RuntimeError("live game exited 7 during injection"),
                 subprocess.TimeoutExpired("input", 10),
                 evidence.DriverInterrupted("bounded driver interrupted by signal 15")]
        for error in cases:
            with self.subTest(error=str(error)), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                source = root / "robin"
                source.write_bytes(b"fake game, never executed")
                output = root / "evidence"
                children = []
                original_popen = subprocess.Popen
                def launch(argv, **kwargs):
                    if argv[0] == "Xvfb":
                        descriptor = kwargs["pass_fds"][0]
                        code = f"import os,time; os.write({descriptor}, b'77\\n'); os.close({descriptor}); time.sleep(60)"
                    else:
                        code = "import time; time.sleep(60)"
                    child = original_popen([sys.executable, "-c", code], **kwargs)
                    children.append(child)
                    return child
                argv = ["frame_steps_live", "--binary", str(source), "--data", str(root),
                        "--evidence", str(output), "--snapshot", "fixture"]
                # One poll: missing windows is retriable at startup, then the
                # ordinary bounded wait must fail with retained evidence.
                with patch.object(sys, "argv", argv), \
                     patch.object(frame, "require_python_xlib"), \
                     patch.object(frame.subprocess, "Popen", launch), \
                     patch.object(frame.subprocess, "check_output", return_value=b'[{"ifname":"lo"}]'), \
                     patch.object(frame.subprocess, "run"), \
                     patch.object(frame, "dismiss_briefings", side_effect=error), \
                     patch.object(frame.urllib.request, "urlopen", side_effect=OSError("no game RPC")), \
                     patch.object(frame.time, "monotonic", side_effect=[0, 0, 91]), \
                     patch.object(frame.time, "sleep"), \
                     patch.object(frame.signal, "signal"), \
                     patch.object(frame.signal, "alarm"), \
                     contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(frame.main(), 1)
                result = json.loads((output / "summary.json").read_text())
                self.assertFalse(result["completed"])
                self.assertIn("error", result)
                self.assertTrue(result["binary_sha256"])
                self.assertEqual(len(children), 2)
                for child in children:
                    self.assertIsNotNone(child.poll())
                    with self.assertRaises(ProcessLookupError):
                        os.kill(child.pid, 0)

    def test_missing_binary_still_writes_structured_summary(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            argv = ["frame_steps_live", "--binary", str(root / "missing"), "--data", str(root),
                    "--evidence", str(root / "evidence"), "--snapshot", "fixture"]
            with patch.object(sys, "argv", argv), patch.object(frame.signal, "signal"), \
                 patch.object(frame.signal, "alarm"), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(frame.main(), 1)
            report = json.loads((root / "evidence/summary.json").read_text())
            self.assertEqual(report["error_type"], "FileNotFoundError")
            self.assertFalse(report["completed"])


if __name__ == "__main__":
    unittest.main()
