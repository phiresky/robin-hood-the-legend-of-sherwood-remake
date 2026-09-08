#!/usr/bin/python3
"""Failure-injection tests for deploy-release.sh and rollback-release.sh.

The scripts under test run byte-for-byte inside bubblewrap.  A private bind at
``/home/robinhood`` contains the fixture while selected absolute host commands
are bind-overlaid with ``transaction_host_command.py``.  No host service or VPS
is contacted.

Until this harness is cherry-picked beside the transaction implementation, use
``ROBIN_TX_SOURCE_DIR=/path/to/crates/robin_highscores/deploy`` to select the
reviewed script tree.
"""

from __future__ import annotations

import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import stat
import subprocess
import tempfile
import time
import unittest


HERE = Path(__file__).resolve().parent
DEFAULT_SOURCE = HERE.parent
SOURCE = Path(os.environ.get("ROBIN_TX_SOURCE_DIR", DEFAULT_SOURCE)).resolve()
HOST_DOUBLE = HERE / "transaction_host_command.py"
OLD = "1" * 40
NEW = "2" * 40
THIRD = "3" * 40
SUMS = "e" * 64
DATABASE_SCHEMA_VERSION = 2
RELEASE_MANIFEST = "vps-release-manifest-v2.json"
PUBLICATION_SCHEMA_VERSION = 3
PUBLICATION_MANIFEST = "publication-manifest-v3.json"
PUBLICATION_MANIFEST_SIDECAR = "publication-manifest-v3.sha256"
PUBLICATION_LOCK = "publication-lock-v3.json"
PUBLICATION_LOCK_SIDECAR = "publication-lock-v3.sha256"
BACKUP_AUTHORITY_KEY = "backup-authority-hmac.key"
RUNTIME_FENCE = "runtime-fence"
DB_ADMISSION_LOCK = "db-admission.lock"
DB_QUIESCENCE_LOCK = "db-quiescence.lock"
TRANSACTION_TIMEOUT_SECONDS = int(os.environ.get("ROBIN_TX_TIMEOUT_SECONDS", "180"))
if TRANSACTION_TIMEOUT_SECONDS < 30:
    raise RuntimeError("ROBIN_TX_TIMEOUT_SECONDS must be at least 30")
INITIAL_SECRET_BYTES = {
    "cursor-hmac.key": b"c" * 32,
    "competition-run-grant.key": b"g" * 32,
    "run-preflight-grant.key": b"p" * 32,
    "moderation-bearer.token": b"m" * 64,
}

ADMIN_TOOL = r'''#!/usr/bin/python3
from __future__ import annotations

import hashlib
import fcntl
import hmac
import json
import os
from pathlib import Path
import shutil
import sqlite3
import stat
import subprocess
import sys
import tempfile


STATE_ROOT = Path("/home/robinhood/.local/share/robin-highscores")
SECRET_ROOT = STATE_ROOT / "api-secrets"
RUNTIME_FENCE = STATE_ROOT / "runtime-fence"
RELEASE_MANIFEST = "vps-release-manifest-v2.json"
EXPECTED_SECRETS = {
    "cursor-hmac.key": 32,
    "competition-run-grant.key": 32,
    "run-preflight-grant.key": 32,
    "moderation-bearer.token": 64,
}
BACKUP_AUTHORITY_INTENT = ".backup-authority-hmac-key.intent-v1.json"
BACKUP_AUTHORITY_INTENT_TEMPORARY = BACKUP_AUTHORITY_INTENT + ".new"
BACKUP_AUTHORITY_KEY_TEMPORARY = ".backup-authority-hmac-key.payload-v1.new"


def fail(message: str) -> None:
    print(message, file=sys.stderr)
    raise SystemExit(65)


def fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def fsync_file(path: Path) -> None:
    with path.open("rb") as stream:
        os.fsync(stream.fileno())


def exact_file(path: Path, mode: int, size: int) -> None:
    metadata = path.lstat()
    if (
        not stat.S_ISREG(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != mode
        or metadata.st_uid != os.getuid()
        or metadata.st_nlink != 1
        or metadata.st_size != size
    ):
        fail(f"inexact authority file: {path}")


def option(args: list[str], name: str) -> str:
    if args.count(name) != 1:
        fail(f"option must occur exactly once: {name}")
    index = args.index(name)
    if index + 1 >= len(args):
        fail(f"option has no value: {name}")
    return args[index + 1]


def candidate_authority(args: list[str]) -> tuple[Path, dict[str, object], str]:
    fd_text = option(args, "--candidate-release-root-fd")
    expected_digest = option(args, "--expected-vps-release-manifest-sha256")
    if not fd_text.isdigit() or len(expected_digest) != 64:
        fail("invalid candidate descriptor authority")
    root_fd = int(fd_text)
    root_metadata = os.fstat(root_fd)
    if not stat.S_ISDIR(root_metadata.st_mode):
        fail("candidate release root descriptor is not a directory")
    root = Path(f"/proc/self/fd/{root_fd}")
    manifest_path = root / RELEASE_MANIFEST
    manifest_bytes = manifest_path.read_bytes()
    actual_digest = hashlib.sha256(manifest_bytes).hexdigest()
    if actual_digest != expected_digest:
        fail("candidate release manifest digest mismatch")
    manifest = json.loads(manifest_bytes)
    if (
        not isinstance(manifest, dict)
        or manifest.get("schema_version") != 2
        or not isinstance(manifest.get("source_commit"), str)
        or len(manifest["source_commit"]) != 40
    ):
        fail("candidate VpsManifestV2 identity is invalid")
    executable = root / "bin/robin-highscores-admin"
    expected = executable.lstat()
    actual = Path(sys.argv[0]).lstat()
    if (
        not stat.S_ISREG(expected.st_mode)
        or stat.S_IMODE(expected.st_mode) != 0o550
        or expected.st_nlink != 1
        or (actual.st_dev, actual.st_ino) != (expected.st_dev, expected.st_ino)
    ):
        fail("candidate admin did not self-attest")
    return root, manifest, actual_digest


def validate_common_runtime_authority() -> None:
    metadata = SECRET_ROOT.lstat()
    if not stat.S_ISDIR(metadata.st_mode) or stat.S_IMODE(metadata.st_mode) != 0o700:
        fail("API secret root is inexact")
    for name, size in EXPECTED_SECRETS.items():
        exact_file(SECRET_ROOT / name, 0o400, size)


def validate_fence() -> None:
    metadata = RUNTIME_FENCE.lstat()
    if (
        not stat.S_ISDIR(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o500
        or metadata.st_uid != os.getuid()
    ):
        fail("runtime fence root is inexact")
    if {entry.name for entry in RUNTIME_FENCE.iterdir()} != {
        "db-admission.lock",
        "db-quiescence.lock",
    }:
        fail("runtime fence inventory is inexact")
    exact_file(RUNTIME_FENCE / "db-admission.lock", 0o400, 0)
    exact_file(RUNTIME_FENCE / "db-quiescence.lock", 0o400, 0)


def emit(value: dict[str, object]) -> None:
    sys.stdout.write(json.dumps(value, sort_keys=True, separators=(",", ":")))


def file_fingerprint(path: Path) -> tuple[bool, int, int, int, str]:
    if not path.exists():
        return (False, 0, 0, 0, "")
    metadata = path.lstat()
    return (
        True,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
        hashlib.sha256(path.read_bytes()).hexdigest(),
    )


def probe_runtime_authority(args: list[str]) -> None:
    if len(args) != 6 or args[0] != "--candidate-release-root-fd" or args[2] != "--expected-vps-release-manifest-sha256" or args[4] != "--backup-authority-state":
        fail("runtime-authority probe argv is not canonical")
    root, manifest, digest = candidate_authority(args)
    del root
    expected_state = option(args, "--backup-authority-state")
    if expected_state not in ("absent", "present"):
        fail("invalid backup-authority state")
    validate_common_runtime_authority()
    backup_key = SECRET_ROOT / "backup-authority-hmac.key"
    if expected_state == "absent":
        if backup_key.exists() or backup_key.is_symlink():
            fail("backup authority unexpectedly exists")
        if RUNTIME_FENCE.exists() or RUNTIME_FENCE.is_symlink():
            fail("runtime fence unexpectedly exists")
    else:
        exact_file(backup_key, 0o400, 32)
        validate_fence()
    subprocess.run(
        ["/usr/bin/true", f"admin.probe-runtime-authority-v2.{expected_state}"],
        check=True,
    )
    emit(
        {
            "backup_authority_state": expected_state,
            "schema_version": 2,
            "source_commit": manifest["source_commit"],
            "vps_release_manifest_sha256": digest,
        }
    )


def activation_lock_authority(value: str) -> os.stat_result:
    if not value.isdigit() or int(value) < 3:
        fail("backup-authority activation-lock FD is invalid")
    metadata = os.fstat(int(value))
    canonical = Path("/home/robinhood/.local/opt/robin-highscores/activation.lock")
    if (
        not stat.S_ISREG(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o600
        or metadata.st_uid != os.getuid()
        or metadata.st_nlink != 1
        or Path(os.readlink(f"/proc/self/fd/{value}")) != canonical
        or (metadata.st_dev, metadata.st_ino)
        != (canonical.stat().st_dev, canonical.stat().st_ino)
    ):
        fail("backup-authority activation-lock authority is inexact")
    fcntl.flock(int(value), fcntl.LOCK_EX | fcntl.LOCK_NB)
    return metadata


def backup_authority_config(config: Path, source_commit: str) -> None:
    if config.name != "highscores-server.toml" or not config.is_file():
        fail("backup-authority config authority is invalid")
    release = config.parent.parent
    manifest = json.loads((release / RELEASE_MANIFEST).read_bytes())
    if manifest.get("source_commit") != source_commit:
        fail("backup-authority config belongs to a different release")


def load_backup_authority_intent(
    path: Path, source_commit: str, lock: os.stat_result
) -> dict[str, object]:
    metadata = path.lstat()
    document = json.loads(path.read_bytes())
    canonical = json.dumps(document, sort_keys=True, separators=(",", ":")).encode()
    parent = SECRET_ROOT.stat()
    if (
        not stat.S_ISREG(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o400
        or metadata.st_uid != os.getuid()
        or metadata.st_nlink != 1
        or path.read_bytes() != canonical
        or set(document)
        != {
            "activation_lock_device",
            "activation_lock_inode",
            "key_device",
            "key_inode",
            "key_sha256",
            "schema_version",
            "secret_parent_device",
            "secret_parent_inode",
            "source_commit",
        }
        or document["schema_version"] != 1
        or document["source_commit"] != source_commit
        or document["activation_lock_device"] != lock.st_dev
        or document["activation_lock_inode"] != lock.st_ino
        or document["secret_parent_device"] != parent.st_dev
        or document["secret_parent_inode"] != parent.st_ino
    ):
        fail("backup-authority intent is not exact transaction authority")
    return document


def initialize_backup_authority_v2(args: list[str]) -> None:
    if (
        len(args) != 7
        or args[0] != "--config"
        or args[2] != "initialize-backup-authority-key-v2"
        or args[3] != "--source-commit"
        or args[5] != "--activation-lock-fd"
    ):
        fail("backup-authority V2 initializer argv is not canonical")
    config = Path(args[1])
    source_commit = args[4]
    backup_authority_config(config, source_commit)
    lock = activation_lock_authority(args[6])
    destination = SECRET_ROOT / "backup-authority-hmac.key"
    intent = SECRET_ROOT / BACKUP_AUTHORITY_INTENT
    intent_temporary = SECRET_ROOT / BACKUP_AUTHORITY_INTENT_TEMPORARY
    key_temporary = SECRET_ROOT / BACKUP_AUTHORITY_KEY_TEMPORARY
    if intent_temporary.exists() or intent_temporary.is_symlink():
        if intent.exists() or destination.exists():
            fail("backup-authority intent temporary is ambiguous")
        exact_file(intent_temporary, 0o400, intent_temporary.stat().st_size)
        intent_temporary.unlink()
        fsync_directory(SECRET_ROOT)
    if intent.exists() or intent.is_symlink():
        document = load_backup_authority_intent(intent, source_commit, lock)
        if destination.exists() and not destination.is_symlink():
            exact_file(destination, 0o400, 32)
            key_metadata = destination.stat()
            if (
                document["key_device"] != key_metadata.st_dev
                or document["key_inode"] != key_metadata.st_ino
                or document["key_sha256"]
                != hashlib.sha256(destination.read_bytes()).hexdigest()
            ):
                fail("published backup-authority key differs from its intent")
            subprocess.run(
                ["/usr/bin/true", "admin.initialize-backup-authority-key-v2"],
                check=True,
            )
            return
        if destination.is_symlink():
            fail("backup-authority key path is linked during recovery")
        intent.unlink()
        if key_temporary.exists() and not key_temporary.is_symlink():
            exact_file(key_temporary, 0o400, 32)
            key_temporary.unlink()
        elif key_temporary.is_symlink():
            fail("backup-authority key temporary is linked during recovery")
        fsync_directory(SECRET_ROOT)
    if destination.exists() or destination.is_symlink():
        fail("backup-authority key exists without its transaction intent")
    if key_temporary.exists() or key_temporary.is_symlink():
        exact_file(key_temporary, 0o400, 32)
        key_temporary.unlink()
    descriptor = os.open(key_temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o400)
    try:
        os.write(descriptor, b"b" * 32)
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    key_metadata = key_temporary.stat()
    parent = SECRET_ROOT.stat()
    document = {
        "activation_lock_device": lock.st_dev,
        "activation_lock_inode": lock.st_ino,
        "key_device": key_metadata.st_dev,
        "key_inode": key_metadata.st_ino,
        "key_sha256": hashlib.sha256(key_temporary.read_bytes()).hexdigest(),
        "schema_version": 1,
        "secret_parent_device": parent.st_dev,
        "secret_parent_inode": parent.st_ino,
        "source_commit": source_commit,
    }
    intent_temporary.write_bytes(
        json.dumps(document, sort_keys=True, separators=(",", ":")).encode()
    )
    intent_temporary.chmod(0o400)
    fsync_file(intent_temporary)
    os.replace(intent_temporary, intent)
    fsync_directory(SECRET_ROOT)
    subprocess.run(
        ["/usr/bin/true", "admin.initialize-backup-authority-key-v2.intent-published"],
        check=True,
    )
    os.replace(key_temporary, destination)
    fsync_directory(SECRET_ROOT)
    subprocess.run(
        ["/usr/bin/true", "admin.initialize-backup-authority-key-v2.key-linked"],
        check=True,
    )
    load_backup_authority_intent(intent, source_commit, lock)
    subprocess.run(
        ["/usr/bin/true", "admin.initialize-backup-authority-key-v2"], check=True
    )


def complete_backup_authority_v2(args: list[str]) -> None:
    expected = (
        "--config",
        "complete-backup-authority-key-v2",
        "--source-commit",
        "--activation-lock-fd",
        "--candidate-release-root-fd",
        "--expected-vps-release-manifest-sha256",
    )
    if len(args) != 11 or (args[0], args[2], args[3], args[5], args[7], args[9]) != expected:
        fail("backup-authority V2 completion argv is not canonical")
    config = Path(args[1])
    source_commit = args[4]
    backup_authority_config(config, source_commit)
    lock = activation_lock_authority(args[6])
    _, manifest, digest = candidate_authority(
        [
            "--candidate-release-root-fd",
            args[8],
            "--expected-vps-release-manifest-sha256",
            args[10],
        ]
    )
    if manifest.get("source_commit") != source_commit or digest != args[10]:
        fail("backup-authority completion release identity differs")
    validate_common_runtime_authority()
    exact_file(SECRET_ROOT / "backup-authority-hmac.key", 0o400, 32)
    validate_fence()
    subprocess.run(
        ["/usr/bin/true", "admin.complete-backup-authority-key-v2.outer-present"],
        check=True,
    )
    intent = SECRET_ROOT / BACKUP_AUTHORITY_INTENT
    if intent.exists() or intent.is_symlink():
        document = load_backup_authority_intent(intent, source_commit, lock)
        key = SECRET_ROOT / "backup-authority-hmac.key"
        metadata = key.stat()
        if (
            document["key_device"] != metadata.st_dev
            or document["key_inode"] != metadata.st_ino
            or document["key_sha256"] != hashlib.sha256(key.read_bytes()).hexdigest()
        ):
            fail("backup-authority completion key differs from intent")
        intent.unlink()
        fsync_directory(SECRET_ROOT)
        subprocess.run(
            ["/usr/bin/true", "admin.complete-backup-authority-key-v2.intent-removed"],
            check=True,
        )
    subprocess.run(
        ["/usr/bin/true", "admin.complete-backup-authority-key-v2"], check=True
    )


def verify_live_schema(args: list[str]) -> None:
    if len(args) != 4 or args[0] != "--candidate-release-root-fd" or args[2] != "--expected-vps-release-manifest-sha256":
        fail("live-schema verifier argv is not canonical")
    root, manifest, digest = candidate_authority(args)
    del root
    validate_fence()
    admission = (RUNTIME_FENCE / "db-admission.lock").open("rb")
    quiescence = (RUNTIME_FENCE / "db-quiescence.lock").open("rb")
    fcntl.flock(admission, fcntl.LOCK_EX)
    fcntl.flock(quiescence, fcntl.LOCK_EX)
    database = STATE_ROOT / "database/highscores.sqlite3"
    live_paths = [
        database,
        database.with_name(database.name + "-wal"),
        database.with_name(database.name + "-shm"),
    ]
    before = [file_fingerprint(path) for path in live_paths]
    with tempfile.TemporaryDirectory(prefix="robin-live-schema-") as temporary:
        copied_database = Path(temporary) / database.name
        shutil.copyfile(database, copied_database)
        if live_paths[1].exists():
            shutil.copyfile(live_paths[1], copied_database.with_name(database.name + "-wal"))
        connection = sqlite3.connect(f"file:{copied_database}?mode=ro", uri=True)
        try:
            row = connection.execute(
                "SELECT max(version) FROM _sqlx_migrations WHERE success = 1"
            ).fetchone()
        finally:
            connection.close()
    if [file_fingerprint(path) for path in live_paths] != before:
        fail("live database bytes or metadata changed during schema verification")
    if row is None or not isinstance(row[0], int) or row[0] < 2:
        fail("live database has no supported schema")
    subprocess.run(["/usr/bin/true", "admin.verify-live-database-schema-v2"], check=True)
    emit(
        {
            "database_schema_version": row[0],
            "schema_version": 2,
            "source_commit": manifest["source_commit"],
            "vps_release_manifest_sha256": digest,
        }
    )
    fcntl.flock(quiescence, fcntl.LOCK_UN)
    fcntl.flock(admission, fcntl.LOCK_UN)


def migrate_database(args: list[str]) -> None:
    if len(args) != 3 or args[0] != "--config" or args[2] != "migrate":
        fail("migration argv is not canonical")
    config = Path(args[1])
    if config.name != "highscores-server.toml" or not config.is_file():
        fail("migration config authority is invalid")
    database = STATE_ROOT / "database/highscores.sqlite3"
    connection = sqlite3.connect(database)
    try:
        connection.execute(
            "CREATE TABLE IF NOT EXISTS _sqlx_migrations ("
            "version INTEGER PRIMARY KEY, success BOOLEAN NOT NULL)"
        )
        connection.execute(
            "INSERT OR REPLACE INTO _sqlx_migrations(version, success) VALUES(2, 1)"
        )
        connection.commit()
    finally:
        connection.close()
    (STATE_ROOT / "database").chmod(0o2770)
    subprocess.run(["/usr/bin/true", "admin.migrate"], check=True)


def estimate_backup_space(args: list[str]) -> None:
    if len(args) < 10 or args[0] != "--config" or args[2] != "estimate-backup-space":
        fail("backup-space estimate argv is not canonical")
    config = Path(args[1])
    if config.name != "highscores-server.toml" or not config.is_file():
        fail("backup-space estimate config authority is invalid")
    if "/proc/self/fd/" in str(config):
        fail("backup-space estimate reopened the installed release through procfs")
    options = args[3:]
    expected_paths = [
        *(str(SECRET_ROOT / name) for name in EXPECTED_SECRETS),
        *(
            f"/home/robinhood/.config/systemd/user/{name}"
            for name in (
                "robin-highscores.target",
                "robin-highscores-api.service",
                "robin-highscores-worker.service",
                "robin-highscores-backup.service",
                "robin-highscores-backup.timer",
            )
        ),
    ]
    expected_options = [
        "--release-manifest-path",
        str(config.parent.parent / RELEASE_MANIFEST),
        "--backup-root",
        str(STATE_ROOT / "backups"),
        "--status-path",
        str(STATE_ROOT / "status/backup-status.json"),
        "--require-available",
    ]
    for path in expected_paths:
        expected_options.extend(("--restore-source-map", f"{path}={path}"))
    if options != expected_options:
        fail("backup-space estimate argv differs from the frozen nine-map contract")
    (STATE_ROOT / "replays").chmod(0o2770)
    (STATE_ROOT / "campaign-states").chmod(0o2770)
    available = int(os.environ.get("TX_AVAILABLE_BYTES", "17179869184"))
    required = 4096
    if available < required:
        fail("insufficient typed backup capacity")
    subprocess.run(["/usr/bin/true", "admin.estimate-backup-space"], check=True)
    emit(
        {
            "observed_available_bytes": available,
            "required_available_bytes": required,
            "restore_source_map_count": len(expected_paths),
            "schema_version": 1,
        }
    )


def verify_transaction_backup(args: list[str]) -> None:
    expected_names = (
        "--backup-root-fd",
        "--status-envelope-fd",
        "--backup-authority-key-fd",
        "--expected-release-manifest-fd",
        "--expected-source-commit",
        "--expected-vps-release-manifest-sha256",
        "--expected-publication-lock-sha256",
    )
    if len(args) != 14 or tuple(args[::2]) != expected_names:
        fail("transaction backup verifier argv is not canonical")
    values = dict(zip(args[::2], args[1::2], strict=True))

    def inherited(name: str, directory: bool) -> tuple[Path, os.stat_result]:
        value = values[name]
        if not value.isdigit() or int(value) < 3:
            fail(f"invalid inherited descriptor: {name}")
        target = Path(os.readlink(f"/proc/self/fd/{value}"))
        metadata = os.fstat(int(value))
        expected_type = stat.S_ISDIR if directory else stat.S_ISREG
        if not expected_type(metadata.st_mode) or metadata.st_uid != os.getuid():
            fail(f"unsafe inherited descriptor: {name}")
        return target, metadata

    backup_root, backup_root_metadata = inherited("--backup-root-fd", True)
    status_path, status_metadata = inherited("--status-envelope-fd", False)
    key_path, key_metadata = inherited("--backup-authority-key-fd", False)
    release_path, release_metadata = inherited("--expected-release-manifest-fd", False)
    if (
        backup_root != STATE_ROOT / "backups"
        or stat.S_IMODE(backup_root_metadata.st_mode) != 0o700
        or status_path != STATE_ROOT / "status/backup-status.json"
        or stat.S_IMODE(status_metadata.st_mode) != 0o400
        or status_metadata.st_nlink != 1
        or key_path != SECRET_ROOT / "backup-authority-hmac.key"
        or stat.S_IMODE(key_metadata.st_mode) != 0o400
        or key_metadata.st_nlink != 1
        or key_metadata.st_size != 32
        or release_path.name != RELEASE_MANIFEST
        or stat.S_IMODE(release_metadata.st_mode) != 0o440
        or release_metadata.st_nlink != 1
    ):
        fail("transaction backup descriptor topology is inexact")

    release_bytes = release_path.read_bytes()
    vps_sha = hashlib.sha256(release_bytes).hexdigest()
    release = json.loads(release_bytes)
    source_commit = values["--expected-source-commit"]
    publication_sha = values["--expected-publication-lock-sha256"]
    if (
        source_commit != release.get("source_commit")
        or values["--expected-vps-release-manifest-sha256"] != vps_sha
        or publication_sha != release.get("publication_lock_sha256")
    ):
        fail("transaction backup out-of-band release identity mismatch")

    status_bytes = status_path.read_bytes()
    status = json.loads(status_bytes)
    if json.dumps(status, sort_keys=True, separators=(",", ":")).encode() != status_bytes:
        fail("transaction backup status is not canonical JSON")
    status_keys = {
        "backup_directory",
        "backup_id",
        "backup_manifest_sha256",
        "created_at_unix_ms",
        "database_schema_version",
        "directory_count",
        "file_count",
        "hmac_sha256",
        "release_identity",
        "schema_version",
        "total_bytes",
    }
    backup_id = status.get("backup_id")
    backup_id_parts = str(backup_id).split("-")
    if (
        set(status) != status_keys
        or status.get("schema_version") != 4
        or len(backup_id_parts) != 4
        or backup_id_parts[:2] != ["backup", "v4"]
        or not backup_id_parts[2].isdigit()
        or int(backup_id_parts[2]) != status.get("created_at_unix_ms")
        or len(backup_id_parts[3]) != 32
        or any(character not in "0123456789abcdef" for character in backup_id_parts[3])
    ):
        fail("transaction BackupStatusV4 shape or backup ID is inexact")
    key = key_path.read_bytes()
    unsigned = dict(status)
    claimed_hmac = unsigned.pop("hmac_sha256", "")
    signing = json.dumps(unsigned, sort_keys=True, separators=(",", ":")).encode()
    actual_hmac = hmac.new(
        key,
        b"robinhood/highscores/backup-status/4\0" + signing,
        hashlib.sha256,
    ).hexdigest()
    identity = status.get("release_identity")
    unit_names = (
        "robin-highscores-api.service",
        "robin-highscores-backup.service",
        "robin-highscores-backup.timer",
        "robin-highscores-worker.service",
        "robin-highscores.target",
    )
    expected_units = []
    release_root = release_path.parent
    for unit_name in unit_names:
        unit_bytes = (release_root / "systemd/user" / unit_name).read_bytes()
        expected_units.append(
            {
                "artifact": {
                    "byte_length": len(unit_bytes),
                    "media_type": "text/plain; charset=utf-8",
                    "sha256": hashlib.sha256(unit_bytes).hexdigest(),
                },
                "release_relative_path": f"systemd/user/{unit_name}",
                "unix_mode": 0o440,
            }
        )
    if (
        not hmac.compare_digest(claimed_hmac, actual_hmac)
        or not isinstance(identity, dict)
        or set(identity)
        != {
            "database_schema_version",
            "installed_user_units",
            "publication_lock_sha256",
            "source_commit",
            "vps_release_manifest_sha256",
        }
        or identity.get("source_commit") != source_commit
        or identity.get("vps_release_manifest_sha256") != vps_sha
        or identity.get("publication_lock_sha256") != publication_sha
        or identity.get("database_schema_version")
        != release.get("database_schema_version")
        or identity.get("installed_user_units") != expected_units
    ):
        fail("transaction backup status authentication or release identity failed")
    backup_directory = backup_root / str(backup_id)
    if status.get("backup_directory") != str(backup_directory) or not backup_directory.is_dir():
        fail("transaction status does not select the exact backup child")
    manifest = backup_directory / "backup-manifest.json"
    envelope = backup_directory / "backup-verification-envelope.json"
    manifest_bytes = manifest.read_bytes()
    envelope_bytes = envelope.read_bytes()
    if hashlib.sha256(manifest_bytes).hexdigest() != status.get("backup_manifest_sha256"):
        fail("transaction backup manifest differs from authenticated status")
    if (
        json.dumps(
            json.loads(manifest_bytes), sort_keys=True, separators=(",", ":")
        ).encode()
        != manifest_bytes
    ):
        fail("transaction backup manifest is not canonical JSON")
    manifest_document = json.loads(manifest_bytes)
    expected_directories = [
        {"relative_path": relative, "unix_mode": 0o700}
        for relative in (
            "campaigns",
            "replays",
            "restore",
            "restore/state",
            "restore/systemd",
            "restore/systemd/user",
        )
    ]
    state_root = "/home/robinhood/.local/share/robin-highscores"
    unit_root = "/home/robinhood/.config/systemd/user"
    expected_sources = sorted(
        [
            (f"{state_root}/database/highscores.sqlite3", "highscores.sqlite3"),
            (f"{state_root}/replays", "replays"),
            (f"{state_root}/campaign-states", "campaigns"),
            *(
                (f"{state_root}/api-secrets/{name}", f"restore/state/{name}")
                for name in EXPECTED_SECRETS
            ),
            *(
                (f"{unit_root}/{name}", f"restore/systemd/user/{name}")
                for name in unit_names
            ),
        ],
        key=lambda item: item[1],
    )
    if (
        set(manifest_document)
        != {
            "created_at_unix_ms",
            "database_schema_version",
            "directories",
            "files",
            "release_identity",
            "restore_sources",
            "root_unix_mode",
            "schema_version",
        }
        or manifest_document["schema_version"] != 4
        or manifest_document["root_unix_mode"] != 0o700
        or manifest_document["release_identity"] != identity
        or manifest_document["created_at_unix_ms"] != status["created_at_unix_ms"]
        or manifest_document["database_schema_version"]
        != status["database_schema_version"]
        or manifest_document["directories"] != expected_directories
        or manifest_document["restore_sources"]
        != [
            {
                "archive_relative_path": archive,
                "original_absolute_path": original,
            }
            for original, archive in expected_sources
        ]
    ):
        fail("transaction BackupV4 manifest projection is inexact")
    files = manifest_document.get("files")
    if (
        not isinstance(files, list)
        or len(files) != 10
        or [file.get("relative_path") for file in files]
        != sorted(file.get("relative_path") for file in files)
    ):
        fail("transaction BackupV4 file closure is inexact")
    for file in files:
        payload_path = backup_directory / file["relative_path"]
        payload = payload_path.read_bytes()
        if (
            len(payload) != file.get("byte_length")
            or hashlib.sha256(payload).hexdigest() != file.get("sha256")
        ):
            fail("transaction BackupV4 payload differs from its manifest")

    envelope_document = json.loads(envelope_bytes)
    if (
        json.dumps(
            envelope_document, sort_keys=True, separators=(",", ":")
        ).encode()
        != envelope_bytes
    ):
        fail("transaction backup verification envelope is not canonical JSON")
    envelope_unsigned = dict(envelope_document)
    envelope_hmac = envelope_unsigned.pop("hmac_sha256", "")
    expected_envelope_hmac = hmac.new(
        key,
        b"robinhood/highscores/backup-verification-envelope/2\0"
        + json.dumps(
            envelope_unsigned, sort_keys=True, separators=(",", ":")
        ).encode(),
        hashlib.sha256,
    ).hexdigest()
    expected_summary = {
        "backup_id": backup_id,
        "backup_manifest_sha256": status["backup_manifest_sha256"],
        "created_at_unix_ms": status["created_at_unix_ms"],
        "database_schema_version": status["database_schema_version"],
        "directory_count": len(expected_directories) + 1,
        "file_count": len(files),
        "release_identity": identity,
        "result": "verified",
        "schema_version": 2,
        "total_bytes": sum(file["byte_length"] for file in files),
    }
    if (
        not hmac.compare_digest(envelope_hmac, expected_envelope_hmac)
        or envelope_unsigned != expected_summary
        or status["directory_count"] != expected_summary["directory_count"]
        or status["file_count"] != expected_summary["file_count"]
        or status["total_bytes"] != expected_summary["total_bytes"]
    ):
        fail("transaction BackupVerificationEnvelopeV2 is inexact")

    subprocess.run(["/usr/bin/true", "admin.verify-transaction-backup"], check=True)
    emit(
        {
            "backup_directory": str(backup_directory),
            "backup_id": backup_id,
            "backup_manifest_sha256": status["backup_manifest_sha256"],
            "current_status": {
                "byte_length": len(status_bytes),
                "sha256": hashlib.sha256(status_bytes).hexdigest(),
            },
            "database_schema_version": status["database_schema_version"],
            "directory_count": status["directory_count"],
            "file_count": status["file_count"],
            "release_identity": identity,
            "schema_version": 2,
            "total_bytes": status["total_bytes"],
            "verification_envelope_byte_length": len(envelope_bytes),
            "verification_envelope_sha256": hashlib.sha256(envelope_bytes).hexdigest(),
        }
    )


def main() -> None:
    args = sys.argv[1:]
    if not args:
        fail("missing command")
    command = args.pop(0)
    if command == "probe-runtime-authority-v2":
        probe_runtime_authority(args)
    elif command == "verify-live-database-schema-v2":
        verify_live_schema(args)
    elif command == "--config" and len(args) >= 2 and args[1] == "initialize-backup-authority-key-v2":
        initialize_backup_authority_v2([command, *args])
    elif command == "--config" and len(args) >= 2 and args[1] == "complete-backup-authority-key-v2":
        complete_backup_authority_v2([command, *args])
    elif command == "--config" and args and args[-1] == "migrate":
        migrate_database([command, *args])
    elif command == "--config" and len(args) >= 2 and args[1] == "estimate-backup-space":
        estimate_backup_space([command, *args])
    elif command == "verify-transaction-backup":
        verify_transaction_backup(args)
    else:
        fail("unexpected candidate-admin command")


if __name__ == "__main__":
    main()
'''
UNITS = (
    "robin-highscores.target",
    "robin-highscores-api.service",
    "robin-highscores-worker.service",
    "robin-highscores-backup.service",
    "robin-highscores-backup.timer",
)

