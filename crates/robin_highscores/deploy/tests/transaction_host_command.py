#!/usr/bin/python3
"""Stateful host-command doubles for the deploy transaction integration tests.

This file is bind-mounted over selected absolute /usr/bin paths in a bubblewrap
sandbox.  The production scripts themselves are never rewritten.
"""

from __future__ import annotations

import fcntl
import fnmatch
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time


STATE = Path(os.environ["TX_STATE"])
REAL = Path("/run/tx-real")


def state_file(group: str, name: str) -> Path:
    return STATE / group / name.replace("/", "_")


def read_state(group: str, name: str, default: str) -> str:
    path = state_file(group, name)
    return path.read_text().strip() if path.exists() else default


def write_state(group: str, name: str, value: str) -> None:
    path = state_file(group, name)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + f".new-{os.getpid()}")
    temporary.write_text(value + "\n")
    os.replace(temporary, path)


def record(event: str, argv: list[str]) -> int:
    STATE.mkdir(parents=True, exist_ok=True)
    log = STATE / "events.log"
    with log.open("a+", encoding="utf-8") as stream:
        fcntl.flock(stream, fcntl.LOCK_EX)
        stream.seek(0)
        occurrence = sum(
            1 for line in stream if line.split("\t", 1)[0] == event
        ) + 1
        rendered = " ".join(arg.replace("\n", "\\n") for arg in argv)
        stream.write(f"{event}\t{occurrence}\t{rendered}\n")
        stream.flush()
        os.fsync(stream.fileno())
        return occurrence


def selected(pattern_name: str, event: str, occurrence: int) -> bool:
    pattern = os.environ.get(pattern_name, "")
    wanted = int(os.environ.get(pattern_name + "_OCCURRENCE", "1"))
    return bool(pattern) and occurrence == wanted and fnmatch.fnmatchcase(event, pattern)


def pause_if_selected(event: str, occurrence: int, phase: str) -> None:
    if not selected("TX_PAUSE_EVENT", event, occurrence):
        return
    wanted_phase = os.environ.get("TX_PAUSE_PHASE", "before")
    if phase != wanted_phase:
        return
    (STATE / "pause.ready").write_text(f"{event}\n")
    while not (STATE / "pause.release").exists():
        time.sleep(0.02)


def inject_if_selected(event: str, occurrence: int, phase: str) -> int | None:
    if not selected("TX_FAIL_EVENT", event, occurrence):
        return None
    mode = os.environ.get("TX_FAIL_MODE", "before-error")
    if not mode.startswith(phase + "-"):
        return None
    effect = mode[len(phase) + 1 :]
    if effect == "error":
        return 70
    target = os.getppid()
    # Validator/admin/promoter hooks run through a release-local shell child;
    # kill the transaction shell itself to model abrupt host loss rather than
    # merely a child-command crash that can run EXIT cleanup.
    if event.startswith(("admin.", "validator", "mv.release-install")):
        stat_fields = Path(f"/proc/{target}/stat").read_text().split()
        target = int(stat_fields[3])
    if effect == "sigterm":
        os.kill(target, signal.SIGTERM)
        time.sleep(0.05)
        return 143
    if effect == "sigkill":
        os.kill(target, signal.SIGKILL)
        time.sleep(0.05)
        return 137
    raise RuntimeError(f"unsupported failure effect: {effect}")


def invoke(event: str, argv: list[str], action) -> int:
    occurrence = record(event, argv)
    pause_if_selected(event, occurrence, "before")
    injected = inject_if_selected(event, occurrence, "before")
    if injected is not None:
        return injected
    result = action()
    pause_if_selected(event, occurrence, "after")
    injected = inject_if_selected(event, occurrence, "after")
    return injected if injected is not None else result


def run_real(command: str, argv: list[str]) -> int:
    # Descriptor-pinned bootstrap and manifest-tool paths deliberately cross
    # this host-command boundary.  Preserve inherited descriptors exactly as a
    # normal exec of the real utility would.
    return subprocess.run(
        [str(REAL / command), *argv], check=False, close_fds=False
    ).returncode


