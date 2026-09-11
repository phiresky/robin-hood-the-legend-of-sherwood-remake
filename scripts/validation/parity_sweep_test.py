#!/usr/bin/env python3
import json
from pathlib import Path
import subprocess
import tempfile
import threading
import time
import types
import unittest
from unittest.mock import patch
import parity_sweep as sweep


class SweepTests(unittest.TestCase):
    def test_footer_is_extent_not_a_gameplay_verdict(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/"trace"
            path.write_bytes(sweep.FOOTER.pack(sweep.MAGIC, 68, 123, 456))
            self.assertEqual(sweep.extent(path), dict(native_version=68, frames=123, final_frame=456))
            path.write_bytes(sweep.FOOTER.pack(b"x"*16, 68, 123, 456))
            with self.assertRaises(ValueError):
                sweep.extent(path)

    def test_selection_is_stable_unique_and_breadth_first(self):
        entries = [dict(path=f"{campaign}/{index}", campaign=campaign,
                        save=f"save{index}", bytes=index+1, frames=100+index)
                   for campaign in ("interactive", "a", "b") for index in range(10)]
        chosen = sweep.choose(entries, 24)
        self.assertEqual(chosen, sweep.choose(list(reversed(entries)), 24))
        self.assertEqual(len({e["path"] for e in chosen}), 24)
        self.assertEqual({e["campaign"] for e in chosen[:3]}, {"a", "b", "interactive"})
        self.assertEqual(sum(e["campaign"] == "interactive" for e in chosen), 10)

    def case(self, changes=None, status=0, timeout=False):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            for name in ("traces", "locks", "logs", "results"):
                (output/name).mkdir()
            trace = output/"traces/trace"
            trace.write_bytes(b"native")
            runner = output/"runner"
            runner.write_bytes(b"runner")
            entry = dict(path="trace", campaign="fixture", sha256=sweep.digest(trace), frames=2, final_frame=7)
            result = dict(result_version=1, trace_path=str(trace), native_trace_sha256=entry["sha256"],
                          executable_path=str(runner), executable_sha256=sweep.digest(runner),
                          expected_frames=2, processed_frames=2, expected_final_frame=7, final_frame=7,
                          terminator_validated=True, divergent_frames=0, outcome="exact_eof",
                          capabilities=dict(policy_version=1, trace_schema=16, native_version=68, exceptions=[]))
            result.update(changes or {})
            def execute(*args, **kwargs):
                self.assertIn("--core-datadir", args[0])
                self.assertTrue(Path(args[0][args[0].index("--core-datadir") + 1]).is_absolute())
                if timeout:
                    raise subprocess.TimeoutExpired("fixture", 1)
                kwargs["stdout"].write(("ROBIN_PARITY_RESULT "+json.dumps(result)+"\n").encode())
                return types.SimpleNamespace(returncode=status)
            with patch.object(sweep.subprocess, "run", execute):
                return sweep.run_case(entry, 0, output, runner, sweep.digest(runner), output, 1)

    def test_exact_eof_requires_identity_and_full_extent(self):
        self.assertEqual(self.case()["classification"], "exact_eof")
        self.assertEqual(self.case(dict(executable_sha256="a"*64))["classification"], "error")
        self.assertEqual(self.case(dict(processed_frames=1))["classification"], "error")

    def test_divergence_and_timeout_remain_distinct(self):
        self.assertEqual(self.case(dict(outcome="divergence", divergent_frames=1), status=1)["classification"], "divergence")
        self.assertEqual(self.case(timeout=True)["classification"], "timeout")

    def test_interactive_families_are_not_collapsed(self):
        self.assertEqual(sweep.identity(Path("interactive/interactive-session-003-session-0004.jsonl.zst.parity.bitcode.zst")),
                         ("interactive", "interactive-session-003"))

    def test_campaign_budget_preserves_final_counts_and_two_worker_bound(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            corpus, output = root/"corpus", root/"audit"
            corpus.mkdir()
            output.mkdir()
            entries = []
            for index in range(8):
                path = corpus/str(index)
                path.write_bytes(b"fixture")
                entries.append(dict(path=str(index), campaign="fixture", sha256=sweep.digest(path)))
            (output/"manifest.json").write_text(json.dumps(dict(manifest_version=1,
                source_commit="fixture", source_corpus=str(corpus), artifacts=entries)))
            runner = root/"runner"
            runner.write_bytes(b"runner fixture")
            counts = dict(active=0, peak=0)
            lock = threading.Lock()
            def worker(entry, index, output, runner, runner_sha, datadir, timeout, core_datadir):
                with lock:
                    counts["active"] += 1
                    counts["peak"] = max(counts["peak"], counts["active"])
                time.sleep(timeout + 0.03)
                with lock:
                    counts["active"] -= 1
                return dict(index=index, classification="timeout", status=124)
            args = types.SimpleNamespace(output=output, runner=runner, datadir=corpus,
                                         workers=2, timeout=10, hours=1/3600)
            with patch.object(sweep, "run_case", worker), patch("builtins.print"):
                self.assertEqual(sweep.run(args), 1)
            report = json.loads((output/"campaign-result.json").read_text())
            self.assertEqual(counts["peak"], 2)
            self.assertEqual(report["counts"], dict(timeout=2, not_run_budget=6))
            with sweep.sqlite3.connect(output/"ledger.sqlite3") as db:
                self.assertEqual(db.execute("SELECT count(*) FROM cases WHERE state IN ('pending','running')").fetchone()[0], 0)
            db.close()

    def resumed_campaign(self, expired=False, corrupt=False, changed_core=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            corpus, output = root/"corpus", root/"audit"
            corpus.mkdir()
            output.mkdir()
            entries = []
            for index in range(3):
                path = corpus/str(index)
                path.write_bytes(b"fixture")
                entries.append(dict(path=str(index), campaign="fixture", sha256=sweep.digest(path)))
            sweep.write_json(output/"manifest.json", dict(manifest_version=1,
                source_commit="fixture", source_corpus=str(corpus), artifacts=entries))
            runner = root/"runner"
            runner.write_bytes(b"runner fixture")
            args = types.SimpleNamespace(output=output, runner=runner, datadir=corpus,
                                         workers=1, timeout=10, hours=1/3600)
            def worker(entry, index, output, *unused):
                log = output/"logs"/f"{index:04d}.log"
                log.write_text("partial external interruption")
                if index == 1 and not getattr(args, "resume", False):
                    raise RuntimeError("simulated controller interruption")
                record = dict(index=index, trace=entry["path"], classification="timeout",
                              status=124, log=str(log), log_sha256=sweep.digest(log))
                sweep.write_json(output/"results"/f"{index:04d}.json", record)
                return record
            with patch.object(sweep, "run_case", worker), patch("builtins.print"):
                with self.assertRaisesRegex(RuntimeError, "simulated"):
                    sweep.run(args)
            preserved = (output/"results/0000.json").read_bytes()
            launch = json.loads((output/"launch.json").read_text())
            if expired:
                launch["started_unix"] -= 10
                sweep.write_json(output/"launch.json", launch)
            if corrupt:
                (output/"logs/0000.log").write_text("tampered")
            args.resume = True
            if changed_core:
                args.core_datadir = root / "different-core"
                (args.core_datadir / "Data").mkdir(parents=True)
                (args.core_datadir / "Data/AudioDurations.json").write_text("different timing")
            calls = []
            def resumed(*arguments):
                calls.append(arguments[1])
                return worker(*arguments)
            with patch.object(sweep, "run_case", resumed), patch("builtins.print"):
                if changed_core:
                    with self.assertRaisesRegex(ValueError, "resume provenance"):
                        sweep.run(args)
                    self.assertFalse(calls)
                    self.assertFalse((output/"interruptions").exists())
                    return
                if corrupt:
                    with self.assertRaisesRegex(ValueError, "saved result mismatch"):
                        sweep.run(args)
                    self.assertFalse((output/"interruptions").exists())
                    return
                self.assertEqual(sweep.run(args), 1)
            self.assertEqual(calls, [] if expired else [1, 2])
            self.assertEqual((output/"results/0000.json").read_bytes(), preserved)
            self.assertEqual(json.loads((output/"launch.json").read_text()), launch)
            archives = list((output/"interruptions").iterdir())
            self.assertEqual((archives[0]/"0001.log").read_text(), "partial external interruption")
            report = json.loads((output/"campaign-result.json").read_text())
            self.assertEqual(report["counts"], dict(timeout=1, not_run_budget=2) if expired else dict(timeout=3))

    def test_resume_preserves_completed_and_archives_interruption(self):
        self.resumed_campaign()

    def test_resume_does_not_reset_original_deadline(self):
        self.resumed_campaign(expired=True)

    def test_resume_rejects_modified_evidence_before_mutation(self):
        self.resumed_campaign(corrupt=True)

    def test_resume_rejects_different_core_authority(self):
        self.resumed_campaign(changed_core=True)

    def test_running_sweep_rejects_changed_core_before_publishing_success(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            corpus, output, core = root/"corpus", root/"audit", root/"core"
            corpus.mkdir()
            output.mkdir()
            (core/"Data").mkdir(parents=True)
            timing = core/"Data/AudioDurations.json"
            timing.write_text("original timing")
            trace = corpus/"trace"
            trace.write_bytes(b"fixture")
            sweep.write_json(output/"manifest.json", dict(manifest_version=1,
                source_commit="fixture", source_corpus=str(corpus), artifacts=[
                    dict(path="trace", campaign="fixture", sha256=sweep.digest(trace))]))
            runner = root/"runner"
            runner.write_bytes(b"runner fixture")
            args = types.SimpleNamespace(output=output, runner=runner, datadir=corpus,
                core_datadir=core, workers=1, timeout=10, hours=1/3600)
            def worker(*unused):
                timing.write_text("replacement timing")
                return dict(index=0, classification="exact_eof", status=0)
            with patch.object(sweep, "run_case", worker), patch("builtins.print"):
                with self.assertRaisesRegex(ValueError, "core timing input changed"):
                    sweep.run(args)
            self.assertFalse((output/"campaign-result.json").exists())


if __name__ == "__main__":
    unittest.main()