MANIFEST_TOOL = r'''#!/bin/sh
set -eu
PATH=/usr/bin:/bin
export PATH

fail() { echo "manifest-tool harness: $*" >&2; exit 65; }
valid_fd() {
  case "$1" in /proc/self/fd/[0-9]*) [ "${1#/proc/self/fd/}" -ge 3 ] 2>/dev/null ;; *) return 1 ;; esac
}
exact_file_fd() {
  valid_fd "$1" && [ -f "$1" ] && [ "$(stat -Lc %u -- "$1")" -eq "$(id -u)" ] &&
    [ "$(stat -Lc %h -- "$1")" -eq 1 ] && [ "$(stat -Lc %a -- "$1")" = "$2" ]
}
valid_digest() {
  case "$1" in *[!0-9a-f]*|'') return 1 ;; esac
  [ "${#1}" -eq 64 ]
}
lock_authority() {
  lock_number=$1
  case "$lock_number" in ''|*[!0-9]*) return 1 ;; esac
  lock_fd=/proc/self/fd/$lock_number
  exact_file_fd "$lock_fd" 600 || return 1
  [ "$(stat -Lc %d:%i -- "$lock_fd")" = "$(stat -c %d:%i -- /home/robinhood/.local/opt/robin-highscores/activation.lock)" ] || return 1
  # A duplicate of the wrapper's already-locked OFD succeeds. A separately
  # reopened descriptor for the same inode conflicts and therefore fails.
  /usr/bin/flock -n "$lock_number"
}

case "${1:-}" in
  exec-vps-activation-v2)
    shift
    operation=${1:-}
    shift || :
    case "$operation" in
      deploy)
        [ "$#" -ge 8 ] || fail "short deploy outer argv"
        script_fd=$1
        manifest_fd=$2
        validator_fd=$3
        manifest_tool_fd=$4
        plan_fd=$5
        expected_plan_sha=$6
        expected_vps_sha=$7
        shift 7
        [ "$1" = -- ] || fail "deploy outer argv omits delimiter"
        shift
        [ "$#" -eq 4 ] || [ "$#" -eq 5 ] || fail "invalid deploy business argv"
        case "${1:-}" in
          --resume-installed)
            [ "$#" -eq 5 ] || fail "invalid resume business argv"
            candidate=$2; commit=$3; bootstrap_sha=$5
            expected_candidate=/home/robinhood/.local/opt/robin-highscores/releases/$commit
            ;;
          *)
            [ "$#" -eq 4 ] || fail "invalid candidate business argv"
            candidate=$1; commit=$2; bootstrap_sha=$4
            expected_candidate=/home/robinhood/.local/opt/robin-highscores/releases/$commit.partial
            ;;
        esac
        [ "$candidate" = "$expected_candidate" ] || fail "candidate path is not canonical"
        case "$commit" in *[!0-9a-f]*|'') fail "invalid source commit" ;; esac
        [ "${#commit}" -eq 40 ] || fail "invalid source commit length"
        exact_file_fd "$script_fd" 500 || fail "unsafe deploy descriptor"
        exact_file_fd "$manifest_fd" 400 || fail "unsafe bootstrap descriptor"
        exact_file_fd "$validator_fd" 500 || fail "unsafe validator descriptor"
        exact_file_fd "$manifest_tool_fd" 550 || fail "unsafe manifest-tool descriptor"
        exact_file_fd "$plan_fd" 400 || fail "unsafe plan descriptor"
        [ "$(printf '%s\n' "$script_fd" "$manifest_fd" "$validator_fd" "$manifest_tool_fd" "$plan_fd" | sort -u | wc -l)" -eq 5 ] || fail "deploy descriptors are not distinct"
        [ "$(stat -Lc %d:%i -- "$0")" = "$(stat -Lc %d:%i -- "$manifest_tool_fd")" ] || fail "manifest tool failed self-attestation"
        valid_digest "$expected_plan_sha" && valid_digest "$expected_vps_sha" && valid_digest "$bootstrap_sha" || fail "invalid out-of-band digest"
        [ "$(sha256sum "$plan_fd" | cut -d' ' -f1)" = "$expected_plan_sha" ] || fail "plan differs from OOB digest"
        [ -d "$candidate" ] && [ ! -L "$candidate" ] && [ "$(stat -c %u -- "$candidate")" -eq "$(id -u)" ] && [ "$(stat -c %a -- "$candidate")" = 550 ] || fail "unsafe candidate root"
        [ "$(sha256sum "$candidate/vps-release-manifest-v2.json" | cut -d' ' -f1)" = "$expected_vps_sha" ] || fail "candidate differs from OOB V2 digest"
        [ ! -e "$candidate/vps-release-manifest-v1.json" ] && [ ! -L "$candidate/vps-release-manifest-v1.json" ] || fail "candidate carries legacy V1 authority"
        case "$(cat -- "$candidate/vps-release-manifest-v2.json")" in
          *'"schema_version":2'*'"database_schema_version":2'*) ;;
          *) fail "candidate V2 manifest schema is unsupported" ;;
        esac
        grep -Fq '"source_commit":"'"$commit"'"' "$plan_fd" || fail "plan source commit mismatch"
        grep -Fq '"publication_v3":"/home/robinhood/.local/opt/robin-highscores/incoming/.sources-'"$commit"'/publication-v3"' "$plan_fd" || fail "plan source closure mismatch"

        # Pin the candidate before the lock and retain this exact directory
        # descriptor through partial->final promotion in the release root.
        exec 9<"$candidate"
        [ "$(stat -Lc %d:%i -- /proc/self/fd/9)" = "$(stat -c %d:%i -- "$candidate")" ] || fail "candidate pin mismatch"
        ;;
      rollback)
        [ "$#" -ge 5 ] || fail "short rollback outer argv"
        script_fd=$1
        manifest_fd=$2
        validator_fd=$3
        manifest_tool_fd=$4
        shift 4
        [ "$1" = -- ] || fail "rollback outer argv omits delimiter"
        shift
        [ "$#" -eq 3 ] || [ "$#" -eq 4 ] || fail "invalid rollback business argv"
        exact_file_fd "$script_fd" 500 || fail "unsafe rollback descriptor"
        exact_file_fd "$manifest_fd" 400 || fail "unsafe bootstrap descriptor"
        exact_file_fd "$validator_fd" 500 || fail "unsafe validator descriptor"
        exact_file_fd "$manifest_tool_fd" 550 || fail "unsafe manifest-tool descriptor"
        [ "$(printf '%s\n' "$script_fd" "$manifest_fd" "$validator_fd" "$manifest_tool_fd" | sort -u | wc -l)" -eq 4 ] || fail "rollback descriptors are not distinct"
        [ "$(stat -Lc %d:%i -- "$0")" = "$(stat -Lc %d:%i -- "$manifest_tool_fd")" ] || fail "manifest tool failed self-attestation"
        case " $* " in *' /proc/self/fd/'*|*' .sources-'*) fail "rollback admitted deploy source authority" ;; esac
        ;;
      *) fail "unknown activation operation" ;;
    esac

    activation_lock=/home/robinhood/.local/opt/robin-highscores/activation.lock
    umask 077
    if [ ! -e "$activation_lock" ] && [ ! -L "$activation_lock" ]; then
      : >"$activation_lock"
      /usr/bin/sync -f /home/robinhood/.local/opt/robin-highscores
    fi
    [ -f "$activation_lock" ] && [ ! -L "$activation_lock" ] &&
      [ "$(stat -c %u -- "$activation_lock")" -eq "$(id -u)" ] &&
      [ "$(stat -c %h -- "$activation_lock")" -eq 1 ] &&
      [ "$(stat -c %a -- "$activation_lock")" = 600 ] || fail "unsafe activation lock"
    exec 8<>"$activation_lock"
    /usr/bin/flock -n 8 || fail "activation lock is held"
    if [ "$operation" = deploy ]; then
      exec "$script_fd" "$@" "$manifest_fd" "$validator_fd" "$manifest_tool_fd" \
        "$plan_fd" /proc/self/fd/9 /proc/self/fd/8 "$expected_plan_sha" "$expected_vps_sha"
    fi
    exec "$script_fd" "$@" "$manifest_fd" "$validator_fd" "$manifest_tool_fd" /proc/self/fd/8
    ;;

  validate-vps-release-v2)
    [ "$#" -eq 2 ] || fail "invalid V2 validator argv"
    release_root=$2
    case "$release_root" in /proc/self/fd/[0-9]*/.) ;; *) fail "V2 validator requires a pinned release root" ;; esac
    [ -d "$release_root" ] && [ ! -L "$release_root" ] &&
      [ "$(stat -c %u -- "$release_root")" -eq "$(id -u)" ] &&
      [ "$(stat -c %a -- "$release_root")" = 550 ] || fail "unsafe pinned V2 release root"
    release_manifest=$release_root/vps-release-manifest-v2.json
    [ -f "$release_manifest" ] && [ ! -L "$release_manifest" ] &&
      [ "$(stat -c %u -- "$release_manifest")" -eq "$(id -u)" ] &&
      [ "$(stat -c %h -- "$release_manifest")" -eq 1 ] &&
      [ "$(stat -c %a -- "$release_manifest")" = 440 ] || fail "unsafe pinned V2 release manifest"
    [ ! -e "$release_root/vps-release-manifest-v1.json" ] &&
      [ ! -L "$release_root/vps-release-manifest-v1.json" ] || fail "legacy V1 release authority exists"
    sha256sum "$release_manifest" | cut -d' ' -f1
    ;;

  project-vps-publication-lock-v2)
    [ "$#" -eq 5 ] && [ "$2" = --release-manifest-fd ] && [ "$4" = --expected-vps-release-manifest-sha256 ] || fail "invalid publication projection argv"
    release_number=$3; expected_vps_sha=$5
    release_fd=/proc/self/fd/$release_number
    exact_file_fd "$release_fd" 440 && valid_digest "$expected_vps_sha" || fail "publication projection authority failed"
    [ "$(sha256sum "$release_fd" | cut -d' ' -f1)" = "$expected_vps_sha" ] || fail "publication projection V2 digest mismatch"
    publication_sha=$(sed -n 's/.*"publication_lock_sha256":"\([0-9a-f]*\)".*/\1/p' "$release_fd")
    valid_digest "$publication_sha" || fail "release manifest omits publication lock identity"
    printf '%s\n' "$publication_sha"
    ;;

  initialize-vps-runtime-fence-v1)
    [ "$#" -eq 4 ] && [ "$3" = --activation-lock-fd ] || fail "invalid runtime-fence initializer argv"
    commit=$2; lock_number=$4
    case "$commit" in *[!0-9a-f]*|'') fail "invalid runtime-fence source commit" ;; esac
    [ "${#commit}" -eq 40 ] && lock_authority "$lock_number" || fail "runtime-fence initializer authority failed"
    state=/home/robinhood/.local/share/robin-highscores
    final=$state/runtime-fence
    staging=$state/.runtime-fence-$commit.partial
    [ ! -e "$final" ] || [ ! -e "$staging" ] || fail "ambiguous runtime-fence initializer state"
    if [ ! -e "$final" ]; then
      if [ ! -e "$staging" ]; then
        /usr/bin/mkdir -m 0700 -- "$staging"
        /usr/bin/sync -f -- "$state"
      fi
      [ -d "$staging" ] && [ ! -L "$staging" ] && [ "$(stat -c %u -- "$staging")" -eq "$(id -u)" ] && [ "$(stat -c %a -- "$staging")" = 700 ] || fail "unsafe runtime-fence staging root"
      for leaf in db-admission.lock db-quiescence.lock; do
        path=$staging/$leaf
        if [ ! -e "$path" ] && [ ! -L "$path" ]; then
          : >"$path"
          /usr/bin/chmod 0400 -- "$path"
          /usr/bin/sync -f -- "$path"
        fi
        [ -f "$path" ] && [ ! -L "$path" ] && [ "$(stat -c %u -- "$path")" -eq "$(id -u)" ] && [ "$(stat -c %h -- "$path")" -eq 1 ] && [ "$(stat -c %a -- "$path")" = 400 ] && [ "$(stat -c %s -- "$path")" -eq 0 ] || fail "unsafe runtime-fence leaf"
      done
      [ "$(find "$staging" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)" = "$(printf '%s\n' db-admission.lock db-quiescence.lock | sort)" ] || fail "runtime-fence staging inventory is inexact"
      /usr/bin/chmod 0500 -- "$staging"
      /usr/bin/sync -f -- "$staging"
      /usr/bin/mv -T --no-clobber -- "$staging" "$final"
      /usr/bin/sync -f -- "$state"
    fi
    [ -d "$final" ] && [ ! -L "$final" ] && [ "$(stat -c %u -- "$final")" -eq "$(id -u)" ] && [ "$(stat -c %a -- "$final")" = 500 ] || fail "runtime-fence final root is inexact"
    [ "$(find "$final" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)" = "$(printf '%s\n' db-admission.lock db-quiescence.lock | sort)" ] || fail "runtime-fence final inventory is inexact"
    for leaf in db-admission.lock db-quiescence.lock; do
      path=$final/$leaf
      [ -f "$path" ] && [ ! -L "$path" ] && [ "$(stat -c %u -- "$path")" -eq "$(id -u)" ] && [ "$(stat -c %h -- "$path")" -eq 1 ] && [ "$(stat -c %a -- "$path")" = 400 ] && [ "$(stat -c %s -- "$path")" -eq 0 ] || fail "runtime-fence final leaf is inexact"
    done
    ;;

  promote-inherited-vps-release-v2)
    [ "$#" -eq 6 ] && [ "$3" = --candidate-root-fd ] && [ "$5" = --activation-lock-fd ] || fail "invalid inherited promotion argv"
    expected_vps_sha=$2; candidate_number=$4; lock_number=$6
    valid_digest "$expected_vps_sha" && lock_authority "$lock_number" || fail "promotion authority failed"
    candidate_fd=/proc/self/fd/$candidate_number
    [ -d "$candidate_fd" ] && [ "$(stat -Lc %a -- "$candidate_fd")" = 550 ] || fail "invalid inherited candidate"
    [ "$(sha256sum "$candidate_fd/vps-release-manifest-v2.json" | cut -d' ' -f1)" = "$expected_vps_sha" ] || fail "promotion candidate digest mismatch"
    commit=$(cat "$candidate_fd/SOURCE_COMMIT")
    source=$(readlink "$candidate_fd")
    destination=/home/robinhood/.local/opt/robin-highscores/releases/$commit
    case "$source" in /home/robinhood/.local/opt/robin-highscores/releases/"$commit".partial) ;; *) fail "candidate is not the release-partial inode" ;; esac
    /usr/bin/mv -T --no-clobber -- "$source" "$destination"
    [ "$(stat -Lc %d:%i -- "$candidate_fd")" = "$(stat -c %d:%i -- "$destination")" ] || fail "promotion changed candidate inode"
    printf '%s\n' "$expected_vps_sha"
    ;;

  consume-vps-sources-v2)
    [ "$#" -eq 8 ] && [ "$5" = --candidate-root-fd ] && [ "$7" = --activation-lock-fd ] || fail "invalid source-consume argv"
    plan_fd=$2; expected_plan_sha=$3; expected_vps_sha=$4; candidate_number=$6; lock_number=$8
    exact_file_fd "$plan_fd" 400 && valid_digest "$expected_plan_sha" && valid_digest "$expected_vps_sha" && lock_authority "$lock_number" || fail "source-consume authority failed"
    [ "$(sha256sum "$plan_fd" | cut -d' ' -f1)" = "$expected_plan_sha" ] || fail "source-consume plan mismatch"
    candidate_fd=/proc/self/fd/$candidate_number
    [ -d "$candidate_fd" ] && [ "$(sha256sum "$candidate_fd/vps-release-manifest-v2.json" | cut -d' ' -f1)" = "$expected_vps_sha" ] || fail "source-consume candidate mismatch"
    commit=$(cat "$candidate_fd/SOURCE_COMMIT")
    source=/home/robinhood/.local/opt/robin-highscores/incoming/.sources-$commit
    consuming=/home/robinhood/.local/opt/robin-highscores/incoming/.sources-$commit.consuming
    [ ! -e "$source" ] || [ ! -e "$consuming" ] || fail "ambiguous source roots"
    if [ -d "$source" ]; then
      [ "$(stat -c %a -- "$source")" = 700 ] && [ "$(stat -c %u -- "$source")" -eq "$(id -u)" ] || fail "unsafe source root"
      [ "$(sha256sum "$source/vps-release-plan-v2.json" | cut -d' ' -f1)" = "$expected_plan_sha" ] || fail "source plan mismatch"
      [ "$(sha256sum "$source/publication-v3/publication-lock-v3.json" | cut -d' ' -f1)" = "$(cat "$candidate_fd/publication/publication-lock-v3.sha256")" ] || fail "source publication mismatch"
      /usr/bin/mv -T -- "$source" "$consuming"
      /usr/bin/sync -f /home/robinhood/.local/opt/robin-highscores/incoming
    fi
    if [ -d "$consuming" ]; then
      [ "$(sha256sum "$consuming/vps-release-plan-v2.json" | cut -d' ' -f1)" = "$expected_plan_sha" ] || fail "quarantined source plan mismatch"
      /usr/bin/rm -rf -- "$consuming"
      /usr/bin/sync -f /home/robinhood/.local/opt/robin-highscores/incoming
    fi
    [ ! -e "$source" ] && [ ! -e "$consuming" ] || fail "source consume did not finish"
    printf '%s\n' "$expected_vps_sha"
    ;;
  *) fail "unexpected command: ${1:-missing}" ;;
esac
'''

