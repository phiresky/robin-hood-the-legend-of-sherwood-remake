#!/usr/bin/env python3
"""Focused tests for the authoritative replay-state ledger."""

from __future__ import annotations

import hashlib
import importlib.util
import io
import os
import sqlite3
import subprocess
import tempfile
import threading
import unittest
from concurrent.futures import ThreadPoolExecutor
from contextlib import redirect_stdout
from datetime import datetime, timezone
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
    def test_structured_eof_binds_identity_and_extent_without_human_marker(self):
        from test_parity_result import result_log
        logical = "parity-save-replays/corpus/traces/save/replay-001-session-0001.jsonl.zst"
        log = result_log(trace_path=str(self.root / logical),
                         native_trace_sha256="3" * 64, executable_sha256="1" * 64)
        evidence = self.evidence("structured", "0", log)
        DB.import_result(self.connection, evidence, self.root / "audit", self.root, "test")
        row = self.connection.execute("SELECT * FROM replay_runs").fetchone()
        self.assertEqual(row["outcome"], "exact_eof")
        self.assertEqual(row["eof_marker_count"], 0)
        self.assertEqual(row["recorded_frames"], 100)
        self.assertEqual(row["terminal_frame"], 140)
        self.assertEqual(row["progress_precision"], "exact")

    def test_structured_result_cannot_borrow_another_runner_or_trace(self):
        from test_parity_result import result_log
        logical = "parity-save-replays/corpus/traces/save/replay-001-session-0001.jsonl.zst"
        for index, changes in enumerate((dict(native_trace_sha256="4" * 64),
                                          dict(executable_sha256="5" * 64),
                                          dict(trace_path="/another-trace"))):
            values = dict(trace_path=str(self.root / logical), native_trace_sha256="3" * 64,
                          executable_sha256="1" * 64)
            values.update(changes)
            evidence = self.evidence(f"wrong-{index}", "0", result_log(**values))
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                DB.import_result(self.connection, evidence, self.root / "audit", self.root, "test")

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

    def evidence(
        self,
        name: str,
        status: str,
        log: str,
        *,
        logical: str | None = None,
        runner_trust: str | None = None,
        runner_raw: str | None = None,
        native_pre: str | None = None,
        native_post: str | None = None,
        finished_utc: str = "2026-08-25T00:00:05Z",
    ) -> Path:
        result = self.root / "audit" / "results" / name
        result.mkdir(parents=True)
        logical = logical or "parity-save-replays/corpus/traces/save/replay-001-session-0001.jsonl.zst"
        runner_trust = runner_trust or "2" * 64
        runner_raw = runner_raw or "1" * 64
        native_pre = native_pre or "3" * 64
        native_post = native_post or native_pre
        (result / "trace.path").write_text(f"{logical}\n")
        (result / "status").write_text(f"{status}\n")
        (result / "log").write_text(log)
        command_status = status if status.isdigit() else "143"
        marker_count = int(log.strip() == DB.EOF_MARKER)
        (result / "attestation.env").write_text(
            "FORMAT=schema16-incremental-eof-v1\n"
            "STARTED_UTC=2026-08-25T00:00:00Z\n"
            f"FINISHED_UTC={finished_utc}\n"
            f"RUNNER_RAW_SHA256={runner_raw}\n"
            f"RUNNER_BUNDLE_TRUST_SHA256={runner_trust}\n"
            f"NATIVE_SHA256_PRE={native_pre}\n"
            f"NATIVE_SHA256_POST={native_post}\n"
            f"RUNNER_COMMAND_STATUS={command_status}\n"
            f"EXACT_EOF_MARKER_COUNT={marker_count}\n"
            f"LOG_SHA256={digest(log.encode())}\n"
        )
        entries = []
        for filename in ("attestation.env", "log", "status", "trace.path"):
            entries.append(f"{DB.sha256_file(result / filename)}  {filename}\n")
        (result / "MANIFEST.sha256").write_text("".join(entries))
        return result

    def rewrite_evidence_logical(self, result: Path, logical: str) -> None:
        (result / "trace.path").write_text(f"{logical}\n")
        entries = []
        for filename in ("attestation.env", "log", "status", "trace.path"):
            entries.append(f"{DB.sha256_file(result / filename)}  {filename}\n")
        (result / "MANIFEST.sha256").write_text("".join(entries))

    def add_replay_work(
        self,
        logical: str,
        *,
        source_sha256: str | None = "3" * 64,
        priority: int = 100,
    ) -> int:
        DB.upsert_replay(self.connection, logical, None)
        return DB.add_work(
            self.connection,
            logical,
            "replay",
            "2" * 64,
            None,
            None,
            source_sha256,
            priority,
        )

    def reblock_audit(
        self,
        name: str,
        logical: str,
        source_sha: str,
        target_bytes: bytes,
    ) -> tuple[Path, str, str]:
        workspace = self.root
        native = Path(f"{workspace / logical}.parity.bitcode.zst")
        native.parent.mkdir(parents=True, exist_ok=True)
        native.write_bytes(target_bytes)
        target_sha = DB.sha256_file(native)

        bundle = workspace / "binaries" / "test-reblock-runner"
        (bundle / "lib").mkdir(parents=True, exist_ok=True)
        (bundle / "original_parity_replay").write_bytes(b"authenticated reblock runner\n")
        (bundle / "original_parity_replay.remote").write_bytes(b"authenticated wrapper\n")
        (bundle / "lib/libtest.so").write_bytes(b"authenticated library\n")
        (bundle / "LIB_SHA256SUMS").write_text(
            f"{DB.sha256_file(bundle / 'lib/libtest.so')}  lib/libtest.so\n"
        )
        (bundle / "SHA256SUMS").write_text(
            f"{DB.sha256_file(bundle / 'original_parity_replay')}  original_parity_replay\n"
            f"{DB.sha256_file(bundle / 'original_parity_replay.remote')}  original_parity_replay.remote\n"
            f"{DB.sha256_file(bundle / 'LIB_SHA256SUMS')}  LIB_SHA256SUMS\n"
        )
        trust = DB.sha256_bytes(
            ("schema16-runner-bundle-v1\n"
             f"SHA256SUMS={DB.sha256_file(bundle / 'SHA256SUMS')}\n"
             f"LIB_SHA256SUMS={DB.sha256_file(bundle / 'LIB_SHA256SUMS')}\n").encode()
        )

        audit = workspace / "audits" / name
        (audit / "logs").mkdir(parents=True)
        (audit / "status").mkdir()
        path_text = str(native.resolve())
        relative_native = native.relative_to(workspace).as_posix()
        key = DB.sha256_bytes(relative_native.encode())
        log_name = f"{key}.attempt-0001.log"
        (audit / "logs" / log_name).write_text(
            f"reblocked {path_text}: 1 frames, 1 MiB -> 1 MiB\n"
        )
        (audit / "status" / f"{key}.status").write_text(f"0\t1\t{log_name}\n")
        paths = path_text.encode() + b"\0"
        before = source_sha.encode() + b"  " + path_text.encode() + b"\0"
        after = target_sha.encode() + b"  " + path_text.encode() + b"\0"
        (audit / "native-paths.nul").write_bytes(paths)
        (audit / "native-before.sha256z").write_bytes(before)
        (audit / "native-after.sha256z").write_bytes(after)
        corpus = workspace / logical.split("/traces/", 1)[0]
        (audit / "provenance.env").write_text(
            "CREATED_UTC=2026-08-30T00:00:00Z\n"
            f"WORKSPACE={workspace.resolve()}\nCORPUS={corpus.resolve()}\n"
            f"BUNDLE={bundle.resolve()}\n"
            f"RUNNER_RAW_SHA256={DB.sha256_file(bundle / 'original_parity_replay')}\n"
            f"RUNNER_TRUST_SHA256={trust}\nEXPECTED_COUNT=1\n"
            f"NATIVE_PATHS_SHA256={DB.sha256_file(audit / 'native-paths.nul')}\n"
            f"NATIVE_BEFORE_MANIFEST_SHA256={DB.sha256_file(audit / 'native-before.sha256z')}\n"
        )
        (audit / "COMPLETE").write_text(
            "COMPLETED_UTC=2026-08-30T00:01:00Z\nCOUNT=1\n"
            f"BEFORE_MANIFEST_SHA256={DB.sha256_file(audit / 'native-before.sha256z')}\n"
            f"AFTER_MANIFEST_SHA256={DB.sha256_file(audit / 'native-after.sha256z')}\n"
        )
        sealed = [
            "provenance.env", "native-paths.nul", "native-before.sha256z",
            "native-after.sha256z", "COMPLETE", f"logs/{log_name}",
            f"status/{key}.status",
        ]
        (audit / "MANIFEST.sha256").write_text("".join(
            f"{DB.sha256_file(audit / relative)}  {relative}\n" for relative in sealed
        ))
        return audit, target_sha, trust

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

    def test_reblock_lineage_preserves_exact_eof_and_scheduler_skip(self) -> None:
        exact = self.evidence("lineage-exact", "0", f"{DB.EOF_MARKER}\n")
        logical = (self.root / "audit/results/lineage-exact/trace.path").read_text().strip()
        source_sha = "3" * 64
        with self.connection:
            DB.import_result(self.connection, exact, self.root / "audit", None, "host-a")
            DB.activate_corpus(
                self.connection, "parity-save-replays/corpus", 1, None, None, None,
                str(self.root / "parity-save-replays/corpus"),
            )
            DB.set_current_runner(self.connection, "2" * 64)
        marker = self.root / "parity-save-replays/corpus/traces/save/replay-001.complete"
        marker.parent.mkdir(parents=True, exist_ok=True)
        marker.write_text("complete\n")
        self.connection.execute(
            "UPDATE replays SET completion_marker=? WHERE logical_path=?",
            ("parity-save-replays/corpus/traces/save/replay-001.complete", logical),
        )
        self.connection.commit()
        target = b"reblocked native\nRHPRTRACEFOOTER!" + bytes(20)
        audit, target_sha, _ = self.reblock_audit(
            "lineage-direct", logical, source_sha, target
        )
        imported = DB.import_reblock_audit(self.connection, audit, self.root)
        self.assertEqual(imported["lineage_inserted"], 1)
        self.assertTrue(DB.has_attested_exact(
            self.connection, logical, "2" * 64, target_sha
        ))
        self.assertIsNotNone(DB.exact_evidence_key(
            self.connection, logical, "2" * 64, target_sha
        ))
        queued = DB.enqueue_corpus_replay_work(
            self.connection, "parity-save-replays/corpus", "2" * 64, 100
        )
        self.assertEqual((queued["skipped_exact"], queued["enqueued"]), (1, 0))
        report = DB.overview(self.connection)
        self.assertEqual(report["totals"]["current_exact"], 1)

    def test_overview_accepts_direct_exact_after_stale_lineage_head(self) -> None:
        old_exact = self.evidence("lineage-old-exact", "0", f"{DB.EOF_MARKER}\n")
        logical = (old_exact / "trace.path").read_text().strip()
        with self.connection:
            DB.import_result(
                self.connection, old_exact, self.root / "audit", None, "host-a"
            )
            DB.activate_corpus(
                self.connection, "parity-save-replays/corpus", 1, None, None, None,
                str(self.root / "parity-save-replays/corpus"),
            )
        lineage_audit, _, _ = self.reblock_audit(
            "lineage-before-later-direct", logical, "3" * 64, b"lineage target"
        )
        DB.import_reblock_audit(self.connection, lineage_audit, self.root)

        later_trust = "4" * 64
        later_native = "5" * 64
        later_exact = self.evidence(
            "lineage-later-direct", "0", f"{DB.EOF_MARKER}\n",
            runner_trust=later_trust, native_pre=later_native,
        )
        with self.connection:
            DB.import_result(
                self.connection, later_exact, self.root / "audit", None, "host-a"
            )
        self.assertTrue(DB.has_attested_exact(
            self.connection, logical, later_trust, later_native
        ))

        report = DB.overview(self.connection)
        self.assertEqual(report["totals"]["current_exact"], 1)
        self.assertEqual(report["final_set"][0]["current_failed"], 0)
        self.assertEqual(
            report["evidence_runners"][0]["bundle_trust_sha256"], later_trust
        )

    def test_reblock_lineage_is_transitive_and_scoped_to_one_replay(self) -> None:
        first = self.evidence("lineage-first", "0", f"{DB.EOF_MARKER}\n")
        logical_one = (first / "trace.path").read_text().strip()
        second = self.evidence("lineage-second", "0", f"{DB.EOF_MARKER}\n")
        logical_two = logical_one.replace("replay-001", "replay-002")
        self.rewrite_evidence_logical(second, logical_two)
        with self.connection:
            DB.import_result(self.connection, first, self.root / "audit", None, "host-a")
            DB.import_result(self.connection, second, self.root / "audit", None, "host-a")
        source_sha = "3" * 64
        audit_one, middle_sha, _ = self.reblock_audit(
            "lineage-hop-one", logical_one, source_sha, b"middle native"
        )
        DB.import_reblock_audit(self.connection, audit_one, self.root)
        audit_two, target_sha, _ = self.reblock_audit(
            "lineage-hop-two", logical_one, middle_sha, b"target native"
        )
        DB.import_reblock_audit(self.connection, audit_two, self.root)
        self.assertTrue(DB.has_attested_exact(
            self.connection, logical_one, "2" * 64, target_sha
        ))
        self.assertFalse(DB.has_attested_exact(
            self.connection, logical_two, "2" * 64, target_sha
        ))

    def test_reblock_work_reconciliation_is_transitive_idempotent_and_exact(self) -> None:
        exact = self.evidence("reblock-work-exact", "0", f"{DB.EOF_MARKER}\n")
        logical = (exact / "trace.path").read_text().strip()
        source_sha = "3" * 64
        with self.connection:
            DB.import_result(self.connection, exact, self.root / "audit", None, "host-a")
            old_work = self.add_replay_work(logical, source_sha256=source_sha, priority=900)
            old = self.connection.execute(
                "SELECT * FROM work_items WHERE work_id=?", (old_work,)
            ).fetchone()
            duplicate_work = self.connection.execute(
                """INSERT INTO work_items(
                     work_key,operation,replay_id,corpus_id,save_group,stripe_key,
                     runner_id,source_sha256,priority)
                   VALUES(?,'replay',?,?,?,?,?,?,?)""",
                (
                    "f" * 64, old["replay_id"], old["corpus_id"], old["save_group"],
                    old["stripe_key"], old["runner_id"], source_sha, 800,
                ),
            ).lastrowid
            completed_old = self.connection.execute(
                """INSERT INTO work_items(
                     work_key,operation,replay_id,corpus_id,save_group,stripe_key,
                     runner_id,source_sha256,priority)
                   VALUES(?,'replay',?,?,?,?,?,?,?)""",
                (
                    "e" * 64, old["replay_id"], old["corpus_id"], old["save_group"],
                    old["stripe_key"], old["runner_id"], source_sha, 700,
                ),
            ).lastrowid
            self.connection.execute(
                """INSERT INTO work_completions(
                     work_id,claim_token,completed_utc,outcome)
                   VALUES(?,'historical','2026-08-29T00:00:00Z','mismatch')""",
                (completed_old,),
            )
        evidence_key = self.connection.execute(
            "SELECT evidence_key FROM replay_runs"
        ).fetchone()[0]

        first_audit, middle_sha, _ = self.reblock_audit(
            "reblock-work-hop-one", logical, source_sha, b"middle work artifact"
        )
        DB.import_reblock_audit(self.connection, first_audit, self.root)
        second_audit, target_sha, _ = self.reblock_audit(
            "reblock-work-hop-two", logical, middle_sha, b"target work artifact"
        )
        DB.import_reblock_audit(self.connection, second_audit, self.root)

        reconciled = DB.reconcile_reblock_work(self.connection, second_audit)
        self.assertEqual(
            reconciled,
            {"lineage_heads": 1, "stale_work": 2, "superseded": 2,
             "target_enqueued": 1, "target_reused": 0},
        )
        supersession = self.connection.execute(
            "SELECT * FROM work_supersessions WHERE work_id=?", (old_work,)
        ).fetchone()
        replacement = self.connection.execute(
            "SELECT * FROM work_items WHERE work_id=?",
            (supersession["replacement_work_id"],),
        ).fetchone()
        self.assertEqual(replacement["source_sha256"], target_sha)
        self.assertEqual(replacement["priority"], 900)
        self.assertEqual(
            self.connection.execute(
                "SELECT replacement_work_id FROM work_supersessions WHERE work_id=?",
                (duplicate_work,),
            ).fetchone()[0],
            replacement["work_id"],
        )
        self.assertEqual(
            DB.reconcile_reblock_work(self.connection, second_audit),
            {"lineage_heads": 1, "stale_work": 0, "superseded": 0,
             "target_enqueued": 0, "target_reused": 0},
        )

        claim = DB.claim_work(
            self.connection, "replay", "host-target", 60,
            "2" * 64, "parity-save-replays/corpus",
        )
        self.assertEqual(claim["work_id"], replacement["work_id"])
        self.assertEqual(claim["exact_evidence_key"], evidence_key)
        DB.complete_work(
            self.connection, claim["claim_token"], "exact_eof", evidence_key
        )
        report = DB.overview(self.connection)["work"][0]
        self.assertEqual((report["completed"], report["superseded"], report["queued"]),
                         (2, 2, 0))
        lineage_id = supersession["lineage_id"]
        with self.assertRaisesRegex(sqlite3.IntegrityError, "invalid work supersession"):
            self.connection.execute(
                """INSERT INTO work_supersessions(
                     work_id,lineage_id,replacement_work_id,reason)
                   VALUES(?,?,?,'must reject completed work')""",
                (completed_old, lineage_id, replacement["work_id"]),
            )
        self.connection.rollback()
        with self.assertRaisesRegex(sqlite3.IntegrityError, "complete superseded"):
            self.connection.execute(
                """INSERT INTO work_completions(
                     work_id,claim_token,completed_utc,outcome)
                   VALUES(?,'late','2026-08-31T00:00:00Z','exact_eof')""",
                (old_work,),
            )
        self.connection.rollback()
        with self.assertRaisesRegex(sqlite3.IntegrityError, "append-only"):
            self.connection.execute(
                "UPDATE work_supersessions SET reason='changed' WHERE work_id=?",
                (old_work,),
            )
        self.connection.rollback()

    def test_reblock_work_reconciliation_rejects_active_but_not_expired_claim(self) -> None:
        run = self.evidence("reblock-work-active", "1", "divergent frames\n")
        logical = (run / "trace.path").read_text().strip()
        source_sha = "3" * 64
        with self.connection:
            DB.import_result(self.connection, run, self.root / "audit", None, "host-a")
            old_work = self.add_replay_work(logical, source_sha256=source_sha)
        claim = DB.claim_work(
            self.connection, "replay", "host-active", 3600,
            "2" * 64, "parity-save-replays/corpus",
        )
        audit, target_sha, _ = self.reblock_audit(
            "reblock-work-active", logical, source_sha, b"active target artifact"
        )
        DB.import_reblock_audit(self.connection, audit, self.root)
        with self.assertRaisesRegex(ValueError, "actively claimed"):
            DB.reconcile_reblock_work(self.connection, audit)
        self.assertEqual(
            self.connection.execute("SELECT count(*) FROM work_supersessions").fetchone()[0],
            0,
        )
        self.assertEqual(
            self.connection.execute(
                "SELECT count(*) FROM work_items WHERE source_sha256=?", (target_sha,)
            ).fetchone()[0],
            0,
        )

        self.connection.execute(
            "UPDATE work_claims SET lease_until_utc='2000-01-01T00:00:00.000Z' "
            "WHERE claim_token=?", (claim["claim_token"],)
        )
        self.connection.commit()
        reconciled = DB.reconcile_reblock_work(self.connection, audit)
        self.assertEqual((reconciled["superseded"], reconciled["target_enqueued"]), (1, 1))
        self.assertIsNotNone(self.connection.execute(
            "SELECT 1 FROM work_supersessions WHERE work_id=?", (old_work,)
        ).fetchone())
        self.connection.execute(
            "UPDATE work_claims SET lease_until_utc='2099-01-01T00:00:00.000Z' "
            "WHERE claim_token=?", (claim["claim_token"],)
        )
        self.connection.commit()
        with self.assertRaisesRegex(ValueError, "superseded"):
            DB.renew_work(self.connection, claim["claim_token"], 60)
        with self.assertRaisesRegex(ValueError, "superseded"):
            DB.complete_work(
                self.connection, claim["claim_token"], "mismatch", None
            )

    def test_reblock_work_reconciliation_stages_each_lineage_hop(self) -> None:
        run = self.evidence("reblock-work-staged", "1", "divergent frames\n")
        logical = (run / "trace.path").read_text().strip()
        source_sha = "3" * 64
        with self.connection:
            DB.import_result(self.connection, run, self.root / "audit", None, "host-a")
            source_work = self.add_replay_work(logical, source_sha256=source_sha)

        first_audit, middle_sha, _ = self.reblock_audit(
            "reblock-work-staged-one", logical, source_sha, b"staged middle artifact"
        )
        DB.import_reblock_audit(self.connection, first_audit, self.root)
        first = DB.reconcile_reblock_work(self.connection, first_audit)
        self.assertEqual((first["superseded"], first["target_enqueued"]), (1, 1))
        middle_work = self.connection.execute(
            "SELECT replacement_work_id FROM work_supersessions WHERE work_id=?",
            (source_work,),
        ).fetchone()[0]
        self.assertEqual(self.connection.execute(
            "SELECT source_sha256 FROM work_items WHERE work_id=?", (middle_work,)
        ).fetchone()[0], middle_sha)

        second_audit, target_sha, _ = self.reblock_audit(
            "reblock-work-staged-two", logical, middle_sha, b"staged target artifact"
        )
        DB.import_reblock_audit(self.connection, second_audit, self.root)
        second = DB.reconcile_reblock_work(self.connection, second_audit)
        self.assertEqual((second["superseded"], second["target_enqueued"]), (1, 1))
        target_work = self.connection.execute(
            "SELECT replacement_work_id FROM work_supersessions WHERE work_id=?",
            (middle_work,),
        ).fetchone()[0]
        self.assertEqual(self.connection.execute(
            "SELECT source_sha256 FROM work_items WHERE work_id=?", (target_work,)
        ).fetchone()[0], target_sha)
        claim = DB.claim_work(
            self.connection, "replay", "host-staged", 60,
            "2" * 64, "parity-save-replays/corpus",
        )
        self.assertEqual(claim["work_id"], target_work)

    def test_reblock_work_reuses_target_and_rejects_nonexact_ancestor_evidence(self) -> None:
        mismatch = self.evidence(
            "reblock-work-mismatch", "1", "first parity divergence after frame 4\n"
        )
        logical = (mismatch / "trace.path").read_text().strip()
        source_sha = "3" * 64
        with self.connection:
            DB.import_result(
                self.connection, mismatch, self.root / "audit", None, "host-a"
            )
            self.add_replay_work(logical, source_sha256=source_sha, priority=100)
        evidence_key = self.connection.execute(
            "SELECT evidence_key FROM replay_runs"
        ).fetchone()[0]
        audit, target_sha, _ = self.reblock_audit(
            "reblock-work-reuse", logical, source_sha, b"reused target artifact"
        )
        DB.import_reblock_audit(self.connection, audit, self.root)
        with self.connection:
            target_work = DB.add_work(
                self.connection, logical, "replay", "2" * 64,
                None, None, target_sha, 50,
            )
        reconciled = DB.reconcile_reblock_work(self.connection, audit)
        self.assertEqual((reconciled["target_enqueued"], reconciled["target_reused"]), (0, 1))
        claim = DB.claim_work(
            self.connection, "replay", "host-reuse", 60,
            "2" * 64, "parity-save-replays/corpus",
        )
        self.assertEqual(claim["work_id"], target_work)
        with self.assertRaisesRegex(ValueError, "native digest"):
            DB.complete_work(
                self.connection, claim["claim_token"], "mismatch", evidence_key
            )

    def test_reblock_import_rejects_incomplete_and_tampered_audits_atomically(self) -> None:
        exact = self.evidence("lineage-reject", "0", f"{DB.EOF_MARKER}\n")
        logical = (exact / "trace.path").read_text().strip()
        with self.connection:
            DB.import_result(self.connection, exact, self.root / "audit", None, "host-a")
        incomplete, _, _ = self.reblock_audit(
            "lineage-incomplete", logical, "3" * 64, b"incomplete target"
        )
        (incomplete / "MANIFEST.sha256").unlink()
        with self.assertRaisesRegex(ValueError, "MANIFEST"):
            DB.import_reblock_audit(self.connection, incomplete, self.root)
        self.assertEqual(
            self.connection.execute("SELECT count(*) FROM native_reblock_audits").fetchone()[0],
            0,
        )

        tampered, _, _ = self.reblock_audit(
            "lineage-tampered", logical, "3" * 64, b"tampered target"
        )
        status = next((tampered / "status").iterdir())
        status.write_text(status.read_text().replace("0\t", "1\t"))
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            DB.import_reblock_audit(self.connection, tampered, self.root)
        self.assertEqual(
            self.connection.execute("SELECT count(*) FROM native_artifact_lineage").fetchone()[0],
            0,
        )

    def test_reblock_lineage_tables_are_append_only(self) -> None:
        exact = self.evidence("lineage-immutable", "0", f"{DB.EOF_MARKER}\n")
        logical = (exact / "trace.path").read_text().strip()
        with self.connection:
            DB.import_result(self.connection, exact, self.root / "audit", None, "host-a")
        audit, _, _ = self.reblock_audit(
            "lineage-immutable", logical, "3" * 64, b"immutable target"
        )
        DB.import_reblock_audit(self.connection, audit, self.root)
        with self.assertRaisesRegex(sqlite3.IntegrityError, "append-only"):
            self.connection.execute(
                "UPDATE native_artifact_lineage SET source_sha256=?", ("4" * 64,)
            )
        self.connection.rollback()
        with self.assertRaisesRegex(sqlite3.IntegrityError, "append-only"):
            self.connection.execute("DELETE FROM native_reblock_audits")
        self.connection.rollback()

    def test_reblock_snapshot_driver_seals_and_reuses_complete_audit(self) -> None:
        workspace = self.root
        corpus = workspace / "corpus"
        native = corpus / "traces/replay.parity.bitcode.zst"
        native.parent.mkdir(parents=True)
        native.write_bytes(b"source trace")
        bundle = workspace / "bundle"
        (bundle / "lib").mkdir(parents=True)
        runner = bundle / "original_parity_replay"
        wrapper = bundle / "original_parity_replay.remote"
        runner.write_text("#!/bin/sh\nexit 0\n")
        wrapper.write_text(
            "#!/bin/sh\n"
            "printf reblocked >>\"$2\"\n"
            "printf 'reblocked %s: test fixture\\n' \"$2\" >&2\n"
        )
        runner.chmod(0o755)
        wrapper.chmod(0o755)
        (bundle / "lib/library").write_bytes(b"library")
        (bundle / "LIB_SHA256SUMS").write_text(
            f"{DB.sha256_file(bundle / 'lib/library')}  lib/library\n"
        )
        (bundle / "SHA256SUMS").write_text(
            f"{DB.sha256_file(runner)}  original_parity_replay\n"
            f"{DB.sha256_file(wrapper)}  original_parity_replay.remote\n"
            f"{DB.sha256_file(bundle / 'LIB_SHA256SUMS')}  LIB_SHA256SUMS\n"
        )
        trust = DB.sha256_bytes(
            ("schema16-runner-bundle-v1\n"
             f"SHA256SUMS={DB.sha256_file(bundle / 'SHA256SUMS')}\n"
             f"LIB_SHA256SUMS={DB.sha256_file(bundle / 'LIB_SHA256SUMS')}\n").encode()
        )
        audit = workspace / "audits/reblock"
        environment = os.environ | {
            "NATIVE_REBLOCK_JOBS": "1",
            "NATIVE_REBLOCK_TIMEOUT_SECONDS": "3600",
            "NATIVE_REBLOCK_OUTER_LOCK": str(workspace / "runner.lock"),
        }
        command = [
            str(ROOT / "scripts/run_native_reblock_snapshot.sh"), str(workspace),
            str(corpus), str(bundle), trust, str(audit), "1",
        ]
        subprocess.run(command, check=True, env=environment, capture_output=True, text=True)
        seal_sha = DB.sha256_file(audit / "MANIFEST.sha256")
        subprocess.run(
            ["sha256sum", "--strict", "-c", "MANIFEST.sha256"], cwd=audit,
            check=True, capture_output=True, text=True,
        )
        subprocess.run(command, check=True, env=environment, capture_output=True, text=True)
        self.assertEqual(DB.sha256_file(audit / "MANIFEST.sha256"), seal_sha)

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

        with self.connection:
            DB.add_work(
                self.connection, logical, "replay", "2" * 64,
                None, None, "3" * 64, 100,
            )
        claim = DB.claim_work(
            self.connection, "replay", "host-a", 60,
            "2" * 64, "parity-save-replays/corpus",
        )
        self.assertEqual(claim["exact_evidence_key"], evidence_key)

        with self.connection:
            DB.add_work(
                self.connection, logical, "replay", "2" * 64,
                None, None, "4" * 64, 100,
            )
        wrong_native = DB.claim_work(
            self.connection, "replay", "host-b", 60,
            "2" * 64, "parity-save-replays/corpus", save_group_cap=2,
        )
        self.assertIsNone(wrong_native["exact_evidence_key"])

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

    def test_work_lease_time_is_sampled_after_write_lock(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            self.add_replay_work(
                "parity-save-replays/corpus/traces/save/"
                "replay-002-session-0001.jsonl.zst"
            )

        began = False
        original_now = DB.utc_now

        def trace(statement: str) -> None:
            nonlocal began
            if statement == "BEGIN IMMEDIATE":
                began = True

        def guarded_now() -> datetime:
            self.assertTrue(began, "lease clock was sampled before acquiring write lock")
            return datetime(2030, 1, 1, tzinfo=timezone.utc)

        self.connection.set_trace_callback(trace)
        DB.utc_now = guarded_now
        try:
            claim = DB.claim_work(self.connection, "replay", "host-a", 60)
            self.assertIsNotNone(claim)
            began = False
            DB.renew_work(self.connection, claim["claim_token"], 60)
            began = False
            DB.complete_work(
                self.connection, claim["claim_token"], "exact_eof", None
            )
        finally:
            DB.utc_now = original_now
            self.connection.set_trace_callback(None)

    def test_concurrent_claims_obey_save_group_cap(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            for save in ("save-a", "save-b"):
                for replay in (1, 2):
                    self.add_replay_work(
                        f"parity-save-replays/corpus/traces/{save}/"
                        f"replay-{replay:03}-session-0001.jsonl.zst"
                    )

        barrier = threading.Barrier(4)

        def concurrent_claim(index: int) -> dict[str, object] | None:
            connection = DB.connect(self.database)
            try:
                barrier.wait()
                return DB.claim_work(
                    connection, "replay", f"host-{index}", 3600,
                    "2" * 64, "parity-save-replays/corpus",
                )
            finally:
                connection.close()

        with ThreadPoolExecutor(max_workers=4) as pool:
            claims = list(pool.map(concurrent_claim, range(4)))
        claimed = [claim for claim in claims if claim is not None]
        self.assertEqual(len(claimed), 2)
        self.assertEqual(
            len({DB.replay_save_group(str(claim["logical_path"])) for claim in claimed}),
            2,
        )

    def test_claims_are_striped_across_save_groups(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            for save in ("save-a", "save-b"):
                for replay in (1, 2):
                    self.add_replay_work(
                        f"parity-save-replays/corpus/traces/{save}/"
                        f"replay-{replay:03}-session-0001.jsonl.zst"
                    )

        paths = []
        for index in range(4):
            claim = DB.claim_work(
                self.connection, "replay", f"host-{index}", 3600,
                "2" * 64, "parity-save-replays/corpus", save_group_cap=2,
            )
            self.assertIsNotNone(claim)
            paths.append(str(claim["logical_path"]).split("/traces/", 1)[1])
        self.assertEqual(
            paths,
            [
                "save-a/replay-001-session-0001.jsonl.zst",
                "save-b/replay-001-session-0001.jsonl.zst",
                "save-a/replay-002-session-0001.jsonl.zst",
                "save-b/replay-002-session-0001.jsonl.zst",
            ],
        )

    def test_priority_precedes_active_group_count(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        high_first = "parity-save-replays/corpus/traces/save-a/replay-001.jsonl.zst"
        high_second = "parity-save-replays/corpus/traces/save-a/replay-002.jsonl.zst"
        low = "parity-save-replays/corpus/traces/save-b/replay-001.jsonl.zst"
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            self.add_replay_work(high_first, priority=100)
            self.add_replay_work(high_second, priority=100)
            self.add_replay_work(low, priority=99)
        first = DB.claim_work(
            self.connection, "replay", "host-a", 3600,
            "2" * 64, "parity-save-replays/corpus", save_group_cap=2,
        )
        second = DB.claim_work(
            self.connection, "replay", "host-b", 3600,
            "2" * 64, "parity-save-replays/corpus", save_group_cap=2,
        )
        self.assertEqual(first["logical_path"], high_first)
        self.assertEqual(second["logical_path"], high_second)

    def test_unfiltered_runner_claim_uses_each_runners_group_count(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        occupied = "parity-save-replays/corpus/traces/save-a/replay-001.jsonl.zst"
        high = "parity-save-replays/corpus/traces/save-a/replay-002.jsonl.zst"
        low = "parity-save-replays/corpus/traces/save-b/replay-001.jsonl.zst"
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            DB.upsert_runner(self.connection, {
                "RUNNER_BUNDLE_TRUST_SHA256": "4" * 64,
                "RUNNER_RAW_SHA256": "5" * 64,
            })
            self.add_replay_work(occupied, priority=100)
            self.add_replay_work(high, priority=100)
            DB.upsert_replay(self.connection, low, None)
            DB.add_work(
                self.connection, low, "replay", "4" * 64,
                None, None, "6" * 64, 99,
            )
        first = DB.claim_work(self.connection, "replay", "host-a", 3600,
                              save_group_cap=2)
        second = DB.claim_work(self.connection, "replay", "host-b", 3600,
                               save_group_cap=2)
        self.assertEqual(first["logical_path"], occupied)
        self.assertEqual(second["logical_path"], high)

    def test_v6_migration_backfills_schedule_and_preserves_append_only(self) -> None:
        old_database = self.root / "version-6.sqlite3"
        schema = DB.SCHEMA.replace(
            "    corpus_id INTEGER NOT NULL REFERENCES corpora(corpus_id),\n"
            "    save_group TEXT NOT NULL,\n"
            "    stripe_key TEXT NOT NULL,\n",
            "",
        )
        trigger_start = schema.index(
            "CREATE TRIGGER IF NOT EXISTS work_items_no_update"
        )
        trigger_end = schema.index(
            "CREATE TRIGGER IF NOT EXISTS work_items_no_delete", trigger_start
        )
        schema = (
            schema[:trigger_start]
            + "CREATE TRIGGER IF NOT EXISTS work_items_no_update "
              "BEFORE UPDATE ON work_items BEGIN "
              "SELECT RAISE(ABORT, 'work_items is append-only'); END;\n"
            + schema[trigger_end:]
        )
        old = sqlite3.connect(old_database)
        old.executescript(schema)
        old.execute("INSERT INTO schema_meta(key,value) VALUES('schema_version','6')")
        old.execute("INSERT INTO corpora(corpus_id,logical_root) VALUES(1,?)",
                    ("parity-save-replays/corpus",))
        logical = "parity-save-replays/corpus/traces/save/replay-001.jsonl.zst"
        old.execute(
            "INSERT INTO replays(replay_id,corpus_id,replay_key,logical_path) VALUES(1,1,?,?)",
            ("1" * 64, logical),
        )
        old.execute(
            "INSERT INTO runners(runner_id,identity_key,identity_kind,bundle_trust_sha256,raw_sha256) "
            "VALUES(1,?,'authenticated',?,?)",
            ("2" * 64, "3" * 64, "4" * 64),
        )
        old.execute(
            "INSERT INTO work_items(work_key,operation,replay_id,runner_id,source_sha256) "
            "VALUES(?,'replay',1,1,?)",
            ("5" * 64, "6" * 64),
        )
        old.commit()
        old.close()

        migrated = DB.connect(old_database)
        try:
            row = migrated.execute(
                "SELECT corpus_id,save_group,stripe_key FROM work_items"
            ).fetchone()
            self.assertEqual(
                tuple(row),
                (1, "parity-save-replays/corpus/traces/save", "replay-001.jsonl.zst"),
            )
            self.assertIn(
                "work_items_replay_schedule",
                {index[1] for index in migrated.execute("PRAGMA index_list(work_items)")},
            )
            with self.assertRaisesRegex(sqlite3.IntegrityError, "append-only"):
                migrated.execute("UPDATE work_items SET priority=1")
            with self.assertRaisesRegex(sqlite3.IntegrityError, "scheduling keys"):
                migrated.execute(
                    """INSERT INTO work_items(
                         work_key,operation,replay_id,runner_id,source_sha256,
                         corpus_id,save_group,stripe_key)
                       VALUES(?,'replay',1,1,?,NULL,NULL,NULL)""",
                    ("7" * 64, "8" * 64),
                )
        finally:
            migrated.close()

    def test_v7_migration_installs_append_only_work_supersessions(self) -> None:
        database = self.root / "version-7.sqlite3"
        old = DB.connect(database)
        for trigger in (
            "work_completions_not_superseded", "work_supersessions_valid",
            "work_supersessions_no_update", "work_supersessions_no_delete",
        ):
            old.execute(f"DROP TRIGGER {trigger}")
        old.execute("DROP TABLE work_supersessions")
        old.execute(
            "UPDATE schema_meta SET value='7' WHERE key='schema_version'"
        )
        old.commit()
        old.close()
        migrated = DB.connect(database)
        try:
            self.assertEqual(
                migrated.execute(
                    "SELECT value FROM schema_meta WHERE key='schema_version'"
                ).fetchone()[0],
                str(DB.SCHEMA_VERSION),
            )
            self.assertIsNotNone(migrated.execute(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='work_supersessions'"
            ).fetchone())
        finally:
            migrated.close()

    def test_active_duplicate_retry_identity_is_not_claimed(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        logical = "parity-save-replays/corpus/traces/save/replay-002-session-0001.jsonl.zst"
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            work_id = self.add_replay_work(logical, source_sha256=None)
            row = self.connection.execute(
                "SELECT * FROM work_items WHERE work_id=?", (work_id,)
            ).fetchone()
            self.connection.execute(
                """INSERT INTO work_items(
                     work_key,operation,replay_id,corpus_id,save_group,stripe_key,
                     runner_id,conversion_protocol,target_encoding,source_sha256,priority)
                   VALUES(?,?,?,?,?,?,?,?,?,?,?)""",
                (
                    "f" * 64, row["operation"], row["replay_id"], row["corpus_id"],
                    row["save_group"], row["stripe_key"], row["runner_id"],
                    row["conversion_protocol"], row["target_encoding"],
                    row["source_sha256"], row["priority"],
                ),
            )
        first = DB.claim_work(
            self.connection, "replay", "host-a", 3600,
            "2" * 64, "parity-save-replays/corpus", save_group_cap=2,
        )
        self.assertIsNotNone(first)
        self.assertIsNone(DB.claim_work(
            self.connection, "replay", "host-b", 3600,
            "2" * 64, "parity-save-replays/corpus", save_group_cap=2,
        ))

    def test_different_source_digests_remain_independently_claimable(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        logical = "parity-save-replays/corpus/traces/save/replay-002-session-0001.jsonl.zst"
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            self.add_replay_work(logical, source_sha256="4" * 64)
            self.add_replay_work(logical, source_sha256="5" * 64)
        first = DB.claim_work(
            self.connection, "replay", "host-a", 3600,
            "2" * 64, "parity-save-replays/corpus", save_group_cap=2,
        )
        second = DB.claim_work(
            self.connection, "replay", "host-b", 3600,
            "2" * 64, "parity-save-replays/corpus", save_group_cap=2,
        )
        self.assertIsNotNone(first)
        self.assertIsNotNone(second)
        self.assertNotEqual(first["source_sha256"], second["source_sha256"])

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
        with self.assertRaisesRegex(ValueError, "expired"):
            DB.renew_work(self.connection, first["claim_token"], 60)
        with self.assertRaisesRegex(ValueError, "expired"):
            DB.complete_work(
                self.connection, first["claim_token"], "exact_eof", None
            )
        second = DB.claim_work(self.connection, "convert", "host-b:1", 3600)
        self.assertEqual(first["work_id"], second["work_id"])
        self.assertNotEqual(first["claim_token"], second["claim_token"])

    def test_expired_replay_claim_does_not_consume_save_group_cap(self) -> None:
        result = self.evidence("exact", "0", f"{DB.EOF_MARKER}\n")
        first_logical = (
            "parity-save-replays/corpus/traces/save/replay-001-session-0001.jsonl.zst"
        )
        second_logical = (
            "parity-save-replays/corpus/traces/save/replay-002-session-0001.jsonl.zst"
        )
        with self.connection:
            DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            first_work = self.add_replay_work(first_logical)
            self.add_replay_work(second_logical)
        first = DB.claim_work(
            self.connection, "replay", "host-a:1", 3600,
            "2" * 64, "parity-save-replays/corpus", save_group_cap=1,
        )
        self.connection.execute(
            "UPDATE work_claims SET lease_until_utc='2000-01-01T00:00:00.000Z'"
        )
        self.connection.commit()
        second = DB.claim_work(
            self.connection, "replay", "host-a:2", 3600,
            "2" * 64, "parity-save-replays/corpus", save_group_cap=1,
        )
        self.assertEqual(first["work_id"], first_work)
        self.assertEqual(second["work_id"], first_work)

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
        with self.assertRaisesRegex(ValueError, "unknown"):
            DB.renew_work(self.connection, first["claim_token"], 60)
        with self.assertRaisesRegex(ValueError, "unknown"):
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

    def test_merged_markerless_corpus_enqueues_from_workspace_and_claims_by_membership(
        self,
    ) -> None:
        source_root = "parity-save-replays/source-corpus"
        target_root = "parity-save-replays/final-corpus"
        logical = f"{source_root}/traces/save/replay-001-session-0001.jsonl.zst"
        native = Path(f"{self.root / logical}.parity.bitcode.zst")
        native.parent.mkdir(parents=True)
        native.write_bytes(b"native\nRHPRTRACEFOOTER!" + bytes(20))
        native_sha = DB.sha256_file(native)
        invalid_logical = f"{source_root}/traces/save/replay-002-session-0001.jsonl.zst"
        invalid_native = Path(f"{self.root / invalid_logical}.parity.bitcode.zst")
        invalid_native.write_bytes(b"native without terminal footer")
        unrelated = "parity-save-replays/unrelated/traces/save/replay-001-session-0001.jsonl.zst"
        with self.connection:
            source_id = DB.upsert_corpus(self.connection, source_root, expected=2)
            self.connection.execute(
                "UPDATE corpora SET corpus_status='active' WHERE corpus_id=?",
                (source_id,),
            )
            replay_id = DB.upsert_replay(self.connection, logical, None)
            self.connection.execute(
                "UPDATE replays SET corpus_id=? WHERE replay_id=?", (source_id, replay_id)
            )
            self.connection.execute(
                "INSERT INTO final_corpus_members(corpus_id,replay_id,source_ledger) "
                "VALUES(?,?,?)",
                (source_id, replay_id, "test-ledger"),
            )
            invalid_id = DB.upsert_replay(self.connection, invalid_logical, None)
            self.connection.execute(
                "UPDATE replays SET corpus_id=? WHERE replay_id=?",
                (source_id, invalid_id),
            )
            self.connection.execute(
                "INSERT INTO final_corpus_members(corpus_id,replay_id,source_ledger) "
                "VALUES(?,?,?)",
                (source_id, invalid_id, "test-ledger"),
            )
            DB.upsert_runner(
                self.connection,
                {
                    "RUNNER_BUNDLE_TRUST_SHA256": "2" * 64,
                    "RUNNER_RAW_SHA256": "1" * 64,
                },
            )
            DB.merge_corpora(
                self.connection, target_root, [source_root], 2, None, 16
            )
            DB.upsert_replay(self.connection, unrelated, None)
            DB.add_work(
                self.connection, unrelated, "replay", "2" * 64,
                None, None, "4" * 64, 1000,
            )

        queued = DB.enqueue_corpus_replay_work(
            self.connection, target_root, "2" * 64, 100, self.root
        )
        self.assertEqual(
            queued,
            {"members": 2, "native_ready": 1, "enqueued": 1,
             "skipped_exact": 0, "missing_native": 0,
             "missing_marker": 0, "invalid_footer": 1},
        )
        claim = DB.claim_work(
            self.connection, "replay", "host-a:merged", 60,
            "2" * 64, target_root,
        )
        self.assertIsNotNone(claim)
        self.assertEqual(claim["logical_path"], logical)
        self.assertIsNone(claim["completion_marker"])
        self.assertEqual(claim["source_sha256"], native_sha)

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

        # Completed campaigns retain their append-only scheduling history, but
        # the operator-facing overview must not present stale queue rows as
        # current work.
        report["work"] = [{
            "operation": "replay", "total": 2, "queued": 1,
            "claimed": 0, "completed": 1, "superseded": 0,
        }]
        output = io.StringIO()
        with redirect_stdout(output):
            DB.print_overview(report)
        self.assertIn("No actionable work", output.getvalue())
        self.assertNotIn("replay: total=2", output.getvalue())

    def test_overview_folds_targeted_authenticated_runner_chain(self) -> None:
        root = "parity-save-replays/corpus"
        paths = [
            f"{root}/traces/save/replay-{index:03}-session-0001.jsonl.zst"
            for index in range(1, 5)
        ]
        old_trust, targeted_trust, repaired_trust, aborted_trust = (
            character * 64 for character in "2456"
        )
        old_runs = [
            self.evidence(
                "old-exact-1", "0", f"{DB.EOF_MARKER}\n",
                logical=paths[0], runner_trust=old_trust,
            ),
            self.evidence(
                "old-failed-2", "1",
                "first parity divergence after frame 12 (1 difference):\n",
                logical=paths[1], runner_trust=old_trust,
            ),
            self.evidence(
                "old-exact-3", "0", f"{DB.EOF_MARKER}\n",
                logical=paths[2], runner_trust=old_trust,
            ),
        ]
        targeted_runs = [
            self.evidence(
                "targeted-exact-2", "0", f"{DB.EOF_MARKER}\n",
                logical=paths[1], runner_trust=targeted_trust,
            ),
            self.evidence(
                "targeted-failed-3", "1",
                "first parity divergence after frame 34 (1 difference):\n",
                logical=paths[2], runner_trust=targeted_trust,
            ),
        ]
        with self.connection:
            for result in old_runs + targeted_runs:
                DB.import_result(self.connection, result, self.root / "audit", None, "host-a")
            corpus_id = self.connection.execute(
                "SELECT corpus_id FROM corpora WHERE logical_root=?", (root,)
            ).fetchone()[0]
            self.connection.execute(
                """UPDATE corpora
                   SET corpus_status='active',expected_replays=3,corpus_path=?
                   WHERE corpus_id=?""",
                (str(self.root / "corpus"), corpus_id),
            )
            for logical in paths[:3]:
                replay_id = self.connection.execute(
                    "SELECT replay_id FROM replays WHERE logical_path=?", (logical,)
                ).fetchone()[0]
                self.connection.execute(
                    "INSERT INTO final_corpus_members VALUES(?,?,?)",
                    (corpus_id, replay_id, "test-final-set"),
                )
            # This registered placeholder is deliberately not in the final
            # snapshot and must not inflate either totals or untested work.
            placeholder = DB.upsert_replay(self.connection, paths[3], None)
            self.connection.execute(
                "UPDATE replays SET corpus_id=? WHERE replay_id=?",
                (corpus_id, placeholder),
            )
            DB.set_current_runner(self.connection, old_trust)
        for logical in paths[:3]:
            relative = logical.removeprefix(root + "/")
            native = Path(f"{self.root / 'corpus' / relative}.parity.bitcode.zst")
            native.parent.mkdir(parents=True, exist_ok=True)
            native.write_text("native")

        report = DB.overview(self.connection)
        self.assertEqual(report["totals"], {
            "expected": 3, "current_exact": 2, "remaining": 1,
        })
        self.assertEqual(report["final_set"][0]["current_failed"], 1)
        self.assertEqual(
            [(row["bundle_trust_sha256"], row["outcomes"])
             for row in report["evidence_runners"]],
            [
                (targeted_trust, {"exact_eof": 1, "mismatch": 1}),
                (old_trust, {"exact_eof": 1}),
            ],
        )
        self.assertEqual(report["current_runner"]["bundle_trust_sha256"], old_trust)

        repaired = self.evidence(
            "repaired-exact-3", "0", f"{DB.EOF_MARKER}\n",
            logical=paths[2], runner_trust=repaired_trust,
        )
        aborted = self.evidence(
            "newer-aborted-1", "aborted-controller-signal", "",
            logical=paths[0], runner_trust=aborted_trust,
        )
        with self.connection:
            DB.import_result(self.connection, repaired, self.root / "audit", None, "host-a")
            DB.import_result(self.connection, aborted, self.root / "audit", None, "host-a")

        report = DB.overview(self.connection)
        self.assertEqual(report["totals"], {
            "expected": 3, "current_exact": 3, "remaining": 0,
        })
        self.assertEqual(report["final_set"][0]["current_failed"], 0)
        self.assertEqual(report["final_set"][0]["current_aborted"], 1)
        self.assertEqual(
            [row["bundle_trust_sha256"] for row in report["evidence_runners"]],
            [repaired_trust, targeted_trust, old_trust],
        )
        self.assertEqual(
            report["next_actions"],
            ["Final set is exact at EOF on authoritative latest attested evidence."],
        )

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