def canonical_bytes(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def transaction_receipt_name(name: str) -> bool:
    """Recognize both transaction implementations' exact receipt namespaces."""
    return bool(
        re.fullmatch(
            r"\.(?:deploy|rollback)-(?:source-|target-)?(?:pre)?backup-"
            r"[0-9a-f]{40}(?:\.receipt-v2)?(?:\.new)?",
            name,
        )
    )


def publish_typed_backup(generation: int) -> None:
    install = Path("/home/robinhood/.local/opt/robin-highscores")
    state = Path("/home/robinhood/.local/share/robin-highscores")
    installed_unit = Path(
        "/home/robinhood/.config/systemd/user/robin-highscores-backup.service"
    )
    commits: set[str] = set()
    if installed_unit.is_file():
        commits.update(
            re.findall(
                r"(?<![0-9a-f])[0-9a-f]{40}(?![0-9a-f])",
                installed_unit.read_text(),
            )
        )
    if len(commits) == 1:
        release = install / "releases" / commits.pop()
    else:
        authority = sorted(install.glob(".deploy-authority-*"))
        if len(authority) != 1:
            raise RuntimeError("backup service has no exact release authority")
        target = next(
            (
                line.removeprefix("target_commit=")
                for line in authority[0].read_text().splitlines()
                if line.startswith("target_commit=")
            ),
            "",
        )
        if not re.fullmatch(r"[0-9a-f]{40}", target):
            raise RuntimeError("backup authority journal has no target commit")
        release = install / "releases" / target
    if not release.is_dir():
        raise RuntimeError("backup service release authority is not installed")
    manifest_path = release / "vps-release-manifest-v2.json"
    manifest_bytes = manifest_path.read_bytes()
    release_manifest = json.loads(manifest_bytes)
    unit_names = (
        "robin-highscores-api.service",
        "robin-highscores-backup.service",
        "robin-highscores-backup.timer",
        "robin-highscores-worker.service",
        "robin-highscores.target",
    )
    installed_units = []
    for unit_name in unit_names:
        unit_bytes = (release / "systemd/user" / unit_name).read_bytes()
        installed_units.append(
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
    release_identity = {
        "database_schema_version": release_manifest["database_schema_version"],
        "installed_user_units": installed_units,
        "publication_lock_sha256": release_manifest["publication_lock_sha256"],
        "source_commit": release_manifest["source_commit"],
        "vps_release_manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(),
    }
    created_at = 1_900_000_000_000 + generation
    backup_id = f"backup-v4-{created_at}-{generation:032x}"
    backup_root = state / "backups"
    status_root = state / "status"
    backup_root.mkdir(mode=0o700, parents=True, exist_ok=True)
    status_root.mkdir(mode=0o700, parents=True, exist_ok=True)
    backup = backup_root / backup_id
    backup.mkdir(mode=0o700)

    directories = (
        "campaigns",
        "replays",
        "restore",
        "restore/state",
        "restore/systemd",
        "restore/systemd/user",
    )
    for relative in directories:
        destination = backup / relative
        destination.mkdir(mode=0o700)

    sources = [
        (state / "database/highscores.sqlite3", "highscores.sqlite3"),
        (state / "replays", "replays"),
        (state / "campaign-states", "campaigns"),
        *(
            (state / "api-secrets" / name, f"restore/state/{name}")
            for name in (
                "competition-run-grant.key",
                "cursor-hmac.key",
                "moderation-bearer.token",
                "run-preflight-grant.key",
            )
        ),
        *(
            (
                Path("/home/robinhood/.config/systemd/user") / name,
                f"restore/systemd/user/{name}",
            )
            for name in unit_names
        ),
    ]
    restore_sources = [
        {
            "archive_relative_path": archive,
            "original_absolute_path": str(source),
        }
        for source, archive in sorted(sources, key=lambda item: item[1])
    ]
    files = []
    for source, archive in sorted(sources, key=lambda item: item[1]):
        if source.is_dir():
            continue
        payload = source.read_bytes()
        destination = backup / archive
        destination.write_bytes(payload)
        destination.chmod(0o600)
        files.append(
            {
                "byte_length": len(payload),
                "relative_path": archive,
                "sha256": hashlib.sha256(payload).hexdigest(),
            }
        )
    total_bytes = sum(file["byte_length"] for file in files)
    directory_count = len(directories) + 1
    file_count = len(files)
    manifest = {
        "created_at_unix_ms": created_at,
        "database_schema_version": release_identity["database_schema_version"],
        "directories": [
            {"relative_path": relative, "unix_mode": 0o700}
            for relative in directories
        ],
        "files": files,
        "release_identity": release_identity,
        "restore_sources": restore_sources,
        "root_unix_mode": 448,
        "schema_version": 4,
    }
    backup_manifest_bytes = canonical_bytes(manifest)
    backup_manifest = backup / "backup-manifest.json"
    backup_manifest.write_bytes(backup_manifest_bytes)
    backup_manifest.chmod(0o600)
    envelope_unsigned = {
        "backup_id": backup_id,
        "backup_manifest_sha256": hashlib.sha256(backup_manifest_bytes).hexdigest(),
        "created_at_unix_ms": created_at,
        "database_schema_version": release_identity["database_schema_version"],
        "directory_count": directory_count,
        "file_count": file_count,
        "release_identity": release_identity,
        "result": "verified",
        "schema_version": 2,
        "total_bytes": total_bytes,
    }
    authority_key = (state / "api-secrets/backup-authority-hmac.key").read_bytes()
    envelope = dict(envelope_unsigned)
    envelope["hmac_sha256"] = hmac.new(
        authority_key,
        b"robinhood/highscores/backup-verification-envelope/2\0"
        + canonical_bytes(envelope_unsigned),
        hashlib.sha256,
    ).hexdigest()
    envelope_path = backup / "backup-verification-envelope.json"
    envelope_path.write_bytes(canonical_bytes(envelope))
    envelope_path.chmod(0o400)

    status_unsigned = {
        "backup_directory": str(backup),
        "backup_id": backup_id,
        "backup_manifest_sha256": hashlib.sha256(backup_manifest_bytes).hexdigest(),
        "created_at_unix_ms": created_at,
        "database_schema_version": release_identity["database_schema_version"],
        "directory_count": directory_count,
        "file_count": file_count,
        "release_identity": release_identity,
        "schema_version": 4,
        "total_bytes": total_bytes,
    }
    status = dict(status_unsigned)
    status["hmac_sha256"] = hmac.new(
        authority_key,
        b"robinhood/highscores/backup-status/4\0" + canonical_bytes(status_unsigned),
        hashlib.sha256,
    ).hexdigest()
    status_path = status_root / "backup-status.json"
    temporary = status_path.with_name(f"backup-status.json.new-{os.getpid()}")
    temporary.write_bytes(canonical_bytes(status))
    temporary.chmod(0o400)
    os.replace(temporary, status_path)


def systemctl(argv: list[str]) -> int:
    args = [arg for arg in argv if arg != "--user"]
    if args == ["show-environment"]:
        return invoke("systemctl.show-environment", argv, lambda: 0)
    command = args[0]
    if command == "daemon-reload":
        return invoke("systemctl.daemon-reload", argv, lambda: 0)
    unit = args[-1]
    if command == "show":
        def show() -> int:
            print(read_state("active", unit, "inactive"))
            return 0
        return invoke(f"systemctl.show.ActiveState.{unit}", argv, show)
    if command == "is-active":
        def is_active() -> int:
            active = read_state("active", unit, "inactive")
            if "--quiet" not in args:
                print(active)
            return 0 if active == "active" else 3
        return invoke(f"systemctl.is-active.{unit}", argv, is_active)
    if command == "is-enabled":
        def is_enabled() -> int:
            if (
                os.environ.get("TX_EMPTY_MISSING_ENABLED") == "1"
                and not state_file("enabled", unit).exists()
            ):
                return 1
            enabled = read_state("enabled", unit, "disabled")
            print(enabled)
            return 0 if enabled in ("enabled", "enabled-runtime") else 1
        return invoke(f"systemctl.is-enabled.{unit}", argv, is_enabled)

    def mutate() -> int:
        if command == "start":
            write_state("active", unit, "active")
            if unit == "robin-highscores-backup.service":
                generation = int(read_state("meta", "backup-generation", "0")) + 1
                publish_typed_backup(generation)
                write_state("meta", "backup-generation", str(generation))
                write_state("active", unit, "inactive")
        elif command == "stop":
            write_state("active", unit, "inactive")
        elif command == "disable":
            write_state("enabled", unit, "disabled")
            if "--now" in args:
                write_state("active", unit, "inactive")
        elif command == "enable":
            enabled = "enabled-runtime" if "--runtime" in args else "enabled"
            write_state("enabled", unit, enabled)
            unit_root = Path("/home/robinhood/.config/systemd/user")
            (unit_root / "default.target.wants").mkdir(parents=True, exist_ok=True)
            (unit_root / "timers.target.wants").mkdir(parents=True, exist_ok=True)
            if "--now" in args:
                write_state("active", unit, "active")
        else:
            print(f"unsupported mock systemctl invocation: {args}", file=sys.stderr)
            return 64
        return 0

    return invoke(f"systemctl.{command}.{unit}", argv, mutate)


def mv(argv: list[str]) -> int:
    operands = [arg for arg in argv if arg not in ("-T", "--", "--no-clobber")]
    source, destination = operands[-2:]
    dest = Path(destination)
    if "--no-clobber" in argv and dest.parent.name == "releases":
        event = "mv.release-install"
    elif dest.parent.name == "incoming" and dest.name.endswith(".consuming"):
        event = "mv.incoming-consuming"
    elif dest.name == "current":
        event = "mv.current-select"
    elif dest.name.startswith((".deploy-prepared-", ".rollback-prepared-")):
        event = "mv.activation-journal"
    elif transaction_receipt_name(dest.name):
        event = "mv.prebackup-receipt"
    elif dest.name.startswith("robin-highscores"):
        event = f"mv.unit.{dest.name}"
    else:
        event = "mv.other"

    occurrence = record(event, argv)
    if selected("TX_FAIL_EVENT", event, occurrence):
        special_mode = os.environ.get("TX_FAIL_MODE")
        if special_mode == "race":
            dest.mkdir(parents=True, exist_ok=False)
            (dest / ".race-winner").write_text("independent winner\n")
            return 73
        if special_mode in ("after-remove-error", "after-ambiguous-error"):
            result = run_real("mv", argv)
            if result != 0:
                return result
            if special_mode == "after-remove-error":
                dest.unlink()
            else:
                dest.unlink()
                dest.write_text("ambiguous selector\n")
            return 70
    pause_if_selected(event, occurrence, "before")
    injected = inject_if_selected(event, occurrence, "before")
    if injected is not None:
        return injected
    result = run_real("mv", argv)
    if result == 0 and event == "mv.activation-journal" and dest.is_file():
        phase = "activation_complete" if "phase=activation_complete\n" in dest.read_text() else "prepared"
        write_state(
            "meta", "terminal-journal-durable",
            "1" if phase == "activation_complete" else "0",
        )
        write_state("meta", "terminal-journal-unlink-pending", "0")
    pause_if_selected(event, occurrence, "after")
    injected = inject_if_selected(event, occurrence, "after")
    return injected if injected is not None else result


def sync(argv: list[str]) -> int:
    target = argv[-1] if argv else "all"
    terminal_journal_was_durable = (
        read_state("meta", "terminal-journal-durable", "0") == "1"
    )
    target_runtime_active = terminal_journal_was_durable and all(
        read_state("active", unit, "inactive") == "active"
        for unit in (
            "robin-highscores.target",
            "robin-highscores-api.service",
            "robin-highscores-worker.service",
            "robin-highscores-backup.timer",
        )
    )
    if target.endswith("/releases"):
        event = "sync.release-root"
    elif target.endswith("/incoming"):
        incoming = Path(target)
        event = (
            "sync.incoming-root.quarantine"
            if any(incoming.glob(".*.consuming"))
            else "sync.incoming-root.consumed"
        )
    elif target.endswith("/.local/opt/robin-highscores"):
        event = (
            "sync.opt-root.terminal-cleanup"
            if target_runtime_active
            else "sync.opt-root"
        )
    elif "/.deploy-prepared-" in target or "/.rollback-prepared-" in target:
        event = "sync.activation-journal"
    elif transaction_receipt_name(Path(target).name):
        event = "sync.prebackup-receipt"
    elif ".stage/units/" in target:
        event = f"sync.stage-unit.{Path(target).name}"
    elif ".recovery/units/" in target:
        event = f"sync.recovery-unit.{Path(target).name}"
    elif any(
        component in target
        for component in (
            ".recovery/active/",
            ".recovery/enabled/",
            ".recovery/absent/",
        )
    ):
        event = f"sync.recovery-state.{Path(target).name}"
    elif target.endswith(".stage"):
        event = "sync.unit-stage-root"
    elif target.endswith(".recovery"):
        event = "sync.unit-recovery-root"
    elif "/systemd/user/robin-highscores" in target:
        event = f"sync.unit.{Path(target).name}"
    elif target.endswith("/.config/systemd/user"):
        event = (
            "sync.unit-root.terminal-cleanup"
            if target_runtime_active
            else "sync.unit-root"
        )
    else:
        event = "sync.other"
    def sync_and_publish_namespace() -> int:
        result = run_real("sync", argv)
        if (
            result == 0
            and event == "sync.opt-root.terminal-cleanup"
            and read_state("meta", "terminal-journal-unlink-pending", "0") == "1"
        ):
            # The activation-complete journal unlink becomes durable at this
            # parent sync.  Publish that model transition before an injected
            # after-sigkill, while a before-sigkill retains the durable journal.
            write_state("meta", "terminal-journal-durable", "0")
            write_state("meta", "terminal-journal-unlink-pending", "0")
        return result

    return invoke(event, argv, sync_and_publish_namespace)


def sha256sum(argv: list[str]) -> int:
    nonoptions = [arg for arg in argv if not arg.startswith("-")]
    if nonoptions == ["/usr/bin/mv"]:
        print(
            "a781be46e6f27ca5d7e119225429a4a12ef941eea15ed2c44eea0c8bca7e4ebe  /usr/bin/mv"
        )
        return 0
    operand = nonoptions[-1] if nonoptions else ""
    fd_events = {
        "/proc/self/fd/3": "sha.bootstrap-manifest",
        "/proc/self/fd/4": "sha.bootstrap-script",
        "/proc/self/fd/5": "sha.bootstrap-validator",
    }
    event = fd_events.get(operand, "sha.other")
    return invoke(event, argv, lambda: run_real("sha256sum", argv))


def stat_command(argv: list[str]) -> int:
    target = argv[-1] if argv else ""
    wrong_owner = os.environ.get("TX_WRONG_OWNER_PATH", "")
    if wrong_owner and target == wrong_owner and any("%u" in arg for arg in argv):
        def report_wrong_owner() -> int:
            print(os.getuid() + 1)
            return 0

        return invoke("stat.owner.user-unit-root", argv, report_wrong_owner)
    return run_real("stat", argv)


def findmnt(argv: list[str]) -> int:
    target = argv[-1]
    injected = os.environ.get("TX_NESTED_MOUNT", "")
    if injected and (target == injected or injected.startswith(target.rstrip("/") + "/")):
        print(injected)
    return 0


def find(argv: list[str]) -> int:
    injected_root = os.environ.get("TX_MIXED_OWNER_ROOT", "")
    if injected_root and argv and argv[0] == injected_root and (
        "-uid" in argv or "-user" in argv
    ):
        print(injected_root + "/foreign-owner")
        return 0
    return run_real("find", argv)


def rm(argv: list[str]) -> int:
    target = argv[-1] if argv else ""
    target_name = Path(target).name
    if "/incoming/." in target and target.endswith(".consuming"):
        event = "rm.incoming-consuming"
    elif "/releases/" in target and target.endswith(".partial"):
        event = "rm.release-partial"
    elif target_name.startswith((".deploy-source-backup-", ".rollback-prebackup-")):
        event = "rm.source-backup-receipt"
    elif target_name.startswith((".deploy-target-backup-", ".rollback-target-backup-")):
        event = "rm.target-backup-receipt"
    elif target_name.startswith(".deploy-authority-"):
        event = "rm.deploy-authority-journal"
    elif target_name.startswith((".deploy-prepared-", ".rollback-prepared-")):
        event = "rm.activation-journal"
    elif target.endswith(".stage"):
        event = "rm.unit-stage"
    elif target.endswith(".recovery"):
        event = "rm.unit-recovery"
    elif Path(target).name.startswith(".current-") and target.endswith(".restore"):
        event = "rm.source-selector-stage"
    elif Path(target).name.startswith(".current-"):
        event = "rm.target-selector-stage"
    else:
        event = "rm.recovery"
    def remove_and_track_namespace() -> int:
        result = run_real("rm", argv)
        if result == 0 and event == "rm.activation-journal":
            # The unlink is visible but not durable until opt_root is synced.
            write_state("meta", "terminal-journal-unlink-pending", "1")
        return result

    return invoke(event, argv, remove_and_track_namespace)


def mkdir(argv: list[str]) -> int:
    targets = [arg for arg in argv if not arg.startswith("-") and arg != "0700"]
    rendered = " ".join(targets)
    if any(target.endswith("/.config/systemd/user") for target in targets):
        event = "mkdir.user-unit-root"
    elif any(target.endswith(".stage") for target in targets):
        event = "mkdir.unit-stage-root"
    elif any(target.endswith(".recovery") for target in targets):
        event = "mkdir.unit-recovery-root"
    elif ".stage/units" in rendered:
        event = "mkdir.unit-stage-subdirs"
    elif ".recovery/" in rendered:
        event = "mkdir.unit-recovery-subdirs"
    else:
        event = "mkdir.other"
    return invoke(event, argv, lambda: run_real("mkdir", argv))


def install(argv: list[str]) -> int:
    target = argv[-1] if argv else ""
    if target.endswith("/.config/systemd/user"):
        event = "install.user-unit-root"
    elif ".stage/units/" in target:
        event = f"install.stage-unit.{Path(target).name}"
    else:
        event = "install.other"
    return invoke(event, argv, lambda: run_real("install", argv))


def cp(argv: list[str]) -> int:
    target = argv[-1] if argv else ""
    if ".recovery/units/" in target:
        event = f"cp.recovery-unit.{Path(target).name}"
    else:
        event = "cp.other"
    return invoke(event, argv, lambda: run_real("cp", argv))


def ln(argv: list[str]) -> int:
    target = argv[-1] if argv else ""
    if target.endswith(".restore") and "/.current-" in target:
        event = "ln.source-selector-stage"
    elif "/.current-" in target:
        event = "ln.target-selector-stage"
    else:
        event = "ln.other"
    return invoke(event, argv, lambda: run_real("ln", argv))


def chmod(argv: list[str]) -> int:
    target = argv[-1] if argv else ""
    if target.endswith((".deploy-prepared-", ".rollback-prepared-")):
        event = "chmod.activation-journal"
    elif Path(target).name.startswith((".deploy-prepared-", ".rollback-prepared-")):
        event = "chmod.activation-journal-new"
    elif transaction_receipt_name(Path(target).name):
        event = "chmod.prebackup-receipt-new"
    else:
        event = "chmod.other"
    occurrence = record(event, argv)
    if selected("TX_FAIL_EVENT", event, occurrence) and os.environ.get(
        "TX_FAIL_MODE"
    ) == "partial-sigkill":
        path = Path(target)
        payload = path.read_bytes()
        path.write_bytes(payload[: max(1, len(payload) // 2)])
        os.kill(os.getppid(), signal.SIGKILL)
        time.sleep(0.05)
        return 137
    pause_if_selected(event, occurrence, "before")
    injected = inject_if_selected(event, occurrence, "before")
    if injected is not None:
        return injected
    result = run_real("chmod", argv)
    pause_if_selected(event, occurrence, "after")
    injected = inject_if_selected(event, occurrence, "after")
    return injected if injected is not None else result


def curl(argv: list[str]) -> int:
    endpoint = argv[-1]
    event = "curl.readyz" if endpoint.endswith("/readyz") else "curl.healthz"

    def probe() -> int:
        if read_state("active", "robin-highscores-api.service", "inactive") != "active":
            return 22
        if endpoint.endswith("/readyz"):
            status = Path(
                "/home/robinhood/.local/share/robin-highscores/status/backup-status.json"
            )
            if not status.is_file():
                return 22
        return 0

    return invoke(event, argv, probe)


def hook(argv: list[str]) -> int:
    event = argv[0]

    def action() -> int:
        if event == "admin.migrate":
            count = int(read_state("meta", "migration-count", "0")) + 1
            write_state("meta", "migration-count", str(count))
        return 0

    return invoke(event, argv, action)


def main() -> int:
    command = Path(sys.argv[0]).name
    argv = sys.argv[1:]
    if command == "systemctl":
        return systemctl(argv)
    if command == "mv":
        return mv(argv)
    if command == "sync":
        return sync(argv)
    if command == "sha256sum":
        return sha256sum(argv)
    if command == "stat":
        return stat_command(argv)
    if command == "findmnt":
        return findmnt(argv)
    if command == "find":
        return find(argv)
    if command == "curl":
        return curl(argv)
    if command == "df":
        # GNU df right-aligns this column; the production parser deliberately
        # requires and removes its whitespace.
        print("Avail\n " + os.environ.get("TX_AVAILABLE_BYTES", "17179869184"))
        return 0
    if command == "sleep":
        return 0
    if command == "flock":
        return invoke("flock.acquire", argv, lambda: run_real("flock", argv))
    if command == "true":
        return hook(argv)
    if command == "rm":
        return rm(argv)
    if command == "mkdir":
        return mkdir(argv)
    if command == "install":
        return install(argv)
    if command == "cp":
        return cp(argv)
    if command == "ln":
        return ln(argv)
    if command == "chmod":
        return chmod(argv)
    print(f"unexpected transaction host double name: {command}", file=sys.stderr)
    return 64


if __name__ == "__main__":
    raise SystemExit(main())
