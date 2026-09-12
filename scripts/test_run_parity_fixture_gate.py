#!/usr/bin/env python3
import json
from pathlib import Path
import tempfile
import types
import unittest
from unittest.mock import patch

import run_parity_fixture_gate as gate


class FixtureGateTests(unittest.TestCase):
    def run_gate(self, *, legacy=False, mutation=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            corpus, core, output = root / "corpus", root / "core", root / "audit"
            corpus.mkdir()
            timing = core / "Data/AudioDurations.json"
            if not legacy:
                timing.parent.mkdir(parents=True)
                timing.write_text("original timing")
                timing_sha = gate.digest(timing)
            runner = root / "runner"
            runner.write_bytes(b"runner fixture")
            artifacts = []
            for index in range(2):
                trace = corpus / str(index)
                trace.write_bytes(b"native fixture")
                artifacts.append(dict(path=str(index), sha256=gate.digest(trace),
                                      frames=2, final_frame=7, run=True))
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps(dict(manifest_version=1, artifacts=artifacts)))
            argv = ["fixture-gate", "--runner", str(runner), "--corpus", str(corpus),
                    "--datadir", str(corpus), "--output", str(output), "--manifest", str(manifest)]
            argv += ["--allow-legacy-result"] if legacy else ["--core-datadir", str(core)]
            calls = []

            def execute(command, **kwargs):
                calls.append(command)
                if legacy:
                    self.assertNotIn("--core-datadir", command)
                    log = "parity trace matched every recorded frame\n"
                else:
                    self.assertEqual(command[command.index("--core-datadir") + 1], str(core))
                    result = dict(result_version=1, trace_path=command[-1],
                        native_trace_sha256=artifacts[0]["sha256"], executable_path=command[0],
                        executable_sha256=gate.digest(Path(command[0])), expected_frames=2,
                        processed_frames=2, expected_final_frame=7, final_frame=7,
                        terminator_validated=True, divergent_frames=0, outcome="exact_eof",
                        capabilities=dict(policy_version=1, trace_schema=16, native_version=68, exceptions=[]))
                    log = "ROBIN_PARITY_RESULT " + json.dumps(result) + "\n"
                kwargs["stdout"].write(log.encode())
                if mutation == "during_case":
                    timing.write_text("changed timing")
                return types.SimpleNamespace(returncode=0)

            def printed(message, **kwargs):
                if mutation == "between_cases" and message.startswith("  exact EOF"):
                    timing.write_text("changed timing")

            with patch.object(gate.subprocess, "run", execute), patch("sys.argv", argv), patch("builtins.print", printed):
                if mutation:
                    with self.assertRaisesRegex(ValueError, "core timing input changed"):
                        gate.main()
                    self.assertEqual(len(calls), 1)
                    self.assertFalse((output / "gate-result.json").exists())
                    return
                self.assertEqual(gate.main(), 0)
            report = json.loads((output / "gate-result.json").read_text())
            self.assertEqual(len(calls), 2)
            self.assertEqual(len(report["results"]), 2)
            self.assertTrue(all(record["exact_eof"] for record in report["results"]))
            self.assertEqual(report["legacy_result_allowed"], legacy)
            if legacy:
                self.assertIsNone(report["core_input"])
                self.assertIsNone(report["core_datadir"])
            else:
                self.assertEqual(report["core_input"], dict(path=str(core), audio_durations_sha256=timing_sha))

    def test_report_records_core_timing_digest(self):
        self.run_gate()

    def test_rejects_timing_mutation_during_case(self):
        self.run_gate(mutation="during_case")

    def test_rejects_timing_mutation_between_cases(self):
        self.run_gate(mutation="between_cases")

    def test_legacy_runner_does_not_require_core_authority(self):
        self.run_gate(legacy=True)


if __name__ == "__main__":
    unittest.main()
