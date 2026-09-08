"""Fixture-only orchestration checks; never invoke game, Cargo or real browser."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import lifecycle_gate as gate


class LifecycleGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.evidence = self.root / "evidence"
        self.evidence.mkdir()
        self.binary = self.root / "robin"
        self.binary.write_bytes(b"immutable fixture, not executable")
        (self.root / "Data").mkdir()
        self.env = dict(ROBIN_LIFECYCLE_BINARY=str(self.binary),
                        ROBINHOOD_DATA_DIR=str(self.root),
                        ROBIN_LIFECYCLE_BINARY_SHA256=gate.digest(self.binary),
                        ROBIN_LIFECYCLE_SNAPSHOT="fixture-source")

    def test_missing_executable_is_an_error(self):
        with self.assertRaisesRegex(RuntimeError, "required executable"):
            gate.executable(str(self.root / "missing"))

    def test_ambiguous_wasm_bindgen_lockfile_fails(self):
        (self.root / "Cargo.lock").write_text(
            '[[package]]\nname="wasm-bindgen"\nversion="0.2.1"\n'
            '[[package]]\nname="wasm-bindgen"\nversion="0.2.2"\n')
        with patch.object(gate, "ROOT", self.root):
            with self.assertRaisesRegex(RuntimeError, "expected one"):
                gate.lock_bindgen_version()

    def test_native_requires_matching_binary_before_execution(self):
        with patch.dict(os.environ, self.env), patch.object(gate, "executable"), \
                patch.object(gate, "run") as run:
            os.environ["ROBIN_LIFECYCLE_BINARY_SHA256"] = "bad"
            with self.assertRaisesRegex(RuntimeError, "SHA256"):
                gate.native(self.evidence, {})
            run.assert_not_called()

    def test_native_requires_real_data_layout(self):
        (self.root / "Data").rmdir()
        with patch.dict(os.environ, self.env), patch.object(gate, "run") as run:
            with self.assertRaisesRegex(RuntimeError, "contain Data"):
                gate.native(self.evidence, {})
            run.assert_not_called()

    def fake_native_run(self, argv, **kwargs):
        self.assertEqual(argv[:5], ["unshare", "--user", "--map-root-user", "--net", sys.executable])
        self.assertEqual(kwargs["timeout"], 330)
        destination = Path(argv[argv.index("--evidence") + 1])
        destination.mkdir()
        result = {"completed": True, "binary_sha256": gate.digest(self.binary),
                  "snapshot": "fixture-source",
                  "checks": dict.fromkeys(gate.REPLAY_CHECKS | gate.LIVE_CHECKS | gate.SAVE_CHECKS, True),
                  "save_load": {"load_record_frame": 149, "final_record_frame": 259}}
        (destination / "summary.json").write_text(json.dumps(result))
        (destination / "export.rhrec").write_bytes(b"fixture replay")
        (destination / "playback.log").write_text("fixture log")

    def test_native_runs_both_exports_in_both_playback_modes(self):
        summary = {}
        with patch.dict(os.environ, self.env), patch.object(gate, "executable"), \
                patch.object(gate, "run", side_effect=self.fake_native_run) as run, \
                patch("save_load_live.verify_replay") as verify:
            gate.native(self.evidence, summary)
        self.assertEqual(run.call_count, 4)
        commands = [call.args[0] for call in run.call_args_list]
        self.assertNotIn("--save-load", commands[0])
        self.assertIn("--save-load", commands[2])
        for index in (1, 3):
            self.assertIn("--graphical-replay", commands[index])
            self.assertIn("--replay-file", commands[index])
        verify.assert_called_once_with("fixture log", 149, 259)
        self.assertEqual(len(summary["checks"]), 4)

    def test_native_rejects_binary_change_between_phases(self):
        def mutate(argv, **kwargs):
            self.fake_native_run(argv, **kwargs)
            self.binary.write_bytes(b"changed")
        with patch.dict(os.environ, self.env), patch.object(gate, "executable"), \
                patch.object(gate, "run", side_effect=mutate) as run:
            with self.assertRaisesRegex(RuntimeError, "binary changed"):
                gate.native(self.evidence, {})
        self.assertEqual(run.call_count, 1)

    def test_native_rejects_incomplete_summary(self):
        def incomplete(argv, **kwargs):
            self.fake_native_run(argv, **kwargs)
            target = Path(argv[argv.index("--evidence") + 1]) / "summary.json"
            target.write_text('{"completed": false}')
        with patch.dict(os.environ, self.env), patch.object(gate, "executable"), \
                patch.object(gate, "run", side_effect=incomplete):
            with self.assertRaisesRegex(RuntimeError, "invalid acceptance"):
                gate.native(self.evidence, {})

    def test_native_rejects_completed_summary_missing_evidence_or_wrong_source(self):
        for change in ({"checks": {}}, {"checks": {"native_playback_finished": False}},
                       {"snapshot": "different-source"}):
            with self.subTest(change=change):
                destination = self.root / ("case-" + str(len(list(self.root.iterdir()))))
                destination.mkdir()
                def invalid(argv, **kwargs):
                    self.fake_native_run(argv, **kwargs)
                    target = Path(argv[argv.index("--evidence") + 1]) / "summary.json"
                    result = json.loads(target.read_text())
                    result.update(change)
                    target.write_text(json.dumps(result))
                with patch.dict(os.environ, self.env), patch.object(gate, "executable"), \
                        patch.object(gate, "run", side_effect=invalid):
                    with self.assertRaisesRegex(RuntimeError, "invalid acceptance"):
                        gate.native(destination, {})

    def test_browser_rejects_runner_version_before_build(self):
        with patch.dict(os.environ, CHROME="chrome", CHROMEDRIVER="driver",
                        WASM_BINDGEN_TEST_RUNNER="runner"), \
                patch.object(gate, "executable", side_effect=lambda x: x), \
                patch.object(gate.subprocess, "check_output", return_value="runner 0.0.1"), \
                patch.object(gate, "run") as run:
            with self.assertRaisesRegex(RuntimeError, "does not match"):
                gate.browser(self.evidence, {})
            run.assert_not_called()

    def test_browser_rejects_mismatched_driver_before_build(self):
        with patch.dict(os.environ, CHROME="chrome", CHROMEDRIVER="driver",
                        WASM_BINDGEN_TEST_RUNNER="runner"), \
                patch.object(gate, "executable", side_effect=lambda x: x), \
                patch.object(gate.subprocess, "check_output", side_effect=[
                    "runner " + gate.lock_bindgen_version(), "Chrome 152.1", "ChromeDriver 151.1"]), \
                patch.object(gate, "run") as run:
            with self.assertRaisesRegex(RuntimeError, "major versions"):
                gate.browser(self.evidence, {})
            run.assert_not_called()

    def test_browser_links_exact_artifact_and_rejects_zero_tests(self):
        artifact = self.root / "fixture.wasm"
        artifact.write_bytes(b"fixture wasm")
        event = {"reason": "compiler-artifact", "target": {"name": "robin_rs"},
                 "profile": {"test": True}, "executable": str(artifact)}
        for count in (0, 5):
            def fake_run(argv, **kwargs):
                if "log" in kwargs:
                    options = json.loads(Path(kwargs["env"]["WASM_BINDGEN_TEST_WEBDRIVER_JSON"]).read_text())
                    args = options["goog:chromeOptions"]["args"]
                    self.assertTrue(any(x.startswith("--user-data-dir=") for x in args))
                    kwargs["log"].write_text(f"test web_audio_backend::ownership ... ok\ntest result: ok. {count} passed; 0 failed")
                else:
                    self.assertIn("audio", argv[argv.index("--features") + 1].split(","))
                    self.assertIn("check", argv)
                    self.assertNotIn("timeout", kwargs)
            with patch.dict(os.environ, CHROME="chrome", CHROMEDRIVER="driver",
                            WASM_BINDGEN_TEST_RUNNER="runner"), \
                    patch.object(gate, "executable", side_effect=lambda x: x), \
                    patch.object(gate.subprocess, "check_output", side_effect=[
                        "runner " + gate.lock_bindgen_version(), "Chrome 152.1", "Driver 152.1"]), \
                    patch.object(gate, "run", side_effect=fake_run), \
                    patch.object(gate.subprocess, "Popen") as popen:
                child = popen.return_value.__enter__.return_value
                child.stdout = [json.dumps(event)]
                child.wait.return_value = 0
                if count:
                    summary = {}
                    gate.browser(self.evidence, summary)
                    self.assertEqual(summary["browser_tests_passed"], count)
                else:
                    with self.assertRaisesRegex(RuntimeError, "nonempty"):
                        gate.browser(self.evidence, {})
                command = popen.call_args.args[0]
                self.assertIn("--no-run", command)
                self.assertIn("--locked", command)
                self.assertNotIn("--target-dir", command)

    def test_timeout_reaps_owned_process(self):
        pid_file = self.root / "pid"
        command = [sys.executable, "-c", "import os,pathlib,signal,time; "
                   "signal.signal(signal.SIGTERM,signal.SIG_IGN); "
                   f"pathlib.Path({str(pid_file)!r}).write_text(str(os.getpid())); time.sleep(60)"]
        with self.assertRaises(subprocess.TimeoutExpired):
            gate.run(command, timeout=0.5)
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pid_file.read_text()), 0)


if __name__ == "__main__":
    unittest.main()