DEPLOY_BOOTSTRAP_COMMAND = r'''
set -euo pipefail
bootstrap=$1
candidate=$2
plan=$3
commit=$4
sums_sha=$5
bootstrap_sha=$6
manifestctl=$7
manifestctl_sha=$8
plan_sha=$9
vps_sha=${10}
exec 3<"$bootstrap/DEPLOY_BOOTSTRAP_SHA256SUMS"
exec 4<"$bootstrap/deploy-release.sh"
exec 5<"$bootstrap/validate-release-bundle.sh"
exec 6<"$manifestctl"
exec 7<"$plan"
[ "$(sha256sum /proc/self/fd/3 | cut -d" " -f1)" = "$bootstrap_sha" ]
[ "$(wc -l < /proc/self/fd/3 | tr -d " ")" -eq 3 ]
deploy_sha=$(awk '$2 == "deploy-release.sh" { print $1 }' /proc/self/fd/3)
validator_sha=$(awk '$2 == "validate-release-bundle.sh" { print $1 }' /proc/self/fd/3)
[ "$(awk '$2 == "deploy-release.sh" { count += 1 } END { print count + 0 }' /proc/self/fd/3)" -eq 1 ]
[ "$(awk '$2 == "rollback-release.sh" { count += 1 } END { print count + 0 }' /proc/self/fd/3)" -eq 1 ]
[ "$(awk '$2 == "validate-release-bundle.sh" { count += 1 } END { print count + 0 }' /proc/self/fd/3)" -eq 1 ]
[ "$(sha256sum /proc/self/fd/4 | cut -d" " -f1)" = "$deploy_sha" ]
[ "$(sha256sum /proc/self/fd/5 | cut -d" " -f1)" = "$validator_sha" ]
[ -f /proc/self/fd/6 ] && [ "$(stat -Lc %U /proc/self/fd/6)" = robinhood ]
[ "$(stat -Lc %h /proc/self/fd/6)" -eq 1 ] && [ "$(stat -Lc %a /proc/self/fd/6)" = 550 ]
[ "$(sha256sum /proc/self/fd/6 | cut -d" " -f1)" = "$manifestctl_sha" ]
[ -f /proc/self/fd/7 ] && [ "$(stat -Lc %U /proc/self/fd/7)" = robinhood ]
[ "$(stat -Lc %h /proc/self/fd/7)" -eq 1 ] && [ "$(stat -Lc %a /proc/self/fd/7)" = 400 ]
[ "$(sha256sum /proc/self/fd/7 | cut -d" " -f1)" = "$plan_sha" ]
[ "$(sha256sum "$candidate/vps-release-manifest-v2.json" | cut -d" " -f1)" = "$vps_sha" ]
exec /proc/self/fd/6 exec-vps-activation-v2 deploy \
  /proc/self/fd/4 /proc/self/fd/3 /proc/self/fd/5 /proc/self/fd/6 \
  /proc/self/fd/7 "$plan_sha" "$vps_sha" -- \
  "$candidate" "$commit" "$sums_sha" "$bootstrap_sha"
'''

RESUME_BOOTSTRAP_COMMAND = DEPLOY_BOOTSTRAP_COMMAND.replace(
    '  "$candidate" "$commit"',
    '  --resume-installed "$candidate" "$commit"',
)

ROLLBACK_BOOTSTRAP_COMMAND = r'''
set -euo pipefail
bootstrap=$1
target=$2
sums_sha=$3
bootstrap_sha=$4
manifestctl=$5
manifestctl_sha=$6
exec 3<"$bootstrap/DEPLOY_BOOTSTRAP_SHA256SUMS"
exec 4<"$bootstrap/rollback-release.sh"
exec 5<"$bootstrap/validate-release-bundle.sh"
exec 6<"$manifestctl"
[ "$(sha256sum /proc/self/fd/3 | cut -d" " -f1)" = "$bootstrap_sha" ]
rollback_sha=$(awk '$2 == "rollback-release.sh" { print $1 }' /proc/self/fd/3)
validator_sha=$(awk '$2 == "validate-release-bundle.sh" { print $1 }' /proc/self/fd/3)
[ "$(wc -l < /proc/self/fd/3 | tr -d " ")" -eq 3 ]
[ "$(awk '$2 == "deploy-release.sh" { count += 1 } END { print count + 0 }' /proc/self/fd/3)" -eq 1 ]
[ "$(awk '$2 == "rollback-release.sh" { count += 1 } END { print count + 0 }' /proc/self/fd/3)" -eq 1 ]
[ "$(awk '$2 == "validate-release-bundle.sh" { count += 1 } END { print count + 0 }' /proc/self/fd/3)" -eq 1 ]
[ "$(sha256sum /proc/self/fd/4 | cut -d" " -f1)" = "$rollback_sha" ]
[ "$(sha256sum /proc/self/fd/5 | cut -d" " -f1)" = "$validator_sha" ]
[ -f /proc/self/fd/6 ] && [ "$(stat -Lc %U /proc/self/fd/6)" = robinhood ]
[ "$(stat -Lc %h /proc/self/fd/6)" -eq 1 ] && [ "$(stat -Lc %a /proc/self/fd/6)" = 550 ]
[ "$(sha256sum /proc/self/fd/6 | cut -d" " -f1)" = "$manifestctl_sha" ]
exec /proc/self/fd/6 exec-vps-activation-v2 rollback \
  /proc/self/fd/4 /proc/self/fd/3 /proc/self/fd/5 /proc/self/fd/6 -- \
  "$target" "$sums_sha" "$bootstrap_sha"
'''

ROLLBACK_RESUME_BOOTSTRAP_COMMAND = ROLLBACK_BOOTSTRAP_COMMAND.replace(
    '  "$target" "$sums_sha"',
    '  --resume-target "$target" "$sums_sha"',
)


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fsync_file(path: Path) -> None:
    with path.open("rb") as stream:
        os.fsync(stream.fileno())


def fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def tree_fingerprint(
    root: Path,
) -> tuple[tuple[str, str, int, int, int, int, int, int, int, str], ...]:
    """Capture security-relevant tree state without following symbolic links."""

    if not root.exists() and not root.is_symlink():
        return ((".", "absent", 0, 0, 0, 0, 0, 0, 0, ""),)
    if root.is_symlink():
        metadata = root.lstat()
        return (
            (
                ".",
                "symlink",
                stat.S_IMODE(metadata.st_mode),
                metadata.st_dev,
                metadata.st_ino,
                metadata.st_nlink,
                metadata.st_size,
                metadata.st_mtime_ns,
                metadata.st_ctime_ns,
                os.readlink(root),
            ),
        )
    entries: list[
        tuple[str, str, int, int, int, int, int, int, int, str]
    ] = []

    def visit(directory: Path) -> None:
        for entry in sorted(os.scandir(directory), key=lambda item: item.name):
            path = Path(entry.path)
            metadata = path.lstat()
            relative = str(path.relative_to(root))
            mode = stat.S_IMODE(metadata.st_mode)
            if stat.S_ISDIR(metadata.st_mode):
                kind = "directory"
                payload = ""
            elif stat.S_ISREG(metadata.st_mode):
                kind = "file"
                try:
                    payload = hashlib.sha256(path.read_bytes()).hexdigest()
                except PermissionError:
                    payload = "permission-denied"
            elif stat.S_ISLNK(metadata.st_mode):
                kind = "symlink"
                payload = os.readlink(path)
            elif stat.S_ISFIFO(metadata.st_mode):
                kind = "fifo"
                payload = ""
            else:
                kind = f"special-{stat.S_IFMT(metadata.st_mode):o}"
                payload = ""
            entries.append(
                (
                    relative,
                    kind,
                    mode,
                    metadata.st_dev,
                    metadata.st_ino,
                    metadata.st_nlink,
                    metadata.st_size,
                    metadata.st_mtime_ns,
                    metadata.st_ctime_ns,
                    payload,
                )
            )
            if kind == "directory":
                visit(path)

    visit(root)
    return tuple(entries)


def selected_boundaries(
    boundaries: tuple[tuple[str, str], ...]
) -> tuple[tuple[str, str], ...]:
    wanted = os.environ.get("ROBIN_TX_BOUNDARY", "")
    if not wanted:
        return boundaries
    return tuple(
        boundary for boundary in boundaries
        if wanted in (boundary[0], ":".join(boundary))
    )


class Result:
    def __init__(self, completed: subprocess.CompletedProcess[str]):
        self.returncode = completed.returncode
        self.stdout = completed.stdout
        self.stderr = completed.stderr

    def describe(self) -> str:
        return (
            f"exit={self.returncode}\n--- stdout ---\n{self.stdout}"
            f"\n--- stderr ---\n{self.stderr}"
        )


