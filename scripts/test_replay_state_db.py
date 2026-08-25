#!/usr/bin/env python3
"""Focused tests for the authoritative replay-state ledger."""

from __future__ import annotations

import hashlib
import importlib.util
import sqlite3
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "replay_state_db", ROOT / "scripts" / "replay_state_db.py"
)
assert SPEC and SPEC.loader
DB = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DB)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class ReplayStateDatabaseTests(unittest.TestCase):
    def setUp(self) -> None:
        (ROOT / ".agent-debug").mkdir(exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(
            prefix="replay-state-test.", dir=ROOT / ".agent-debug"
        )
        self.root = Path(self.temporary.name)
        self.database = self.root / "state.sqlite3"
        self.connection = DB.connect(self.database)

    def tearDown(self) -> None:
        self.connection.close()
        self.temporary.cleanup()

    def evidence(self, name: str, status: str, log: str) -> Path:
        result = self.root / "audit" / "results" / name
        result.mkdir(parents=True)
        logical = "parity-save-replays/corpus/traces/save/replay-001-session-0001.jsonl.zst"
        (result / "trace.path").write_text(f"{logical}\n")
        (result / "status").write_text(f"{status}\n")
        (result / "log").write_text(log)
        command_status = status if status.isdigit() else "143"
        marker_count = int(log.strip() == DB.EOF_MARKER)
        (result / "attestation.env").write_text(
            "FORMAT=schema16-incremental-eof-v1\n"
            "STARTED_UTC=2026-08-25T00:00:00Z\n"
            "FINISHED_UTC=2026-08-25T00:00:05Z\n"
            f"RUNNER_RAW_SHA256={'1' * 64}\n"
            f"RUNNER_BUNDLE_TRUST_SHA256={'2' * 64}\n"
            f"NATIVE_SHA256_PRE={'3' * 64}\n"
            f"NATIVE_SHA256_POST={'3' * 64}\n"
            f"RUNNER_COMMAND_STATUS={command_status}\n"
            f"EXACT_EOF_MARKER_COUNT={marker_count}\n"
            f"LOG_SHA256={digest(log.encode())}\n"
        )
        entries = []
        for filename in ("attestation.env", "log", "status", "trace.path"):
            entries.append(f"{DB.sha256_file(result / filename)}  {filename}\n")
        (result / "MANIFEST.sha256").write_text("".join(entries))
        return result

    def test_import_is_idempotent_and_preserves_every_distinct_run(self) -> None:
        exact = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        audit = self.root / "audit"
        with self.connection:
            self.assertTrue(DB.import_result(self.connection, exact, audit, None, "host-a"))
            self.assertFalse(DB.import_result(self.connection, exact, audit, None, "host-a"))
        mismatch = self.evidence(
            "mismatch", "1", "first parity divergence after frame 1566 (1 difference):\n"
        )
        with self.connection:
            self.assertTrue(DB.import_result(self.connection, mismatch, audit, None, "host-a"))
        rows = self.connection.execute(
            "SELECT outcome,divergence_frame FROM replay_runs ORDER BY run_id"
        ).fetchall()
        self.assertEqual(
            [(row["outcome"], row["divergence_frame"]) for row in rows],
            [("exact_eof", None), ("mismatch", 1566)],
        )

    def test_tampered_manifest_is_rejected(self) -> None:
        result = self.evidence("tamper", "0", f"{DB.EOF_MARKER}\n")
        (result / "log").write_text("changed\n")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")

    def test_aborted_latest_attempt_remains_untested_not_failed(self) -> None:
        result = self.evidence("aborted", "aborted-controller-signal", "")
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            DB.activate_corpus(
                self.connection, "parity-save-replays/corpus", 1,
                None, None, None, str(self.root / "corpus"),
            )
            DB.set_current_runner(self.connection, "2" * 64)
        report = DB.overview(self.connection)
        corpus = report["final_set"][0]
        self.assertEqual(corpus["current_failed"], 0)
        self.assertEqual(corpus["current_aborted"], 1)
        self.assertEqual(corpus["current_untested"], 1)

    def test_resource_signal_is_aborted_even_for_legacy_numeric_status(self) -> None:
        result = self.evidence("oom", "137", "")
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
        row = self.connection.execute(
            "SELECT outcome,result_status,command_status FROM replay_runs"
        ).fetchone()
        self.assertEqual(
            (row["outcome"], row["result_status"], row["command_status"]),
            ("aborted", "137", 137),
        )

    def test_legacy_resource_crash_is_corrected_and_requeued_append_only(self) -> None:
        result = self.evidence("legacy-oom", "137", "")
        original_classify = DB.classify
        DB.classify = lambda status, command_status, marker_count, log: "crash"
        try:
            with self.connection:
                DB.import_result(
                    self.connection, result, self.root / "audit", None, "host-a"
                )
        finally:
            DB.classify = original_classify
        logical = "parity-save-replays/corpus/traces/save/replay-001-session-0001.jsonl.zst"
        with self.connection:
            work_id = DB.add_work(
                self.connection, logical, "replay", "2" * 64,
                None, None, "3" * 64, 100,
            )
        claim = DB.claim_work(self.connection, "replay", "host-a:1", 60)
        evidence_key = self.connection.execute(
            "SELECT evidence_key FROM replay_runs"
        ).fetchone()[0]
        DB.complete_work(self.connection, claim["claim_token"], "crash", evidence_key)

        result_counts = DB.retry_resource_aborts(
            self.connection, "2" * 64, str((self.root / "audit").resolve())
        )
        self.assertEqual(result_counts, {"corrected": 1, "requeued": 1})
        self.assertEqual(
            self.connection.execute("SELECT count(*) FROM replay_runs").fetchone()[0], 1
        )
        correction = self.connection.execute(
            "SELECT corrected_outcome FROM replay_run_corrections"
        ).fetchone()[0]
        self.assertEqual(correction, "aborted")
        retry = DB.claim_work(self.connection, "replay", "host-a:2", 60)
        self.assertIsNotNone(retry)
        self.assertNotEqual(retry["work_id"], work_id)

    def test_run_evidence_is_append_only(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
        with self.assertRaisesRegex(sqlite3.IntegrityError, "append-only"):
            self.connection.execute("UPDATE replay_runs SET result_status='1'")
        with self.assertRaisesRegex(sqlite3.IntegrityError, "append-only"):
            self.connection.execute("DELETE FROM replay_runs")

    def test_attested_exact_lookup_is_bound_to_runner_and_native_bytes(self) -> None:
        exact = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        audit = self.root / "audit"
        logical = "parity-save-replays/corpus/traces/save/replay-001-session-0001.jsonl.zst"
        with self.connection:
            DB.import_result(self.connection, exact, audit, None, "host-a")
        self.assertTrue(DB.has_attested_exact(
            self.connection, logical, "2" * 64, "3" * 64
        ))
        evidence_key = self.connection.execute(
            "SELECT evidence_key FROM replay_runs"
        ).fetchone()[0]
        self.assertEqual(
            DB.exact_evidence_key(self.connection, logical, "2" * 64, "3" * 64),
            evidence_key,
        )
        self.assertIsNone(
            DB.exact_evidence_key(self.connection, logical, "2" * 64, "4" * 64)
        )
        self.assertFalse(DB.has_attested_exact(
            self.connection, logical, "2" * 64, "4" * 64
        ))
        self.assertFalse(DB.has_attested_exact(
            self.connection, logical, "5" * 64, "3" * 64
        ))

    def test_atomic_claim_prevents_duplicate_work(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            work_id = DB.add_work(
                self.connection,
                "parity-save-replays/corpus/traces/save/replay-001-session-0001.jsonl.zst",
                "replay",
                "2" * 64,
                None,
                None,
                None,
                0,
            )
        contender = DB.connect(self.database)
        try:
            claim = DB.claim_work(self.connection, "replay", "host-a:1", 3600)
            self.assertIsNotNone(claim)
            self.assertEqual(claim["work_id"], work_id)
            self.assertIsNone(DB.claim_work(contender, "replay", "host-b:1", 3600))
            DB.complete_work(self.connection, claim["claim_token"], "exact_eof", None)
            self.assertIsNone(DB.claim_work(contender, "replay", "host-b:1", 3600))
        finally:
            contender.close()

    def test_work_completion_rejects_unrelated_evidence(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            work_id = DB.add_work(
                self.connection,
                "parity-save-replays/corpus/traces/save/replay-001-session-0001.jsonl.zst",
                "replay",
                "2" * 64,
                None,
                None,
                "4" * 64,
                0,
            )
            evidence_key = self.connection.execute(
                "SELECT evidence_key FROM replay_runs"
            ).fetchone()[0]
        claim = DB.claim_work(self.connection, "replay", "host-a:1", 60)
        with self.assertRaisesRegex(ValueError, "native digest"):
            DB.complete_work(
                self.connection, claim["claim_token"], "exact_eof", evidence_key
            )
        self.assertEqual(claim["work_id"], work_id)
        self.assertIsNotNone(
            self.connection.execute(
                "SELECT 1 FROM work_claims WHERE claim_token=?",
                (claim["claim_token"],),
            ).fetchone()
        )

    def test_expired_claim_is_reassigned(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            DB.add_work(
                self.connection,
                "parity-save-replays/corpus/traces/save/replay-001-session-0001.jsonl.zst",
                "convert",
                None,
                2,
                "parity-bitcode-v2.zst",
                "3" * 64,
                0,
            )
        first = DB.claim_work(self.connection, "convert", "host-a:1", 3600)
        self.connection.execute(
            "UPDATE work_claims SET lease_until_utc='2000-01-01T00:00:00.000Z'"
        )
        self.connection.commit()
        second = DB.claim_work(self.connection, "convert", "host-b:1", 3600)
        self.assertEqual(first["work_id"], second["work_id"])
        self.assertNotEqual(first["claim_token"], second["claim_token"])

    def test_work_claim_renewal_is_token_authenticated(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            DB.add_work(
                self.connection,
                "parity-save-replays/corpus/traces/save/replay-001-session-0001.jsonl.zst",
                "replay",
                "2" * 64,
                None,
                None,
                "3" * 64,
                0,
            )
        self.assertIsNone(DB.claim_work(
            self.connection, "replay", "host-a:wrong", 60,
            "2" * 64, "parity-save-replays/another-corpus",
        ))
        first = DB.claim_work(
            self.connection, "replay", "host-a:1", 60,
            "2" * 64, "parity-save-replays/corpus",
        )
        renewed = DB.renew_work(self.connection, first["claim_token"], 120)
        self.assertEqual(renewed["work_id"], first["work_id"])
        self.assertEqual(renewed["worker_id"], "host-a:1")
        self.assertGreater(renewed["lease_until_utc"], first["lease_until_utc"])

        self.connection.execute(
            "UPDATE work_claims SET lease_until_utc='2000-01-01T00:00:00.000Z'"
        )
        self.connection.commit()
        second = DB.claim_work(self.connection, "replay", "host-b:1", 60)
        with self.assertRaisesRegex(ValueError, "unknown or superseded"):
            DB.renew_work(self.connection, first["claim_token"], 60)
        with self.assertRaisesRegex(ValueError, "unknown or expired"):
            DB.complete_work(self.connection, first["claim_token"], "exact_eof", None)
        self.assertEqual(
            DB.renew_work(self.connection, second["claim_token"], 60)["work_id"],
            second["work_id"],
        )

    def test_enqueue_corpus_replays_hashes_native_and_skips_exact(self) -> None:
        exact = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        logical = "parity-save-replays/corpus/traces/save/replay-001-session-0001.jsonl.zst"
        corpus = self.root / "corpus"
        native = corpus / "traces/save/replay-001-session-0001.jsonl.zst.parity.bitcode.zst"
        native.parent.mkdir(parents=True)
        native.write_bytes(b"current native\nRHPRTRACEFOOTER!" + bytes(20))
        marker = corpus / "traces/save/replay-001.complete"
        marker.write_text("complete\n")
        native_sha = DB.sha256_file(native)
        with self.connection:
            DB.import_result(self.connection, exact, self.root / "audit", None, "host-a")
            self.connection.execute(
                "UPDATE corpora SET corpus_path=? WHERE logical_root=?",
                (str(corpus), "parity-save-replays/corpus"),
            )
            self.connection.execute(
                "UPDATE replays SET completion_marker=? WHERE logical_path=?",
                ("parity-save-replays/corpus/traces/save/replay-001.complete", logical),
            )
        queued = DB.enqueue_corpus_replay_work(
            self.connection, "parity-save-replays/corpus", "2" * 64, 100
        )
        self.assertEqual(
            queued,
            {"members": 1, "native_ready": 1, "enqueued": 1,
             "skipped_exact": 0, "missing_native": 0,
             "missing_marker": 0, "invalid_footer": 0},
        )
        work = self.connection.execute(
            "SELECT source_sha256,priority FROM work_items"
        ).fetchone()
        self.assertEqual((work["source_sha256"], work["priority"]), (native_sha, 100))

        # Import a distinct evidence directory attesting the current native.
        current = self.evidence("current", "0", f"{DB.EOF_MARKER}\n")
        attestation = current / "attestation.env"
        contents = attestation.read_text().replace("3" * 64, native_sha)
        attestation.write_text(contents)
        entries = []
        for filename in ("attestation.env", "log", "status", "trace.path"):
            entries.append(f"{DB.sha256_file(current / filename)}  {filename}\n")
        (current / "MANIFEST.sha256").write_text("".join(entries))
        with self.connection:
            DB.import_result(self.connection, current, self.root / "audit", None, "host-a")
        skipped = DB.enqueue_corpus_replay_work(
            self.connection, "parity-save-replays/corpus", "2" * 64, 100
        )
        self.assertEqual(skipped["skipped_exact"], 1)
        self.assertEqual(skipped["enqueued"], 0)

    def test_corpus_work_lease_prevents_duplicate_and_expires(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            self.connection.execute("UPDATE corpora SET corpus_status='active'")
        logical_root = "parity-save-replays/corpus"
        first = DB.claim_corpus_work(
            self.connection, logical_root, "convert", "worker-a", "host-a",
            "/audit/a", "conversion", 3600,
        )
        with self.assertRaisesRegex(ValueError, "already leased"):
            DB.claim_corpus_work(
                self.connection, logical_root, "convert", "worker-b", "host-b",
                "/audit/b", None, 3600,
            )
        self.connection.execute(
            "UPDATE corpus_work_leases SET lease_until_utc='2000-01-01T00:00:00.000Z'"
        )
        self.connection.commit()
        second = DB.claim_corpus_work(
            self.connection, logical_root, "convert", "worker-b", "host-b",
            "/audit/b", None, 3600,
        )
        self.assertNotEqual(first["claim_token"], second["claim_token"])
        report = DB.overview(self.connection)
        self.assertEqual(report["corpus_work"][0]["worker_id"], "worker-b")
        with self.connection:
            DB.finish_corpus_work(
                self.connection, second["claim_token"], "completed", "done"
            )
        self.assertEqual(DB.overview(self.connection)["corpus_work"], [])

    def test_overview_reports_global_state(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            self.connection.execute("UPDATE corpora SET corpus_status='active'")
            DB.set_current_runner(self.connection, "2" * 64)
        report = DB.overview(self.connection)
        self.assertEqual(report["totals"], {"expected": 1, "current_exact": 1, "remaining": 0})
        self.assertEqual(report["final_set"][0]["current_exact"], 1)
        self.assertEqual(report["final_set"][0]["current_failed"], 0)

    def test_retired_corpus_is_hidden_from_operational_overview(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            logical_root = "parity-save-replays/corpus"
            assigned = DB.activate_corpus(
                self.connection, logical_root, 1, "host-a", "/corpus", "test", None
            )
            self.assertEqual(assigned, 1)
            DB.retire_corpus(self.connection, logical_root, "not in final set")
        report = DB.overview(self.connection)
        self.assertEqual(report["final_set"], [])
        self.assertEqual(report["hidden_corpora"], {"retired": 1})

    def test_merge_corpora_preserves_artifact_roots_and_membership(self) -> None:
        roots = [
            "parity-save-replays/seed1-base",
            "parity-save-replays/seed1-replacements",
            "parity-save-replays/seed1-recapture",
        ]
        with self.connection:
            for index, root in enumerate(roots):
                physical_root = self.root / f"source-{index}"
                logical = f"{root}/traces/save/replay-001-session-0001.jsonl.zst"
                physical = physical_root / "traces/save/replay-001-session-0001.jsonl.zst"
                physical.parent.mkdir(parents=True)
                physical.write_text("recording")
                corpus_id = DB.upsert_corpus(self.connection, root, expected=1)
                self.connection.execute(
                    "UPDATE corpora SET corpus_status='active',corpus_path=? WHERE corpus_id=?",
                    (str(physical_root), corpus_id),
                )
                replay_id = DB.upsert_replay(self.connection, logical, None)
                self.connection.execute(
                    "UPDATE replays SET corpus_id=? WHERE replay_id=?",
                    (corpus_id, replay_id),
                )
                self.connection.execute(
                    "INSERT INTO final_corpus_members VALUES(?,?,?)",
                    (corpus_id, replay_id, "test"),
                )
        target = "parity-save-replays/schema16-seed1000000-final"
        with self.connection:
            merged = DB.merge_corpora(
                self.connection, target, roots, 3, 1_000_000, 16
            )
        self.assertEqual(merged["final_members"], 3)
        report = DB.overview(self.connection)
        self.assertEqual(len(report["final_set"]), 1)
        self.assertEqual(report["final_set"][0]["logical_root"], target)
        self.assertEqual(report["final_set"][0]["registered"], 3)
        self.assertEqual(report["final_set"][0]["source_only"], 3)
        self.assertEqual(report["hidden_corpora"], {"retired": 3})

    def test_absorb_corpora_preserves_existing_target_and_artifact_roots(self) -> None:
        roots = ["parity-save-replays/30s", "parity-save-replays/30s/replacements"]
        with self.connection:
            for index, root in enumerate(roots):
                physical_root = self.root / f"absorb-{index}"
                physical_root.mkdir()
                corpus_id = DB.upsert_corpus(self.connection, root, expected=1)
                self.connection.execute(
                    "UPDATE corpora SET corpus_status='active',corpus_path=? WHERE corpus_id=?",
                    (str(physical_root), corpus_id),
                )
                self.connection.execute(
                    "INSERT INTO corpus_locations(corpus_id,host,path,note) VALUES(?,?,?,?)",
                    (corpus_id, "host-a", str(physical_root), "test"),
                )
                logical = f"{root}/traces/replay-{index:03}.jsonl.zst"
                replay_id = DB.upsert_replay(self.connection, logical, None)
                self.connection.execute(
                    "UPDATE replays SET corpus_id=? WHERE replay_id=?",
                    (corpus_id, replay_id),
                )
                self.connection.execute(
                    "INSERT INTO final_corpus_members VALUES(?,?,?)",
                    (corpus_id, replay_id, "test"),
                )
        target_path = self.connection.execute(
            "SELECT corpus_path FROM corpora WHERE logical_root=?", (roots[0],)
        ).fetchone()[0]
        with self.connection:
            absorbed = DB.absorb_corpora(self.connection, roots[0], roots[1:], 2)
        self.assertEqual(absorbed["final_members"], 2)
        target = self.connection.execute(
            "SELECT corpus_id,expected_replays,corpus_path FROM corpora WHERE logical_root=?",
            (roots[0],),
        ).fetchone()
        self.assertEqual(target["expected_replays"], 2)
        self.assertEqual(target["corpus_path"], target_path)
        self.assertEqual(
            self.connection.execute(
                "SELECT count(*) FROM final_corpus_members WHERE corpus_id=?",
                (target["corpus_id"],),
            ).fetchone()[0],
            2,
        )
        locations = self.connection.execute(
            "SELECT count(*) FROM corpus_locations WHERE corpus_id=?",
            (target["corpus_id"],),
        ).fetchone()[0]
        self.assertEqual(locations, 2)
        report = DB.overview(self.connection)
        self.assertEqual(len(report["final_set"]), 1)
        self.assertEqual(report["final_set"][0]["expected_replays"], 2)
        self.assertEqual(report["hidden_corpora"], {"retired": 1})
        with self.connection:
            DB.set_corpus_path(
                self.connection, roots[1], "/srv/replays/replacements", "host-b", "copy"
            )
        source = self.connection.execute(
            "SELECT corpus_status,corpus_path FROM corpora WHERE logical_root=?",
            (roots[1],),
        ).fetchone()
        self.assertEqual(source["corpus_status"], "retired")
        self.assertEqual(source["corpus_path"], "/srv/replays/replacements")

    def test_final_snapshot_excludes_nonmember_replays(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        logical = "parity-save-replays/corpus/traces/save/replay-001-session-0001.jsonl.zst"
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            DB.add_replay(
                self.connection,
                "parity-save-replays/corpus/traces/save/replay-002-session-0001.jsonl.zst",
            )
            snapshot = self.root / "final.snapshot"
            snapshot.write_text(logical.replace("/", "__") + "\n")
            imported = DB.import_final_snapshot(self.connection, snapshot)
            DB.set_current_runner(self.connection, "2" * 64)
        self.assertEqual(imported, {"parity-save-replays/corpus": 1})
        report = DB.overview(self.connection)
        self.assertEqual(report["final_set"][0]["registered"], 1)
        self.assertEqual(report["totals"]["expected"], 1)

    def test_overview_prioritizes_eof_over_conversion(self) -> None:
        with self.connection:
            eof_corpus = DB.upsert_corpus(self.connection, "parity-save-replays/eof", 1)
            convert_corpus = DB.upsert_corpus(
                self.connection, "parity-save-replays/convert", 1
            )
            for corpus_id, name in ((eof_corpus, "eof"), (convert_corpus, "convert")):
                corpus_path = self.root / name
                self.connection.execute(
                    "UPDATE corpora SET corpus_status='active',corpus_path=? WHERE corpus_id=?",
                    (str(corpus_path), corpus_id),
                )
                replay_id = DB.upsert_replay(
                    self.connection,
                    f"parity-save-replays/{name}/traces/replay-001.jsonl.zst",
                    None,
                )
                self.connection.execute(
                    "UPDATE replays SET corpus_id=? WHERE replay_id=?",
                    (corpus_id, replay_id),
                )
                self.connection.execute(
                    "INSERT INTO final_corpus_members VALUES(?,?,?)",
                    (corpus_id, replay_id, "test"),
                )
                artifact = corpus_path / "traces/replay-001.jsonl.zst"
                artifact.parent.mkdir(parents=True, exist_ok=True)
                if name == "eof":
                    Path(f"{artifact}.parity.bitcode.zst").write_text("native")
                else:
                    artifact.write_text("source")
        report = DB.overview(self.connection)
        eof_action = next(
            index for index, action in enumerate(report["next_actions"]) if "EOF" in action
        )
        conversion_action = next(
            index
            for index, action in enumerate(report["next_actions"])
            if action.startswith("Convert ")
        )
        self.assertLess(eof_action, conversion_action)


if __name__ == "__main__":
    unittest.main()
