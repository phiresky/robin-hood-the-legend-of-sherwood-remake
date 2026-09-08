#!/usr/bin/env python3
"""Hermetic checks for the mandatory authentic-process release gate."""

from __future__ import annotations

import hashlib
import importlib.util
import inspect
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest
from unittest import mock


HARNESS = Path(__file__).with_name("real-runtime-fence-e2e.py")
SPEC = importlib.util.spec_from_file_location("real_runtime_fence_e2e", HARNESS)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class HarnessSelfTest(unittest.TestCase):
    def candidate_fixture(self, root: Path) -> tuple[str, str]:
        commit = "a" * 40
        manifest = {
            "database_schema_version": 14,
            "deployment": {
                "current_link": str(MODULE.INSTALL / "current"),
                "home": "/home/robinhood",
                "install_root": str(MODULE.INSTALL),
                "persistent_state_root": str(MODULE.STATE),
                "user": "robinhood",
            },
            "files": [],
            "publication_lock_sha256": "b" * 64,
            "publication_manifest_sha256": "c" * 64,
            "schema_version": 2,
            "source_commit": commit,
            "verifier_sha256": "d" * 64,
        }
        payload = json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()
        (root / "vps-release-manifest-v2.json").write_bytes(payload)
        (root / "vps-release-manifest-v2.json").chmod(0o440)
        digest = hashlib.sha256(payload).hexdigest()
        sums = f"{digest}  vps-release-manifest-v2.json\n".encode()
        (root / "SHA256SUMS").write_bytes(sums)
        (root / "SHA256SUMS").chmod(0o440)
        root.chmod(0o550)
        return commit, hashlib.sha256(sums).hexdigest()

    def test_oob_inventory_rejects_candidate_byte_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary)
            _, sums_sha = self.candidate_fixture(candidate)
            manifest = MODULE.candidate_inventory(candidate, sums_sha)
            self.assertEqual(manifest["schema_version"], 2)
            candidate.chmod(0o700)
            (candidate / "vps-release-manifest-v2.json").chmod(0o600)
            (candidate / "vps-release-manifest-v2.json").write_bytes(b"{}")
            (candidate / "vps-release-manifest-v2.json").chmod(0o440)
            candidate.chmod(0o550)
            with self.assertRaisesRegex(RuntimeError, "differs from authenticated"):
                MODULE.candidate_inventory(candidate, sums_sha)

    def test_supervisor_is_one_private_exact_authority_namespace(self) -> None:
        secrets = {
            name: index + 20 for index, (name, _) in enumerate(MODULE.ORDINARY_SECRETS)
        }
        command = MODULE.supervisor_command(
            10,
            11,
            12,
            13,
            secrets,
            "a" * 40,
            "b" * 64,
            "c" * 64,
        )
        self.assertEqual(command[0], "/usr/bin/bwrap")
        self.assertEqual(command.count("--unshare-net"), 1)
        self.assertEqual(command.count("--unshare-pid"), 1)
        self.assertEqual(command.count("--ro-bind-data"), 5)
        self.assertEqual(command.count("--ro-bind-fd"), 3)
        self.assertIn(str(MODULE.STATE), command)
        self.assertIn(str(MODULE.INSTALL / "releases" / ("a" * 40)), command)
        self.assertNotIn("--share-net", command)
        self.assertNotIn("target/debug", " ".join(command))

    def test_bwrap_ro_bind_data_is_in_memory_private_and_read_only(self) -> None:
        read_fd, write_fd = os.pipe()
        try:
            os.write(write_fd, b"x" * 32)
            os.close(write_fd)
            write_fd = -1
            subprocess.run(
                [
                    "/usr/bin/bwrap",
                    "--die-with-parent",
                    "--ro-bind", "/", "/",
                    "--tmpfs", "/mnt",
                    "--perms", "0400",
                    "--ro-bind-data", str(read_fd), "/mnt/secret",
                    "--unshare-net",
                    "--unshare-pid",
                    "--proc", "/proc",
                    "--",
                    "/bin/sh", "-c",
                    "test \"$(stat -c %a /mnt/secret)\" = 400 && "
                    "test \"$(wc -c </mnt/secret)\" = 32 && "
                    "! printf x >>/mnt/secret",
                ],
                check=True,
                pass_fds=(read_fd,),
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        finally:
            os.close(read_fd)
            if write_fd >= 0:
                os.close(write_fd)

    def test_bwrap_ro_bind_fd_pins_directory_inode_read_only(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            authority = Path(temporary) / "authority"
            authority.mkdir()
            (authority / "proof").write_bytes(b"pinned")
            descriptor = MODULE.open_directory_authority(authority)
            try:
                subprocess.run(
                    [
                        "/usr/bin/bwrap",
                        "--die-with-parent",
                        "--ro-bind", "/", "/",
                        "--tmpfs", "/mnt",
                        "--dir", "/mnt/authority",
                        "--ro-bind-fd", str(descriptor), "/mnt/authority",
                        "--unshare-pid",
                        "--proc", "/proc",
                        "--",
                        "/bin/sh", "-c",
                        "test \"$(cat /mnt/authority/proof)\" = pinned && "
                        "! printf x >>/mnt/authority/proof",
                    ],
                    check=True,
                    pass_fds=(descriptor,),
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
            finally:
                os.close(descriptor)

    def test_anonymous_secret_is_materialized_as_one_private_tmpfs_link(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "anonymous"
            destination = root / "durable"
            source.write_bytes(b"s" * 32)
            source.chmod(0o400)
            MODULE.copy_in_memory_secret_authority(source, destination, 32)
            metadata = destination.stat(follow_symlinks=False)
            self.assertEqual(metadata.st_mode & 0o777, 0o400)
            self.assertEqual(metadata.st_nlink, 1)
            self.assertEqual(metadata.st_size, 32)
            self.assertEqual(destination.read_bytes(), b"s" * 32)

    def test_inner_mode_refuses_the_host_state_mount(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "disposable tmpfs"):
            MODULE.assert_disposable_mount_namespace(Path("/nonexistent-release"))

    def test_every_exclusive_ttl_poll_revalidates_both_locks_and_competitor(self) -> None:
        backup = mock.Mock()
        competitor = mock.Mock()
        backup.process.poll.return_value = None
        competitor.process.poll.return_value = None
        with (
            mock.patch.object(MODULE, "group_holds_write", return_value=True),
            mock.patch.object(MODULE, "group_waits_shared", return_value=True),
            mock.patch.object(MODULE, "group_holds_shared", return_value=False),
        ):
            MODULE.assert_exclusive_drain_retained(backup, competitor, 1, 2)

        for label, write, pending, crossed, backup_code, competitor_code in (
            ("exclusive runtime fence", False, True, False, None, None),
            ("stopped waiting", True, False, False, None, None),
            ("crossed", True, True, True, None, None),
            ("exited", True, True, False, 1, None),
            ("stopped waiting", True, True, False, None, 0),
        ):
            backup.process.poll.return_value = backup_code
            competitor.process.poll.return_value = competitor_code
            with (
                mock.patch.object(MODULE, "group_holds_write", return_value=write),
                mock.patch.object(MODULE, "group_waits_shared", return_value=pending),
                mock.patch.object(MODULE, "group_holds_shared", return_value=crossed),
                self.assertRaisesRegex(RuntimeError, label),
            ):
                MODULE.assert_exclusive_drain_retained(backup, competitor, 1, 2)

        execute_source = inspect.getsource(MODULE.execute_inner)
        self.assertEqual(execute_source.count("assert_exclusive_drain_retained("), 3)

    def test_stopped_capture_requires_sqlite_writer_slot_to_be_free(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary)
            fence = state / "runtime-fence"
            fence.mkdir()
            (fence / "db-admission.lock").touch()
            (fence / "db-quiescence.lock").touch()
            database_root = state / "database"
            database_root.mkdir()
            database = database_root / "highscores.sqlite3"
            writer = sqlite3.connect(database, isolation_level=None)
            try:
                writer.execute("PRAGMA journal_mode = WAL")
                writer.execute(
                    "CREATE TABLE maintenance_write_leases ("
                    "token TEXT NOT NULL, writer_class TEXT NOT NULL, "
                    "expires_at_ms INTEGER NOT NULL)"
                )
                writer.execute(
                    "INSERT INTO maintenance_write_leases VALUES (?, ?, ?)",
                    ("committed-worker", "worker", 2**62),
                )
                writer.execute("BEGIN IMMEDIATE")
                with mock.patch.object(MODULE, "STATE", state):
                    self.assertEqual(
                        MODULE.active_worker_lease(),
                        ("committed-worker", 2**62),
                    )
                with (
                    mock.patch.object(MODULE, "STATE", state),
                    self.assertRaisesRegex(sqlite3.OperationalError, "locked"),
                ):
                    MODULE.database_row_after_immediate_rollback_fenced("SELECT 1")
                writer.execute("ROLLBACK")
                with mock.patch.object(MODULE, "STATE", state):
                    self.assertEqual(
                        MODULE.database_row_after_immediate_rollback_fenced("SELECT 1"),
                        (1,),
                    )
            finally:
                if writer.in_transaction:
                    writer.rollback()
                writer.close()

        execute_source = inspect.getsource(MODULE.execute_inner)
        capture = execute_source.index("capture_stopped_worker_lease(")
        reap = execute_source.index("worker.kill_stopped_transactionally()", capture)
        backup = execute_source.index('backup = Service(', reap)
        self.assertLess(capture, reap)
        self.assertLess(reap, backup)
        self.assertIn(
            "database_row_after_immediate_rollback_fenced(",
            execute_source[reap:backup],
        )
        self.assertEqual(execute_source.count("worker.kill_stopped_transactionally()"), 1)
        self.assertEqual(
            execute_source.count("capture_stopped_shared_without_sqlite_writer("),
            1,
        )
        self.assertEqual(execute_source.count("capture_stopped_shared("), 2)

    def test_metadata_fingerprints_cover_children_and_raw_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stable = root / "stable"
            stable.mkdir()
            (stable / "child").write_bytes(b"one")
            (stable / "child").chmod(0o640)
            first_stable = MODULE.metadata_tree(stable)
            self.assertEqual(len(first_stable), 2)
            (stable / "child").write_bytes(b"two")
            (stable / "child").chmod(0o600)
            self.assertNotEqual(MODULE.metadata_tree(stable), first_stable)

            raw = root / "raw"
            raw.mkdir()
            leaf = raw / "leaf"
            leaf.write_bytes(b"raw-one")
            leaf.chmod(0o440)
            raw.chmod(0o550)
            first_raw = MODULE.immutable_raw_metadata_tree(raw)
            first_raw_digest = first_raw[-1][1]
            leaf.chmod(0o600)
            leaf.write_bytes(b"raw-two")
            leaf.chmod(0o440)
            second_raw = MODULE.immutable_raw_metadata_tree(raw)
            self.assertNotEqual(second_raw, first_raw)
            self.assertNotEqual(second_raw[-1][1], first_raw_digest)
            raw.chmod(0o700)

    def test_candidate_authentication_stays_on_the_pinned_directory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            candidate = root / "candidate"
            candidate.mkdir()
            _, expected_sums = self.candidate_fixture(candidate)
            descriptor = MODULE.open_directory_authority(candidate)
            try:
                displaced = root / "displaced"
                candidate.rename(displaced)
                candidate.mkdir()
                self.candidate_fixture(candidate)
                pinned = MODULE.descriptor_path(descriptor)
                manifest = MODULE.candidate_inventory(
                    pinned,
                    expected_sums,
                    pinned_root=True,
                )
                self.assertEqual(manifest["source_commit"], "a" * 40)
                self.assertFalse(MODULE.descriptor_matches_path(descriptor, candidate))
            finally:
                os.close(descriptor)

    def test_release_workflow_must_inherit_the_exact_candidate_descriptor(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary)
            descriptor = MODULE.open_directory_authority(candidate)
            try:
                os.environ[MODULE.PINNED_CANDIDATE_FD_ENV] = str(descriptor)
                duplicate = MODULE.inherit_pinned_candidate_authority(candidate)
                try:
                    self.assertTrue(MODULE.descriptor_matches_path(duplicate, candidate))
                    self.assertNotIn(MODULE.PINNED_CANDIDATE_FD_ENV, os.environ)
                finally:
                    os.close(duplicate)
                with self.assertRaisesRegex(RuntimeError, "omitted"):
                    MODULE.inherit_pinned_candidate_authority(candidate)
            finally:
                os.environ.pop(MODULE.PINNED_CANDIDATE_FD_ENV, None)
                os.close(descriptor)

    def test_optional_first_deploy_runtime_fence_is_explicitly_absent(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            missing = Path(temporary) / "runtime-fence"
            self.assertEqual(MODULE.optional_metadata_tree(missing), ("absent", str(missing)))
            missing.mkdir()
            self.assertEqual(MODULE.optional_metadata_tree(missing)[0], "present")

    def test_harness_source_uses_only_candidate_runtime_binaries(self) -> None:
        source = HARNESS.read_text(encoding="utf-8")
        self.assertNotIn("target/debug", source)
        self.assertNotIn("patch_assignment", source)
        self.assertNotIn('"initialize-backup-authority-key"', source)
        self.assertIn("initialize-backup-authority-key-v2", source)
        self.assertIn("complete-backup-authority-key-v2", source)
        self.assertIn("initialize-vps-runtime-fence-v1", source)
        self.assertIn("bin/robin-highscores-server", source)
        self.assertIn("bin/robin-highscores-worker", source)
        self.assertIn("bin/robin-highscores-admin", source)
        self.assertIn("verify-live-database-schema-v2", source)
        self.assertIn("verify-transaction-backup", source)


if __name__ == "__main__":
    unittest.main()