class Fixture:
    def __init__(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="robin-deploy-transaction-")
        self.root = Path(self.temporary.name)
        self.home = self.root / "home"
        self.state = self.home / ".tx-state"
        self.opt = self.home / ".local/opt/robin-highscores"
        self.incoming = self.opt / "incoming"
        self.releases = self.opt / "releases"
        self.unit_root = self.home / ".config/systemd/user"
        self.data = self.home / ".local/share/robin-highscores"
        self.bootstrap = self.home / ".robin-highscores-bootstrap"
        self.manifestctl_dir = self.home / ".robin-highscores-manifestctl"
        self.manifestctl = self.manifestctl_dir / "robin-highscores-manifestctl"
        self.plan_root = self.home / ".robin-highscores-plans"
        self.passwd = self.root / "passwd"
        self._provision()

    def cleanup(self) -> None:
        self.temporary.cleanup()

    def _provision(self) -> None:
        for path, mode in (
            (self.incoming, 0o750),
            (self.releases, 0o750),
            (self.data / "database", 0o700),
            (self.data / "replays", 0o700),
            (self.data / "campaign-states", 0o700),
            (self.data / "api-secrets", 0o700),
            (self.data / "raw-content/demo", 0o750),
            (self.data / "raw-content/full", 0o750),
            (self.plan_root, 0o700),
            (self.state, 0o700),
        ):
            path.mkdir(parents=True, exist_ok=True)
            path.chmod(mode)
        self.data.chmod(0o700)
        self.opt.chmod(0o750)
        self.manifestctl_dir.mkdir(mode=0o700)
        self.manifestctl.write_text(MANIFEST_TOOL)
        self.manifestctl.chmod(0o550)
        fsync_file(self.manifestctl)
        self.manifestctl_dir.chmod(0o500)
        fsync_directory(self.manifestctl_dir)
        fsync_directory(self.manifestctl_dir.parent)
        self.manifestctl_digest = digest(self.manifestctl)
        for name, payload in INITIAL_SECRET_BYTES.items():
            path = self.data / "api-secrets" / name
            path.write_bytes(payload)
            path.chmod(0o400)
        for edition in ("demo", "full"):
            edition_root = self.data / "raw-content" / edition
            path = edition_root / "fixture.dat"
            path.write_text(f"{edition} fixture\n")
            path.chmod(0o440)
            edition_root.chmod(0o550)
        (self.data / "raw-content").chmod(0o550)
        uid = os.getuid()
        gid = os.getgid()
        lines = [f"robinhood:x:{uid}:{gid}::/home/robinhood:/bin/sh\n"]
        for line in Path("/etc/passwd").read_text().splitlines(keepends=True):
            fields = line.split(":")
            if len(fields) > 3 and fields[0] != "robinhood" and int(fields[2]) != uid:
                lines.append(line)
        self.passwd.write_text("".join(lines))
        self.passwd.chmod(0o444)
        self._make_bootstrap()

    def _make_bootstrap(self) -> None:
        self.bootstrap.mkdir(mode=0o700)
        for name in ("deploy-release.sh", "rollback-release.sh"):
            shutil.copyfile(SOURCE / name, self.bootstrap / name)
            (self.bootstrap / name).chmod(0o500)
        validator = self.bootstrap / "validate-release-bundle.sh"
        validator.write_text(
            """#!/bin/sh
set -eu
[ "$#" -eq 4 ]
bundle=$1
commit=$2
expected=$3
manifest_tool_fd=$4
[ "$expected" = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee" ]
[ -f "$bundle/SOURCE_COMMIT" ] && [ ! -L "$bundle/SOURCE_COMMIT" ]
[ "$(cat -- "$bundle/SOURCE_COMMIT")" = "$commit" ]
[ -f "$bundle/vps-release-manifest-v2.json" ]
[ ! -e "$bundle/vps-release-manifest-v1.json" ] && [ ! -L "$bundle/vps-release-manifest-v1.json" ]
case "$(cat "$bundle/vps-release-manifest-v2.json")" in
  *'"schema_version":2'*'"database_schema_version":2'*) ;;
  *) exit 65 ;;
esac
[ -f "$bundle/publication/publication-manifest-v3.json" ]
[ -f "$bundle/publication/publication-manifest-v3.sha256" ]
[ -f "$bundle/publication/publication-lock-v3.json" ]
[ -f "$bundle/publication/publication-lock-v3.sha256" ]
[ ! -e "$bundle/publication/publication-manifest-v2.json" ] && [ ! -L "$bundle/publication/publication-manifest-v2.json" ]
[ ! -e "$bundle/publication/publication-lock-v2.json" ] && [ ! -L "$bundle/publication/publication-lock-v2.json" ]
case "$(cat "$bundle/publication/publication-manifest-v3.json")" in
  *'"schema_version":3'*) ;;
  *) exit 65 ;;
esac
case "$(cat "$bundle/publication/publication-lock-v3.json")" in
  *'"schema_version":3'*) ;;
  *) exit 65 ;;
esac
[ "$(sha256sum "$bundle/publication/publication-manifest-v3.json" | cut -d" " -f1)" = "$(cat "$bundle/publication/publication-manifest-v3.sha256")" ]
[ "$(sha256sum "$bundle/publication/publication-lock-v3.json" | cut -d" " -f1)" = "$(cat "$bundle/publication/publication-lock-v3.sha256")" ]
[ -f "$manifest_tool_fd" ]
case "$bundle" in
  /home/robinhood/.local/opt/robin-highscores/releases/"$commit".partial|\
  /home/robinhood/.local/opt/robin-highscores/releases/"$commit"|\
  /proc/self/fd/[0-9]*|/proc/self/fd/[0-9]*/.) ;;
  *) exit 65 ;;
esac
exec /usr/bin/true validator "$bundle"
"""
        )
        validator.chmod(0o500)
        manifest = self.bootstrap / "DEPLOY_BOOTSTRAP_SHA256SUMS"
        manifest.write_text(
            "".join(
                f"{digest(self.bootstrap / name)}  {name}\n"
                for name in (
                    "deploy-release.sh",
                    "rollback-release.sh",
                    "validate-release-bundle.sh",
                )
            )
        )
        manifest.chmod(0o400)
        # Mirror the documented bootstrap durability order: each final file,
        # then the sealed directory, then its parent.
        for name in (
            "DEPLOY_BOOTSTRAP_SHA256SUMS",
            "deploy-release.sh",
            "rollback-release.sh",
            "validate-release-bundle.sh",
        ):
            fsync_file(self.bootstrap / name)
        self.bootstrap.chmod(0o500)
        fsync_directory(self.bootstrap)
        fsync_directory(self.bootstrap.parent)
        self.bootstrap_digest = digest(manifest)

    def plan_path(self, commit: str) -> Path:
        return self.plan_root / f"vps-release-plan-v2-{commit}.json"

    def _make_source_closure(self, commit: str, candidate: Path) -> None:
        source = self.incoming / f".sources-{commit}"
        retained_plan = self.plan_path(commit)
        if source.is_dir() and retained_plan.is_file():
            self.assert_source_plan_matches_candidate(source, retained_plan, candidate)
            return
        source.mkdir(mode=0o700)
        publication = source / "publication-v3"
        shutil.copytree(candidate / "publication", publication)
        for directory in [publication, *(path for path in publication.rglob("*") if path.is_dir())]:
            directory.chmod(0o700)
        for file in (path for path in publication.rglob("*") if path.is_file()):
            file.chmod(0o440)

        source_root = f"/home/robinhood/.local/opt/robin-highscores/incoming/.sources-{commit}"
        plan_bytes = json.dumps(
            {
                "binaries": [],
                "configs": [],
                "host_files": [],
                "private_raw_roots": [
                    {
                        "edition": "demo",
                        "root": "/home/robinhood/.local/share/robin-highscores/raw-content/demo",
                    },
                    {
                        "edition": "full",
                        "root": "/home/robinhood/.local/share/robin-highscores/raw-content/full",
                    },
                ],
                "publication_v3": f"{source_root}/publication-v3",
                "schema_version": 2,
                "source_commit": commit,
            },
            sort_keys=True,
            separators=(",", ":"),
        ).encode()
        source_plan = source / "vps-release-plan-v2.json"
        source_plan.write_bytes(plan_bytes)
        source_plan.chmod(0o400)
        retained_plan.write_bytes(plan_bytes)
        retained_plan.chmod(0o400)
        fsync_file(source_plan)
        fsync_file(retained_plan)
        fsync_directory(publication)
        fsync_directory(source)
        fsync_directory(self.incoming)
        fsync_directory(self.plan_root)

    @staticmethod
    def assert_source_plan_matches_candidate(
        source: Path, retained_plan: Path, candidate: Path
    ) -> None:
        source_plan = source / "vps-release-plan-v2.json"
        if source_plan.read_bytes() != retained_plan.read_bytes():
            raise RuntimeError("retained and uploader VPS plans differ")
        candidate_lock = candidate / "publication/publication-lock-v3.sha256"
        source_lock = source / "publication-v3/publication-lock-v3.json"
        if digest(source_lock) != candidate_lock.read_text():
            raise RuntimeError("retained uploader publication differs from candidate")

    def candidate(
        self, commit: str, *, database_schema_version: int = DATABASE_SCHEMA_VERSION
    ) -> Path:
        root = self.releases / f"{commit}.partial"
        root.mkdir(mode=0o750)
        for relative in (
            "bin",
            "config",
            "deploy",
            "deploy/tests",
            "publication",
            "systemd/user",
        ):
            (root / relative).mkdir(parents=True, exist_ok=True)
        (root / "SOURCE_COMMIT").write_text(commit + "\n")
        admin = root / "bin/robin-highscores-admin"
        admin.write_text(ADMIN_TOOL)
        manifest_tool = root / "bin/robin-highscores-manifestctl"
        shutil.copyfile(self.manifestctl, manifest_tool)
        publication = root / "publication"
        publication_manifest = publication / PUBLICATION_MANIFEST
        publication_manifest.write_text(
            f'{{"schema_version":{PUBLICATION_SCHEMA_VERSION},'
            f'"source_commit":"{commit}"}}\n'
        )
        publication_manifest_sha256 = digest(publication_manifest)
        (publication / PUBLICATION_MANIFEST_SIDECAR).write_text(
            publication_manifest_sha256
        )
        publication_lock = publication / PUBLICATION_LOCK
        publication_lock.write_text(
            f'{{"schema_version":{PUBLICATION_SCHEMA_VERSION},'
            f'"publication_manifest_sha256":"{publication_manifest_sha256}",'
            f'"source_commit":"{commit}"}}\n'
        )
        publication_lock_sha256 = digest(publication_lock)
        (publication / PUBLICATION_LOCK_SIDECAR).write_text(publication_lock_sha256)
        (root / RELEASE_MANIFEST).write_text(
            f'{{"schema_version":2,"source_commit":"{commit}",'
            f'"database_schema_version":{database_schema_version},'
            f'"publication_manifest_sha256":"{publication_manifest_sha256}",'
            f'"publication_lock_sha256":"{publication_lock_sha256}"}}\n'
        )
        config = root / "config/highscores-server.toml"
        config.write_text(
            'database_path = "/home/robinhood/.local/share/robin-highscores/database/highscores.sqlite3"\n'
            'runtime_fence_directory = "/home/robinhood/.local/share/robin-highscores/runtime-fence"\n'
            'backup_manifest_path = "/home/robinhood/.local/share/robin-highscores/status/backup-status.json"\n'
            'backup_authority_hmac_secret_path = "/home/robinhood/.local/share/robin-highscores/api-secrets/backup-authority-hmac.key"\n'
            f'release_manifest_path = "/home/robinhood/.local/opt/robin-highscores/releases/{commit}/{RELEASE_MANIFEST}"\n'
        )
        shutil.copyfile(
            self.bootstrap / "validate-release-bundle.sh",
            root / "deploy/validate-release-bundle.sh",
        )
        (root / "deploy/tests/real-runtime-fence-release-gate.sh").write_text(
            """#!/bin/sh
set -eu
[ "$#" -eq 5 ]
[ "$1" = "/home/robinhood/.local/opt/robin-highscores/releases/$2.partial" ]
[ "$3" = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee" ]
[ "$4" = "/home/robinhood/.local/share/robin-highscores/raw-content/demo" ]
[ "$5" = "/home/robinhood/.local/share/robin-highscores/raw-content/full" ]
case "${ROBIN_REAL_FENCE_PINNED_CANDIDATE_FD:-}" in ''|*[!0-9]*) exit 65 ;; esac
[ "$(readlink -f "/proc/self/fd/$ROBIN_REAL_FENCE_PINNED_CANDIDATE_FD")" = "$1" ]
printf '%s\n' real-runtime-fence-gate >>/home/robinhood/.tx-state/events.log
"""
        )
        for name in (
            "real-runtime-fence-e2e.py",
            "real-runtime-fence-e2e-selftest.py",
        ):
            (root / "deploy/tests" / name).write_text(
                "#!/usr/bin/python3\nraise SystemExit('transaction fixture only')\n"
            )
        for unit in UNITS:
            (root / "systemd/user" / unit).write_text(
                f"# transaction fixture {commit}\n[Unit]\nDescription={unit} {commit}\n"
            )
        for directory in [root, *(path for path in root.rglob("*") if path.is_dir())]:
            directory.chmod(0o550)
        for file in (path for path in root.rglob("*") if path.is_file()):
            file.chmod(0o440)
        admin.chmod(0o550)
        manifest_tool.chmod(0o550)
        (root / "deploy/validate-release-bundle.sh").chmod(0o550)
        for path in (root / "deploy/tests").iterdir():
            path.chmod(0o550)
        self._make_source_closure(commit, root)
        return root

    def provision_runtime_fence(self) -> None:
        fence = self.data / RUNTIME_FENCE
        fence.mkdir(mode=0o700)
        for name in (DB_ADMISSION_LOCK, DB_QUIESCENCE_LOCK):
            path = fence / name
            descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o400)
            os.fsync(descriptor)
            os.close(descriptor)
        fence.chmod(0o500)
        fsync_directory(fence)
        fsync_directory(self.data)

    def candidate_admin(
        self, commit: str, arguments: list[str], **kwargs
    ) -> Result:
        return self.run(self.candidate_admin_command(commit, arguments), **kwargs)

    def candidate_admin_command(
        self, commit: str, arguments: list[str]
    ) -> list[str]:
        root = f"/home/robinhood/.local/opt/robin-highscores/releases/{commit}.partial"
        return [
            "/bin/bash",
            "-c",
            'root=$1; shift; lock=/home/robinhood/.local/opt/robin-highscores/activation.lock; '
            '[ -e "$lock" ] || { : >"$lock"; /run/tx-real/chmod 0600 "$lock"; }; '
            'exec 7<"$root"; exec 8<>"$lock"; '
            'exec "$root/bin/robin-highscores-admin" "$@"',
            "robinhood-candidate-admin",
            root,
            *arguments,
        ]

    def runtime_authority_probe(self, commit: str, state: str) -> Result:
        candidate = self.releases / f"{commit}.partial"
        manifest_digest = digest(candidate / RELEASE_MANIFEST)
        return self.candidate_admin(
            commit,
            [
                "probe-runtime-authority-v2",
                "--candidate-release-root-fd",
                "7",
                "--expected-vps-release-manifest-sha256",
                manifest_digest,
                "--backup-authority-state",
                state,
            ],
        )

    def initialize_backup_authority(self, commit: str) -> Result:
        config = (
            f"/home/robinhood/.local/opt/robin-highscores/releases/"
            f"{commit}.partial/config/highscores-server.toml"
        )
        return self.candidate_admin(
            commit,
            [
                "--config",
                config,
                "initialize-backup-authority-key-v2",
                "--source-commit",
                commit,
                "--activation-lock-fd",
                "8",
            ],
        )

    def complete_backup_authority(self, commit: str) -> Result:
        candidate = self.releases / f"{commit}.partial"
        config = (
            f"/home/robinhood/.local/opt/robin-highscores/releases/"
            f"{commit}.partial/config/highscores-server.toml"
        )
        return self.candidate_admin(
            commit,
            [
                "--config",
                config,
                "complete-backup-authority-key-v2",
                "--source-commit",
                commit,
                "--activation-lock-fd",
                "8",
                "--candidate-release-root-fd",
                "7",
                "--expected-vps-release-manifest-sha256",
                digest(candidate / RELEASE_MANIFEST),
            ],
        )

    def verify_live_database_schema(self, commit: str) -> Result:
        return self.candidate_admin(commit, self.live_schema_arguments(commit))

    def live_schema_arguments(self, commit: str) -> list[str]:
        candidate = self.releases / f"{commit}.partial"
        manifest_digest = digest(candidate / RELEASE_MANIFEST)
        return [
            "verify-live-database-schema-v2",
            "--candidate-release-root-fd",
            "7",
            "--expected-vps-release-manifest-sha256",
            manifest_digest,
        ]

    def _bwrap(self, command: list[str], extra_env: dict[str, str]) -> list[str]:
        mocked = (
            "systemctl",
            "curl",
            "stat",
            "findmnt",
            "flock",
            "mv",
            "sync",
            "sha256sum",
            "df",
            "find",
            "sleep",
            "true",
            "rm",
            "mkdir",
            "install",
            "cp",
            "ln",
            "chmod",
        )
        delegated = (
            "flock", "mv", "sync", "sha256sum", "stat", "find", "rm",
            "mkdir", "install", "cp", "ln", "chmod",
        )
        result = [
            "/usr/bin/bwrap",
            "--die-with-parent",
            "--ro-bind", "/usr", "/usr",
            "--ro-bind", "/bin", "/bin",
            "--ro-bind", "/lib", "/lib",
            "--ro-bind", "/lib64", "/lib64",
            "--ro-bind", "/etc", "/etc",
            "--ro-bind", str(self.passwd), "/etc/passwd",
            "--proc", "/proc",
            "--dev", "/dev",
            "--tmpfs", "/tmp",
            "--dir", "/home",
            "--bind", str(self.home), "/home/robinhood",
            "--dir", "/run",
            "--dir", "/run/tx-real",
        ]
        for name in delegated:
            result.extend(("--ro-bind", f"/usr/bin/{name}", f"/run/tx-real/{name}"))
        for name in mocked:
            result.extend(("--ro-bind", str(HOST_DOUBLE), f"/usr/bin/{name}"))
        environment = {
            "HOME": "/home/robinhood",
            "TX_STATE": "/home/robinhood/.tx-state",
            **extra_env,
        }
        for name, value in environment.items():
            result.extend(("--setenv", name, value))
        result.extend(("--", *command))
        return result

    def run(
        self,
        command: list[str],
        *,
        fail_event: str = "",
        fail_mode: str = "before-error",
        fail_occurrence: int = 1,
        pause_event: str = "",
        pause_phase: str = "before",
        nested_mount: str = "",
        mixed_owner_root: str = "",
        wrong_owner_path: str = "",
        empty_missing_enabled: bool = False,
        available_bytes: int = 17_179_869_184,
    ) -> Result:
        env = {
            "TX_FAIL_EVENT": fail_event,
            "TX_FAIL_MODE": fail_mode,
            "TX_FAIL_EVENT_OCCURRENCE": str(fail_occurrence),
            "TX_PAUSE_EVENT": pause_event,
            "TX_PAUSE_PHASE": pause_phase,
            "TX_PAUSE_EVENT_OCCURRENCE": "1",
            "TX_NESTED_MOUNT": nested_mount,
            "TX_MIXED_OWNER_ROOT": mixed_owner_root,
            "TX_WRONG_OWNER_PATH": wrong_owner_path,
            "TX_EMPTY_MISSING_ENABLED": "1" if empty_missing_enabled else "0",
            "TX_AVAILABLE_BYTES": str(available_bytes),
        }
        completed = subprocess.run(
            self._bwrap(command, env),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            timeout=TRANSACTION_TIMEOUT_SECONDS,
        )
        return Result(completed)

    def popen(
        self, command: list[str], *, pause_event: str, pause_phase: str = "before"
    ) -> subprocess.Popen[str]:
        env = {
            "TX_FAIL_EVENT": "",
            "TX_FAIL_MODE": "before-error",
            "TX_FAIL_EVENT_OCCURRENCE": "1",
            "TX_PAUSE_EVENT": pause_event,
            "TX_PAUSE_PHASE": pause_phase,
            "TX_PAUSE_EVENT_OCCURRENCE": "1",
            "TX_NESTED_MOUNT": "",
            "TX_MIXED_OWNER_ROOT": "",
            "TX_WRONG_OWNER_PATH": "",
            "TX_AVAILABLE_BYTES": "17179869184",
        }
        return subprocess.Popen(
            self._bwrap(command, env),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

    def deploy(self, commit: str, **kwargs) -> Result:
        return self.run(
            self.deploy_command(commit),
            **kwargs,
        )

    def deploy_command(self, commit: str) -> list[str]:
        candidate = self.releases / f"{commit}.partial"
        plan = self.plan_path(commit)
        return [
            "/bin/bash", "-c", DEPLOY_BOOTSTRAP_COMMAND,
            "robinhood-bootstrap", f"/home/robinhood/{self.bootstrap.name}",
            f"/home/robinhood/.local/opt/robin-highscores/releases/{commit}.partial",
            f"/home/robinhood/{self.plan_root.name}/{plan.name}",
            commit, SUMS, self.bootstrap_digest,
            f"/home/robinhood/{self.manifestctl_dir.name}/robin-highscores-manifestctl",
            self.manifestctl_digest,
            digest(plan),
            digest(candidate / RELEASE_MANIFEST),
        ]

    def resume(self, commit: str, **kwargs) -> Result:
        candidate = self.releases / commit
        plan = self.plan_path(commit)
        return self.run(
            [
                "/bin/bash", "-c", RESUME_BOOTSTRAP_COMMAND,
                "robinhood-bootstrap", f"/home/robinhood/{self.bootstrap.name}",
                f"/home/robinhood/.local/opt/robin-highscores/releases/{commit}",
                f"/home/robinhood/{self.plan_root.name}/{plan.name}",
                commit, SUMS, self.bootstrap_digest,
                f"/home/robinhood/{self.manifestctl_dir.name}/robin-highscores-manifestctl",
                self.manifestctl_digest,
                digest(plan),
                digest(candidate / RELEASE_MANIFEST),
            ],
            **kwargs,
        )

    def rollback(self, commit: str, **kwargs) -> Result:
        return self.run(
            [
                "/bin/bash", "-c", ROLLBACK_BOOTSTRAP_COMMAND,
                "robinhood-bootstrap", f"/home/robinhood/{self.bootstrap.name}",
                commit, SUMS, self.bootstrap_digest,
                f"/home/robinhood/{self.manifestctl_dir.name}/robin-highscores-manifestctl",
                self.manifestctl_digest,
            ],
            **kwargs,
        )

    def rollback_resume(self, commit: str, **kwargs) -> Result:
        return self.run(
            [
                "/bin/bash", "-c", ROLLBACK_RESUME_BOOTSTRAP_COMMAND,
                "robinhood-bootstrap", f"/home/robinhood/{self.bootstrap.name}",
                commit, SUMS, self.bootstrap_digest,
                f"/home/robinhood/{self.manifestctl_dir.name}/robin-highscores-manifestctl",
                self.manifestctl_digest,
            ],
            **kwargs,
        )

    def events(self) -> list[str]:
        path = self.state / "events.log"
        return [line.split("\t", 1)[0] for line in path.read_text().splitlines()] if path.exists() else []

    def reset_events(self) -> None:
        path = self.state / "events.log"
        if path.exists():
            path.write_text("")

    def active(self, unit: str) -> str:
        path = self.state / "active" / unit
        return path.read_text().strip() if path.exists() else "inactive"

    def enabled(self, unit: str) -> str:
        path = self.state / "enabled" / unit
        return path.read_text().strip() if path.exists() else "disabled"

    def set_active(self, unit: str, value: str) -> None:
        path = self.state / "active" / unit
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(value + "\n")

    def set_enabled(self, unit: str, value: str) -> None:
        path = self.state / "enabled" / unit
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(value + "\n")

    def current(self) -> str | None:
        path = self.opt / "current"
        return os.readlink(path) if path.is_symlink() else None

    def unit_commit(self, unit: str) -> str:
        return (self.unit_root / unit).read_text().split()[3]

    def migration_count(self) -> int:
        path = self.state / "meta/migration-count"
        return int(path.read_text()) if path.exists() else 0

    def transaction_artifacts(self, operation: str, commit: str) -> tuple[Path, ...]:
        common = (
            self.unit_root / f".robin-highscores-{operation}-{commit}.stage",
            self.unit_root / f".robin-highscores-{operation}-{commit}.recovery",
            self.opt / f".current-{operation}-{commit}",
            self.opt / f".current-{operation}-{commit}.restore",
            self.opt / f".{operation}-prebackup-{commit}",
            self.opt / f".{operation}-prebackup-{commit}.new",
            self.opt / f".{operation}-source-backup-{commit}.receipt-v2",
            self.opt / f".{operation}-source-backup-{commit}.receipt-v2.new",
            self.opt / f".{operation}-target-backup-{commit}.receipt-v2",
            self.opt / f".{operation}-target-backup-{commit}.receipt-v2.new",
            self.opt / f".{operation}-target-backup-{commit}",
            self.opt / f".{operation}-target-backup-{commit}.new",
            self.opt / f".{operation}-prepared-{commit}",
            self.opt / f".{operation}-prepared-{commit}.new",
        )
        if operation == "deploy":
            return common + (
                self.incoming / f".{commit}.consuming",
                self.releases / f"{commit}.partial",
            )
        return common


class FrozenHarnessContractTests(unittest.TestCase):
    """Self-tests for authorities that are frozen independently of shell argv."""

    def setUp(self) -> None:
        self.fixture = Fixture()

    def tearDown(self) -> None:
        self.fixture.cleanup()

    def test_confirmed_clean_host_has_exact_four_key_authority(self) -> None:
        secret_root = self.fixture.data / "api-secrets"
        self.assertEqual(
            {entry.name for entry in secret_root.iterdir()},
            set(INITIAL_SECRET_BYTES),
        )
        for name, expected in INITIAL_SECRET_BYTES.items():
            path = secret_root / name
            metadata = path.lstat()
            self.assertTrue(stat.S_ISREG(metadata.st_mode), name)
            self.assertEqual(stat.S_IMODE(metadata.st_mode), 0o400, name)
            self.assertEqual(metadata.st_nlink, 1, name)
            self.assertEqual(path.read_bytes(), expected, name)
        for path in (
            secret_root / BACKUP_AUTHORITY_KEY,
            self.fixture.data / RUNTIME_FENCE,
        ):
            self.assertFalse(path.exists(), path)
            self.assertFalse(path.is_symlink(), path)

    def test_candidate_is_only_vps_v2_over_publication_v3(self) -> None:
        candidate = self.fixture.candidate(NEW)
        release_manifest = (candidate / RELEASE_MANIFEST).read_text()
        self.assertIn('"schema_version":2', release_manifest)
        self.assertIn(
            f'"database_schema_version":{DATABASE_SCHEMA_VERSION}',
            release_manifest,
        )
        self.assertFalse((candidate / "vps-release-manifest-v1.json").exists())

        publication = candidate / "publication"
        for document, sidecar in (
            (PUBLICATION_MANIFEST, PUBLICATION_MANIFEST_SIDECAR),
            (PUBLICATION_LOCK, PUBLICATION_LOCK_SIDECAR),
        ):
            self.assertIn(
                f'"schema_version":{PUBLICATION_SCHEMA_VERSION}',
                (publication / document).read_text(),
            )
            self.assertEqual(
                (publication / sidecar).read_text(), digest(publication / document)
            )
        for stale in (
            "publication-manifest-v2.json",
            "publication-manifest-v2.sha256",
            "publication-lock-v2.json",
            "publication-lock-v2.sha256",
        ):
            self.assertFalse((publication / stale).exists(), stale)

    def test_harness_outer_is_disjoint_v2_deploy_and_rollback(self) -> None:
        self.assertIn("exec-vps-activation-v2)", MANIFEST_TOOL)
        self.assertIn('case "$operation" in', MANIFEST_TOOL)
        self.assertNotIn("exec-vps-activation-v1", MANIFEST_TOOL)
        self.assertIn("exec-vps-activation-v2 deploy", DEPLOY_BOOTSTRAP_COMMAND)
        self.assertIn('/proc/self/fd/7 "$plan_sha" "$vps_sha" --', DEPLOY_BOOTSTRAP_COMMAND)
        self.assertIn("exec-vps-activation-v2 rollback", ROLLBACK_BOOTSTRAP_COMMAND)
        self.assertNotIn("plan_sha", ROLLBACK_BOOTSTRAP_COMMAND)
        self.assertNotIn("vps_sha", ROLLBACK_BOOTSTRAP_COMMAND)

    def test_runtime_authority_probe_absent_initialize_present_is_exact(self) -> None:
        candidate = self.fixture.candidate(NEW)
        manifest_digest = digest(candidate / RELEASE_MANIFEST)
        absent = self.fixture.runtime_authority_probe(NEW, "absent")
        self.assertEqual(absent.returncode, 0, absent.describe())
        self.assertEqual(
            absent.stdout,
            f'{{"backup_authority_state":"absent","schema_version":2,'
            f'"source_commit":"{NEW}",'
            f'"vps_release_manifest_sha256":"{manifest_digest}"}}',
        )
        self.assertFalse(absent.stdout.endswith("\n"))

        initialized = self.fixture.initialize_backup_authority(NEW)
        self.assertEqual(initialized.returncode, 0, initialized.describe())
        authority_key = self.fixture.data / "api-secrets" / BACKUP_AUTHORITY_KEY
        intent = (
            self.fixture.data
            / "api-secrets/.backup-authority-hmac-key.intent-v1.json"
        )
        self.assertTrue(intent.is_file())
        self.assertEqual(stat.S_IMODE(intent.stat().st_mode), 0o400)
        key_before = (
            authority_key.lstat().st_dev,
            authority_key.lstat().st_ino,
            authority_key.read_bytes(),
        )
        repeated = self.fixture.initialize_backup_authority(NEW)
        self.assertEqual(repeated.returncode, 0, repeated.describe())
        self.assertTrue(intent.is_file())
        self.fixture.provision_runtime_fence()

        present = self.fixture.runtime_authority_probe(NEW, "present")
        self.assertEqual(present.returncode, 0, present.describe())
        self.assertEqual(
            present.stdout,
            f'{{"backup_authority_state":"present","schema_version":2,'
            f'"source_commit":"{NEW}",'
            f'"vps_release_manifest_sha256":"{manifest_digest}"}}',
        )
        self.assertFalse(present.stdout.endswith("\n"))
        completed = self.fixture.complete_backup_authority(NEW)
        self.assertEqual(completed.returncode, 0, completed.describe())
        self.assertFalse(intent.exists())
        self.assertEqual(
            self.fixture.events(),
            [
                "admin.probe-runtime-authority-v2.absent",
                "admin.initialize-backup-authority-key-v2.intent-published",
                "admin.initialize-backup-authority-key-v2.key-linked",
                "admin.initialize-backup-authority-key-v2",
                "admin.initialize-backup-authority-key-v2",
                "admin.probe-runtime-authority-v2.present",
                "admin.complete-backup-authority-key-v2.outer-present",
                "admin.complete-backup-authority-key-v2.intent-removed",
                "admin.complete-backup-authority-key-v2",
            ],
        )

        repeated_completion = self.fixture.complete_backup_authority(NEW)
        self.assertEqual(
            repeated_completion.returncode, 0, repeated_completion.describe()
        )
        self.assertEqual(
            (
                authority_key.lstat().st_dev,
                authority_key.lstat().st_ino,
                authority_key.read_bytes(),
            ),
            key_before,
        )

    def test_runtime_authority_probe_rejects_noncanonical_or_unbound_authority(self) -> None:
        candidate = self.fixture.candidate(NEW)
        manifest_digest = digest(candidate / RELEASE_MANIFEST)
        wrong_digest = self.fixture.candidate_admin(
            NEW,
            [
                "probe-runtime-authority-v2",
                "--candidate-release-root-fd",
                "7",
                "--expected-vps-release-manifest-sha256",
                "0" * 64,
                "--backup-authority-state",
                "absent",
            ],
        )
        self.assertNotEqual(wrong_digest.returncode, 0, wrong_digest.describe())

        with_config = self.fixture.candidate_admin(
            NEW,
            [
                "probe-runtime-authority-v2",
                "--candidate-release-root-fd",
                "7",
                "--expected-vps-release-manifest-sha256",
                manifest_digest,
                "--backup-authority-state",
                "absent",
                "--config",
                "/tmp/forbidden.toml",
            ],
        )
        self.assertNotEqual(with_config.returncode, 0, with_config.describe())

        root = f"/home/robinhood/.local/opt/robin-highscores/releases/{NEW}.partial"
        wrong_root = self.fixture.run(
            [
                "/bin/bash",
                "-c",
                'root=$1; shift; exec 7<"$root/publication"; exec "$root/bin/robin-highscores-admin" "$@"',
                "robinhood-wrong-candidate-root",
                root,
                "probe-runtime-authority-v2",
                "--candidate-release-root-fd",
                "7",
                "--expected-vps-release-manifest-sha256",
                manifest_digest,
                "--backup-authority-state",
                "absent",
            ]
        )
        self.assertNotEqual(wrong_root.returncode, 0, wrong_root.describe())

    def test_live_schema_verifier_reads_valid_wal_only_schema(self) -> None:
        candidate = self.fixture.candidate(NEW)
        manifest_digest = digest(candidate / RELEASE_MANIFEST)
        self.fixture.provision_runtime_fence()
        database = self.fixture.data / "database/highscores.sqlite3"
        connection = sqlite3.connect(database)
        try:
            self.assertEqual(connection.execute("PRAGMA journal_mode=WAL").fetchone()[0], "wal")
            connection.execute("PRAGMA wal_autocheckpoint=0")
            connection.execute(
                "CREATE TABLE _sqlx_migrations (version INTEGER PRIMARY KEY, success BOOLEAN NOT NULL)"
            )
            connection.execute("INSERT INTO _sqlx_migrations VALUES (1, 1)")
            connection.commit()
            connection.execute("PRAGMA wal_checkpoint(TRUNCATE)")
            connection.execute("INSERT INTO _sqlx_migrations VALUES (2, 1)")
            connection.commit()

            main_only = self.fixture.state / "main-database-without-wal.sqlite3"
            shutil.copyfile(database, main_only)
            copied = sqlite3.connect(f"file:{main_only}?mode=ro", uri=True)
            try:
                self.assertEqual(
                    copied.execute("SELECT max(version) FROM _sqlx_migrations").fetchone()[0],
                    1,
                )
            finally:
                copied.close()
            self.assertTrue(database.with_name(database.name + "-wal").is_file())
            live_database_before = tree_fingerprint(self.fixture.data / "database")

            result = self.fixture.verify_live_database_schema(NEW)
            self.assertEqual(result.returncode, 0, result.describe())
            self.assertEqual(
                result.stdout,
                f'{{"database_schema_version":2,"schema_version":2,'
                f'"source_commit":"{NEW}",'
                f'"vps_release_manifest_sha256":"{manifest_digest}"}}',
            )
            self.assertFalse(result.stdout.endswith("\n"))
            self.assertEqual(
                self.fixture.events(), ["admin.verify-live-database-schema-v2"]
            )
            self.assertEqual(
                tree_fingerprint(self.fixture.data / "database"),
                live_database_before,
            )

            self.fixture.reset_events()
            process = self.fixture.popen(
                self.fixture.candidate_admin_command(
                    NEW, self.fixture.live_schema_arguments(NEW)
                ),
                pause_event="admin.verify-live-database-schema-v2",
            )
            deadline = time.monotonic() + 15
            while not (self.fixture.state / "pause.ready").exists():
                if process.poll() is not None:
                    stdout, stderr = process.communicate()
                    self.fail(
                        f"live-schema verifier exited before lock proof: {stdout}\n{stderr}"
                    )
                self.assertLess(time.monotonic(), deadline)
                time.sleep(0.02)
            held_locks = []
            try:
                for name in (DB_ADMISSION_LOCK, DB_QUIESCENCE_LOCK):
                    lock = (self.fixture.data / RUNTIME_FENCE / name).open("rb")
                    held_locks.append(lock)
                    with self.assertRaises(BlockingIOError, msg=name):
                        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            finally:
                for lock in held_locks:
                    lock.close()
                (self.fixture.state / "pause.release").write_text("continue\n")
            stdout, stderr = process.communicate(timeout=20)
            self.assertEqual(process.returncode, 0, f"{stdout}\n{stderr}")
            self.assertEqual(stdout, result.stdout)
            self.assertEqual(
                tree_fingerprint(self.fixture.data / "database"),
                live_database_before,
            )
        finally:
            connection.close()


class DeployTransactionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        deploy = (SOURCE / "deploy-release.sh").read_text()
        rollback = (SOURCE / "rollback-release.sh").read_text()
        validator = (SOURCE / "validate-release-bundle.sh").read_text()
        transaction_source = "\n".join((deploy, rollback, validator))
        required = (
            "--resume-installed",
            "DEPLOY_BOOTSTRAP_SHA256SUMS",
            ".partial",
            RELEASE_MANIFEST,
            "activation_lock_fd",
            "candidate_root_fd",
            "plan_fd",
            "expected_plan_sha256",
            "consume-vps-sources-v2",
            BACKUP_AUTHORITY_KEY,
            DB_ADMISSION_LOCK,
            DB_QUIESCENCE_LOCK,
        )
        missing = [token for token in required if token not in transaction_source]
        if missing:
            raise RuntimeError(
                f"selected transaction source predates the required contract ({missing}); "
                "the deployment harness never converts a missing release gate into a skip"
            )
        if not HOST_DOUBLE.stat().st_mode & stat.S_IXUSR:
            raise RuntimeError(f"host double must be executable: {HOST_DOUBLE}")

    def setUp(self) -> None:
        self.fixture = Fixture()

    def test_resume_validator_uses_a_real_inherited_directory_fd_descendant(self) -> None:
        deploy = (SOURCE / "deploy-release.sh").read_text()
        self.assertIn(
            'validate_release_descriptor_tree "$candidate_root_fd/."', deploy
        )
        self.assertNotIn(
            'validate_release_descriptor_tree "$candidate_root_fd"', deploy
        )

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "release"
            root.mkdir()
            regular = root / "regular"
            regular.write_bytes(b"not a release root")
            root_fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
            regular_fd = os.open(regular, os.O_RDONLY | os.O_CLOEXEC)
            try:
                bare = f"/proc/self/fd/{root_fd}"
                descendant = f"/proc/self/fd/{root_fd}/."
                self.assertTrue(os.path.islink(bare))
                self.assertFalse(os.path.islink(descendant))
                self.assertTrue(os.path.isdir(descendant))
                descendant_metadata = os.stat(descendant)
                self.assertEqual(
                    (os.fstat(root_fd).st_dev, os.fstat(root_fd).st_ino),
                    (descendant_metadata.st_dev, descendant_metadata.st_ino),
                )
                self.assertFalse(os.path.isdir(f"/proc/self/fd/{regular_fd}/."))
            finally:
                os.close(regular_fd)
                os.close(root_fd)

    def tearDown(self) -> None:
        self.fixture.cleanup()

    def assert_success(self, result: Result) -> None:
        self.assertEqual(result.returncode, 0, result.describe())

    def assert_failure(self, result: Result) -> None:
        self.assertNotEqual(result.returncode, 0, result.describe())

    def assert_no_service_mutation(self, fixture: Fixture | None = None) -> None:
        selected = fixture or self.fixture
        mutating = [
            event for event in selected.events()
            if event.startswith((
                "systemctl.start", "systemctl.stop",
                "systemctl.enable", "systemctl.disable",
            ))
        ]
        self.assertEqual(mutating, [])

    def deploy_old(self) -> None:
        self.fixture.candidate(OLD)
        self.assert_success(self.fixture.deploy(OLD))
        self.assertEqual(self.fixture.current(), f"releases/{OLD}")

    def deploy_upgrade(self) -> None:
        self.deploy_old()
        self.fixture.candidate(NEW)
        self.assert_success(self.fixture.deploy(NEW))
        self.assertEqual(self.fixture.current(), f"releases/{NEW}")

    def assert_selected_active(
        self, commit: str, fixture: Fixture | None = None
    ) -> None:
        selected = fixture or self.fixture
        self.assertEqual(selected.current(), f"releases/{commit}")
        for unit in UNITS:
            if unit != "robin-highscores-backup.service":
                self.assertEqual(selected.active(unit), "active", unit)
        self.assertEqual(selected.active("robin-highscores-backup.service"), "inactive")
        self.assertEqual(selected.enabled("robin-highscores.target"), "enabled")
        self.assertEqual(selected.enabled("robin-highscores-backup.timer"), "enabled")

    def assert_selected_stopped(
        self, commit: str, fixture: Fixture | None = None
    ) -> None:
        selected = fixture or self.fixture
        self.assertEqual(selected.current(), f"releases/{commit}")
        for unit in UNITS:
            self.assertEqual(selected.active(unit), "inactive", unit)
        self.assertEqual(selected.enabled("robin-highscores.target"), "disabled")
        self.assertEqual(selected.enabled("robin-highscores-backup.timer"), "disabled")

    def assert_transaction_artifacts_absent(
        self, operation: str, commit: str, fixture: Fixture | None = None
    ) -> None:
        selected = fixture or self.fixture
        residuals = [
            str(path) for path in selected.transaction_artifacts(operation, commit)
            if path.exists() or path.is_symlink()
        ]
        self.assertEqual(residuals, [])

    def assert_safe_activation_lock(self, fixture: Fixture | None = None) -> None:
        selected = fixture or self.fixture
        lock = selected.opt / "activation.lock"
        metadata = lock.lstat()
        self.assertTrue(stat.S_ISREG(metadata.st_mode))
        self.assertEqual(stat.S_IMODE(metadata.st_mode), 0o600)
        self.assertEqual(metadata.st_uid, os.getuid())
        self.assertEqual(metadata.st_nlink, 1)

    def assert_exact_runtime_authority(self, fixture: Fixture | None = None) -> None:
        selected = fixture or self.fixture
        authority_key = selected.data / "api-secrets" / BACKUP_AUTHORITY_KEY
        metadata = authority_key.lstat()
        self.assertTrue(stat.S_ISREG(metadata.st_mode), authority_key)
        self.assertEqual(stat.S_IMODE(metadata.st_mode), 0o400, authority_key)
        self.assertEqual(metadata.st_uid, os.getuid(), authority_key)
        self.assertEqual(metadata.st_nlink, 1, authority_key)
        self.assertEqual(metadata.st_size, 32, authority_key)
        for residue in (
            ".backup-authority-hmac-key.intent-v1.json",
            ".backup-authority-hmac-key.intent-v1.json.new",
            ".backup-authority-hmac-key.payload-v1.new",
        ):
            path = selected.data / "api-secrets" / residue
            self.assertFalse(path.exists() or path.is_symlink(), path)

        fence = selected.data / RUNTIME_FENCE
        metadata = fence.lstat()
        self.assertTrue(stat.S_ISDIR(metadata.st_mode), fence)
        self.assertEqual(stat.S_IMODE(metadata.st_mode), 0o500, fence)
        self.assertEqual(metadata.st_uid, os.getuid(), fence)
        self.assertEqual(
            {entry.name for entry in fence.iterdir()},
            {DB_ADMISSION_LOCK, DB_QUIESCENCE_LOCK},
        )
        for name in (DB_ADMISSION_LOCK, DB_QUIESCENCE_LOCK):
            path = fence / name
            metadata = path.lstat()
            self.assertTrue(stat.S_ISREG(metadata.st_mode), path)
            self.assertEqual(stat.S_IMODE(metadata.st_mode), 0o400, path)
            self.assertEqual(metadata.st_uid, os.getuid(), path)
            self.assertEqual(metadata.st_nlink, 1, path)
            self.assertEqual(metadata.st_size, 0, path)

    @staticmethod
    def without_activation_lock(
        fingerprint: tuple[
            tuple[str, str, int, int, int, int, int, int, int, str], ...
        ]
    ) -> tuple[
        tuple[str, str, int, int, int, int, int, int, int, str], ...
    ]:
        return tuple(entry for entry in fingerprint if entry[0] != "activation.lock")

    def test_first_deploy_exact_paths_bootstrap_and_candidate_consumption(self) -> None:
        candidate = self.fixture.candidate(NEW)
        result = self.fixture.deploy(NEW)
        self.assert_success(result)
        self.assertFalse(candidate.exists())
        self.assertTrue((self.fixture.releases / NEW).is_dir())
        self.assert_safe_activation_lock()
        self.assert_exact_runtime_authority()
        self.assertEqual(stat.S_IMODE(self.fixture.bootstrap.stat().st_mode), 0o500)
        self.assertEqual(
            stat.S_IMODE((self.fixture.bootstrap / "DEPLOY_BOOTSTRAP_SHA256SUMS").stat().st_mode),
            0o400,
        )
        self.assert_selected_active(NEW)
        events = self.fixture.events()
        gate = events.index("real-runtime-fence-gate")
        for mutation in (
            "mv.activation-journal",
            "mv.release-install",
            "admin.migrate",
            "mv.current-select",
        ):
            self.assertLess(gate, events.index(mutation), mutation)
        self.assertLess(events.index("mv.release-install"), events.index("admin.migrate"))
        self.assertLess(events.index("rm.incoming-consuming"), events.index("admin.migrate"))
        self.assertLess(events.index("admin.migrate"), events.index("mv.current-select"))
        self.assertLess(
            events.index("mv.current-select"),
            events.index("systemctl.start.robin-highscores-api.service"),
        )

    def test_mandatory_real_fence_gate_failure_is_zero_mutation(self) -> None:
        candidate = self.fixture.candidate(NEW)
        gate = candidate / "deploy/tests/real-runtime-fence-release-gate.sh"
        gate.chmod(0o750)
        gate.write_text("#!/bin/sh\nexit 42\n")
        gate.chmod(0o550)
        opt_before = tree_fingerprint(self.fixture.opt)
        units_before = tree_fingerprint(self.fixture.unit_root)
        data_before = tree_fingerprint(self.fixture.data)

        result = self.fixture.deploy(NEW)

        self.assertNotEqual(result.returncode, 0, result.describe())
        self.assertIn("mandatory authentic runtime-fence release gate failed", result.stderr)
        self.assertEqual(
            self.without_activation_lock(tree_fingerprint(self.fixture.opt)),
            self.without_activation_lock(opt_before),
        )
        self.assert_safe_activation_lock()
        self.assertEqual(tree_fingerprint(self.fixture.unit_root), units_before)
        self.assertEqual(tree_fingerprint(self.fixture.data), data_before)
        self.assertNotIn("admin.migrate", self.fixture.events())
        self.assert_no_service_mutation()

    def test_missing_real_fence_gate_is_zero_mutation(self) -> None:
        candidate = self.fixture.candidate(NEW)
        tests = candidate / "deploy/tests"
        tests.chmod(0o750)
        (tests / "real-runtime-fence-release-gate.sh").unlink()
        tests.chmod(0o550)
        opt_before = tree_fingerprint(self.fixture.opt)
        units_before = tree_fingerprint(self.fixture.unit_root)
        data_before = tree_fingerprint(self.fixture.data)

        result = self.fixture.deploy(NEW)

        self.assertNotEqual(result.returncode, 0, result.describe())
        self.assertIn("mandatory authentic runtime-fence release gate failed", result.stderr)
        self.assertEqual(
            self.without_activation_lock(tree_fingerprint(self.fixture.opt)),
            self.without_activation_lock(opt_before),
        )
        self.assert_safe_activation_lock()
        self.assertEqual(tree_fingerprint(self.fixture.unit_root), units_before)
        self.assertEqual(tree_fingerprint(self.fixture.data), data_before)
        self.assertNotIn("admin.migrate", self.fixture.events())
        self.assert_no_service_mutation()

    def test_upgrade_adopts_but_never_replaces_runtime_authority(self) -> None:
        self.deploy_old()
        self.assert_exact_runtime_authority()
        authority_paths = (
            self.fixture.data / "api-secrets" / BACKUP_AUTHORITY_KEY,
            self.fixture.data / RUNTIME_FENCE,
            self.fixture.data / RUNTIME_FENCE / DB_ADMISSION_LOCK,
            self.fixture.data / RUNTIME_FENCE / DB_QUIESCENCE_LOCK,
        )
        before = {
            path: (
                path.lstat().st_dev,
                path.lstat().st_ino,
                path.read_bytes() if path.is_file() else b"",
            )
            for path in authority_paths
        }

        self.fixture.candidate(NEW)
        self.assert_success(self.fixture.deploy(NEW))
        self.assert_exact_runtime_authority()
        after = {
            path: (
                path.lstat().st_dev,
                path.lstat().st_ino,
                path.read_bytes() if path.is_file() else b"",
            )
            for path in authority_paths
        }
        self.assertEqual(after, before)

    def test_upgrade_never_repairs_missing_or_inexact_runtime_authority(self) -> None:
        def missing_key(fixture: Fixture) -> None:
            (fixture.data / "api-secrets" / BACKUP_AUTHORITY_KEY).unlink()

        def key_hardlink(fixture: Fixture) -> None:
            key = fixture.data / "api-secrets" / BACKUP_AUTHORITY_KEY
            os.link(key, key.with_name("backup-authority-hmac.alias"))

        def fence_mode(fixture: Fixture) -> None:
            (fixture.data / RUNTIME_FENCE).chmod(0o700)

        def admission_missing(fixture: Fixture) -> None:
            (fixture.data / RUNTIME_FENCE / DB_ADMISSION_LOCK).unlink()

        def quiescence_symlink(fixture: Fixture) -> None:
            lock = fixture.data / RUNTIME_FENCE / DB_QUIESCENCE_LOCK
            lock.unlink()
            lock.symlink_to("db-admission.lock")

        cases = (
            ("missing-key", missing_key),
            ("key-hardlink", key_hardlink),
            ("fence-mode", fence_mode),
            ("admission-missing", admission_missing),
            ("quiescence-symlink", quiescence_symlink),
        )
        for label, arrange in cases:
            with self.subTest(case=label):
                fixture = Fixture()
                try:
                    fixture.candidate(OLD)
                    self.assert_success(fixture.deploy(OLD))
                    arrange(fixture)
                    fixture.candidate(NEW)
                    opt_before = tree_fingerprint(fixture.opt)
                    units_before = tree_fingerprint(fixture.unit_root)
                    data_before = tree_fingerprint(fixture.data)
                    fixture.reset_events()

                    result = fixture.deploy(NEW)

                    self.assert_failure(result)
                    self.assertEqual(tree_fingerprint(fixture.opt), opt_before)
                    self.assertEqual(tree_fingerprint(fixture.unit_root), units_before)
                    self.assertEqual(tree_fingerprint(fixture.data), data_before)
                    self.assert_no_service_mutation(fixture)
                    self.assertEqual(fixture.current(), f"releases/{OLD}")
                finally:
                    fixture.cleanup()

    def test_clean_host_fixture_matches_confirmed_initial_authorities(self) -> None:
        for path, expected_mode in (
            (self.fixture.data, 0o700),
            (self.fixture.data / "database", 0o700),
            (self.fixture.data / "replays", 0o700),
            (self.fixture.data / "campaign-states", 0o700),
            (self.fixture.data / "api-secrets", 0o700),
            (self.fixture.data / "raw-content", 0o550),
            (self.fixture.data / "raw-content/demo", 0o550),
            (self.fixture.data / "raw-content/full", 0o550),
            (self.fixture.opt, 0o750),
            (self.fixture.incoming, 0o750),
            (self.fixture.releases, 0o750),
            (self.fixture.bootstrap, 0o500),
        ):
            metadata = path.lstat()
            self.assertTrue(stat.S_ISDIR(metadata.st_mode), path)
            self.assertEqual(stat.S_IMODE(metadata.st_mode), expected_mode, path)
        for relative in ("database", "replays", "campaign-states"):
            self.assertEqual(list((self.fixture.data / relative).iterdir()), [], relative)
        for relative in ("backups", "status"):
            path = self.fixture.data / relative
            self.assertFalse(path.exists(), relative)
            self.assertFalse(path.is_symlink(), relative)
        expected_secrets = set(INITIAL_SECRET_BYTES)
        secret_root = self.fixture.data / "api-secrets"
        self.assertEqual({path.name for path in secret_root.iterdir()}, expected_secrets)
        self.assertFalse((secret_root / BACKUP_AUTHORITY_KEY).exists())
        for name in expected_secrets:
            metadata = (secret_root / name).lstat()
            self.assertTrue(stat.S_ISREG(metadata.st_mode), name)
            self.assertEqual(stat.S_IMODE(metadata.st_mode), 0o400, name)
            self.assertEqual(metadata.st_nlink, 1, name)
            self.assertEqual(
                (secret_root / name).read_bytes(), INITIAL_SECRET_BYTES[name], name
            )
        runtime_fence = self.fixture.data / RUNTIME_FENCE
        self.assertFalse(runtime_fence.exists())
        self.assertFalse(runtime_fence.is_symlink())
        self.assertFalse((self.fixture.data / "root-once").exists())
        self.assertFalse(self.fixture.unit_root.exists())
        self.assertFalse(self.fixture.unit_root.is_symlink())
        lock = self.fixture.opt / "activation.lock"
        self.assertFalse(lock.exists())
        self.assertFalse(lock.is_symlink())
        self.assertEqual(
            stat.S_IMODE(
                (self.fixture.bootstrap / "DEPLOY_BOOTSTRAP_SHA256SUMS").lstat().st_mode
            ),
            0o400,
        )
        for name in (
            "deploy-release.sh",
            "rollback-release.sh",
            "validate-release-bundle.sh",
        ):
            self.assertEqual(
                stat.S_IMODE((self.fixture.bootstrap / name).lstat().st_mode),
                0o500,
                name,
            )

    def test_first_deploy_user_unit_root_creation_is_safe_and_fail_closed(self) -> None:
        def ancestor_symlink(fixture: Fixture) -> dict[str, str]:
            (fixture.home / ".config").symlink_to("/tmp")
            return {}

        def root_symlink(fixture: Fixture) -> dict[str, str]:
            fixture.unit_root.parent.mkdir(parents=True, mode=0o750)
            fixture.unit_root.symlink_to("/tmp")
            return {}

        def wrong_mode(fixture: Fixture) -> dict[str, str]:
            fixture.unit_root.mkdir(parents=True, mode=0o755)
            return {}

        def wrong_owner(fixture: Fixture) -> dict[str, str]:
            fixture.unit_root.mkdir(parents=True, mode=0o750)
            return {
                "wrong_owner_path": "/home/robinhood/.config/systemd/user"
            }

        def creation_race_before(_fixture: Fixture) -> dict[str, str]:
            return {
                "fail_event": "*.user-unit-root",
                "fail_mode": "before-error",
            }

        def creation_race_after(_fixture: Fixture) -> dict[str, str]:
            return {
                "fail_event": "*.user-unit-root",
                "fail_mode": "after-error",
            }

        cases = (
            ("ancestor-symlink", ancestor_symlink, True),
            ("root-symlink", root_symlink, True),
            ("wrong-mode", wrong_mode, True),
            ("wrong-owner", wrong_owner, True),
            ("creation-race-before", creation_race_before, True),
            ("creation-race-after", creation_race_after, False),
        )
        for label, arrange, exact_unit_tree in cases:
            with self.subTest(case=label):
                fixture = Fixture()
                try:
                    candidate = fixture.candidate(NEW)
                    arguments = arrange(fixture)
                    opt_before = tree_fingerprint(fixture.opt)
                    units_before = tree_fingerprint(fixture.unit_root)
                    state_before = tree_fingerprint(fixture.data)

                    result = fixture.deploy(NEW, **arguments)

                    self.assert_failure(result)
                    self.assertEqual(fixture.current(), None)
                    self.assertFalse((fixture.releases / NEW).exists())
                    self.assertTrue(candidate.is_dir())
                    self.assertEqual(
                        self.without_activation_lock(tree_fingerprint(fixture.opt)),
                        self.without_activation_lock(opt_before),
                    )
                    self.assert_safe_activation_lock(fixture)
                    if exact_unit_tree:
                        self.assertEqual(
                            tree_fingerprint(fixture.unit_root), units_before
                        )
                    else:
                        self.assertTrue(fixture.unit_root.is_dir())
                        self.assertEqual(list(fixture.unit_root.iterdir()), [])
                    self.assertEqual(tree_fingerprint(fixture.data), state_before)
                    self.assert_no_service_mutation(fixture)
                finally:
                    fixture.cleanup()

    def test_first_deploy_rejects_dirty_managed_state_without_mutation(self) -> None:
        def active_unit(fixture: Fixture) -> None:
            fixture.set_active("robin-highscores-api.service", "active")

        def managed_unit_file(fixture: Fixture) -> None:
            fixture.unit_root.mkdir(parents=True, mode=0o750)
            (fixture.unit_root / "robin-highscores-api.service").write_text(
                "untrusted preexisting unit\n"
            )

        def wants_link(fixture: Fixture) -> None:
            fixture.unit_root.mkdir(parents=True, mode=0o750)
            wants = fixture.unit_root / "default.target.wants"
            wants.mkdir()
            (wants / "robin-highscores.target").symlink_to(
                fixture.unit_root / "robin-highscores.target"
            )

        def database_payload(fixture: Fixture) -> None:
            (fixture.data / "database/highscores.sqlite3").write_bytes(b"dirty database")

        def status_payload(fixture: Fixture) -> None:
            (fixture.data / "status").mkdir(mode=0o700)
            (fixture.data / "status/backup-status.json").write_text("{}\n")

        def unrelated_transaction(fixture: Fixture) -> None:
            (fixture.opt / f".deploy-prepared-{OLD}").write_text(
                "untrusted unrelated journal\n"
            )

        def replay_extra(fixture: Fixture) -> None:
            (fixture.data / "replays/unexpected.rhrec").write_bytes(b"unexpected replay")

        def campaign_symlink(fixture: Fixture) -> None:
            (fixture.data / "campaign-states/escape").symlink_to("/tmp")

        def backup_fifo(fixture: Fixture) -> None:
            (fixture.data / "backups").mkdir(mode=0o700)
            os.mkfifo(fixture.data / "backups/unexpected.fifo", 0o600)

        def hardlinked_database_payload(fixture: Fixture) -> None:
            first = fixture.data / "database/linked-a"
            first.write_bytes(b"hardlinked dirt")
            os.link(first, fixture.data / "database/linked-b")

        def unreadable_status_payload(fixture: Fixture) -> None:
            (fixture.data / "status").mkdir(mode=0o700)
            path = fixture.data / "status/unreadable"
            path.write_bytes(b"preserve me")
            path.chmod(0)

        def obsolete_root_once_directory(fixture: Fixture) -> None:
            (fixture.data / "root-once").mkdir(mode=0o700)

        def obsolete_root_once_file(fixture: Fixture) -> None:
            (fixture.data / "root-once").write_bytes(b"obsolete root authority")

        def obsolete_root_once_symlink(fixture: Fixture) -> None:
            (fixture.data / "root-once").symlink_to("/tmp")

        cases = (
            ("active-unit", active_unit),
            ("managed-unit-file", managed_unit_file),
            ("wants-link", wants_link),
            ("database-payload", database_payload),
            ("status-payload", status_payload),
            ("unrelated-transaction", unrelated_transaction),
            ("replay-extra", replay_extra),
            ("campaign-symlink", campaign_symlink),
            ("backup-fifo", backup_fifo),
            ("hardlinked-database-payload", hardlinked_database_payload),
            ("unreadable-status-payload", unreadable_status_payload),
            ("obsolete-root-once-directory", obsolete_root_once_directory),
            ("obsolete-root-once-file", obsolete_root_once_file),
            ("obsolete-root-once-symlink", obsolete_root_once_symlink),
        )
        wanted = os.environ.get("ROBIN_TX_DIRTY_CASE", "")
        if wanted:
            cases = tuple(case for case in cases if case[0] == wanted)
        for label, arrange in cases:
            with self.subTest(case=label):
                fixture = Fixture()
                try:
                    candidate = fixture.candidate(NEW)
                    arrange(fixture)
                    managed_before = tree_fingerprint(fixture.opt)
                    units_before = tree_fingerprint(fixture.unit_root)
                    state_before = tree_fingerprint(fixture.data)
                    active_before = {
                        unit: fixture.active(unit)
                        for unit in UNITS
                    }
                    enabled_before = {
                        unit: fixture.enabled(unit)
                        for unit in UNITS
                    }

                    result = fixture.deploy(NEW)

                    self.assert_failure(result)
                    self.assertEqual(fixture.current(), None)
                    self.assertFalse((fixture.releases / NEW).exists())
                    self.assertTrue(candidate.is_dir())
                    self.assertEqual(
                        self.without_activation_lock(tree_fingerprint(fixture.opt)),
                        self.without_activation_lock(managed_before),
                    )
                    self.assert_safe_activation_lock(fixture)
                    self.assertEqual(tree_fingerprint(fixture.unit_root), units_before)
                    self.assertEqual(tree_fingerprint(fixture.data), state_before)
                    self.assertEqual(
                        {unit: fixture.active(unit) for unit in UNITS}, active_before
                    )
                    self.assertEqual(
                        {unit: fixture.enabled(unit) for unit in UNITS}, enabled_before
                    )
                    self.assert_no_service_mutation(fixture)
                finally:
                    fixture.cleanup()

    def test_v1_release_manifest_extra_is_rejected_without_mutation(self) -> None:
        candidate = self.fixture.candidate(NEW)
        candidate.chmod(0o750)
        stale = candidate / "vps-release-manifest-v1.json"
        stale.write_text('{"schema_version":1}\n')
        stale.chmod(0o440)
        candidate.chmod(0o550)
        managed_before = tree_fingerprint(self.fixture.opt)
        units_before = tree_fingerprint(self.fixture.unit_root)
        state_before = tree_fingerprint(self.fixture.data)

        result = self.fixture.deploy(NEW)

        self.assert_failure(result)
        self.assertEqual(self.fixture.current(), None)
        self.assertFalse((self.fixture.releases / NEW).exists())
        self.assertEqual(
            self.without_activation_lock(tree_fingerprint(self.fixture.opt)),
            self.without_activation_lock(managed_before),
        )
        self.assert_safe_activation_lock()
        self.assertEqual(tree_fingerprint(self.fixture.unit_root), units_before)
        self.assertEqual(tree_fingerprint(self.fixture.data), state_before)
        self.assert_no_service_mutation()

    def test_publication_v2_extra_is_rejected_without_mutation(self) -> None:
        candidate = self.fixture.candidate(NEW)
        candidate.chmod(0o750)
        publication = candidate / "publication"
        publication.chmod(0o750)
        stale = publication / "publication-manifest-v2.json"
        stale.write_text('{"schema_version":2}\n')
        stale.chmod(0o440)
        publication.chmod(0o550)
        candidate.chmod(0o550)
        managed_before = tree_fingerprint(self.fixture.opt)
        units_before = tree_fingerprint(self.fixture.unit_root)
        state_before = tree_fingerprint(self.fixture.data)

        result = self.fixture.deploy(NEW)

        self.assert_failure(result)
        self.assertEqual(self.fixture.current(), None)
        self.assertFalse((self.fixture.releases / NEW).exists())
        self.assertEqual(
            self.without_activation_lock(tree_fingerprint(self.fixture.opt)),
            self.without_activation_lock(managed_before),
        )
        self.assert_safe_activation_lock()
        self.assertEqual(tree_fingerprint(self.fixture.unit_root), units_before)
        self.assertEqual(tree_fingerprint(self.fixture.data), state_before)
        self.assert_no_service_mutation()

    def test_candidate_schema_outside_current_v2_is_rejected_without_mutation(self) -> None:
        for schema in (DATABASE_SCHEMA_VERSION - 1, DATABASE_SCHEMA_VERSION + 1):
            with self.subTest(database_schema_version=schema):
                fixture = Fixture()
                try:
                    candidate = fixture.candidate(
                        NEW, database_schema_version=schema
                    )
                    managed_before = tree_fingerprint(fixture.opt)
                    units_before = tree_fingerprint(fixture.unit_root)
                    state_before = tree_fingerprint(fixture.data)

                    result = fixture.deploy(NEW)

                    self.assert_failure(result)
                    self.assertTrue(candidate.is_dir())
                    self.assertIsNone(fixture.current())
                    self.assertEqual(
                        self.without_activation_lock(tree_fingerprint(fixture.opt)),
                        self.without_activation_lock(managed_before),
                    )
                    self.assert_safe_activation_lock(fixture)
                    self.assertEqual(tree_fingerprint(fixture.unit_root), units_before)
                    self.assertEqual(tree_fingerprint(fixture.data), state_before)
                    self.assert_no_service_mutation(fixture)
                finally:
                    fixture.cleanup()

    def test_rollback_schema_mismatch_is_zero_mutation(self) -> None:
        self.deploy_upgrade()
        target = self.fixture.releases / OLD
        manifest = target / RELEASE_MANIFEST
        target.chmod(0o750)
        manifest.chmod(0o640)
        manifest.write_text(
            manifest.read_text().replace(
                f'"database_schema_version":{DATABASE_SCHEMA_VERSION}',
                f'"database_schema_version":{DATABASE_SCHEMA_VERSION - 1}',
            )
        )
        manifest.chmod(0o440)
        target.chmod(0o550)
        opt_before = tree_fingerprint(self.fixture.opt)
        units_before = tree_fingerprint(self.fixture.unit_root)
        data_before = tree_fingerprint(self.fixture.data)
        active_before = {unit: self.fixture.active(unit) for unit in UNITS}
        enabled_before = {unit: self.fixture.enabled(unit) for unit in UNITS}
        self.fixture.reset_events()

        result = self.fixture.rollback(OLD)

        self.assert_failure(result)
        self.assertEqual(tree_fingerprint(self.fixture.opt), opt_before)
        self.assertEqual(tree_fingerprint(self.fixture.unit_root), units_before)
        self.assertEqual(tree_fingerprint(self.fixture.data), data_before)
        self.assertEqual(
            {unit: self.fixture.active(unit) for unit in UNITS}, active_before
        )
        self.assertEqual(
            {unit: self.fixture.enabled(unit) for unit in UNITS}, enabled_before
        )
        self.assert_no_service_mutation()
        self.assertEqual(self.fixture.current(), f"releases/{NEW}")

    def test_clean_first_accepts_debian_empty_failed_is_enabled_for_absent_units(
        self,
    ) -> None:
        self.fixture.candidate(NEW)

        result = self.fixture.deploy(NEW, empty_missing_enabled=True)

        self.assert_success(result)
        self.assertEqual(self.fixture.current(), f"releases/{NEW}")
        for unit in UNITS:
            self.assertIn(
                f"systemctl.is-enabled.{unit}",
                self.fixture.events(),
            )

    def test_clean_first_deploy_rejects_inexact_raw_authority(self) -> None:
        def root_mode(fixture: Fixture) -> dict[str, str]:
            (fixture.data / "raw-content").chmod(0o500)
            return {}

        def edition_mode(fixture: Fixture) -> dict[str, str]:
            (fixture.data / "raw-content/demo").chmod(0o500)
            return {}

        def file_mode(fixture: Fixture) -> dict[str, str]:
            (fixture.data / "raw-content/demo/fixture.dat").chmod(0o400)
            return {}

        def extra_file(fixture: Fixture) -> dict[str, str]:
            edition = fixture.data / "raw-content/demo"
            edition.chmod(0o750)
            extra = edition / "unexpected.dat"
            extra.write_bytes(b"unexpected raw authority")
            extra.chmod(0o440)
            edition.chmod(0o550)
            return {}

        def extra_symlink(fixture: Fixture) -> dict[str, str]:
            edition = fixture.data / "raw-content/demo"
            edition.chmod(0o750)
            (edition / "escape").symlink_to("/tmp")
            edition.chmod(0o550)
            return {}

        def extra_fifo(fixture: Fixture) -> dict[str, str]:
            edition = fixture.data / "raw-content/demo"
            edition.chmod(0o750)
            os.mkfifo(edition / "unexpected.fifo", 0o440)
            edition.chmod(0o550)
            return {}

        def hardlink(fixture: Fixture) -> dict[str, str]:
            edition = fixture.data / "raw-content/demo"
            edition.chmod(0o750)
            os.link(edition / "fixture.dat", edition / "fixture.alias")
            edition.chmod(0o550)
            return {}

        def mixed_owner(fixture: Fixture) -> dict[str, str]:
            return {
                "mixed_owner_root": "/home/robinhood/.local/share/robin-highscores/raw-content"
            }

        def nested_mount(fixture: Fixture) -> dict[str, str]:
            return {
                "nested_mount": "/home/robinhood/.local/share/robin-highscores/raw-content/demo"
            }

        cases = (
            ("root-mode", root_mode),
            ("edition-mode", edition_mode),
            ("file-mode", file_mode),
            ("extra-file", extra_file),
            ("extra-symlink", extra_symlink),
            ("extra-fifo", extra_fifo),
            ("hardlink", hardlink),
            ("mixed-owner", mixed_owner),
            ("nested-mount", nested_mount),
        )
        for label, arrange in cases:
            with self.subTest(case=label):
                fixture = Fixture()
                try:
                    candidate = fixture.candidate(NEW)
                    arguments = arrange(fixture)
                    opt_before = tree_fingerprint(fixture.opt)
                    units_before = tree_fingerprint(fixture.unit_root)
                    state_before = tree_fingerprint(fixture.data)

                    result = fixture.deploy(NEW, **arguments)

                    self.assert_failure(result)
                    self.assertEqual(fixture.current(), None)
                    self.assertFalse((fixture.releases / NEW).exists())
                    self.assertTrue(candidate.is_dir())
                    self.assertEqual(
                        self.without_activation_lock(tree_fingerprint(fixture.opt)),
                        self.without_activation_lock(opt_before),
                    )
                    self.assert_safe_activation_lock(fixture)
                    self.assertEqual(tree_fingerprint(fixture.unit_root), units_before)
                    self.assertEqual(tree_fingerprint(fixture.data), state_before)
                    self.assert_no_service_mutation(fixture)
                finally:
                    fixture.cleanup()

    def test_raw_authority_is_revalidated_for_upgrade_resume_and_rollback(self) -> None:
        for operation in ("upgrade", "resume", "rollback"):
            with self.subTest(operation=operation):
                fixture = Fixture()
                try:
                    fixture.candidate(OLD)
                    self.assert_success(fixture.deploy(OLD))
                    source = OLD
                    if operation == "rollback":
                        fixture.candidate(NEW)
                        self.assert_success(fixture.deploy(NEW))
                        source = NEW
                    elif operation == "resume":
                        fixture.candidate(NEW)
                        interrupted = fixture.deploy(
                            NEW,
                            fail_event="mv.release-install",
                            fail_mode="after-sigkill",
                        )
                        self.assert_failure(interrupted)
                    raw_file = fixture.data / "raw-content/demo/fixture.dat"
                    raw_file.chmod(0o400)
                    fixture.reset_events()

                    if operation == "upgrade":
                        fixture.candidate(NEW)
                        result = fixture.deploy(NEW)
                    elif operation == "resume":
                        result = fixture.resume(NEW)
                    else:
                        result = fixture.rollback(OLD)

                    self.assert_failure(result)
                    self.assert_selected_active(source, fixture)
                    self.assertEqual(stat.S_IMODE(raw_file.lstat().st_mode), 0o400)
                    self.assert_no_service_mutation(fixture)
                finally:
                    fixture.cleanup()

    def test_bootstrap_tamper_and_unsafe_descriptor_modes_fail_before_mutation(self) -> None:
        cases = (
            ("manifest-bytes", "DEPLOY_BOOTSTRAP_SHA256SUMS", None),
            ("deploy-bytes", "deploy-release.sh", None),
            ("validator-bytes", "validate-release-bundle.sh", None),
            ("manifest-mode", "DEPLOY_BOOTSTRAP_SHA256SUMS", 0o600),
            ("deploy-mode", "deploy-release.sh", 0o700),
            ("validator-mode", "validate-release-bundle.sh", 0o700),
        )
        for label, name, mode in cases:
            with self.subTest(case=label):
                fixture = Fixture()
                try:
                    fixture.candidate(NEW)
                    path = fixture.bootstrap / name
                    fixture.bootstrap.chmod(0o700)
                    if mode is None:
                        path.chmod(0o600)
                        path.write_bytes(path.read_bytes() + b"\n# tampered\n")
                    else:
                        path.chmod(mode)
                    fixture.bootstrap.chmod(0o500)
                    result = fixture.deploy(NEW)
                    self.assert_failure(result)
                    self.assert_no_service_mutation(fixture)
                    self.assertFalse((fixture.releases / NEW).exists())
                finally:
                    fixture.cleanup()

    def test_duplicate_bootstrap_manifest_entry_is_rejected_even_with_oob_digest(self) -> None:
        self.fixture.candidate(NEW)
        manifest = self.fixture.bootstrap / "DEPLOY_BOOTSTRAP_SHA256SUMS"
        self.fixture.bootstrap.chmod(0o700)
        manifest.chmod(0o600)
        first_line = manifest.read_text().splitlines()[0]
        manifest.write_text(manifest.read_text() + first_line + "\n")
        manifest.chmod(0o400)
        self.fixture.bootstrap.chmod(0o500)
        self.fixture.bootstrap_digest = digest(manifest)
        result = self.fixture.deploy(NEW)
        self.assert_failure(result)
        self.assert_no_service_mutation()
        self.assertFalse((self.fixture.releases / NEW).exists())

    def test_activation_lock_metadata_rejections_precede_transaction_mutation(self) -> None:
        for attack in ("mode", "symlink", "hardlink"):
            with self.subTest(attack=attack):
                fixture = Fixture()
                try:
                    fixture.candidate(NEW)
                    lock = fixture.opt / "activation.lock"
                    if attack == "mode":
                        lock.write_text("")
                        lock.chmod(0o600)
                        lock.chmod(0o640)
                    elif attack == "symlink":
                        lock.symlink_to("/etc/passwd")
                    else:
                        lock.write_text("")
                        lock.chmod(0o600)
                        os.link(lock, fixture.opt / "activation.lock.alias")
                    result = fixture.deploy(NEW)
                    self.assert_failure(result)
                    self.assert_no_service_mutation(fixture)
                    self.assertIsNone(fixture.current())
                finally:
                    fixture.cleanup()

    def test_final_lock_inode_recheck_failure_restores_old_without_false_restart(self) -> None:
        self.deploy_old()
        self.fixture.reset_events()
        self.fixture.candidate(NEW)
        process = self.fixture.popen(
            self.fixture.deploy_command(NEW),
            pause_event="mv.prebackup-receipt",
            pause_phase="after",
        )
        deadline = time.monotonic() + 15
        while not (self.fixture.state / "pause.ready").exists():
            if process.poll() is not None:
                stdout, stderr = process.communicate()
                self.fail(f"deploy exited before final lock race: {stdout}\n{stderr}")
            self.assertLess(time.monotonic(), deadline)
            time.sleep(0.02)
        lock = self.fixture.opt / "activation.lock"
        pinned = self.fixture.opt / "activation.lock.pinned"
        os.rename(lock, pinned)
        lock.write_text("")
        lock.chmod(0o600)
        (self.fixture.state / "pause.release").write_text("continue\n")
        stdout, stderr = process.communicate(timeout=TRANSACTION_TIMEOUT_SECONDS)
        self.assertNotEqual(process.returncode, 0, f"{stdout}\n{stderr}")
        self.assert_selected_active(OLD)
        events = self.fixture.events()
        self.assertNotIn("admin.migrate", events)
        os.replace(pinned, lock)

    def test_descriptor_closed_reused_and_direct_path_are_rejected(self) -> None:
        self.fixture.candidate(NEW)
        bootstrap = f"/home/robinhood/{self.fixture.bootstrap.name}"
        candidate = f"/home/robinhood/.local/opt/robin-highscores/releases/{NEW}.partial"
        commands = (
            [
                "/bin/sh", f"{bootstrap}/deploy-release.sh", candidate, NEW, SUMS,
                self.fixture.bootstrap_digest,
                f"{bootstrap}/DEPLOY_BOOTSTRAP_SHA256SUMS",
                f"{bootstrap}/validate-release-bundle.sh",
                f"/home/robinhood/{self.fixture.manifestctl_dir.name}/robin-highscores-manifestctl",
                "/home/robinhood/.local/opt/robin-highscores/activation.lock",
            ],
            [
                "/bin/bash", "-c",
                'exec 3<"$1"; exec 4<"$2"; exec 5<"$3"; exec 6<"$4"; exec 7<>"$5"; '
                'exec /bin/sh /proc/self/fd/3 "$6" "$7" "$8" "$9" /proc/self/fd/8 /proc/self/fd/5 /proc/self/fd/6 /proc/self/fd/7',
                "closed-fd", f"{bootstrap}/deploy-release.sh",
                f"{bootstrap}/DEPLOY_BOOTSTRAP_SHA256SUMS",
                f"{bootstrap}/validate-release-bundle.sh",
                f"/home/robinhood/{self.fixture.manifestctl_dir.name}/robin-highscores-manifestctl",
                "/home/robinhood/.local/opt/robin-highscores/activation.lock",
                candidate, NEW, SUMS,
                self.fixture.bootstrap_digest,
            ],
            [
                "/bin/bash", "-c",
                'exec 3<"$1"; exec 4<"$2"; exec 5<"$3"; exec 6<"$4"; exec 7<>"$5"; '
                'exec /bin/sh /proc/self/fd/3 "$6" "$7" "$8" "$9" /proc/self/fd/4 /proc/self/fd/4 /proc/self/fd/6 /proc/self/fd/7',
                "reused-fd", f"{bootstrap}/deploy-release.sh",
                f"{bootstrap}/DEPLOY_BOOTSTRAP_SHA256SUMS",
                f"{bootstrap}/validate-release-bundle.sh",
                f"/home/robinhood/{self.fixture.manifestctl_dir.name}/robin-highscores-manifestctl",
                "/home/robinhood/.local/opt/robin-highscores/activation.lock",
                candidate, NEW, SUMS,
                self.fixture.bootstrap_digest,
            ],
        )
        for command in commands:
            with self.subTest(command=command[0:3]):
                result = self.fixture.run(command)
                self.assert_failure(result)
                self.assert_no_service_mutation()
                self.assertFalse((self.fixture.releases / NEW).exists())

        cloexec = r'''
import os, sys
script = os.open(sys.argv[1], os.O_RDONLY | os.O_CLOEXEC)
manifest = os.open(sys.argv[2], os.O_RDONLY | os.O_CLOEXEC)
validator = os.open(sys.argv[3], os.O_RDONLY | os.O_CLOEXEC)
manifest_tool = os.open(sys.argv[4], os.O_RDONLY | os.O_CLOEXEC)
lock = os.open(sys.argv[5], os.O_RDWR | os.O_CLOEXEC)
os.execv("/bin/sh", ["/bin/sh", f"/proc/self/fd/{script}", sys.argv[6], sys.argv[7], sys.argv[8], sys.argv[9], f"/proc/self/fd/{manifest}", f"/proc/self/fd/{validator}", f"/proc/self/fd/{manifest_tool}", f"/proc/self/fd/{lock}"])
'''
        result = self.fixture.run([
            "/usr/bin/python3", "-c", cloexec,
            f"{bootstrap}/deploy-release.sh",
            f"{bootstrap}/DEPLOY_BOOTSTRAP_SHA256SUMS",
            f"{bootstrap}/validate-release-bundle.sh",
            f"/home/robinhood/{self.fixture.manifestctl_dir.name}/robin-highscores-manifestctl",
            "/home/robinhood/.local/opt/robin-highscores/activation.lock",
            candidate, NEW, SUMS, self.fixture.bootstrap_digest,
        ])
        self.assert_failure(result)
        self.assert_no_service_mutation()

    def test_bootstrap_directory_path_swap_cannot_substitute_pinned_bytes(self) -> None:
        self.fixture.candidate(NEW)
        process = self.fixture.popen(
            self.fixture.deploy_command(NEW),
            pause_event="sha.bootstrap-validator",
            pause_phase="after",
        )
        deadline = time.monotonic() + 30
        while not (self.fixture.state / "pause.ready").exists():
            if process.poll() is not None:
                stdout, stderr = process.communicate()
                self.fail(f"bootstrap command exited before swap: {stdout}\n{stderr}")
            self.assertLess(time.monotonic(), deadline)
            time.sleep(0.02)
        pinned = self.home_path(".robin-highscores-bootstrap.pinned")
        os.rename(self.fixture.bootstrap, pinned)
        self.fixture.bootstrap.mkdir(mode=0o700)
        for name in (
            "deploy-release.sh", "rollback-release.sh", "validate-release-bundle.sh",
            "DEPLOY_BOOTSTRAP_SHA256SUMS",
        ):
            replacement = self.fixture.bootstrap / name
            replacement.write_text("#!/bin/sh\nexit 99\n")
            replacement.chmod(0o500 if name != "DEPLOY_BOOTSTRAP_SHA256SUMS" else 0o400)
        self.fixture.bootstrap.chmod(0o500)
        (self.fixture.state / "pause.release").write_text("continue\n")
        stdout, stderr = process.communicate(timeout=20)
        try:
            self.assertEqual(process.returncode, 0, f"{stdout}\n{stderr}")
            self.assert_selected_active(NEW)
        finally:
            self.fixture.bootstrap.chmod(0o700)
            shutil.rmtree(self.fixture.bootstrap)
            os.rename(pinned, self.fixture.bootstrap)

    def home_path(self, relative: str) -> Path:
        return self.fixture.home / relative

    def test_upgrade_prebackup_timer_migration_and_receipt_cleanup(self) -> None:
        self.deploy_old()
        self.fixture.reset_events()
        self.fixture.candidate(NEW)
        self.assert_success(self.fixture.deploy(NEW))
        self.assert_selected_active(NEW)
        self.assertTrue((self.fixture.releases / OLD).is_dir())
        self.assertFalse(
            (self.fixture.opt / f".deploy-source-backup-{NEW}.receipt-v2").exists()
        )
        self.assertFalse(
            (self.fixture.opt / f".deploy-target-backup-{NEW}.receipt-v2").exists()
        )
        events = self.fixture.events()
        self.assertLess(
            events.index("mv.activation-journal"),
            events.index("systemctl.disable.robin-highscores-backup.timer"),
        )
        self.assertLess(events.index("systemctl.disable.robin-highscores-backup.timer"), events.index("admin.migrate"))
        self.assertLess(events.index("systemctl.start.robin-highscores-backup.service"), events.index("admin.migrate"))
        self.assertIn("mv.prebackup-receipt", events)

    def test_coherent_inactive_source_still_gets_verified_prebackup(self) -> None:
        for operation in ("deploy", "rollback"):
            with self.subTest(operation=operation):
                fixture = Fixture()
                try:
                    fixture.candidate(OLD)
                    self.assert_success(fixture.deploy(OLD))
                    if operation == "rollback":
                        fixture.candidate(NEW)
                        self.assert_success(fixture.deploy(NEW))
                    for unit in UNITS:
                        fixture.set_active(unit, "inactive")
                    fixture.set_enabled("robin-highscores.target", "disabled")
                    fixture.set_enabled("robin-highscores-backup.timer", "disabled")
                    fixture.reset_events()
                    if operation == "deploy":
                        fixture.candidate(NEW)
                        result = fixture.deploy(NEW)
                        target = NEW
                    else:
                        result = fixture.rollback(OLD)
                        target = OLD
                    self.assert_success(result)
                    events = fixture.events()
                    backup = events.index(
                        "systemctl.start.robin-highscores-backup.service"
                    )
                    boundary = events.index(
                        "admin.migrate" if operation == "deploy" else "mv.current-select"
                    )
                    self.assertLess(backup, boundary)
                    self.assertEqual(fixture.current(), f"releases/{target}")
                    self.assert_transaction_artifacts_absent(operation, target, fixture)
                finally:
                    fixture.cleanup()

    def test_source_backup_is_offline_and_precedes_migration_or_selection(self) -> None:
        writers = (
            "robin-highscores.target",
            "robin-highscores-api.service",
            "robin-highscores-worker.service",
        )
        for operation in ("deploy", "rollback"):
            with self.subTest(operation=operation):
                fixture = Fixture()
                try:
                    fixture.candidate(OLD)
                    self.assert_success(fixture.deploy(OLD))
                    if operation == "rollback":
                        fixture.candidate(NEW)
                        self.assert_success(fixture.deploy(NEW))
                    fixture.reset_events()
                    if operation == "deploy":
                        fixture.candidate(NEW)
                        result = fixture.deploy(NEW)
                        boundary_event = "admin.migrate"
                    else:
                        result = fixture.rollback(OLD)
                        boundary_event = "mv.current-select"
                    self.assert_success(result)
                    events = fixture.events()
                    backup = events.index(
                        "systemctl.start.robin-highscores-backup.service"
                    )
                    self.assertLess(
                        events.index("curl.healthz"),
                        events.index("systemctl.disable.robin-highscores-backup.timer"),
                    )
                    self.assertLess(
                        events.index("curl.readyz"),
                        events.index("systemctl.disable.robin-highscores-backup.timer"),
                    )
                    for unit in writers:
                        stopped = events.index(f"systemctl.stop.{unit}")
                        self.assertLess(stopped, backup, unit)
                        inactive_proofs = [
                            index for index, event in enumerate(events)
                            if event == f"systemctl.show.ActiveState.{unit}"
                            and stopped < index < backup
                        ]
                        self.assertTrue(inactive_proofs, unit)
                    self.assertLess(backup, events.index(boundary_event))
                finally:
                    fixture.cleanup()

    def test_target_backup_precedes_api_worker_and_enablement(self) -> None:
        for operation in ("first-deploy", "upgrade", "rollback"):
            with self.subTest(operation=operation):
                fixture = Fixture()
                try:
                    if operation != "first-deploy":
                        fixture.candidate(OLD)
                        self.assert_success(fixture.deploy(OLD))
                    if operation == "rollback":
                        fixture.candidate(NEW)
                        self.assert_success(fixture.deploy(NEW))
                    fixture.reset_events()
                    if operation == "first-deploy":
                        fixture.candidate(OLD)
                        result = fixture.deploy(OLD)
                    elif operation == "upgrade":
                        fixture.candidate(NEW)
                        result = fixture.deploy(NEW)
                    else:
                        result = fixture.rollback(OLD)
                    self.assert_success(result)
                    events = fixture.events()
                    selection = events.index("mv.current-select")
                    target_backup = max(
                        index for index, event in enumerate(events)
                        if event
                        == "systemctl.start.robin-highscores-backup.service"
                    )
                    api_start = next(
                        index for index, event in enumerate(events)
                        if index > selection
                        and event
                        == "systemctl.start.robin-highscores-api.service"
                    )
                    ready = next(
                        index for index, event in enumerate(events)
                        if index > api_start and event == "curl.readyz"
                    )
                    worker_start = next(
                        index for index, event in enumerate(events)
                        if index > selection
                        and event
                        == "systemctl.start.robin-highscores-worker.service"
                    )
                    enable_target = next(
                        index for index, event in enumerate(events)
                        if index > selection
                        and event
                        == "systemctl.enable.robin-highscores.target"
                    )
                    self.assertLess(target_backup, selection)
                    self.assertLess(selection, api_start)
                    self.assertLess(api_start, ready)
                    self.assertLess(ready, worker_start)
                    self.assertLess(worker_start, enable_target)
                    for unit in (
                        "robin-highscores.target",
                        "robin-highscores-api.service",
                        "robin-highscores-worker.service",
                    ):
                        starts_before_backup = [
                            index for index, event in enumerate(events)
                            if selection < index < target_backup
                            and event == f"systemctl.start.{unit}"
                        ]
                        self.assertEqual(starts_before_backup, [], unit)
                finally:
                    fixture.cleanup()

    def test_offline_prebackup_failure_restores_exact_source_runtime(self) -> None:
        for operation in ("deploy", "rollback"):
            with self.subTest(operation=operation):
                fixture = Fixture()
                try:
                    fixture.candidate(OLD)
                    self.assert_success(fixture.deploy(OLD))
                    source = OLD
                    if operation == "rollback":
                        fixture.candidate(NEW)
                        self.assert_success(fixture.deploy(NEW))
                        source = NEW
                    fixture.reset_events()
                    if operation == "deploy":
                        fixture.candidate(NEW)
                        result = fixture.deploy(
                            NEW,
                            fail_event="systemctl.start.robin-highscores-backup.service",
                            fail_mode="before-error",
                        )
                    else:
                        result = fixture.rollback(
                            OLD,
                            fail_event="systemctl.start.robin-highscores-backup.service",
                            fail_mode="before-error",
                        )
                    self.assert_failure(result)
                    self.assert_selected_active(source, fixture)
                    for unit in UNITS:
                        self.assertIn(
                            source, (fixture.unit_root / unit).read_text()
                        )
                    self.assertNotIn("admin.migrate", fixture.events())
                finally:
                    fixture.cleanup()

    def test_preactivation_error_restores_old_runtime_then_resume_all_old(self) -> None:
        self.deploy_old()
        self.fixture.reset_events()
        self.fixture.candidate(NEW)
        failed = self.fixture.deploy(
            NEW,
            fail_event="systemctl.stop.robin-highscores-api.service",
            fail_mode="before-error",
        )
        self.assert_failure(failed)
        self.assert_selected_active(OLD)
        self.assertTrue((self.fixture.releases / NEW).is_dir())
        self.assertFalse((self.fixture.releases / f"{NEW}.partial").exists())
        self.assert_success(self.fixture.resume(NEW))
        self.assert_selected_active(NEW)

    def test_sigkill_after_migration_preserves_receipt_and_repeated_resume(self) -> None:
        self.deploy_old()
        self.fixture.reset_events()
        self.fixture.candidate(NEW)
        failed = self.fixture.deploy(
            NEW, fail_event="admin.migrate", fail_mode="after-sigkill"
        )
        self.assert_failure(failed)
        receipt = self.fixture.opt / f".deploy-source-backup-{NEW}.receipt-v2"
        self.assertTrue(receipt.is_file())
        self.assertEqual(self.fixture.current(), f"releases/{OLD}")
        first_count = self.fixture.migration_count()
        self.assert_success(self.fixture.resume(NEW))
        self.assertGreater(self.fixture.migration_count(), first_count)
        after_recovery = self.fixture.migration_count()
        self.assert_success(self.fixture.resume(NEW))
        self.assertEqual(self.fixture.migration_count(), after_recovery)
        self.assert_selected_active(NEW)

    def test_mixed_unit_failure_converges_new_stopped_then_resumes(self) -> None:
        self.deploy_old()
        self.fixture.reset_events()
        self.fixture.candidate(NEW)
        failed = self.fixture.deploy(
            NEW,
            fail_event="mv.unit.robin-highscores-api.service",
            fail_mode="before-error",
        )
        self.assert_failure(failed)
        self.assert_selected_stopped(NEW)
        for unit in UNITS:
            self.assertIn(NEW, (self.fixture.unit_root / unit).read_text())
        migrations = self.fixture.migration_count()
        self.assert_success(self.fixture.resume(NEW))
        self.assertEqual(self.fixture.migration_count(), migrations)
        self.assert_selected_active(NEW)

    def test_current_side_effect_then_error_converges_new_stopped(self) -> None:
        self.deploy_old()
        self.fixture.reset_events()
        self.fixture.candidate(NEW)
        failed = self.fixture.deploy(
            NEW, fail_event="mv.current-select", fail_mode="after-error"
        )
        self.assert_failure(failed)
        self.assert_selected_stopped(NEW)
        self.assert_success(self.fixture.resume(NEW))
        self.assert_selected_active(NEW)

    def test_current_selection_consumed_temp_and_absent_selector_is_reconstructed(self) -> None:
        self.deploy_old()
        self.fixture.reset_events()
        self.fixture.candidate(NEW)
        failed = self.fixture.deploy(
            NEW, fail_event="mv.current-select", fail_mode="after-remove-error"
        )
        self.assert_failure(failed)
        self.assert_selected_stopped(NEW)
        for unit in UNITS:
            self.assertIn(NEW, (self.fixture.unit_root / unit).read_text())

    def test_release_noreplace_race_preserves_independent_winner(self) -> None:
        candidate = self.fixture.candidate(NEW)
        failed = self.fixture.deploy(
            NEW, fail_event="mv.release-install", fail_mode="race"
        )
        self.assert_failure(failed)
        self.assertTrue(candidate.is_dir())
        self.assertTrue((self.fixture.releases / NEW / ".race-winner").is_file())
        self.assertIsNone(self.fixture.current())
        self.assertFalse(any(event.startswith("systemctl.start") for event in self.fixture.events()))

    def test_typed_promotion_side_effect_then_error_requires_explicit_resume(self) -> None:
        candidate = self.fixture.candidate(NEW)
        failed = self.fixture.deploy(
            NEW, fail_event="mv.release-install", fail_mode="after-error"
        )
        self.assert_failure(failed)
        self.assertTrue(candidate.exists())
        self.assertTrue((self.fixture.releases / NEW).is_dir())
        self.assertIsNone(self.fixture.current())
        self.assert_no_service_mutation()
        self.assert_success(self.fixture.resume(NEW))
        self.assert_selected_active(NEW)
        self.assertTrue(candidate.exists())

    def test_sigkill_after_sealing_private_partial_reuses_exact_snapshot(self) -> None:
        candidate = self.fixture.candidate(NEW)
        failed = self.fixture.deploy(
            NEW, fail_event="validator", fail_mode="after-sigkill"
        )
        self.assert_failure(failed)
        partial = self.fixture.releases / f"{NEW}.partial"
        self.assertTrue(partial.is_dir())
        self.assertEqual(stat.S_IMODE(partial.stat().st_mode), 0o550)
        self.assertTrue(candidate.is_dir())
        self.assertFalse((self.fixture.releases / NEW).exists())
        self.assertIsNone(self.fixture.current())
        self.assert_success(self.fixture.deploy(NEW))
        self.assertFalse(partial.exists())
        self.assertFalse(candidate.exists())
        self.assert_selected_active(NEW)

    def test_unit_move_side_effect_then_error_is_accepted_only_after_exact_proof(self) -> None:
        self.deploy_old()
        self.fixture.reset_events()
        self.fixture.candidate(NEW)
        result = self.fixture.deploy(
            NEW,
            fail_event="mv.unit.robin-highscores-api.service",
            fail_mode="after-error",
        )
        self.assert_success(result)
        self.assert_selected_active(NEW)
        for unit in UNITS:
            self.assertIn(NEW, (self.fixture.unit_root / unit).read_text())

    def test_release_root_sync_failure_resumes_installed_without_recopy(self) -> None:
        candidate = self.fixture.candidate(NEW)
        failed = self.fixture.deploy(
            NEW, fail_event="sync.release-root", fail_mode="before-error"
        )
        self.assert_failure(failed)
        self.assertTrue((self.fixture.releases / NEW).is_dir())
        self.assertTrue(candidate.is_dir())
        self.assert_success(self.fixture.resume(NEW))
        self.assert_selected_active(NEW)
        # Resume intentionally does not consume an unrelated incoming path.
        self.assertTrue(candidate.is_dir())

    def test_low_disk_rejects_before_candidate_consumption_or_service_mutation(self) -> None:
        candidate = self.fixture.candidate(NEW)
        result = self.fixture.deploy(NEW, available_bytes=1)
        self.assert_failure(result)
        self.assertTrue(candidate.is_dir())
        self.assertFalse((self.fixture.releases / NEW).exists())
        self.assertIsNone(self.fixture.current())
        self.assert_no_service_mutation()
        self.assertIn("rm.release-partial", self.fixture.events())

    def test_candidate_consumption_precedes_prebackup_and_all_service_mutation(self) -> None:
        self.deploy_old()
        self.fixture.reset_events()
        self.fixture.candidate(NEW)
        self.assert_success(self.fixture.deploy(NEW))
        events = self.fixture.events()
        consumed = events.index("rm.incoming-consuming")
        mutations = [
            index for index, event in enumerate(events)
            if event.startswith((
                "systemctl.start", "systemctl.stop",
                "systemctl.enable", "systemctl.disable",
            ))
        ]
        self.assertTrue(mutations)
        self.assertLess(consumed, min(mutations))

    def test_installed_release_resume_finishes_exact_incoming_consumption(self) -> None:
        boundaries = (
            ("mv.release-install", "after-sigkill"),
            ("sync.release-root", "before-sigkill"),
            ("sync.release-root", "after-sigkill"),
            ("mv.incoming-consuming", "before-sigkill"),
            ("mv.incoming-consuming", "after-sigkill"),
            ("sync.incoming-root.quarantine", "before-sigkill"),
            ("sync.incoming-root.quarantine", "after-sigkill"),
            ("rm.incoming-consuming", "before-sigkill"),
            ("rm.incoming-consuming", "after-sigkill"),
            ("sync.incoming-root.consumed", "before-sigkill"),
            ("sync.incoming-root.consumed", "after-sigkill"),
        )
        for event, mode in selected_boundaries(boundaries):
            with self.subTest(event=event, mode=mode):
                fixture = Fixture()
                try:
                    fixture.candidate(OLD)
                    self.assert_success(fixture.deploy(OLD))
                    fixture.candidate(NEW)
                    fixture.reset_events()
                    failed = fixture.deploy(
                        NEW, fail_event=event, fail_mode=mode
                    )
                    self.assert_failure(failed)
                    self.assertTrue((fixture.releases / NEW).is_dir())
                    self.assert_success(fixture.resume(NEW))
                    self.assert_selected_active(NEW, fixture)
                    self.assert_transaction_artifacts_absent("deploy", NEW, fixture)
                    self.assertFalse((fixture.releases / f"{NEW}.partial").exists())
                finally:
                    fixture.cleanup()

    def test_current_absent_resume_requires_exact_clean_authenticated_state(self) -> None:
        def external_final(fixture: Fixture) -> Path:
            incoming = fixture.candidate(NEW)
            incoming.rename(fixture.releases / NEW)
            return incoming

        allowed = Fixture()
        try:
            incoming = external_final(allowed)
            self.assert_success(allowed.resume(NEW))
            self.assert_selected_active(NEW, allowed)
            self.assertFalse(incoming.exists())
            self.assert_transaction_artifacts_absent("deploy", NEW, allowed)
        finally:
            allowed.cleanup()

        def database_payload(fixture: Fixture) -> None:
            (fixture.data / "database/highscores.sqlite3").write_bytes(b"dirty database")

        def status_residue(fixture: Fixture) -> None:
            (fixture.data / "status").mkdir(mode=0o700)

        def other_journal(fixture: Fixture) -> None:
            (fixture.opt / f".deploy-prepared-{OLD}").write_text("other transaction\n")

        def other_candidate(fixture: Fixture) -> None:
            fixture.candidate(OLD)

        def other_quarantine(fixture: Fixture) -> None:
            (fixture.incoming / f".{OLD}.consuming").mkdir(mode=0o550)

        def other_source_consume(fixture: Fixture) -> None:
            (fixture.incoming / f".sources-{OLD}").mkdir(mode=0o700)

        def current_temporary(fixture: Fixture) -> None:
            (fixture.opt / f".current-deploy-{OLD}").symlink_to(f"releases/{OLD}")

        def unit_stage(fixture: Fixture) -> None:
            fixture.unit_root.mkdir(parents=True, mode=0o750)
            (fixture.unit_root / f".robin-highscores-deploy-{OLD}.stage").mkdir(
                mode=0o700
            )

        def other_receipt(fixture: Fixture) -> None:
            receipt = fixture.opt / f".deploy-source-backup-{OLD}.receipt-v2"
            receipt.write_text("other receipt\n")
            receipt.chmod(0o400)

        def other_release(fixture: Fixture) -> None:
            other = fixture.candidate(OLD)
            other.rename(fixture.releases / OLD)

        cases = (
            ("database-payload", database_payload),
            ("status-residue", status_residue),
            ("other-journal", other_journal),
            ("other-candidate", other_candidate),
            ("other-quarantine", other_quarantine),
            ("other-source-consume", other_source_consume),
            ("current-temporary", current_temporary),
            ("unit-stage", unit_stage),
            ("other-receipt", other_receipt),
            ("other-release", other_release),
        )
        for label, arrange in cases:
            with self.subTest(case=label):
                fixture = Fixture()
                try:
                    external_final(fixture)
                    arrange(fixture)
                    opt_before = tree_fingerprint(fixture.opt)
                    units_before = tree_fingerprint(fixture.unit_root)
                    state_before = tree_fingerprint(fixture.data)

                    result = fixture.resume(NEW)

                    self.assert_failure(result)
                    self.assertEqual(fixture.current(), None)
                    self.assertEqual(
                        self.without_activation_lock(tree_fingerprint(fixture.opt)),
                        self.without_activation_lock(opt_before),
                    )
                    self.assert_safe_activation_lock(fixture)
                    self.assertEqual(tree_fingerprint(fixture.unit_root), units_before)
                    self.assertEqual(tree_fingerprint(fixture.data), state_before)
                    self.assert_no_service_mutation(fixture)
                finally:
                    fixture.cleanup()

    def test_installed_resume_rejects_substituted_or_changed_release_partial(self) -> None:
        for attack in ("inode", "content"):
            with self.subTest(attack=attack):
                fixture = Fixture()
                try:
                    fixture.candidate(OLD)
                    self.assert_success(fixture.deploy(OLD))
                    candidate = fixture.candidate(NEW)
                    fixture.reset_events()
                    failed = fixture.deploy(
                        NEW,
                        fail_event="mv.release-install",
                        fail_mode="after-sigkill",
                    )
                    self.assert_failure(failed)
                    if attack == "inode":
                        preserved = fixture.releases / f".{NEW}.preserved"
                        candidate.rename(preserved)
                        replacement = fixture.candidate(NEW)
                    else:
                        preserved = candidate
                        replacement = candidate
                        source_commit = replacement / "SOURCE_COMMIT"
                        source_commit.chmod(0o640)
                        source_commit.write_text(THIRD + "\n")
                        source_commit.chmod(0o440)
                    result = fixture.resume(NEW)
                    self.assert_failure(result)
                    self.assertTrue(preserved.is_dir())
                    self.assertTrue(replacement.is_dir())
                    self.assertEqual(fixture.current(), f"releases/{OLD}")
                    self.assert_no_service_mutation(fixture)
                finally:
                    fixture.cleanup()

    def test_api_and_worker_notify_failures_converge_selected_release_stopped(self) -> None:
        for event in (
            "systemctl.start.robin-highscores-api.service",
            "systemctl.start.robin-highscores-worker.service",
        ):
            with self.subTest(event=event):
                fixture = Fixture()
                try:
                    fixture.candidate(OLD)
                    self.assert_success(fixture.deploy(OLD))
                    fixture.reset_events()
                    fixture.candidate(NEW)
                    result = fixture.deploy(
                        NEW, fail_event=event, fail_mode="after-error"
                    )
                    self.assert_failure(result)
                    self.assertEqual(fixture.current(), f"releases/{NEW}")
                    for unit in UNITS:
                        self.assertEqual(fixture.active(unit), "inactive", unit)
                        self.assertIn(NEW, (fixture.unit_root / unit).read_text())
                    self.assertEqual(fixture.enabled("robin-highscores.target"), "disabled")
                    self.assertEqual(fixture.enabled("robin-highscores-backup.timer"), "disabled")
                    self.assertTrue(
                        (
                            fixture.opt
                            / f".deploy-source-backup-{NEW}.receipt-v2"
                        ).is_file()
                    )
                    self.assert_success(fixture.resume(NEW))
                finally:
                    fixture.cleanup()

    def test_timer_disable_and_stop_side_effect_errors_restore_old_runtime(self) -> None:
        for event in (
            "systemctl.disable.robin-highscores-backup.timer",
            "systemctl.stop.robin-highscores-backup.timer",
        ):
            with self.subTest(event=event):
                fixture = Fixture()
                try:
                    fixture.candidate(OLD)
                    self.assert_success(fixture.deploy(OLD))
                    fixture.reset_events()
                    fixture.candidate(NEW)
                    result = fixture.deploy(
                        NEW, fail_event=event, fail_mode="after-error"
                    )
                    self.assert_failure(result)
                    self.assertEqual(fixture.current(), f"releases/{OLD}")
                    self.assertEqual(fixture.active("robin-highscores-backup.timer"), "active")
                    self.assertEqual(fixture.enabled("robin-highscores-backup.timer"), "enabled")
                    self.assertEqual(fixture.active("robin-highscores-api.service"), "active")
                finally:
                    fixture.cleanup()

    def test_stale_receipt_status_hash_blocks_stopped_resume(self) -> None:
        self.deploy_old()
        self.fixture.reset_events()
        self.fixture.candidate(NEW)
        result = self.fixture.deploy(
            NEW, fail_event="admin.migrate", fail_mode="after-sigkill"
        )
        self.assert_failure(result)
        receipt = self.fixture.opt / f".deploy-source-backup-{NEW}.receipt-v2"
        self.assertTrue(receipt.is_file())
        status = self.fixture.data / "status/backup-status.json"
        status.chmod(0o600)
        status.write_text('{"verified":true,"generation":999}\n')
        status.chmod(0o400)
        migrations = self.fixture.migration_count()
        result = self.fixture.resume(NEW)
        self.assert_failure(result)
        self.assertEqual(self.fixture.migration_count(), migrations)
        self.assertEqual(self.fixture.current(), f"releases/{OLD}")
        for unit in UNITS:
            self.assertEqual(self.fixture.active(unit), "inactive", unit)

    def test_two_upgrade_backup_rotations_refresh_generation_and_clean_receipts(self) -> None:
        self.deploy_upgrade()
        first_generation = int(
            (self.fixture.state / "meta/backup-generation").read_text()
        )
        self.fixture.candidate(THIRD)
        self.assert_success(self.fixture.deploy(THIRD))
        second_generation = int(
            (self.fixture.state / "meta/backup-generation").read_text()
        )
        self.assertEqual(second_generation, first_generation + 2)
        self.assert_selected_active(THIRD)
        self.assertFalse(
            (
                self.fixture.opt
                / f".deploy-source-backup-{THIRD}.receipt-v2"
            ).exists()
        )
        self.assertFalse(
            (
                self.fixture.opt
                / f".deploy-target-backup-{THIRD}.receipt-v2"
            ).exists()
        )
        for commit in (OLD, NEW, THIRD):
            self.assertTrue((self.fixture.releases / commit).is_dir())

    def test_symlink_mount_and_mixed_owner_reject_before_systemctl_mutation(self) -> None:
        for attack in ("symlink", "mount", "owner"):
            with self.subTest(attack=attack):
                fixture = self.fixture if attack == "symlink" else Fixture()
                try:
                    candidate = fixture.candidate(NEW)
                    kwargs: dict[str, str] = {}
                    if attack == "symlink":
                        candidate.chmod(0o750)
                        (candidate / "external").symlink_to("/etc/passwd")
                        candidate.chmod(0o550)
                    elif attack == "mount":
                        kwargs["nested_mount"] = (
                            f"/home/robinhood/.local/opt/robin-highscores/releases/{NEW}.partial/nested"
                        )
                    else:
                        kwargs["mixed_owner_root"] = (
                            f"/home/robinhood/.local/opt/robin-highscores/releases/{NEW}.partial"
                        )
                    result = fixture.deploy(NEW, **kwargs)
                    self.assert_failure(result)
                    mutating = (
                        event for event in fixture.events()
                        if event.startswith(("systemctl.start", "systemctl.stop", "systemctl.enable", "systemctl.disable"))
                    )
                    self.assertEqual(list(mutating), [])
                    self.assertTrue(candidate.is_dir())
                finally:
                    if fixture is not self.fixture:
                        fixture.cleanup()

    def test_activation_lock_rejects_concurrent_resume(self) -> None:
        self.deploy_old()
        self.fixture.reset_events()
        self.fixture.candidate(NEW)
        command = self.fixture.deploy_command(NEW)
        process = self.fixture.popen(command, pause_event="admin.migrate")
        deadline = time.monotonic() + 10
        while not (self.fixture.state / "pause.ready").exists():
            if process.poll() is not None:
                stdout, stderr = process.communicate()
                self.fail(f"paused deploy exited early: {process.returncode}\n{stdout}\n{stderr}")
            self.assertLess(time.monotonic(), deadline, "timed out awaiting deployment pause")
            time.sleep(0.02)
        concurrent = self.fixture.resume(NEW)
        self.assert_failure(concurrent)
        self.assertIn("activation lock", concurrent.stderr)
        (self.fixture.state / "pause.release").write_text("continue\n")
        stdout, stderr = process.communicate(timeout=20)
        self.assertEqual(process.returncode, 0, f"{stdout}\n{stderr}")
        self.assert_selected_active(NEW)

    def test_deploy_and_rollback_contend_on_the_same_activation_lock(self) -> None:
        self.deploy_upgrade()
        self.fixture.reset_events()
        rollback_process = self.fixture.popen(
            [
                "/bin/bash", "-c", ROLLBACK_BOOTSTRAP_COMMAND,
                "robinhood-bootstrap", f"/home/robinhood/{self.fixture.bootstrap.name}",
                OLD, SUMS, self.fixture.bootstrap_digest,
                f"/home/robinhood/{self.fixture.manifestctl_dir.name}/robin-highscores-manifestctl",
                self.fixture.manifestctl_digest,
            ],
            pause_event="mv.current-select",
        )
        self._await_pause(rollback_process, "rollback")
        self.fixture.candidate(THIRD)
        concurrent_deploy = self.fixture.deploy(THIRD)
        self._release_pause(rollback_process)
        self.assert_failure(concurrent_deploy)
        self.assert_selected_active(OLD)

        self.fixture.reset_events()
        deploy_process = self.fixture.popen(
            self.fixture.deploy_command(THIRD), pause_event="admin.migrate"
        )
        self._await_pause(deploy_process, "deploy")
        concurrent_rollback = self.fixture.rollback(NEW)
        self._release_pause(deploy_process)
        self.assert_failure(concurrent_rollback)
        self.assert_selected_active(THIRD)

    def _await_pause(self, process: subprocess.Popen[str], operation: str) -> None:
        deadline = time.monotonic() + 30
        while not (self.fixture.state / "pause.ready").exists():
            if process.poll() is not None:
                stdout, stderr = process.communicate()
                self.fail(
                    f"paused {operation} exited early: {process.returncode}\n"
                    f"{stdout}\n{stderr}"
                )
            self.assertLess(
                time.monotonic(), deadline, f"timed out awaiting {operation} pause"
            )
            time.sleep(0.02)

    def _release_pause(self, process: subprocess.Popen[str]) -> None:
        (self.fixture.state / "pause.release").write_text("continue\n")
        stdout, stderr = process.communicate(timeout=30)
        self.assertEqual(process.returncode, 0, f"{stdout}\n{stderr}")
        (self.fixture.state / "pause.ready").unlink(missing_ok=True)
        (self.fixture.state / "pause.release").unlink(missing_ok=True)

    def test_journal_and_receipt_rename_crashes_are_resumable(self) -> None:
        cases = (
            ("deploy-preparing", "deploy", "mv.activation-journal", 1),
            ("deploy-prepared", "deploy", "mv.activation-journal", 2),
            ("deploy-terminal", "deploy", "mv.activation-journal", 3),
            ("deploy-receipt", "deploy", "mv.prebackup-receipt", 1),
            ("rollback-preparing", "rollback", "mv.activation-journal", 1),
            ("rollback-prepared", "rollback", "mv.activation-journal", 2),
            ("rollback-terminal", "rollback", "mv.activation-journal", 3),
            ("rollback-receipt", "rollback", "mv.prebackup-receipt", 1),
        )
        for label, operation, event, occurrence in cases:
            with self.subTest(boundary=label):
                fixture = Fixture()
                try:
                    fixture.candidate(OLD)
                    self.assert_success(fixture.deploy(OLD))
                    if operation == "deploy":
                        fixture.candidate(NEW)
                        fixture.reset_events()
                        failed = fixture.deploy(
                            NEW,
                            fail_event=event,
                            fail_mode="before-sigkill",
                            fail_occurrence=occurrence,
                        )
                        resume = (
                            fixture.deploy
                            if label in ("deploy-preparing", "deploy-prepared")
                            else fixture.resume
                        )
                    else:
                        fixture.candidate(NEW)
                        self.assert_success(fixture.deploy(NEW))
                        fixture.reset_events()
                        failed = fixture.rollback(
                            OLD,
                            fail_event=event,
                            fail_mode="before-sigkill",
                            fail_occurrence=occurrence,
                        )
                        resume = (
                            fixture.rollback
                            if label in ("rollback-preparing", "rollback-prepared")
                            else fixture.rollback_resume
                        )
                    self.assert_failure(failed)
                    if label == "deploy-receipt":
                        temporary = (
                            fixture.opt
                            / f".deploy-source-backup-{NEW}.receipt-v2.new"
                        )
                    elif label == "rollback-receipt":
                        temporary = fixture.opt / f".rollback-prebackup-{OLD}.new"
                    else:
                        target = OLD if operation == "rollback" else NEW
                        temporary = (
                            fixture.opt / f".{operation}-prepared-{target}.new"
                        )
                    self.assertTrue(temporary.is_file(), temporary)
                    target = OLD if operation == "rollback" else NEW
                    self.assert_success(resume(target))
                    self.assert_selected_active(target, fixture)
                    self.assert_transaction_artifacts_absent(operation, target, fixture)
                finally:
                    fixture.cleanup()

    def test_partial_journal_new_at_each_phase_is_safely_reconciled(self) -> None:
        cases = (
            ("deploy-preparing-partial", "deploy", 1),
            ("deploy-prepared-partial", "deploy", 2),
            ("deploy-terminal-partial", "deploy", 3),
            ("rollback-preparing-partial", "rollback", 1),
            ("rollback-prepared-partial", "rollback", 2),
            ("rollback-terminal-partial", "rollback", 3),
        )
        wanted = os.environ.get("ROBIN_TX_BOUNDARY", "")
        if wanted:
            cases = tuple(case for case in cases if case[0] == wanted)
        for label, operation, occurrence in cases:
            with self.subTest(boundary=label):
                fixture = Fixture()
                try:
                    fixture.candidate(OLD)
                    self.assert_success(fixture.deploy(OLD))
                    if operation == "deploy":
                        fixture.candidate(NEW)
                        fixture.reset_events()
                        failed = fixture.deploy(
                            NEW,
                            fail_event="chmod.activation-journal-new",
                            fail_mode="partial-sigkill",
                            fail_occurrence=occurrence,
                        )
                        recover = (
                            fixture.resume
                            if "terminal" in label
                            else fixture.deploy
                        )
                        target = NEW
                    else:
                        fixture.candidate(NEW)
                        self.assert_success(fixture.deploy(NEW))
                        fixture.reset_events()
                        failed = fixture.rollback(
                            OLD,
                            fail_event="chmod.activation-journal-new",
                            fail_mode="partial-sigkill",
                            fail_occurrence=occurrence,
                        )
                        recover = (
                            fixture.rollback_resume
                            if "terminal" in label
                            else fixture.rollback
                        )
                        target = OLD
                    self.assert_failure(failed)
                    temporary = fixture.opt / f".{operation}-prepared-{target}.new"
                    self.assertTrue(temporary.is_file())
                    self.assertNotEqual(
                        stat.S_IMODE(temporary.stat().st_mode), 0o400
                    )
                    self.assert_success(recover(target))
                    self.assert_selected_active(target, fixture)
                    self.assert_transaction_artifacts_absent(
                        operation, target, fixture
                    )
                finally:
                    fixture.cleanup()

    def test_partial_and_unsynced_prebackup_receipt_new_are_resumable(self) -> None:
        cases = (
            ("deploy-receipt-partial", "deploy", "chmod.prebackup-receipt-new", "partial-sigkill"),
            ("deploy-receipt-before-chmod", "deploy", "chmod.prebackup-receipt-new", "before-sigkill"),
            ("deploy-receipt-before-sync", "deploy", "sync.prebackup-receipt", "before-sigkill"),
            ("deploy-receipt-after-sync", "deploy", "sync.prebackup-receipt", "after-sigkill"),
            ("rollback-receipt-partial", "rollback", "chmod.prebackup-receipt-new", "partial-sigkill"),
            ("rollback-receipt-before-chmod", "rollback", "chmod.prebackup-receipt-new", "before-sigkill"),
            ("rollback-receipt-before-sync", "rollback", "sync.prebackup-receipt", "before-sigkill"),
            ("rollback-receipt-after-sync", "rollback", "sync.prebackup-receipt", "after-sigkill"),
        )
        wanted = os.environ.get("ROBIN_TX_BOUNDARY", "")
        if wanted:
            cases = tuple(case for case in cases if case[0] == wanted)
        for label, operation, event, mode in cases:
            with self.subTest(boundary=label):
                fixture = Fixture()
                try:
                    fixture.candidate(OLD)
                    self.assert_success(fixture.deploy(OLD))
                    fixture.candidate(NEW)
                    if operation == "deploy":
                        fixture.reset_events()
                        failed = fixture.deploy(
                            NEW, fail_event=event, fail_mode=mode
                        )
                        recover = fixture.resume
                        target = NEW
                    else:
                        self.assert_success(fixture.deploy(NEW))
                        fixture.reset_events()
                        failed = fixture.rollback(
                            OLD, fail_event=event, fail_mode=mode
                        )
                        recover = fixture.rollback_resume
                        target = OLD
                    self.assert_failure(failed)
                    self.assert_success(recover(target))
                    self.assert_selected_active(target, fixture)
                    self.assert_transaction_artifacts_absent(
                        operation, target, fixture
                    )
                finally:
                    fixture.cleanup()

    def test_deploy_prejournal_preparation_orphans_are_reconciled(self) -> None:
        boundaries = (
            ("mkdir.unit-stage-root", "after-sigkill"),
            ("mkdir.unit-stage-subdirs", "after-sigkill"),
            ("install.stage-unit.robin-highscores.target", "after-sigkill"),
            ("sync.stage-unit.robin-highscores.target", "after-sigkill"),
            ("mkdir.unit-recovery-root", "after-sigkill"),
            ("mkdir.unit-recovery-subdirs", "after-sigkill"),
            ("cp.recovery-unit.robin-highscores.target", "after-sigkill"),
            ("sync.recovery-unit.robin-highscores.target", "after-sigkill"),
            ("ln.target-selector-stage", "after-sigkill"),
            ("ln.source-selector-stage", "after-sigkill"),
            ("chmod.activation-journal-new", "after-sigkill"),
            ("sync.activation-journal", "before-sigkill"),
            ("sync.activation-journal", "after-sigkill"),
        )
        for event, mode in selected_boundaries(boundaries):
            with self.subTest(event=event, mode=mode):
                fixture = Fixture()
                try:
                    fixture.candidate(OLD)
                    self.assert_success(fixture.deploy(OLD))
                    fixture.candidate(NEW)
                    fixture.reset_events()
                    failed = fixture.deploy(
                        NEW, fail_event=event, fail_mode=mode
                    )
                    self.assert_failure(failed)
                    self.assertEqual(fixture.current(), f"releases/{OLD}")
                    self.assert_success(fixture.deploy(NEW))
                    self.assert_selected_active(NEW, fixture)
                    self.assert_transaction_artifacts_absent("deploy", NEW, fixture)
                finally:
                    fixture.cleanup()

    def test_rollback_prejournal_preparation_orphans_are_reconciled(self) -> None:
        boundaries = (
            ("mkdir.unit-stage-root", "after-sigkill"),
            ("mkdir.unit-stage-subdirs", "after-sigkill"),
            ("install.stage-unit.robin-highscores.target", "after-sigkill"),
            ("sync.stage-unit.robin-highscores.target", "after-sigkill"),
            ("mkdir.unit-recovery-root", "after-sigkill"),
            ("mkdir.unit-recovery-subdirs", "after-sigkill"),
            ("cp.recovery-unit.robin-highscores.target", "after-sigkill"),
            ("sync.recovery-unit.robin-highscores.target", "after-sigkill"),
            ("ln.target-selector-stage", "after-sigkill"),
            ("ln.source-selector-stage", "after-sigkill"),
            ("chmod.activation-journal-new", "after-sigkill"),
            ("sync.activation-journal", "before-sigkill"),
            ("sync.activation-journal", "after-sigkill"),
        )
        for event, mode in selected_boundaries(boundaries):
            with self.subTest(event=event, mode=mode):
                fixture = Fixture()
                try:
                    fixture.candidate(OLD)
                    self.assert_success(fixture.deploy(OLD))
                    fixture.candidate(NEW)
                    self.assert_success(fixture.deploy(NEW))
                    fixture.reset_events()
                    failed = fixture.rollback(
                        OLD, fail_event=event, fail_mode=mode
                    )
                    self.assert_failure(failed)
                    self.assertEqual(fixture.current(), f"releases/{NEW}")
                    self.assert_success(fixture.rollback(OLD))
                    self.assert_selected_active(OLD, fixture)
                    self.assert_transaction_artifacts_absent("rollback", OLD, fixture)
                finally:
                    fixture.cleanup()

    def test_activation_complete_cleanup_crashes_keep_target_live_and_resume(self) -> None:
        wanted = os.environ.get("ROBIN_TX_BOUNDARY", "")
        for operation in ("deploy", "rollback"):
            common_events = (
                ("rm.source-selector-stage", 1),
                ("rm.unit-stage", 1),
                ("rm.unit-recovery", 1),
                ("rm.source-backup-receipt", 1),
                ("rm.target-backup-receipt", 1),
                ("rm.activation-journal", 1),
            )
            operation_events = common_events + (
                (("rm.deploy-authority-journal", 1),)
                if operation == "deploy"
                else ()
            )
            unit_sync_count = 1 if operation == "deploy" else 2
            opt_sync_count = 4 if operation == "deploy" else 3
            operation_events += tuple(
                ("sync.unit-root.terminal-cleanup", occurrence)
                for occurrence in range(1, unit_sync_count + 1)
            )
            operation_events += tuple(
                ("sync.opt-root.terminal-cleanup", occurrence)
                for occurrence in range(1, opt_sync_count + 1)
            )
            cleanup_boundaries = tuple(
                (event, occurrence, mode)
                for event, occurrence in operation_events
                for mode in ("before-sigkill", "after-sigkill")
            )
            if wanted:
                cleanup_boundaries = tuple(
                    boundary for boundary in cleanup_boundaries
                    if wanted in (
                        boundary[0],
                        f"{boundary[0]}:{boundary[1]}",
                        f"{boundary[0]}:{boundary[2]}",
                    )
                )
            for event, occurrence, mode in cleanup_boundaries:
                with self.subTest(operation=operation, event=event, mode=mode):
                    fixture = Fixture()
                    try:
                        fixture.candidate(OLD)
                        self.assert_success(fixture.deploy(OLD))
                        fixture.candidate(NEW)
                        self.assert_success(fixture.deploy(NEW))
                        if operation == "deploy":
                            fixture.candidate(THIRD)
                            fixture.reset_events()
                            failed = fixture.deploy(
                                THIRD,
                                fail_event=event,
                                fail_mode=mode,
                                fail_occurrence=occurrence,
                            )
                            target = THIRD
                            resume = fixture.resume
                        else:
                            fixture.reset_events()
                            failed = fixture.rollback(
                                OLD,
                                fail_event=event,
                                fail_mode=mode,
                                fail_occurrence=occurrence,
                            )
                            target = OLD
                            resume = fixture.rollback_resume
                        self.assert_failure(failed)
                        self.assert_selected_active(target, fixture)
                        self.assert_success(resume(target))
                        self.assert_selected_active(target, fixture)
                        self.assert_transaction_artifacts_absent(
                            operation, target, fixture
                        )
                    finally:
                        fixture.cleanup()

    def test_rollback_failure_before_selection_restores_source(self) -> None:
        self.deploy_upgrade()
        self.fixture.reset_events()
        failed = self.fixture.rollback(
            OLD, fail_event="mv.current-select", fail_mode="before-error"
        )
        self.assert_failure(failed)
        self.assert_selected_active(NEW)
        for unit in UNITS:
            self.assertIn(NEW, (self.fixture.unit_root / unit).read_text())

    def test_rollback_side_effect_error_converges_target_stopped(self) -> None:
        self.deploy_upgrade()
        self.fixture.reset_events()
        failed = self.fixture.rollback(
            OLD, fail_event="mv.current-select", fail_mode="after-error"
        )
        self.assert_failure(failed)
        self.assert_selected_stopped(OLD)
        for unit in UNITS:
            self.assertIn(OLD, (self.fixture.unit_root / unit).read_text())
        self.assertTrue((self.fixture.opt / f".rollback-prebackup-{OLD}").is_file())
        self.assert_success(self.fixture.rollback_resume(OLD))
        self.assert_selected_active(OLD)

    def test_rollback_consumed_temp_and_absent_selector_reconstructs_target(self) -> None:
        self.deploy_upgrade()
        self.fixture.reset_events()
        failed = self.fixture.rollback(
            OLD, fail_event="mv.current-select", fail_mode="after-remove-error"
        )
        self.assert_failure(failed)
        self.assert_selected_stopped(OLD)
        for unit in UNITS:
            self.assertIn(OLD, (self.fixture.unit_root / unit).read_text())


if __name__ == "__main__":
    unittest.main(verbosity=2)
