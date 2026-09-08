#!/usr/bin/python3
"""Exhaustive tests for deploy-release.sh's durable authority journal graph.

The production shell functions are extracted byte-for-byte and executed in
fresh private directories.  This keeps the state-machine test independent of
the much larger deployment fixture while still testing the actual shell code.
"""

from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


HERE = Path(__file__).resolve().parent
SOURCE = Path(os.environ.get("ROBIN_TX_SOURCE_DIR", HERE.parent)).resolve()
DEPLOY = SOURCE / "deploy-release.sh"

TARGET = "2" * 40
OTHER_TARGET = "3" * 40
VPS = "e" * 64
OTHER_VPS = "f" * 64
SOURCE_RECEIPT = "a" * 64
TARGET_RECEIPT = "b" * 64
SUBSTITUTED_RECEIPT = "c" * 64
SOURCE_STATUS = "d" * 64
TARGET_STATUS = "9" * 64
SUBSTITUTED_STATUS = "8" * 64
UPGRADE_SOURCE = "1" * 40
OTHER_SOURCE = "4" * 40


@dataclass(frozen=True)
class AuthorityState:
    source: str
    runtime: str
    source_receipt: str
    schema: str
    target_receipt: str
    backup_status_before: str
    phase: str

    def arguments(self) -> str:
        return " ".join(
            (
                self.source,
                self.runtime,
                self.source_receipt,
                self.schema,
                self.target_receipt,
                self.backup_status_before,
                self.phase,
            )
        )


CLEAN = (
    AuthorityState(
        "none", "unobserved", "none", "none", "none", "none", "prepared"
    ),
    AuthorityState(
        "none", "initializing", "none", "none", "none", "none", "prepared"
    ),
    AuthorityState(
        "none", "present", "none", "none", "none", "none", "runtime_ready"
    ),
    # A clean first deployment deliberately has no source-backup phase.
    AuthorityState(
        "none", "present", "none", "verified", "none", "none", "schema_verified"
    ),
    AuthorityState(
        "none",
        "present",
        "none",
        "verified",
        "none",
        "absent",
        "target_backup_started",
    ),
    AuthorityState(
        "none",
        "present",
        "none",
        "verified",
        TARGET_RECEIPT,
        "absent",
        "target_verified",
    ),
)

UPGRADE = (
    AuthorityState(
        UPGRADE_SOURCE, "unobserved", "none", "none", "none", "none", "prepared"
    ),
    AuthorityState(
        UPGRADE_SOURCE, "initializing", "none", "none", "none", "none", "prepared"
    ),
    AuthorityState(
        UPGRADE_SOURCE,
        "present",
        "none",
        "none",
        "none",
        "none",
        "runtime_ready",
    ),
    AuthorityState(
        UPGRADE_SOURCE,
        "present",
        "none",
        "none",
        "none",
        SOURCE_STATUS,
        "source_backup_started",
    ),
    AuthorityState(
        UPGRADE_SOURCE,
        "present",
        SOURCE_RECEIPT,
        "none",
        "none",
        SOURCE_STATUS,
        "source_verified",
    ),
    AuthorityState(
        UPGRADE_SOURCE,
        "present",
        SOURCE_RECEIPT,
        "verified",
        "none",
        SOURCE_STATUS,
        "schema_verified",
    ),
    AuthorityState(
        UPGRADE_SOURCE,
        "present",
        SOURCE_RECEIPT,
        "verified",
        "none",
        TARGET_STATUS,
        "target_backup_started",
    ),
    AuthorityState(
        UPGRADE_SOURCE,
        "present",
        SOURCE_RECEIPT,
        "verified",
        TARGET_RECEIPT,
        TARGET_STATUS,
        "target_verified",
    ),
)

FUNCTION_NAMES = (
    "valid_digest",
    "authority_journal_expected_text",
    "validate_authority_journal_path",
    "validate_authority_journal",
    "authority_tuple_stage",
    "authority_transition_allowed",
    "reconcile_authority_journal_temporary",
    "publish_authority_journal",
)


def extract_shell_function(source: str, name: str) -> str:
    marker = f"{name}() {{"
    start = source.find(marker)
    if start < 0:
        raise RuntimeError(f"deploy script lacks required function {name}")
    depth = 0
    for offset in range(start, len(source)):
        character = source[offset]
        if character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                return source[start : offset + 1]
    raise RuntimeError(f"unterminated shell function {name}")


