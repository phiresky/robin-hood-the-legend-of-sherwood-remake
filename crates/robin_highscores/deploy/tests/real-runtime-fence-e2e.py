#!/usr/bin/env python3
"""Authentic process-level test of the production SQLite backup fence.

This is a release test, rather than a unit-test simulation. It authenticates
and starts the exact staged VpsManifestV2 API, worker, and admin binaries with
the separately installed production authorities. One outer Bubblewrap
supervisor masks persistent state with tmpfs and supplies a private net/PID
namespace without modifying either the candidate or production state.
"""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pwd
import re
import shutil
import signal
import sqlite3
import stat
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request


SOURCE_COMMIT = re.compile(r"^[0-9a-f]{40}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
UNITS = (
    "robin-highscores.target",
    "robin-highscores-api.service",
    "robin-highscores-worker.service",
    "robin-highscores-backup.service",
    "robin-highscores-backup.timer",
)
ORDINARY_SECRETS = (
    ("cursor-hmac.key", 32),
    ("competition-run-grant.key", 32),
    ("run-preflight-grant.key", 32),
    ("moderation-bearer.token", 64),
)
STATE = Path("/home/robinhood/.local/share/robin-highscores")
INSTALL = Path("/home/robinhood/.local/opt/robin-highscores")
USER_UNITS = Path("/home/robinhood/.config/systemd/user")
INJECTED_SECRETS = Path("/run/robin-real-fence-authorities")
PINNED_CANDIDATE_FD_ENV = "ROBIN_REAL_FENCE_PINNED_CANDIDATE_FD"


def fail(message: str) -> None:
    raise RuntimeError(message)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def chmod(path: Path, mode: int) -> None:
    path.chmod(mode, follow_symlinks=False)


def mkdir(path: Path, mode: int) -> None:
    path.mkdir(parents=True, exist_ok=True)
    chmod(path, mode)


class Namespace:
    def __init__(
        self,
        candidate: Path,
    ) -> None:
        self.candidate = candidate

    def host_path(self, guest: Path) -> Path:
        return guest

    def command(self, command: list[str]) -> list[str]:
        return command

    def run(
        self,
        command: list[str],
        *,
        check: bool = True,
        capture_output: bool = False,
        pass_fds: tuple[int, ...] = (),
    ) -> subprocess.CompletedProcess[bytes]:
        return subprocess.run(
            self.command(command),
            check=check,
            capture_output=capture_output,
            pass_fds=pass_fds,
        )


class Service:
    def __init__(self, name: str, namespace: Namespace, command: list[str], logs: Path) -> None:
        self.name = name
        self.log_path = logs / f"{name}.log"
        self.log = self.log_path.open("ab", buffering=0)
        self.process = subprocess.Popen(
            namespace.command(command),
            stdin=subprocess.DEVNULL,
            stdout=self.log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        self.pgid = os.getpgid(self.process.pid)

    def signal(self, value: signal.Signals) -> None:
        if self.process.poll() is None:
            os.killpg(self.pgid, value)

    def terminate_gracefully(self, timeout: float = 15.0) -> None:
        self.signal(signal.SIGTERM)
        try:
            code = self.process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.signal(signal.SIGKILL)
            self.process.wait(timeout=5)
            fail(f"{self.name} did not drain and close its database pool after SIGTERM")
        if code != 0:
            fail(f"{self.name} exited {code} during graceful fenced shutdown; see {self.log_path}")

    def terminate_stopped_gracefully(self, held_inode: int, timeout: float = 15.0) -> None:
        self.signal(signal.SIGTERM)
        time.sleep(0.05)
        if self.process.poll() is not None or not group_holds_shared(self, held_inode):
            fail(f"{self.name} did not retain its active fence with TERM pending under SIGSTOP")
        self.signal(signal.SIGCONT)
        try:
            code = self.process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.signal(signal.SIGKILL)
            self.process.wait(timeout=5)
            fail(f"{self.name} did not drain its active operation and close its database pool")
        if code != 0:
            fail(f"{self.name} exited {code} during active graceful drain; see {self.log_path}")

    def kill_stopped_transactionally(self, term_timeout: float = 0.35) -> None:
        self.signal(signal.SIGTERM)
        try:
            self.process.wait(timeout=term_timeout)
        except subprocess.TimeoutExpired:
            pass
        else:
            fail(f"SIGSTOPed {self.name} unexpectedly handled TERM before transaction timeout")
        self.signal(signal.SIGKILL)
        self.process.wait(timeout=5)

    def close(self) -> None:
        if self.process.poll() is None:
            try:
                self.signal(signal.SIGCONT)
                self.signal(signal.SIGKILL)
                self.process.wait(timeout=5)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                pass
        self.log.close()


def lock_rows(inode: int) -> list[tuple[str, int]]:
    rows: list[tuple[str, int]] = []
    for line in Path("/proc/locks").read_text(encoding="ascii").splitlines():
        fields = line.split()
        if len(fields) < 6 or fields[1] != "FLOCK":
            continue
        try:
            locked_inode = int(fields[5].rsplit(":", 1)[1])
            pid = int(fields[4])
        except (ValueError, IndexError):
            continue
        if locked_inode == inode:
            rows.append((fields[3], pid))
    return rows


def pending_lock_rows(inode: int) -> list[tuple[str, int]]:
    rows: list[tuple[str, int]] = []
    for line in Path("/proc/locks").read_text(encoding="ascii").splitlines():
        fields = line.split()
        if len(fields) < 7 or fields[1:3] != ["->", "FLOCK"]:
            continue
        try:
            locked_inode = int(fields[6].rsplit(":", 1)[1])
            pid = int(fields[5])
        except (ValueError, IndexError):
            continue
        if locked_inode == inode:
            rows.append((fields[4], pid))
    return rows


def group_holds_shared(service: Service, inode: int) -> bool:
    for kind, pid in lock_rows(inode):
        if kind != "READ":
            continue
        try:
            if os.getpgid(pid) == service.pgid:
                return True
        except ProcessLookupError:
            continue
    return False


def group_waits_shared(service: Service, inode: int) -> bool:
    return any(
        kind == "READ" and _same_pgid(pid, service.pgid)
        for kind, pid in pending_lock_rows(inode)
    )


def wait_until(description: str, predicate, timeout: float = 20.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.005)
    fail(f"timed out waiting for {description}")


def http_ok(port: int, path: str = "/healthz") -> bool:
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=0.25) as response:
            return response.status == 200
    except (OSError, urllib.error.URLError):
        return False


def service_listener_ports(service: Service) -> list[int]:
    socket_inodes: set[str] = set()
    for process in Path("/proc").iterdir():
        if not process.name.isdigit() or not _same_pgid(int(process.name), service.pgid):
            continue
        try:
            descriptors = (process / "fd").iterdir()
            for descriptor in descriptors:
                try:
                    target = os.readlink(descriptor)
                except OSError:
                    continue
                if target.startswith("socket:[") and target.endswith("]"):
                    socket_inodes.add(target[8:-1])
        except OSError:
            continue
    ports: set[int] = set()
    for table in (Path("/proc/net/tcp"), Path("/proc/net/tcp6")):
        for line in table.read_text(encoding="ascii").splitlines()[1:]:
            fields = line.split()
            if len(fields) >= 10 and fields[3] == "0A" and fields[9] in socket_inodes:
                ports.add(int(fields[1].rsplit(":", 1)[1], 16))
    return sorted(ports)


def _discover_and_probe_api(service: Service, destination: list[int]) -> bool:
    if service.process.poll() is not None:
        fail(f"API exited early; see {service.log_path}")
    for port in service_listener_ports(service):
        if http_ok(port):
            destination[:] = [port]
            return True
    return False


def flood_api(port: int, stop: threading.Event) -> list[threading.Thread]:
    def request_loop() -> None:
        while not stop.is_set():
            try:
                urllib.request.urlopen(
                    f"http://127.0.0.1:{port}/api/v1/leaderboards",
                    timeout=0.5,
                ).read(1)
            except (OSError, urllib.error.URLError):
                pass

    threads = [threading.Thread(target=request_loop, daemon=True) for _ in range(32)]
    for thread in threads:
        thread.start()
    return threads


def start_pair(namespace: Namespace, release: Path, logs: Path) -> tuple[Service, Service, int]:
    config = str(release / "config/highscores-server.toml")
    worker_config = str(release / "config/highscores-worker.toml")
    api = Service(
        "api",
        namespace,
        [str(release / "bin/robin-highscores-server"), "--config", config],
        logs,
    )
    worker = Service(
        "worker",
        namespace,
        [str(release / "bin/robin-highscores-worker"), "--config", worker_config],
        logs,
    )
    try:
        port_holder: list[int] = []
        wait_until(
            "API health",
            lambda: _discover_and_probe_api(api, port_holder),
            30,
        )
        quiescence_inode = namespace.host_path(STATE / "runtime-fence/db-quiescence.lock").stat().st_ino
        wait_until(
            "worker startup and a real fenced database operation",
            lambda: (
                fail(f"worker exited early; see {worker.log_path}")
                if worker.process.poll() is not None
                else group_holds_shared(worker, quiescence_inode)
            ),
            60,
        )
        return api, worker, port_holder[0]
    except Exception:
        api.close()
        worker.close()
        raise


def run_admin(namespace: Namespace, release: Path, arguments: list[str], **kwargs):
    return namespace.run(
        [
            str(release / "bin/robin-highscores-admin"),
            "--config",
            str(release / "config/highscores-server.toml"),
            *arguments,
        ],
        **kwargs,
    )


def prepare_activation_lock() -> int:
    chmod(INSTALL, 0o750)
    path = INSTALL / "activation.lock"
    descriptor = os.open(
        path,
        os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW,
        0o600,
    )
    os.fchmod(descriptor, 0o600)
    os.fsync(descriptor)
    directory = os.open(INSTALL, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)
    fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
    metadata = os.fstat(descriptor)
    if (
        not stat.S_ISREG(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or metadata.st_nlink != 1
        or metadata.st_mode & 0o777 != 0o600
        or metadata.st_size != 0
    ):
        fail("disposable activation lock has a noncanonical identity")
    contender = os.open(path, os.O_RDWR | os.O_CLOEXEC | os.O_NOFOLLOW)
    try:
        try:
            fcntl.flock(contender, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            pass
        else:
            fcntl.flock(contender, fcntl.LOCK_UN)
            fail("disposable activation lock does not retain exclusive OFD ownership")
    finally:
        os.close(contender)
    return descriptor


def require_private_regular(
    path: Path,
    expected_mode: int,
    expected_size: int | None,
) -> os.stat_result:
    metadata = path.stat(follow_symlinks=False)
    if (
        not stat.S_ISREG(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or metadata.st_nlink != 1
        or metadata.st_mode & 0o777 != expected_mode
        or (expected_size is not None and metadata.st_size != expected_size)
    ):
        fail(f"private authority has a noncanonical identity: {path}")
    return metadata


def provision(
    namespace: Namespace,
    release: Path,
    source_commit: str,
    expected_manifest_sha: str,
    activation_lock_fd: int,
) -> None:
    candidate_fd = os.open(
        release,
        os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW,
    )
    inherited = (activation_lock_fd, candidate_fd)
    try:
        prepared = run_admin(
            namespace,
            release,
            [
                "initialize-backup-authority-key-v2",
                "--source-commit", source_commit,
                "--activation-lock-fd", str(activation_lock_fd),
            ],
            capture_output=True,
            pass_fds=(activation_lock_fd,),
        )
        if prepared.stdout != b"backup authority HMAC key transaction is prepared\n":
            fail("fifth-key V2 prepare emitted an unexpected result")
        require_private_regular(
            STATE / "api-secrets/backup-authority-hmac.key", 0o400, 32
        )
        intent = STATE / "api-secrets/.backup-authority-hmac-key.intent-v1.json"
        intent_metadata = require_private_regular(intent, 0o400, None)
        if not 0 < intent_metadata.st_size <= 4096:
            fail("fifth-key V2 prepare intent has a noncanonical size")
        namespace.run(
            [
                str(release / "bin/robin-highscores-manifestctl"),
                "initialize-vps-runtime-fence-v1",
                source_commit,
                "--activation-lock-fd", str(activation_lock_fd),
            ],
            pass_fds=(activation_lock_fd,),
        )
        completed = run_admin(
            namespace,
            release,
            [
                "complete-backup-authority-key-v2",
                "--source-commit", source_commit,
                "--activation-lock-fd", str(activation_lock_fd),
                "--candidate-release-root-fd", str(candidate_fd),
                "--expected-vps-release-manifest-sha256", expected_manifest_sha,
            ],
            capture_output=True,
            pass_fds=inherited,
        )
        if completed.stdout != b"backup authority HMAC key transaction is complete\n":
            fail("fifth-key V2 completion emitted an unexpected result")
    finally:
        os.close(candidate_fd)
    require_private_regular(STATE / "api-secrets/backup-authority-hmac.key", 0o400, 32)
    if (STATE / "api-secrets/.backup-authority-hmac-key.intent-v1.json").exists():
        fail("fifth-key V2 prepare/complete left a noncanonical authority")
    fence = STATE / "runtime-fence"
    fence_metadata = fence.stat(follow_symlinks=False)
    if (
        not stat.S_ISDIR(fence_metadata.st_mode)
        or fence_metadata.st_uid != os.geteuid()
        or fence_metadata.st_mode & 0o777 != 0o500
    ):
        fail("runtime-fence V1 initializer left a noncanonical root")
    for name in ("db-admission.lock", "db-quiescence.lock"):
        leaf = (fence / name).stat(follow_symlinks=False)
        if (
            not stat.S_ISREG(leaf.st_mode)
            or leaf.st_uid != os.geteuid()
            or leaf.st_nlink != 1
            or leaf.st_mode & 0o777 != 0o400
            or leaf.st_size != 0
        ):
            fail(f"runtime-fence V1 initializer left a noncanonical {name}")
    run_admin(namespace, release, ["migrate"])


def backup_arguments(release: Path) -> list[str]:
    arguments = [
        "backup-and-publish-status",
        "--release-manifest-path", str(release / "vps-release-manifest-v2.json"),
        "--backup-root", str(STATE / "backups"),
        "--status-path", str(STATE / "status/backup-status.json"),
        "--retain-complete", "2",
    ]
    for secret in (
        "cursor-hmac.key",
        "competition-run-grant.key",
        "run-preflight-grant.key",
        "moderation-bearer.token",
    ):
        original = STATE / "api-secrets" / secret
        arguments.extend(("--restore-source-map", f"{original}={original}"))
    for unit in UNITS:
        original = USER_UNITS / unit
        arguments.extend(("--restore-source-map", f"{original}={original}"))
    return arguments


def verify_backup(namespace: Namespace, release: Path, manifest: dict) -> dict:
    manifest_path = release / "vps-release-manifest-v2.json"
    manifest_sha = sha256_file(namespace.host_path(manifest_path))
    shell = """
set -eu
exec 3<\"$1\"
exec 4<\"$2\"
exec 5<\"$3\"
exec 6<\"$4\"
exec \"$8/bin/robin-highscores-admin\" verify-transaction-backup \\
  --backup-root-fd 3 --status-envelope-fd 4 --backup-authority-key-fd 5 \\
  --expected-release-manifest-fd 6 --expected-source-commit \"$5\" \\
  --expected-vps-release-manifest-sha256 \"$6\" \\
  --expected-publication-lock-sha256 \"$7\"
"""
    result = namespace.run(
        [
            "/bin/bash", "-c", shell, "real-fence-verify",
            str(STATE / "backups"),
            str(STATE / "status/backup-status.json"),
            str(STATE / "api-secrets/backup-authority-hmac.key"),
            str(release / "vps-release-manifest-v2.json"),
            manifest["source_commit"],
            manifest_sha,
            manifest["publication_lock_sha256"],
            str(release),
        ],
        capture_output=True,
    )
    receipt = json.loads(result.stdout)
    if receipt.get("schema_version") != 2:
        fail("transaction verification did not emit BackupVerificationReceiptV2")
    if receipt.get("release_identity", {}).get("source_commit") != manifest["source_commit"]:
        fail("backup receipt is not bound to the candidate source commit")
    if receipt.get("current_status") is None:
        fail("transaction backup receipt omits authenticated current-status evidence")
    return receipt


def verify_live_schema(namespace: Namespace, release: Path, manifest: dict) -> dict:
    manifest_sha = sha256_file(namespace.candidate / "vps-release-manifest-v2.json")
    shell = """
set -eu
exec 3<\"$1\"
exec \"$1/bin/robin-highscores-admin\" verify-live-database-schema-v2 \\
  --candidate-release-root-fd 3 \\
  --expected-vps-release-manifest-sha256 \"$2\"
"""
    result = namespace.run(
        ["/bin/bash", "-c", shell, "real-fence-live-schema", str(release), manifest_sha],
        capture_output=True,
    )
    receipt = json.loads(result.stdout)
    if receipt.get("schema_version") != 2:
        fail("live database schema verifier did not emit its V2 receipt")
    if receipt.get("database_schema_version") != manifest["database_schema_version"]:
        fail("live database schema receipt differs from VpsManifestV2")
    return receipt


def capture_stopped_shared(service: Service, inode: int, timeout: float = 60.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if service.process.poll() is not None:
            fail(f"{service.name} exited before an active fenced operation was captured")
        if group_holds_shared(service, inode):
            service.signal(signal.SIGSTOP)
            if group_holds_shared(service, inode):
                return
            service.signal(signal.SIGCONT)
        time.sleep(0.001)
    fail(f"timed out capturing stopped {service.name} with SH quiescence")


def capture_stopped_shared_without_sqlite_writer(
    service: Service,
    inode: int,
    timeout: float = 60.0,
) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if service.process.poll() is not None:
            fail(f"{service.name} exited before a writer-free fenced operation was captured")
        if group_holds_shared(service, inode):
            service.signal(signal.SIGSTOP)
            writer_available = False
            if group_holds_shared(service, inode):
                try:
                    database_row_after_immediate_rollback_fenced("SELECT 1")
                    writer_available = True
                except sqlite3.OperationalError as error:
                    # A WAL reader can succeed while this stopped operation
                    # still owns SQLite's single-writer slot.  Never strand
                    # that writer while asking BackupV4 to publish its gate.
                    if getattr(error, "sqlite_errorcode", None) != sqlite3.SQLITE_BUSY:
                        raise
            if writer_available and group_holds_shared(service, inode):
                return
            service.signal(signal.SIGCONT)
        time.sleep(0.001)
    fail(f"timed out capturing stopped {service.name} without a SQLite writer")


def database_row_after_immediate_rollback_fenced(
    statement: str,
    parameters: tuple = (),
) -> tuple | None:
    """Read evidence only after proving SQLite's writer slot is available."""
    admission_path = STATE / "runtime-fence/db-admission.lock"
    quiescence_path = STATE / "runtime-fence/db-quiescence.lock"
    admission = os.open(admission_path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
    quiescence = -1
    try:
        try:
            fcntl.flock(admission, fcntl.LOCK_SH | fcntl.LOCK_NB)
        except BlockingIOError:
            fail("test evidence attempted to bypass exclusive database admission")
        quiescence = os.open(
            quiescence_path,
            os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
        )
        try:
            fcntl.flock(quiescence, fcntl.LOCK_SH | fcntl.LOCK_NB)
        except BlockingIOError:
            fail("test evidence attempted to bypass exclusive database quiescence")
        database = STATE / "database/highscores.sqlite3"
        connection = sqlite3.connect(database, timeout=0.0, isolation_level=None)
        try:
            connection.execute("BEGIN IMMEDIATE")
            row = connection.execute(statement, parameters).fetchone()
            connection.execute("ROLLBACK")
            return row
        finally:
            if connection.in_transaction:
                connection.rollback()
            connection.close()
    finally:
        if quiescence >= 0:
            os.close(quiescence)
        os.close(admission)


def database_row_fenced(statement: str, parameters: tuple = ()) -> tuple | None:
    """Read test evidence while obeying the production SH/SH lock order."""
    admission_path = STATE / "runtime-fence/db-admission.lock"
    quiescence_path = STATE / "runtime-fence/db-quiescence.lock"
    admission = os.open(admission_path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
    quiescence = -1
    try:
        try:
            fcntl.flock(admission, fcntl.LOCK_SH | fcntl.LOCK_NB)
        except BlockingIOError:
            fail("test evidence attempted to bypass exclusive database admission")
        quiescence = os.open(
            quiescence_path,
            os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
        )
        try:
            fcntl.flock(quiescence, fcntl.LOCK_SH | fcntl.LOCK_NB)
        except BlockingIOError:
            fail("test evidence attempted to bypass exclusive database quiescence")
        database = STATE / "database/highscores.sqlite3"
        connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True, timeout=1.0)
        try:
            connection.execute("PRAGMA query_only = ON")
            return connection.execute(statement, parameters).fetchone()
        finally:
            connection.close()
    finally:
        if quiescence >= 0:
            os.close(quiescence)
        os.close(admission)


def sqlite_wal_identity() -> tuple:
    """Observe real heartbeat writes without opening SQLite under BackupV4 EX."""
    database = STATE / "database/highscores.sqlite3"
    path = Path(f"{database}-wal")
    try:
        metadata = path.stat(follow_symlinks=False)
    except FileNotFoundError:
        return ("absent",)
    if (
        not stat.S_ISREG(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or path.is_symlink()
    ):
        fail("SQLite WAL side channel has a noncanonical identity")
    return (
        "present",
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def snapshot_artifacts() -> tuple[str, ...]:
    root = STATE / "backups"
    return tuple(
        sorted(
            entry.name
            for entry in os.scandir(root)
            if entry.name != ".backup-operation.lock"
        )
    )


def active_worker_lease() -> tuple[str, int] | None:
    now_ms = time.time_ns() // 1_000_000
    row = database_row_fenced(
        "SELECT token, expires_at_ms FROM maintenance_write_leases "
        "WHERE writer_class = 'worker' AND expires_at_ms > ? "
        "ORDER BY expires_at_ms DESC LIMIT 1",
        (now_ms,),
    )
    return None if row is None else (str(row[0]), int(row[1]))


def capture_stopped_worker_lease(
    service: Service,
    quiescence_inode: int,
    timeout: float = 60.0,
) -> tuple[str, int]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if service.process.poll() is not None:
            fail("worker exited before its production write lease was captured")
        if group_holds_shared(service, quiescence_inode):
            service.signal(signal.SIGSTOP)
            lease = None
            if group_holds_shared(service, quiescence_inode):
                try:
                    lease = active_worker_lease()
                except sqlite3.OperationalError as error:
                    # Resume a worker whose SQLite state cannot be observed
                    # through the canonical SH/SH evidence lane.
                    if getattr(error, "sqlite_errorcode", None) != sqlite3.SQLITE_BUSY:
                        raise
                    lease = None
            if lease is not None and group_holds_shared(service, quiescence_inode):
                return lease
            service.signal(signal.SIGCONT)
        time.sleep(0.001)
    fail("timed out capturing the real worker with its production maintenance-write lease")


def group_holds_write(service: Service, inode: int) -> bool:
    return any(
        kind == "WRITE" and _same_pgid(pid, service.pgid)
        for kind, pid in lock_rows(inode)
    )


def assert_exclusive_drain_retained(
    backup: Service,
    competitor: Service,
    admission_inode: int,
    quiescence_inode: int,
) -> None:
    if backup.process.poll() is not None:
        fail("BackupV4 exited before completing its exclusive TTL drain")
    if not group_holds_write(backup, admission_inode) or not group_holds_write(
        backup, quiescence_inode
    ):
        fail("BackupV4 dropped an exclusive runtime fence during TTL drain")
    if competitor.process.poll() is not None or not group_waits_shared(
        competitor, admission_inode
    ):
        fail("competing admin stopped waiting behind BackupV4 admission")
    if group_holds_shared(competitor, quiescence_inode):
        fail("competing admin crossed BackupV4's retained exclusive admission")


def copy_in_memory_secret_authority(
    source_path: Path,
    destination_path: Path,
    expected_size: int,
) -> None:
    source = os.open(source_path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
    destination = -1
    try:
        source_metadata = os.fstat(source)
        if (
            not stat.S_ISREG(source_metadata.st_mode)
            or source_metadata.st_mode & 0o777 != 0o400
            or source_metadata.st_uid != os.geteuid()
            or source_metadata.st_size != expected_size
        ):
            fail(f"anonymous in-memory authority has the wrong identity: {source_path.name}")
        destination = os.open(
            destination_path,
            os.O_WRONLY
            | os.O_CREAT
            | os.O_EXCL
            | os.O_CLOEXEC
            | os.O_NOFOLLOW,
            0o400,
        )
        os.fchmod(destination, 0o400)
        remaining = expected_size
        while remaining:
            block = os.read(source, remaining)
            if not block:
                fail(f"anonymous in-memory authority was truncated: {source_path.name}")
            view = memoryview(block)
            while view:
                written = os.write(destination, view)
                view = view[written:]
            remaining -= len(block)
        if os.read(source, 1):
            fail(f"anonymous in-memory authority grew during copy: {source_path.name}")
        os.fsync(destination)
    finally:
        os.close(source)
        if destination >= 0:
            os.close(destination)


def install_in_memory_secret_authorities() -> None:
    destination_root = STATE / "api-secrets"
    for name, expected_size in ORDINARY_SECRETS:
        copy_in_memory_secret_authority(
            INJECTED_SECRETS / name,
            destination_root / name,
            expected_size,
        )
    directory = os.open(destination_root, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def prepare_disposable_state(release: Path) -> None:
    chmod(STATE, 0o700)
    chmod(INSTALL / "releases", 0o750)
    chmod(STATE / "api-secrets", 0o700)
    chmod(STATE / "raw-content", 0o550)
    for relative, mode in (
        ("database", 0o700),
        ("replays", 0o700),
        ("campaign-states", 0o700),
        ("backups", 0o700),
        ("status", 0o700),
    ):
        mkdir(STATE / relative, mode)
    mkdir(Path("/home/robinhood/logs"), 0o700)
    install_in_memory_secret_authorities()
    mkdir(USER_UNITS, 0o700)
    # Reproduce the real deployment copy into the disposable tmpfs. Candidate
    # bytes remain read-only and unchanged.
    for name in UNITS:
        shutil.copyfile(release / "systemd/user" / name, USER_UNITS / name)
        chmod(USER_UNITS / name, 0o440)
    for name, expected_size in ORDINARY_SECRETS:
        secret = STATE / "api-secrets" / name
        metadata = secret.stat(follow_symlinks=False)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_mode & 0o777 != 0o400
            or metadata.st_nlink != 1
            or metadata.st_size != expected_size
        ):
            fail(f"supervisor supplied malformed in-memory authority {name}")


def mount_entry(path: Path) -> tuple[str, set[str]] | None:
    expected = str(path).replace(" ", "\\040")
    for line in Path("/proc/self/mountinfo").read_text(encoding="utf-8").splitlines():
        fields = line.split()
        if len(fields) < 10 or fields[4] != expected or "-" not in fields:
            continue
        separator = fields.index("-")
        return fields[separator + 1], set(fields[5].split(","))
    return None


def assert_disposable_mount_namespace(release: Path) -> None:
    state_mount = mount_entry(STATE)
    if state_mount is None or state_mount[0] != "tmpfs" or "rw" not in state_mount[1]:
        fail("inner mode refuses to run without disposable tmpfs over production state")
    for authority in (
        release,
        STATE / "raw-content/demo",
        STATE / "raw-content/full",
    ):
        mounted = mount_entry(authority)
        if mounted is None or "ro" not in mounted[1]:
            fail(f"inner authority is not a separate read-only mount: {authority}")


def execute_inner(
    candidate: Path,
    expected_commit: str,
    expected_sums_sha: str,
    expected_manifest_sha: str,
) -> None:
    manifest, inner_manifest_sha = validate_candidate(
        candidate,
        expected_commit,
        expected_sums_sha,
        run_typed_validator=True,
    )
    if inner_manifest_sha != expected_manifest_sha:
        fail("inner manifest differs from its out-of-band digest")
    release = INSTALL / "releases" / expected_commit
    if candidate != release:
        fail("inner candidate is not mounted at its canonical commit release path")
    namespace = Namespace(candidate)
    assert_disposable_mount_namespace(release)
    prepare_disposable_state(release)
    activation_lock_fd = prepare_activation_lock()
    provision(
        namespace,
        release,
        expected_commit,
        expected_manifest_sha,
        activation_lock_fd,
    )
    verify_live_schema(namespace, release, manifest)
    logs = Path("/home/robinhood/logs")
    admission_inode = (STATE / "runtime-fence/db-admission.lock").stat().st_ino
    quiescence_inode = (STATE / "runtime-fence/db-quiescence.lock").stat().st_ino

    api, worker, port = start_pair(namespace, release, logs)
    graceful_stop = threading.Event()
    graceful_threads = flood_api(port, graceful_stop)
    try:
        capture_stopped_shared(api, quiescence_inode)
        api.terminate_stopped_gracefully(quiescence_inode)
        capture_stopped_shared(worker, quiescence_inode)
        worker.terminate_stopped_gracefully(quiescence_inode)
        if any(kind == "READ" for kind, _ in lock_rows(quiescence_inode)):
            fail("graceful pool drain left a shared quiescence lock")
        if "shutdown requested; draining database fence" not in worker.log_path.read_text(
            encoding="utf-8", errors="replace"
        ):
            fail("worker did not execute its production fenced pool drain")
    finally:
        graceful_stop.set()
        for thread in graceful_threads:
            thread.join(timeout=1)
        api.close()
        worker.close()

    api, worker, port = start_pair(namespace, release, logs)
    flood_stop = threading.Event()
    threads = flood_api(port, flood_stop)
    backup: Service | None = None
    competitor: Service | None = None
    try:
        capture_stopped_shared_without_sqlite_writer(api, quiescence_inode)
        worker_lease_token, worker_lease_expiry_ms = capture_stopped_worker_lease(
            worker, quiescence_inode
        )
        if worker_lease_expiry_ms - time.time_ns() // 1_000_000 < 120_000:
            fail("captured worker maintenance lease lacks a bounded TTL test margin")
        flood_stop.set()
        # Preserve the real committed maintenance lease as crash-stale
        # authority, but reap the stopped worker before BackupV4 starts.  A
        # WAL reader can observe that committed lease while the worker still
        # owns BEGIN IMMEDIATE; leaving it stopped would manufacture a SQLite
        # deadlock before BackupV4 can publish its durable gate.
        worker.kill_stopped_transactionally()
        retained_lease = database_row_after_immediate_rollback_fenced(
            "SELECT expires_at_ms FROM maintenance_write_leases WHERE token = ?",
            (worker_lease_token,),
        )
        if retained_lease != (worker_lease_expiry_ms,):
            fail("killed worker did not retain its exact committed maintenance lease")

        backup = Service(
            "backup",
            namespace,
            [
                str(release / "bin/robin-highscores-admin"),
                "--config", str(release / "config/highscores-server.toml"),
                *backup_arguments(release),
            ],
            logs,
        )
        wait_until(
            "real BackupV4 process holding EX admission while data drains",
            lambda: group_holds_write(backup, admission_inode),
            30,
        )
        if backup.process.poll() is not None or (STATE / "status/backup-status.json").exists():
            fail("BackupV4 did not remain blocked behind stopped active operations")
        if snapshot_artifacts():
            fail("BackupV4 created snapshot artifacts before active operations drained")

        competitor = Service(
            "competing-admin",
            namespace,
            [
                str(release / "bin/robin-highscores-admin"),
                "--config", str(release / "config/highscores-server.toml"),
                "reports", "--limit", "1",
            ],
            logs,
        )
        wait_until(
            "competing real admin pending on BackupV4's EX admission",
            lambda: group_waits_shared(competitor, admission_inode),
            10,
        )
        if competitor.process.poll() is not None or group_holds_shared(
            competitor, quiescence_inode
        ):
            fail("competing real admin crossed BackupV4's EX admission")

        api.kill_stopped_transactionally()
        wait_until(
            "BackupV4 acquiring EX quiescence after bounded holder reap",
            lambda: group_holds_write(backup, quiescence_inode),
            30,
        )
        if snapshot_artifacts():
            fail("BackupV4 crossed the killed worker's still-live maintenance lease")

        heartbeat_wal = sqlite_wal_identity()
        heartbeat_mutations = 0

        def heartbeat_advanced_twice() -> bool:
            nonlocal heartbeat_wal, heartbeat_mutations
            assert_exclusive_drain_retained(
                backup,
                competitor,
                admission_inode,
                quiescence_inode,
            )
            if (STATE / "status/backup-status.json").exists() or snapshot_artifacts():
                fail("BackupV4 created a snapshot before the killed worker lease TTL")
            if time.time_ns() // 1_000_000 >= worker_lease_expiry_ms:
                fail("BackupV4 produced no observable heartbeat writes before lease expiry")
            current = sqlite_wal_identity()
            if current != heartbeat_wal:
                heartbeat_mutations += 1
                heartbeat_wal = current
            return heartbeat_mutations >= 2

        wait_until(
            "two BackupV4 maintenance-lock WAL writes during exclusive TTL drain",
            heartbeat_advanced_twice,
            max(
                1.0,
                (worker_lease_expiry_ms - time.time_ns() // 1_000_000) / 1000,
            ),
        )
        def stale_ttl_elapsed_without_snapshot() -> bool:
            assert_exclusive_drain_retained(
                backup,
                competitor,
                admission_inode,
                quiescence_inode,
            )
            now_ms = time.time_ns() // 1_000_000
            if now_ms < worker_lease_expiry_ms and (
                (STATE / "status/backup-status.json").exists() or snapshot_artifacts()
            ):
                fail("BackupV4 created a snapshot before the killed worker lease TTL")
            return now_ms >= worker_lease_expiry_ms

        wait_until(
            "stopped worker's real maintenance lease TTL without a snapshot",
            stale_ttl_elapsed_without_snapshot,
            max(
                30.0,
                (worker_lease_expiry_ms - time.time_ns() // 1_000_000) / 1000 + 30,
            ),
        )

        exclusive_release_observed_at: float | None = None

        def backup_completed_after_retained_snapshot_fence() -> bool:
            nonlocal exclusive_release_observed_at
            code = backup.process.poll()
            status_published = (STATE / "status/backup-status.json").exists()
            holds_pair = group_holds_write(
                backup, admission_inode
            ) and group_holds_write(backup, quiescence_inode)
            if code is not None:
                if code != 0:
                    fail(f"BackupV4 failed after bounded TERM/KILL/reap; see {backup.log_path}")
                if not status_published:
                    fail("BackupV4 exited without publishing BackupStatusV4")
                return True
            if not status_published:
                assert_exclusive_drain_retained(
                    backup,
                    competitor,
                    admission_inode,
                    quiescence_inode,
                )
            elif holds_pair:
                if competitor.process.poll() is not None or not group_waits_shared(
                    competitor, admission_inode
                ):
                    fail("competing admin entered before BackupV4 began fenced cleanup")
            else:
                if exclusive_release_observed_at is None:
                    exclusive_release_observed_at = time.monotonic()
                elif time.monotonic() - exclusive_release_observed_at > 5:
                    fail("BackupV4 remained alive after releasing its published snapshot fence")
            return False

        wait_until(
            "BackupV4 snapshot publication and fenced process completion",
            backup_completed_after_retained_snapshot_fence,
            180,
        )
        try:
            competitor_code = competitor.process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            fail("competing admin did not enter after BackupV4 released EX admission")
        if competitor_code != 0:
            fail(f"competing admin exited {competitor_code}; see {competitor.log_path}")
        if lock_rows(admission_inode) or lock_rows(quiescence_inode):
            fail("completed BackupV4/admin pools left runtime fence locks held")
        stale_after_drain = database_row_fenced(
            "SELECT expires_at_ms FROM maintenance_write_leases WHERE token = ?",
            (worker_lease_token,),
        )
        active_after_drain = database_row_fenced(
            "SELECT COUNT(*) FROM maintenance_write_leases WHERE expires_at_ms > ?",
            (time.time_ns() // 1_000_000,),
        )
        maintenance_locks_after_drain = database_row_fenced(
            "SELECT COUNT(*) FROM maintenance_locks"
        )
        if (
            stale_after_drain is None
            or int(stale_after_drain[0]) != worker_lease_expiry_ms
            or worker_lease_expiry_ms > time.time_ns() // 1_000_000
            or active_after_drain != (0,)
            or maintenance_locks_after_drain != (0,)
        ):
            fail("BackupV4 did not finish with an expired worker lease and zero active gates")

        status = json.loads((STATE / "status/backup-status.json").read_bytes())
        if status.get("schema_version") != 4:
            fail("backup did not publish BackupStatusV4")
        backup_directory = Path(status["backup_directory"])
        backup_manifest = json.loads((backup_directory / "backup-manifest.json").read_bytes())
        if backup_manifest.get("schema_version") != 4:
            fail("backup did not create BackupManifestV4")
        receipt = verify_backup(namespace, release, manifest)
        verify_live_schema(namespace, release, manifest)

        restarted_api, restarted_worker, restarted_port = start_pair(namespace, release, logs)
        try:
            wait_until("restarted API readiness", lambda: http_ok(restarted_port, "/readyz"), 30)
            restarted_api.terminate_gracefully()
            restarted_worker.terminate_gracefully()
        finally:
            restarted_api.close()
            restarted_worker.close()
        print(json.dumps({
            "backup_id": status["backup_id"],
            "backup_manifest_schema": backup_manifest["schema_version"],
            "backup_receipt_schema": receipt["schema_version"],
            "database_schema_version": manifest["database_schema_version"],
            "maintenance_lease_ttl_exercised": True,
            "maintenance_lock_heartbeat_exercised": True,
            "source_commit": manifest["source_commit"],
            "status": "ok",
        }, sort_keys=True, separators=(",", ":")))
        os.close(activation_lock_fd)
    finally:
        flood_stop.set()
        for thread in threads:
            thread.join(timeout=1)
        if competitor is not None:
            competitor.close()
        if backup is not None:
            backup.close()
        api.close()
        worker.close()


def safe_release_relative(value: str) -> bool:
    return (
        bool(value)
        and re.fullmatch(r"[A-Za-z0-9_./-]+", value) is not None
        and not value.startswith("/")
        and "//" not in value
        and all(component not in ("", ".", "..") for component in value.split("/"))
    )


def candidate_inventory(
    candidate: Path,
    expected_sums_sha: str,
    *,
    pinned_root: bool = False,
) -> dict:
    if not candidate.is_absolute() or (not pinned_root and candidate.is_symlink()):
        fail("candidate must be an absolute real directory")
    if not pinned_root:
        candidate = candidate.resolve(strict=True)
    if not candidate.is_dir():
        fail("candidate is not a directory")
    root_metadata = candidate.stat(follow_symlinks=pinned_root)
    if (
        root_metadata.st_mode & 0o777 != 0o550
        or root_metadata.st_uid != os.geteuid()
    ):
        fail("candidate root has the wrong owner or immutable release mode")
    sums = candidate / "SHA256SUMS"
    if sha256_file(sums) != expected_sums_sha:
        fail("candidate SHA256SUMS differs from its out-of-band digest")

    listed: list[str] = []
    expected: dict[str, str] = {}
    for number, line in enumerate(sums.read_text(encoding="ascii").splitlines(), 1):
        if len(line) < 67 or line[64:66] != "  ":
            fail(f"noncanonical SHA256SUMS line {number}")
        digest, relative = line[:64], line[66:]
        if SHA256.fullmatch(digest) is None or not safe_release_relative(relative):
            fail(f"unsafe SHA256SUMS line {number}")
        if relative == "SHA256SUMS" or relative in expected:
            fail(f"duplicate or self-referential SHA256SUMS line {number}")
        listed.append(relative)
        expected[relative] = digest
    if not listed or listed != sorted(listed):
        fail("SHA256SUMS inventory is empty or not bytewise sorted")

    actual: list[str] = []
    root_device = root_metadata.st_dev
    for path in candidate.rglob("*"):
        relative = path.relative_to(candidate).as_posix()
        if not safe_release_relative(relative):
            fail(f"candidate contains an unsafe path: {relative!r}")
        metadata = path.stat(follow_symlinks=False)
        if stat.S_ISLNK(metadata.st_mode) or metadata.st_dev != root_device:
            fail(f"candidate contains a link or nested filesystem: {relative}")
        if metadata.st_mode & 0o022:
            fail(f"candidate contains a group/world-writable path: {relative}")
        if stat.S_ISDIR(metadata.st_mode):
            continue
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1:
            fail(f"candidate contains a special or hard-linked file: {relative}")
        if relative != "SHA256SUMS":
            actual.append(relative)
    if sorted(actual) != listed:
        fail("SHA256SUMS does not enumerate the exact candidate file tree")
    for relative in listed:
        if sha256_file(candidate / relative) != expected[relative]:
            fail(f"candidate file differs from authenticated SHA256SUMS: {relative}")

    manifest_path = candidate / "vps-release-manifest-v2.json"
    manifest = json.loads(manifest_path.read_bytes())
    return manifest


def validate_candidate(
    candidate: Path,
    expected_commit: str,
    expected_sums_sha: str,
    *,
    pinned_root: bool = False,
    run_typed_validator: bool,
) -> tuple[dict, str]:
    if SOURCE_COMMIT.fullmatch(expected_commit) is None:
        fail("out-of-band source commit is not canonical 40-hex")
    if SHA256.fullmatch(expected_sums_sha) is None:
        fail("out-of-band SHA256SUMS digest is not canonical 64-hex")
    manifest = candidate_inventory(
        candidate,
        expected_sums_sha,
        pinned_root=pinned_root,
    )
    deployment = manifest.get("deployment")
    if (
        manifest.get("schema_version") != 2
        or manifest.get("source_commit") != expected_commit
        or not isinstance(manifest.get("database_schema_version"), int)
        or SOURCE_COMMIT.fullmatch(manifest.get("source_commit", "")) is None
        or SHA256.fullmatch(manifest.get("publication_lock_sha256", "")) is None
        or deployment
        != {
            "current_link": str(INSTALL / "current"),
            "home": "/home/robinhood",
            "install_root": str(INSTALL),
            "persistent_state_root": str(STATE),
            "user": "robinhood",
        }
    ):
        fail("candidate is not the exact canonical VpsManifestV2 deployment identity")
    if run_typed_validator:
        subprocess.run(
            [
                str(candidate / "bin/robin-highscores-manifestctl"),
                "validate-vps-release-v2",
                str(candidate),
            ],
            check=True,
        )
    return manifest, sha256_file(candidate / "vps-release-manifest-v2.json")


def metadata_identity(path: Path, *, follow_symlinks: bool = False) -> tuple:
    metadata = path.stat(follow_symlinks=follow_symlinks)
    return (
        str(path),
        stat.S_IFMT(metadata.st_mode),
        metadata.st_mode & 0o7777,
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_uid,
        metadata.st_gid,
        metadata.st_nlink,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def metadata_tree(root: Path, *, pinned_root: bool = False) -> tuple[tuple, ...]:
    if (not pinned_root and root.is_symlink()) or not root.is_dir():
        fail(f"production authority is not a real directory: {root}")
    root_identity = metadata_identity(root, follow_symlinks=pinned_root)
    result = [root_identity]
    pending = [root]
    while pending:
        directory = pending.pop()
        for entry in sorted(os.scandir(directory), key=lambda item: item.name):
            path = Path(entry.path)
            identity = metadata_identity(path)
            result.append(identity)
            if entry.is_symlink():
                fail(f"production authority contains a symlink: {path}")
            if entry.is_dir(follow_symlinks=False):
                pending.append(path)
            elif not entry.is_file(follow_symlinks=False):
                fail(f"production authority contains a special leaf: {path}")
    return tuple(result)


def optional_metadata_tree(root: Path) -> tuple:
    try:
        root.stat(follow_symlinks=False)
    except FileNotFoundError:
        if root.is_symlink():
            fail(f"absent production authority is a dangling symlink: {root}")
        return ("absent", str(root))
    return ("present", metadata_tree(root))


def immutable_raw_metadata_tree(
    root: Path,
    *,
    pinned_root: bool = False,
) -> tuple[tuple, ...]:
    root_metadata = root.stat(follow_symlinks=pinned_root)
    if (
        (not pinned_root and root.is_symlink())
        or not stat.S_ISDIR(root_metadata.st_mode)
        or root_metadata.st_uid != os.geteuid()
        or root_metadata.st_mode & 0o7777 != 0o550
    ):
        fail(f"raw authority root has the wrong identity: {root}")
    root_device = root_metadata.st_dev
    root_identity = metadata_identity(root, follow_symlinks=pinned_root)
    result = [(root_identity, None)]
    pending = [root]
    while pending:
        directory = pending.pop()
        for entry in sorted(os.scandir(directory), key=lambda item: item.name):
            path = Path(entry.path)
            metadata = path.stat(follow_symlinks=False)
            identity = metadata_identity(path)
            if (
                entry.is_symlink()
                or metadata.st_uid != os.geteuid()
                or metadata.st_dev != root_device
            ):
                fail(f"raw authority crosses an owner/device/link boundary: {path}")
            if entry.is_dir(follow_symlinks=False):
                if metadata.st_mode & 0o7777 != 0o550:
                    fail(f"raw authority directory mode is not 0550: {path}")
                result.append((identity, None))
                pending.append(path)
            elif entry.is_file(follow_symlinks=False):
                if metadata.st_mode & 0o7777 != 0o440 or metadata.st_nlink != 1:
                    fail(f"raw authority file mode/link count is not canonical: {path}")
                digest = sha256_file(path)
                if metadata_identity(path) != identity:
                    fail(f"raw authority file changed while it was hashed: {path}")
                result.append((identity, digest))
            else:
                fail(f"raw authority contains a special leaf: {path}")
    if metadata_identity(root, follow_symlinks=pinned_root) != root_identity:
        fail(f"raw authority root changed while it was fingerprinted: {root}")
    return tuple(result)


def assert_no_nested_mounts(root: Path) -> None:
    root_prefix = f"{root}/"
    for line in Path("/proc/self/mountinfo").read_text(encoding="utf-8").splitlines():
        fields = line.split()
        if len(fields) >= 5:
            mountpoint = fields[4].replace("\\040", " ")
            if mountpoint.startswith(root_prefix):
                fail(f"raw authority contains a nested mount: {mountpoint}")


def production_authority_fingerprint(
    demo: Path,
    full: Path,
    *,
    pinned_raw_roots: bool = False,
) -> tuple:
    # Do not read or hash secret bytes. The mutable DB/replay/backup children
    # may legitimately change under the old live release; the E2E cannot reach
    # them because STATE is masked by tmpfs. Stable authority metadata must be
    # bit-for-bit unchanged.
    return (
        metadata_identity(STATE),
        metadata_tree(STATE / "api-secrets"),
        optional_metadata_tree(STATE / "runtime-fence"),
        metadata_identity(STATE / "raw-content"),
        immutable_raw_metadata_tree(demo, pinned_root=pinned_raw_roots),
        immutable_raw_metadata_tree(full, pinned_root=pinned_raw_roots),
    )


def open_directory_authority(path: Path) -> int:
    flags = os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW
    flags |= getattr(os, "O_PATH", os.O_RDONLY)
    descriptor = os.open(path, flags)
    metadata = os.fstat(descriptor)
    if not stat.S_ISDIR(metadata.st_mode):
        os.close(descriptor)
        fail(f"authority descriptor is not a directory: {path}")
    return descriptor


def inherit_pinned_candidate_authority(candidate: Path) -> int:
    value = os.environ.pop(PINNED_CANDIDATE_FD_ENV, None)
    if value is None or re.fullmatch(r"[0-9]+", value) is None:
        fail("release workflow omitted the inherited pinned candidate descriptor")
    inherited = int(value)
    if inherited < 3:
        fail("pinned candidate descriptor is not a safe inherited descriptor")
    try:
        descriptor = os.dup(inherited)
    except OSError as error:
        fail(f"pinned candidate descriptor is unavailable: {error}")
    metadata = os.fstat(descriptor)
    if not stat.S_ISDIR(metadata.st_mode) or not descriptor_matches_path(descriptor, candidate):
        os.close(descriptor)
        fail("inherited candidate descriptor does not name the canonical staged release")
    return descriptor


def descriptor_path(descriptor: int) -> Path:
    path = Path(f"/proc/self/fd/{descriptor}")
    if not path.is_dir():
        fail("pinned directory descriptor disappeared")
    return path


def descriptor_matches_path(descriptor: int, path: Path) -> bool:
    pinned = os.fstat(descriptor)
    current = path.stat(follow_symlinks=False)
    return (
        pinned.st_dev == current.st_dev
        and pinned.st_ino == current.st_ino
        and stat.S_IFMT(pinned.st_mode) == stat.S_IFMT(current.st_mode)
    )


def open_secret_authorities() -> dict[str, int]:
    descriptors: dict[str, int] = {}
    try:
        for name, expected_size in ORDINARY_SECRETS:
            path = STATE / "api-secrets" / name
            descriptor = os.open(
                path,
                os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
            )
            metadata = os.fstat(descriptor)
            if (
                not stat.S_ISREG(metadata.st_mode)
                or metadata.st_mode & 0o777 != 0o400
                or metadata.st_uid != os.geteuid()
                or metadata.st_nlink != 1
                or metadata.st_size != expected_size
            ):
                os.close(descriptor)
                fail(f"production secret has the wrong identity: {name}")
            descriptors[name] = descriptor
        return descriptors
    except Exception:
        for descriptor in descriptors.values():
            os.close(descriptor)
        raise


def supervisor_command(
    candidate_fd: int,
    demo_fd: int,
    full_fd: int,
    harness_fd: int,
    secret_fds: dict[str, int],
    expected_commit: str,
    expected_sums_sha: str,
    expected_manifest_sha: str,
) -> list[str]:
    release = INSTALL / "releases" / expected_commit
    inner_harness = Path("/run/robin-real-fence-e2e.py")
    command = [
        "/usr/bin/bwrap",
        "--die-with-parent",
        "--ro-bind", "/", "/",
        "--tmpfs", "/home",
        "--dir", "/home/robinhood",
        "--dir", "/home/robinhood/.local",
        "--dir", "/home/robinhood/.local/opt",
        "--dir", str(INSTALL),
        "--chmod", "0750", str(INSTALL),
        "--dir", str(INSTALL / "releases"),
        "--chmod", "0750", str(INSTALL / "releases"),
        "--dir", str(release),
        "--ro-bind-fd", str(candidate_fd), str(release),
        "--dir", "/home/robinhood/.local/share",
        "--dir", str(STATE),
        "--tmpfs", str(STATE),
        "--dir", str(STATE / "api-secrets"),
        "--chmod", "0700", str(STATE / "api-secrets"),
        "--dir", str(STATE / "raw-content"),
        "--chmod", "0550", str(STATE / "raw-content"),
        "--dir", str(STATE / "raw-content/demo"),
        "--ro-bind-fd", str(demo_fd), str(STATE / "raw-content/demo"),
        "--dir", str(STATE / "raw-content/full"),
        "--ro-bind-fd", str(full_fd), str(STATE / "raw-content/full"),
        "--dir", "/home/robinhood/.config",
        "--dir", "/home/robinhood/.config/systemd",
        "--dir", str(USER_UNITS),
        "--tmpfs", "/run",
        "--dir", str(INJECTED_SECRETS),
        "--chmod", "0700", str(INJECTED_SECRETS),
        "--perms", "0500", "--ro-bind-data", str(harness_fd), str(inner_harness),
    ]
    for name, _ in ORDINARY_SECRETS:
        command.extend(
            (
                "--perms", "0400", "--ro-bind-data", str(secret_fds[name]),
                str(INJECTED_SECRETS / name),
            )
        )
    command.extend(
        (
            "--tmpfs", "/tmp",
            "--dev", "/dev",
            "--proc", "/proc",
            "--unshare-net",
            "--unshare-pid",
            "--new-session",
            "--clearenv",
            "--setenv", "HOME", "/home/robinhood",
            "--setenv", "USER", "robinhood",
            "--setenv", "LOGNAME", "robinhood",
            "--setenv", "PATH", "/usr/bin:/bin",
            "--setenv", "RUST_LOG", "info",
            "--chdir", "/home/robinhood",
            "--",
            "/usr/bin/python3", str(inner_harness),
            "--inner",
            "--release-root", str(release),
            "--expected-source-commit", expected_commit,
            "--expected-sha256s-sha256", expected_sums_sha,
            "--expected-vps-manifest-sha256", expected_manifest_sha,
        )
    )
    return command


def execute_outer(arguments: argparse.Namespace) -> None:
    if os.geteuid() == 0 or pwd.getpwuid(os.geteuid()).pw_name != "robinhood":
        fail("authentic VPS gate must run as the unprivileged robinhood account")
    candidate = Path(arguments.release_root)
    demo = Path(arguments.demo_raw_root)
    full = Path(arguments.full_raw_root)
    expected_commit = arguments.expected_source_commit
    expected_sums_sha = arguments.expected_sha256s_sha256
    expected_candidate = INSTALL / "releases" / f"{expected_commit}.partial"
    expected_demo = STATE / "raw-content/demo"
    expected_full = STATE / "raw-content/full"
    for supplied, expected, label in (
        (candidate, expected_candidate, "candidate"),
        (demo, expected_demo, "Demo raw root"),
        (full, expected_full, "Full raw root"),
    ):
        if not supplied.is_absolute() or supplied != expected:
            fail(f"{label} path is not the exact pre-activation VPS authority")
        if supplied.is_symlink() or supplied.resolve(strict=True) != supplied:
            fail(f"{label} path traverses a symlink or alias")

    descriptors: list[int] = []
    secret_descriptors: dict[str, int] = {}
    try:
        candidate_fd = inherit_pinned_candidate_authority(candidate)
        demo_fd = open_directory_authority(demo)
        full_fd = open_directory_authority(full)
        descriptors.extend((candidate_fd, demo_fd, full_fd))
        secret_descriptors = open_secret_authorities()
        descriptors.extend(secret_descriptors.values())
        for descriptor, path, label in (
            (candidate_fd, candidate, "candidate"),
            (demo_fd, demo, "Demo raw root"),
            (full_fd, full, "Full raw root"),
            *(
                (secret_descriptors[name], STATE / "api-secrets" / name, name)
                for name, _ in ORDINARY_SECRETS
            ),
        ):
            if not descriptor_matches_path(descriptor, path):
                fail(f"{label} changed while its authority descriptor was opened")
        assert_no_nested_mounts(demo)
        assert_no_nested_mounts(full)
        pinned_candidate = descriptor_path(candidate_fd)
        pinned_demo = descriptor_path(demo_fd)
        pinned_full = descriptor_path(full_fd)
        before_host = production_authority_fingerprint(
            pinned_demo,
            pinned_full,
            pinned_raw_roots=True,
        )
        before_candidate = metadata_tree(pinned_candidate, pinned_root=True)
        manifest, expected_manifest_sha = validate_candidate(
            pinned_candidate,
            expected_commit,
            expected_sums_sha,
            pinned_root=True,
            run_typed_validator=False,
        )
        if manifest["source_commit"] != expected_commit:
            fail("authenticated candidate differs from the out-of-band source commit")
        harness_fd = os.open(
            Path(__file__).resolve(strict=True),
            os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
        )
        descriptors.append(harness_fd)
        subprocess.run(
            supervisor_command(
                candidate_fd,
                demo_fd,
                full_fd,
                harness_fd,
                secret_descriptors,
                expected_commit,
                expected_sums_sha,
                expected_manifest_sha,
            ),
            check=True,
            pass_fds=tuple(descriptors),
        )
        validate_candidate(
            pinned_candidate,
            expected_commit,
            expected_sums_sha,
            pinned_root=True,
            run_typed_validator=False,
        )
        if metadata_tree(pinned_candidate, pinned_root=True) != before_candidate:
            fail("candidate metadata changed across its authentic process E2E")
        if (
            production_authority_fingerprint(
                pinned_demo,
                pinned_full,
                pinned_raw_roots=True,
            )
            != before_host
        ):
            fail("production authority metadata changed across the masked-state E2E")
        assert_no_nested_mounts(demo)
        assert_no_nested_mounts(full)
        for descriptor, path, label in (
            (candidate_fd, candidate, "candidate"),
            (demo_fd, demo, "Demo raw root"),
            (full_fd, full, "Full raw root"),
            *(
                (secret_descriptors[name], STATE / "api-secrets" / name, name)
                for name, _ in ORDINARY_SECRETS
            ),
        ):
            if not descriptor_matches_path(descriptor, path):
                fail(f"{label} path identity changed across its authentic process E2E")
    finally:
        for descriptor in descriptors:
            os.close(descriptor)


def _same_pgid(pid: int, pgid: int) -> bool:
    try:
        return os.getpgid(pid) == pgid
    except ProcessLookupError:
        return False


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-root", required=True, help="immutable VpsManifestV2 candidate")
    parser.add_argument("--demo-raw-root", help="installed immutable Demo raw root")
    parser.add_argument("--full-raw-root", help="installed immutable Full raw root")
    parser.add_argument("--expected-source-commit", required=True)
    parser.add_argument("--expected-sha256s-sha256")
    parser.add_argument("--expected-vps-manifest-sha256", help=argparse.SUPPRESS)
    parser.add_argument("--inner", action="store_true", help=argparse.SUPPRESS)
    arguments = parser.parse_args()
    try:
        if arguments.inner:
            if (
                arguments.demo_raw_root is not None
                or arguments.full_raw_root is not None
                or arguments.expected_sha256s_sha256 is None
                or arguments.expected_vps_manifest_sha256 is None
            ):
                fail("invalid inner supervisor arguments")
            execute_inner(
                Path(arguments.release_root),
                arguments.expected_source_commit,
                arguments.expected_sha256s_sha256,
                arguments.expected_vps_manifest_sha256,
            )
        else:
            if (
                arguments.demo_raw_root is None
                or arguments.full_raw_root is None
                or arguments.expected_sha256s_sha256 is None
                or arguments.expected_vps_manifest_sha256 is not None
            ):
                fail("outer gate requires candidate, raw roots, commit, and SHA256SUMS digest")
            execute_outer(arguments)
    except Exception as error:
        print(f"real runtime fence E2E failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
