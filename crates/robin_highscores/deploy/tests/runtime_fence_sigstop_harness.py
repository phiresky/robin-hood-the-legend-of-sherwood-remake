#!/usr/bin/python3
"""Kernel-backed runtime-fence recovery test in an isolated mount namespace.

This is deliberately separate from the transaction command doubles.  API and
worker state in this test is represented by live processes holding real Linux
``flock(2)`` locks.  The complete scenario runs under bubblewrap with a private
state root and no network access or host deployment paths.
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
from pathlib import Path
import secrets
import signal
import sqlite3
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest


HERE = Path(__file__).resolve().parent
SELF = HERE / "runtime_fence_sigstop_harness.py"
SANDBOX_SELF = Path("/run/robin-runtime-fence-harness.py")
SANDBOX_ROOT = Path("/home/robinhood/.local/share/robin-highscores")
FENCE = SANDBOX_ROOT / "runtime-fence"
ADMISSION = FENCE / "db-admission.lock"
QUIESCENCE = FENCE / "db-quiescence.lock"
EVENTS = SANDBOX_ROOT / "events.jsonl"
GATE_DATABASE = SANDBOX_ROOT / "gate.sqlite3"
GATE_TTL_SECONDS = 0.48
HEARTBEAT_SECONDS = 0.06
POLL_SECONDS = 0.01
PROCESS_TIMEOUT_SECONDS = 5.0
SOURCE_COMMIT = "4" * 40
VPS_RELEASE_MANIFEST_SHA256 = "5" * 64
PUBLICATION_LOCK_SHA256 = "6" * 64


def append_event(name: str, **fields: object) -> None:
    payload = {
        "event": name,
        "monotonic_ns": time.monotonic_ns(),
        "pid": os.getpid(),
        **fields,
    }
    descriptor = os.open(EVENTS, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        os.write(
            descriptor,
            (json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n").encode(),
        )
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def touch_durable(path: Path, payload: str) -> None:
    temporary = path.with_name(f".{path.name}.new-{os.getpid()}")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        os.write(descriptor, payload.encode())
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    os.replace(temporary, path)
    parent = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(parent)
    finally:
        os.close(parent)


def exact_fence_leaf(path: Path) -> int:
    descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
    metadata = os.fstat(descriptor)
    if not (
        stat.S_ISREG(metadata.st_mode)
        and stat.S_IMODE(metadata.st_mode) == 0o400
        and metadata.st_uid == os.geteuid()
        and metadata.st_nlink == 1
        and metadata.st_size == 0
    ):
        os.close(descriptor)
        raise RuntimeError(f"inexact runtime-fence leaf: {path}")
    return descriptor


def service_actor(role: str, generation: str) -> int:
    admission = exact_fence_leaf(ADMISSION)
    quiescence = exact_fence_leaf(QUIESCENCE)
    try:
        # The shared admission lock is a turnstile only.  The process-wide
        # shared quiescence lock is retained until orderly shutdown or death.
        fcntl.flock(admission, fcntl.LOCK_SH)
        fcntl.flock(quiescence, fcntl.LOCK_SH)
        fcntl.flock(admission, fcntl.LOCK_UN)
        append_event("service.shared-acquired", role=role, generation=generation)
        touch_durable(
            SANDBOX_ROOT / f"{generation}-{role}.ready",
            f"{os.getpid()}\n",
        )

        terminating = False

        def request_stop(_number: int, _frame: object) -> None:
            nonlocal terminating
            terminating = True

        signal.signal(signal.SIGTERM, request_stop)
        signal.signal(signal.SIGINT, request_stop)
        while not terminating:
            signal.pause()
        append_event("service.orderly-stop", role=role, generation=generation)
        return 0
    finally:
        os.close(quiescence)
        os.close(admission)


def connect_gate() -> sqlite3.Connection:
    connection = sqlite3.connect(GATE_DATABASE, timeout=0.1, isolation_level=None)
    connection.execute("PRAGMA journal_mode=WAL")
    connection.execute("PRAGMA synchronous=FULL")
    connection.execute(
        "CREATE TABLE IF NOT EXISTS backup_gate ("
        " singleton INTEGER PRIMARY KEY CHECK(singleton = 1),"
        " token TEXT NOT NULL UNIQUE,"
        " expires_unix_ns INTEGER NOT NULL,"
        " heartbeat_count INTEGER NOT NULL"
        ") STRICT"
    )
    return connection


def admit_gate(token: str) -> tuple[sqlite3.Connection, bool]:
    connection = connect_gate()
    observed_stale = False
    deadline = time.monotonic() + PROCESS_TIMEOUT_SECONDS
    while True:
        now = time.time_ns()
        expires = now + int(GATE_TTL_SECONDS * 1_000_000_000)
        connection.execute("BEGIN IMMEDIATE")
        row = connection.execute(
            "SELECT token, expires_unix_ns FROM backup_gate WHERE singleton = 1"
        ).fetchone()
        if row is None or int(row[1]) <= now:
            connection.execute(
                "INSERT INTO backup_gate(singleton, token, expires_unix_ns, heartbeat_count) "
                "VALUES(1, ?, ?, 0) "
                "ON CONFLICT(singleton) DO UPDATE SET "
                "token = excluded.token, expires_unix_ns = excluded.expires_unix_ns, "
                "heartbeat_count = 0",
                (token, expires),
            )
            connection.execute("COMMIT")
            append_event("backup.gate-recovered" if observed_stale else "backup.gate-admitted")
            return connection, observed_stale
        connection.execute("ROLLBACK")
        if not observed_stale:
            observed_stale = True
            append_event("backup.gate-waiting-for-ttl", incumbent=str(row[0]))
        if time.monotonic() >= deadline:
            raise TimeoutError("stale backup gate did not expire within the bounded wait")
        time.sleep(POLL_SECONDS)


def gate_heartbeat(
    token: str, stop: threading.Event, failure: list[BaseException]
) -> None:
    try:
        connection = connect_gate()
        try:
            while not stop.wait(HEARTBEAT_SECONDS):
                expires = time.time_ns() + int(GATE_TTL_SECONDS * 1_000_000_000)
                cursor = connection.execute(
                    "UPDATE backup_gate SET expires_unix_ns = ?, "
                    "heartbeat_count = heartbeat_count + 1 "
                    "WHERE singleton = 1 AND token = ?",
                    (expires, token),
                )
                if cursor.rowcount != 1:
                    raise RuntimeError("backup gate authority was lost during heartbeat")
                append_event("backup.gate-heartbeat")
        finally:
            connection.close()
    except BaseException as error:  # surfaced synchronously by the owner
        failure.append(error)
        stop.set()


def backup_actor() -> int:
    token = secrets.token_hex(16)
    gate, recovered = admit_gate(token)
    if not recovered:
        raise RuntimeError("the recovery scenario did not encounter the stale gate")
    stop = threading.Event()
    heartbeat_failure: list[BaseException] = []
    heartbeat = threading.Thread(
        target=gate_heartbeat,
        args=(token, stop, heartbeat_failure),
        name="backup-gate-heartbeat",
        daemon=True,
    )
    heartbeat.start()

    admission = exact_fence_leaf(ADMISSION)
    quiescence = exact_fence_leaf(QUIESCENCE)
    try:
        deadline = time.monotonic() + PROCESS_TIMEOUT_SECONDS
        while True:
            try:
                fcntl.flock(admission, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic() >= deadline:
                    raise TimeoutError("exclusive admission did not become available")
                time.sleep(POLL_SECONDS)
        append_event("backup.admission-exclusive")
        touch_durable(SANDBOX_ROOT / "backup.admission", f"{os.getpid()}\n")

        while True:
            try:
                fcntl.flock(quiescence, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if heartbeat_failure:
                    raise heartbeat_failure[0]
                if time.monotonic() >= deadline:
                    raise TimeoutError("shared runtime holders did not drain")
                time.sleep(POLL_SECONDS)
        append_event("backup.quiescence-exclusive")

        row = gate.execute(
            "SELECT token, expires_unix_ns, heartbeat_count "
            "FROM backup_gate WHERE singleton = 1"
        ).fetchone()
        if (
            row is None
            or row[0] != token
            or int(row[1]) <= time.time_ns()
            or int(row[2]) < 2
        ):
            raise RuntimeError("backup gate was not durably retained by heartbeat")

        # The payload stands for the snapshot boundary: it is published only
        # while both exact exclusive fence locks and the fresh durable gate are
        # held, then fsynced before either authority is released.
        touch_durable(
            SANDBOX_ROOT / "backup.complete",
            json.dumps(
                {
                    "heartbeat_count": int(row[2]),
                    "publication_lock_sha256": PUBLICATION_LOCK_SHA256,
                    "schema_version": 2,
                    "source_commit": SOURCE_COMMIT,
                    "token": token,
                    "vps_release_manifest_sha256": VPS_RELEASE_MANIFEST_SHA256,
                },
                sort_keys=True,
                separators=(",", ":"),
            )
            + "\n",
        )
        append_event(
            "backup.completed",
            heartbeat_count=int(row[2]),
            publication_lock_sha256=PUBLICATION_LOCK_SHA256,
            source_commit=SOURCE_COMMIT,
            vps_release_manifest_sha256=VPS_RELEASE_MANIFEST_SHA256,
        )
        deleted = gate.execute(
            "DELETE FROM backup_gate WHERE singleton = 1 AND token = ?", (token,)
        ).rowcount
        if deleted != 1:
            raise RuntimeError("backup could not release its exact durable gate")
        append_event("backup.gate-released")
        return 0
    finally:
        stop.set()
        heartbeat.join(timeout=1.0)
        if heartbeat.is_alive():
            raise RuntimeError("backup gate heartbeat did not terminate")
        os.close(quiescence)
        os.close(admission)
        gate.close()


def wait_path(path: Path, timeout: float = PROCESS_TIMEOUT_SECONDS) -> None:
    deadline = time.monotonic() + timeout
    while not path.exists():
        if time.monotonic() >= deadline:
            raise TimeoutError(f"timed out waiting for {path}")
        time.sleep(POLL_SECONDS)


def wait_actor_path(
    path: Path, process: subprocess.Popen[bytes], label: str
) -> None:
    deadline = time.monotonic() + PROCESS_TIMEOUT_SECONDS
    while not path.exists():
        return_code = process.poll()
        if return_code is not None:
            stdout = (process.stdout.read() if process.stdout else b"").decode(errors="replace")
            stderr = (process.stderr.read() if process.stderr else b"").decode(errors="replace")
            raise RuntimeError(
                f"{label} exited before publishing {path.name}: exit={return_code}"
                f"\nstdout:\n{stdout}\nstderr:\n{stderr}"
            )
        if time.monotonic() >= deadline:
            raise TimeoutError(f"timed out waiting for {path}")
        time.sleep(POLL_SECONDS)


def spawn_actor(*arguments: str) -> subprocess.Popen[bytes]:
    return subprocess.Popen(
        [sys.executable, str(SANDBOX_SELF), *arguments],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        close_fds=True,
    )


def terminate_bounded(process: subprocess.Popen[bytes], role: str) -> None:
    started = time.monotonic()
    os.kill(process.pid, signal.SIGTERM)
    append_event("supervisor.term-sent", role=role)
    try:
        process.wait(timeout=0.12)
    except subprocess.TimeoutExpired:
        append_event(
            "supervisor.kill-after-timeout",
            role=role,
            elapsed_ms=int((time.monotonic() - started) * 1000),
        )
        os.kill(process.pid, signal.SIGKILL)
        process.wait(timeout=1.0)
        append_event("supervisor.stopped-holder-reaped", role=role)
    else:
        raise RuntimeError(f"SIGSTOPed {role} unexpectedly handled SIGTERM")


def stop_orderly(process: subprocess.Popen[bytes], role: str) -> None:
    process.send_signal(signal.SIGTERM)
    try:
        process.wait(timeout=1.0)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=1.0)
        raise RuntimeError(f"restarted {role} did not stop within its bound")
    if process.returncode != 0:
        stderr = (process.stderr.read() if process.stderr else b"").decode()
        raise RuntimeError(f"restarted {role} failed: {stderr}")


def assert_exclusive_quiescence_blocked() -> None:
    descriptor = exact_fence_leaf(QUIESCENCE)
    try:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return
        fcntl.flock(descriptor, fcntl.LOCK_UN)
        raise RuntimeError("runtime was reported active without a shared quiescence holder")
    finally:
        os.close(descriptor)


def provision_sandbox() -> None:
    SANDBOX_ROOT.mkdir(mode=0o700, exist_ok=True)
    if any(SANDBOX_ROOT.iterdir()):
        raise RuntimeError("private runtime-fence harness root is not empty")
    SANDBOX_ROOT.chmod(0o700)
    FENCE.mkdir(mode=0o700)
    for leaf in (ADMISSION, QUIESCENCE):
        descriptor = os.open(leaf, os.O_RDONLY | os.O_CREAT | os.O_EXCL, 0o400)
        os.fsync(descriptor)
        os.close(descriptor)
    FENCE.chmod(0o500)
    directory = os.open(FENCE, os.O_RDONLY | os.O_DIRECTORY)
    os.fsync(directory)
    os.close(directory)

    gate = connect_gate()
    # Model an abruptly lost older backup owner.  Its durable gate must close
    # admission until TTL, but must not require repair or manual deletion.
    gate.execute(
        "INSERT INTO backup_gate(singleton, token, expires_unix_ns, heartbeat_count) "
        "VALUES(1, 'dead-backup-owner', ?, 7)",
        (time.time_ns() + 2_000_000_000,),
    )
    gate.close()
    append_event("fixture.stale-gate-published")


def run_scenario() -> int:
    provision_sandbox()
    initial: dict[str, subprocess.Popen[bytes]] = {}
    restarted: dict[str, subprocess.Popen[bytes]] = {}
    backup: subprocess.Popen[bytes] | None = None
    try:
        for role in ("api", "worker"):
            initial[role] = spawn_actor("--service", role, "initial")
            wait_path(SANDBOX_ROOT / f"initial-{role}.ready")
        assert_exclusive_quiescence_blocked()

        for role, process in initial.items():
            os.kill(process.pid, signal.SIGSTOP)
            append_event("supervisor.sigstop", role=role)

        backup = spawn_actor("--backup")
        wait_actor_path(SANDBOX_ROOT / "backup.admission", backup, "backup actor")
        # Give the independently running heartbeat multiple periods while EX
        # quiescence is demonstrably blocked by the stopped services.
        time.sleep(HEARTBEAT_SECONDS * 3.5)
        if (SANDBOX_ROOT / "backup.complete").exists():
            raise RuntimeError("backup completed while stopped SH holders survived")

        for role, process in initial.items():
            terminate_bounded(process, role)

        try:
            backup_stdout, backup_stderr = backup.communicate(timeout=PROCESS_TIMEOUT_SECONDS)
        except subprocess.TimeoutExpired:
            backup.kill()
            backup_stdout, backup_stderr = backup.communicate(timeout=1.0)
            raise RuntimeError("backup did not complete after quiescence")
        if backup.returncode != 0:
            raise RuntimeError(
                "backup actor failed:\n"
                + backup_stdout.decode(errors="replace")
                + backup_stderr.decode(errors="replace")
            )
        wait_path(SANDBOX_ROOT / "backup.complete")

        for role in ("api", "worker"):
            restarted[role] = spawn_actor("--service", role, "restart")
            wait_path(SANDBOX_ROOT / f"restart-{role}.ready")
        assert_exclusive_quiescence_blocked()
        append_event("supervisor.runtime-restarted")
        for role, process in restarted.items():
            stop_orderly(process, role)
        return 0
    finally:
        for process in [*initial.values(), *restarted.values()]:
            if process.poll() is None:
                try:
                    os.kill(process.pid, signal.SIGCONT)
                except ProcessLookupError:
                    pass
                process.kill()
                process.wait(timeout=1.0)
        if backup is not None and backup.poll() is None:
            backup.kill()
            backup.wait(timeout=1.0)


class RuntimeFenceHostHarnessTests(unittest.TestCase):
    def test_runtime_fence_sigstop_ttl_backup_recovery(self) -> None:
        if sys.platform != "linux":
            self.fail("the production runtime-fence contract requires Linux")
        bwrap = Path("/usr/bin/bwrap")
        self.assertTrue(bwrap.is_file(), "bubblewrap is required; this test must not skip")
        self.assertTrue(os.access(bwrap, os.X_OK), "bubblewrap is not executable")
        self.assertTrue(SELF.is_file())

        with tempfile.TemporaryDirectory(prefix="robin-runtime-fence-host-") as temporary:
            private_home = Path(temporary) / "home"
            private = private_home / ".local/share/robin-highscores"
            private.mkdir(mode=0o700, parents=True)
            command = [
                str(bwrap),
                "--die-with-parent",
                "--unshare-net",
                "--ro-bind", "/usr", "/usr",
                "--ro-bind", "/bin", "/bin",
                "--ro-bind", "/lib", "/lib",
                "--ro-bind", "/lib64", "/lib64",
                "--proc", "/proc",
                "--dev", "/dev",
                "--tmpfs", "/tmp",
                "--dir", "/home",
                "--bind", str(private_home), "/home/robinhood",
                "--dir", "/run",
                "--ro-bind", str(SELF), str(SANDBOX_SELF),
                "--", "/usr/bin/python3", str(SANDBOX_SELF), "--scenario",
            ]
            completed = subprocess.run(
                command,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                check=False,
                timeout=15.0,
            )
            self.assertEqual(
                completed.returncode,
                0,
                f"bubblewrap scenario failed\nstdout:\n{completed.stdout}"
                f"\nstderr:\n{completed.stderr}",
            )

            events = [json.loads(line) for line in (private / "events.jsonl").read_text().splitlines()]
            names = [event["event"] for event in events]
            for required in (
                "fixture.stale-gate-published",
                "backup.gate-waiting-for-ttl",
                "backup.gate-recovered",
                "backup.admission-exclusive",
                "backup.gate-heartbeat",
                "supervisor.kill-after-timeout",
                "backup.quiescence-exclusive",
                "backup.completed",
                "backup.gate-released",
                "supervisor.runtime-restarted",
            ):
                self.assertIn(required, names)

            self.assertEqual(
                {event["role"] for event in events if event["event"] == "supervisor.sigstop"},
                {"api", "worker"},
            )
            self.assertEqual(
                {event["role"] for event in events if event["event"] == "supervisor.kill-after-timeout"},
                {"api", "worker"},
            )
            shared = [event for event in events if event["event"] == "service.shared-acquired"]
            self.assertEqual(
                {(event["role"], event["generation"]) for event in shared},
                {
                    ("api", "initial"),
                    ("worker", "initial"),
                    ("api", "restart"),
                    ("worker", "restart"),
                },
            )
            self.assertGreaterEqual(names.count("backup.gate-heartbeat"), 2)
            self.assertLess(names.index("backup.gate-recovered"), names.index("backup.admission-exclusive"))
            self.assertLess(names.index("backup.admission-exclusive"), names.index("backup.quiescence-exclusive"))
            quiescence_index = names.index("backup.quiescence-exclusive")
            self.assertTrue(
                all(
                    index < quiescence_index
                    for index, name in enumerate(names)
                    if name == "supervisor.kill-after-timeout"
                )
            )
            self.assertLess(names.index("backup.quiescence-exclusive"), names.index("backup.completed"))
            self.assertLess(names.index("backup.completed"), names.index("backup.gate-released"))
            self.assertLess(names.index("backup.gate-released"), names.index("supervisor.runtime-restarted"))

            receipt = json.loads((private / "backup.complete").read_text())
            self.assertGreaterEqual(receipt["heartbeat_count"], 2)
            self.assertEqual(receipt["schema_version"], 2)
            self.assertEqual(receipt["source_commit"], SOURCE_COMMIT)
            self.assertEqual(
                receipt["vps_release_manifest_sha256"],
                VPS_RELEASE_MANIFEST_SHA256,
            )
            self.assertEqual(
                receipt["publication_lock_sha256"], PUBLICATION_LOCK_SHA256
            )
            connection = sqlite3.connect(private / "gate.sqlite3")
            try:
                self.assertEqual(
                    connection.execute("SELECT count(*) FROM backup_gate").fetchone()[0],
                    0,
                    "successful backup must release only its exact durable gate",
                )
            finally:
                connection.close()


def parse_arguments() -> tuple[argparse.Namespace, list[str]]:
    parser = argparse.ArgumentParser()
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--scenario", action="store_true")
    modes.add_argument("--backup", action="store_true")
    modes.add_argument("--service", nargs=2, metavar=("ROLE", "GENERATION"))
    return parser.parse_known_args()


if __name__ == "__main__":
    arguments, unittest_arguments = parse_arguments()
    if arguments.scenario:
        raise SystemExit(run_scenario())
    if arguments.backup:
        raise SystemExit(backup_actor())
    if arguments.service:
        raise SystemExit(service_actor(*arguments.service))
    unittest.main(argv=[sys.argv[0], *unittest_arguments])