def journal_text(
    state: AuthorityState, *, target: str = TARGET, vps: str = VPS
) -> str:
    return "\n".join(
        (
            "schema=robin-highscores-deploy-authority-v1",
            "operation=deploy",
            f"source_commit={state.source}",
            f"target_commit={target}",
            f"target_vps_release_manifest_sha256={vps}",
            f"runtime_authority_state={state.runtime}",
            f"source_backup_receipt_sha256={state.source_receipt}",
            f"live_schema_version={state.schema}",
            f"target_backup_receipt_sha256={state.target_receipt}",
            f"backup_status_before_sha256={state.backup_status_before}",
            f"phase={state.phase}",
            "",
        )
    )


class AuthorityJournalHarness:
    def __init__(self, functions: str) -> None:
        self._temporary = tempfile.TemporaryDirectory(
            prefix="robin-deploy-authority-journal-"
        )
        self.root = Path(self._temporary.name)
        self.journal = self.root / f".deploy-authority-{TARGET}"
        self.temporary = self.root / f".deploy-authority-{TARGET}.new"
        self._preamble = f"""
set -eu
opt_root=$1
authority_journal=$opt_root/.deploy-authority-{TARGET}
authority_journal_temporary=$opt_root/.deploy-authority-{TARGET}.new
expected_commit={TARGET}
expected_vps_manifest_sha256={VPS}
authority_journal_owned=0
authority_journal_temporary_owned=0
{functions}
"""

    def close(self) -> None:
        self._temporary.cleanup()

    def run(
        self,
        body: str,
        *,
        kill_before_rename: bool = False,
        target: str = TARGET,
        vps: str = VPS,
    ) -> subprocess.CompletedProcess[str]:
        injection = ""
        if kill_before_rename:
            injection = "mv() { kill -KILL $$; }\n"
        script = (
            self._preamble
            + f"expected_commit={target}\n"
            + f"expected_vps_manifest_sha256={vps}\n"
            + injection
            + body
            + "\n"
        )
        return subprocess.run(
            ["/bin/sh", "-c", script, "authority-journal-test", str(self.root)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            timeout=10,
        )

    def install(self, state: AuthorityState) -> None:
        self.journal.write_text(journal_text(state))
        self.journal.chmod(0o400)

    def assert_exact(self, testcase: unittest.TestCase, state: AuthorityState) -> None:
        testcase.assertEqual(self.journal.read_text(), journal_text(state))
        testcase.assertEqual(self.journal.stat().st_mode & 0o777, 0o400)
        testcase.assertFalse(self.temporary.exists())


class DeployAuthorityJournalContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        source = DEPLOY.read_text()
        cls.functions = "\n\n".join(
            extract_shell_function(source, name) for name in FUNCTION_NAMES
        )

    def harness(self) -> AuthorityJournalHarness:
        harness = AuthorityJournalHarness(self.functions)
        self.addCleanup(harness.close)
        return harness

    def assert_success(self, result: subprocess.CompletedProcess[str]) -> None:
        self.assertEqual(
            result.returncode,
            0,
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}",
        )

    def assert_failure(self, result: subprocess.CompletedProcess[str]) -> None:
        self.assertNotEqual(
            result.returncode,
            0,
            f"unexpected success; stdout:\n{result.stdout}\nstderr:\n{result.stderr}",
        )

    def test_every_clean_first_and_upgrade_edge_is_exact_and_idempotent(self) -> None:
        for lane_name, lane in (("clean", CLEAN), ("upgrade", UPGRADE)):
            with self.subTest(lane=lane_name):
                harness = self.harness()
                for index, state in enumerate(lane):
                    with self.subTest(lane=lane_name, edge=index):
                        result = harness.run(
                            f"publish_authority_journal {state.arguments()}"
                        )
                        self.assert_success(result)
                        harness.assert_exact(self, state)
                        # Re-publishing byte-identical evidence is the only legal
                        # same-phase transition.
                        result = harness.run(
                            f"publish_authority_journal {state.arguments()}"
                        )
                        self.assert_success(result)
                        harness.assert_exact(self, state)

    def test_all_skips_regressions_and_cross_lane_edges_are_rejected(self) -> None:
        for lane_name, lane in (("clean", CLEAN), ("upgrade", UPGRADE)):
            for old_index, old in enumerate(lane):
                for new_index, new in enumerate(lane):
                    if new_index in (old_index, old_index + 1):
                        continue
                    with self.subTest(
                        lane=lane_name, old=old_index, proposed=new_index
                    ):
                        harness = self.harness()
                        harness.install(old)
                        result = harness.run(
                            f"publish_authority_journal {new.arguments()}"
                        )
                        self.assert_failure(result)
                        self.assertEqual(harness.journal.read_text(), journal_text(old))

        for old, proposed in ((CLEAN[2], UPGRADE[3]), (UPGRADE[2], CLEAN[3])):
            with self.subTest(cross_lane=(old.phase, proposed.phase)):
                harness = self.harness()
                harness.install(old)
                result = harness.run(
                    f"publish_authority_journal {proposed.arguments()}"
                )
                self.assert_failure(result)
                self.assertEqual(harness.journal.read_text(), journal_text(old))

    def test_same_phase_substituted_authorities_fail_closed(self) -> None:
        substitutions = (
            (
                "source",
                CLEAN[0],
                AuthorityState(
                    OTHER_SOURCE,
                    "unobserved",
                    "none",
                    "none",
                    "none",
                    "none",
                    "prepared",
                ),
                TARGET,
                VPS,
            ),
            (
                "source-receipt-and-publication-binding",
                UPGRADE[4],
                AuthorityState(
                    UPGRADE_SOURCE,
                    "present",
                    SUBSTITUTED_RECEIPT,
                    "none",
                    "none",
                    SOURCE_STATUS,
                    "source_verified",
                ),
                TARGET,
                VPS,
            ),
            (
                "target-receipt-and-publication-binding",
                UPGRADE[7],
                AuthorityState(
                    UPGRADE_SOURCE,
                    "present",
                    SOURCE_RECEIPT,
                    "verified",
                    SUBSTITUTED_RECEIPT,
                    TARGET_STATUS,
                    "target_verified",
                ),
                TARGET,
                VPS,
            ),
            (
                "source-backup-status-boundary",
                UPGRADE[4],
                AuthorityState(
                    UPGRADE_SOURCE,
                    "present",
                    SOURCE_RECEIPT,
                    "none",
                    "none",
                    SUBSTITUTED_STATUS,
                    "source_verified",
                ),
                TARGET,
                VPS,
            ),
            (
                "target-backup-status-boundary",
                UPGRADE[7],
                AuthorityState(
                    UPGRADE_SOURCE,
                    "present",
                    SOURCE_RECEIPT,
                    "verified",
                    TARGET_RECEIPT,
                    SUBSTITUTED_STATUS,
                    "target_verified",
                ),
                TARGET,
                VPS,
            ),
            ("target", UPGRADE[5], UPGRADE[5], OTHER_TARGET, VPS),
            ("vps", UPGRADE[5], UPGRADE[5], TARGET, OTHER_VPS),
        )
        for label, old, proposed, target, vps in substitutions:
            with self.subTest(authority=label):
                harness = self.harness()
                harness.install(old)
                result = harness.run(
                    f"publish_authority_journal {proposed.arguments()}",
                    target=target,
                    vps=vps,
                )
                self.assert_failure(result)
                self.assertEqual(harness.journal.read_text(), journal_text(old))

        for label, replacement in (
            ("schema", "live_schema_version=none"),
            ("backup-status", "backup_status_before_sha256=none"),
            ("target", f"target_commit={OTHER_TARGET}"),
            ("vps", f"target_vps_release_manifest_sha256={OTHER_VPS}"),
        ):
            with self.subTest(corrupt_durable=label):
                harness = self.harness()
                text = journal_text(UPGRADE[5])
                prefix = replacement.split("=", 1)[0] + "="
                text = "\n".join(
                    replacement if line.startswith(prefix) else line
                    for line in text.splitlines()
                ) + "\n"
                harness.journal.write_text(text)
                harness.journal.chmod(0o400)
                self.assert_failure(harness.run("validate_authority_journal"))

    def test_backup_status_boundary_is_single_invariant_and_replaced(self) -> None:
        illegal_edges = (
            # The status observed before a backup is invariant until that
            # backup's receipt is verified.
            (
                UPGRADE[3],
                AuthorityState(
                    UPGRADE_SOURCE,
                    "present",
                    SOURCE_RECEIPT,
                    "none",
                    "none",
                    SUBSTITUTED_STATUS,
                    "source_verified",
                ),
            ),
            # The target boundary must replace, rather than retain, the source
            # boundary after schema verification.
            (
                UPGRADE[5],
                AuthorityState(
                    UPGRADE_SOURCE,
                    "present",
                    SOURCE_RECEIPT,
                    "verified",
                    "none",
                    SOURCE_STATUS,
                    "target_backup_started",
                ),
            ),
            (
                UPGRADE[6],
                AuthorityState(
                    UPGRADE_SOURCE,
                    "present",
                    SOURCE_RECEIPT,
                    "verified",
                    TARGET_RECEIPT,
                    SUBSTITUTED_STATUS,
                    "target_verified",
                ),
            ),
        )
        for predecessor, proposed in illegal_edges:
            with self.subTest(
                old=predecessor.phase,
                proposed=proposed.phase,
                status=proposed.backup_status_before,
            ):
                harness = self.harness()
                harness.install(predecessor)
                result = harness.run(
                    f"publish_authority_journal {proposed.arguments()}"
                )
                self.assert_failure(result)
                self.assertEqual(
                    harness.journal.read_text(), journal_text(predecessor)
                )

    def test_sigkill_temporary_is_adopted_at_every_legal_edge(self) -> None:
        for lane_name, lane in (("clean", CLEAN), ("upgrade", UPGRADE)):
            for index, successor in enumerate(lane):
                with self.subTest(lane=lane_name, edge=index):
                    harness = self.harness()
                    predecessor = lane[index - 1] if index else None
                    if predecessor is not None:
                        harness.install(predecessor)
                    result = harness.run(
                        f"publish_authority_journal {successor.arguments()}",
                        kill_before_rename=True,
                    )
                    self.assertLess(result.returncode, 0, result.stderr)
                    self.assertTrue(harness.temporary.is_file())
                    self.assertEqual(
                        harness.temporary.read_text(), journal_text(successor)
                    )
                    self.assertEqual(harness.temporary.stat().st_mode & 0o777, 0o400)
                    if predecessor is None:
                        self.assertFalse(harness.journal.exists())
                    else:
                        self.assertEqual(
                            harness.journal.read_text(), journal_text(predecessor)
                        )

                    self.assert_success(
                        harness.run("reconcile_authority_journal_temporary")
                    )
                    harness.assert_exact(self, successor)

    def test_backup_receipt_hash_transitively_binds_projected_publication(self) -> None:
        source = DEPLOY.read_text()
        self.assertIn(
            'receipt_publication_sha=$(release_publication_lock_sha256', source
        )
        self.assertIn(
            '"$receipt_source" "$receipt_vps_sha" "$receipt_publication_sha"',
            source,
        )
        for receipt_name in ("source", "target"):
            with self.subTest(receipt=receipt_name):
                assignment = f"{receipt_name}_receipt_sha=$receipt_sha"
                assignment_at = source.find(assignment)
                self.assertGreaterEqual(
                    assignment_at,
                    0,
                    f"{receipt_name} typed receipt digest is not retained",
                )
                publication_at = source.find(
                    f'"${receipt_name}_receipt_sha"', assignment_at
                )
                self.assertGreater(
                    publication_at,
                    assignment_at,
                    f"{receipt_name} typed receipt digest is not journal-bound",
                )

    def test_main_flow_durably_enters_each_backup_started_phase(self) -> None:
        source = DEPLOY.read_text()
        main_at = source.find("reconcile_authority_journal_temporary ||")
        self.assertGreaterEqual(main_at, 0)
        main = " ".join(source[main_at:].replace("\\\n", " ").split())
        backup_side_effect = "systemctl --user start robin-highscores-backup.service"
        for boundary in ("source", "target"):
            with self.subTest(boundary=boundary):
                capture = f"{boundary}_status_before=$(backup_status_sha256)"
                phase = f'"${boundary}_status_before" {boundary}_backup_started'
                capture_at = main.find(capture)
                phase_at = main.find(phase, capture_at)
                side_effect_at = main.find(backup_side_effect, phase_at)
                self.assertGreaterEqual(
                    capture_at,
                    0,
                    f"{boundary} backup does not capture its exact prior status",
                )
                self.assertGreater(
                    phase_at,
                    capture_at,
                    f"{boundary} backup boundary is not durably journaled",
                )
                self.assertGreater(
                    side_effect_at,
                    phase_at,
                    f"{boundary} backup begins before its durable boundary",
                )


if __name__ == "__main__":
    unittest.main(verbosity=2)
