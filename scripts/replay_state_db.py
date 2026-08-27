#!/usr/bin/env python3
"""Authoritative SQLite ledger for replay inventory and parity run evidence."""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import re
import socket
import sqlite3
import sys
import uuid
from pathlib import Path


SCHEMA_VERSION = 5
EOF_MARKER = "parity trace matched every recorded frame"
DIVERGENCE_RE = re.compile(r"first parity divergence after frame (\d+)")
HALTED_RE = re.compile(r"parity replay halted at frame (\d+)")
THROUGH_RE = re.compile(r"(?:through|after) frame[ =](\d+)")

SCHEMA = r"""
PRAGMA application_id = 1380463184;
CREATE TABLE IF NOT EXISTS schema_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS corpora (
    corpus_id INTEGER PRIMARY KEY,
    logical_root TEXT NOT NULL UNIQUE,
    seed_base INTEGER,
    trace_schema INTEGER,
    expected_replays INTEGER,
    campaign_sha256 TEXT,
    corpus_path TEXT,
    corpus_status TEXT NOT NULL DEFAULT 'historical'
        CHECK (corpus_status IN ('active','historical','retired')),
    retirement_reason TEXT,
    retired_utc TEXT,
    registered_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    CHECK (campaign_sha256 IS NULL OR length(campaign_sha256) = 64)
) STRICT;

CREATE TABLE IF NOT EXISTS replays (
    replay_id INTEGER PRIMARY KEY,
    corpus_id INTEGER REFERENCES corpora(corpus_id),
    replay_key TEXT NOT NULL UNIQUE CHECK (length(replay_key) = 64),
    logical_path TEXT UNIQUE,
    legacy_namespace TEXT,
    legacy_key TEXT,
    completion_marker TEXT,
    registered_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE(legacy_namespace, legacy_key),
    CHECK (logical_path IS NOT NULL OR (legacy_namespace IS NOT NULL AND legacy_key IS NOT NULL))
) STRICT;

CREATE TABLE IF NOT EXISTS corpus_locations (
    location_id INTEGER PRIMARY KEY,
    corpus_id INTEGER NOT NULL REFERENCES corpora(corpus_id),
    host TEXT NOT NULL,
    path TEXT NOT NULL,
    note TEXT,
    observed_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE(corpus_id,host,path)
) STRICT;

CREATE TABLE IF NOT EXISTS final_corpus_members (
    corpus_id INTEGER NOT NULL REFERENCES corpora(corpus_id),
    replay_id INTEGER NOT NULL UNIQUE REFERENCES replays(replay_id),
    source_ledger TEXT NOT NULL,
    PRIMARY KEY(corpus_id,replay_id)
) STRICT;

CREATE TABLE IF NOT EXISTS runners (
    runner_id INTEGER PRIMARY KEY,
    identity_key TEXT NOT NULL UNIQUE CHECK (length(identity_key) = 64),
    identity_kind TEXT NOT NULL CHECK (identity_kind IN ('authenticated','provisional_label')),
    runner_label TEXT,
    runner_sha_prefix TEXT,
    bundle_trust_sha256 TEXT UNIQUE CHECK (bundle_trust_sha256 IS NULL OR length(bundle_trust_sha256) = 64),
    raw_sha256 TEXT CHECK (raw_sha256 IS NULL OR length(raw_sha256) = 64),
    bundle_manifest_sha256 TEXT,
    library_manifest_sha256 TEXT,
    wrapper_sha256 TEXT,
    first_seen_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    CHECK (bundle_manifest_sha256 IS NULL OR length(bundle_manifest_sha256) = 64),
    CHECK (library_manifest_sha256 IS NULL OR length(library_manifest_sha256) = 64),
    CHECK (wrapper_sha256 IS NULL OR length(wrapper_sha256) = 64)
) STRICT;

CREATE TABLE IF NOT EXISTS replay_runs (
    run_id INTEGER PRIMARY KEY,
    evidence_key TEXT NOT NULL UNIQUE CHECK (length(evidence_key) = 64),
    replay_id INTEGER NOT NULL REFERENCES replays(replay_id),
    runner_id INTEGER NOT NULL REFERENCES runners(runner_id),
    run_kind TEXT NOT NULL,
    evidence_tier TEXT NOT NULL CHECK (evidence_tier IN ('attested','provisional')),
    outcome TEXT NOT NULL CHECK (outcome IN
        ('exact_eof','mismatch','crash','timeout','aborted','integrity_error','unknown')),
    result_status TEXT NOT NULL,
    command_status INTEGER,
    exact_eof INTEGER NOT NULL CHECK (exact_eof IN (0,1)),
    eof_marker_count INTEGER NOT NULL CHECK (eof_marker_count >= 0),
    furthest_frame INTEGER CHECK (furthest_frame IS NULL OR furthest_frame >= 0),
    divergence_frame INTEGER CHECK (divergence_frame IS NULL OR divergence_frame >= 0),
    matched_prefix_frames INTEGER CHECK (matched_prefix_frames IS NULL OR matched_prefix_frames >= 0),
    recorded_frames INTEGER CHECK (recorded_frames IS NULL OR recorded_frames >= 0),
    terminal_frame INTEGER CHECK (terminal_frame IS NULL OR terminal_frame >= 0),
    progress_precision TEXT NOT NULL CHECK (progress_precision IN
        ('exact','universal_frame','exact_eof_unknown_extent','unknown')),
    started_utc TEXT,
    finished_utc TEXT,
    evidence_mtime_utc TEXT,
    timestamp_source TEXT NOT NULL CHECK (timestamp_source IN ('attested','filesystem_mtime','unknown')),
    host TEXT NOT NULL,
    native_sha256_pre TEXT,
    native_sha256_post TEXT,
    completion_marker_sha256 TEXT,
    log_sha256 TEXT NOT NULL CHECK (length(log_sha256) = 64),
    evidence_manifest_sha256 TEXT,
    audit_path TEXT NOT NULL,
    evidence_path TEXT NOT NULL,
    log_path TEXT NOT NULL,
    command TEXT,
    data_dir TEXT,
    imported_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    CHECK (native_sha256_pre IS NULL OR length(native_sha256_pre) = 64),
    CHECK (native_sha256_post IS NULL OR length(native_sha256_post) = 64),
    CHECK (completion_marker_sha256 IS NULL OR length(completion_marker_sha256) = 64),
    CHECK (evidence_manifest_sha256 IS NULL OR length(evidence_manifest_sha256) = 64),
    CHECK (exact_eof = 0 OR outcome = 'exact_eof')
) STRICT;

CREATE INDEX IF NOT EXISTS replay_runs_replay_finished
    ON replay_runs(replay_id, finished_utc DESC, run_id DESC);
CREATE INDEX IF NOT EXISTS replay_runs_runner_outcome
    ON replay_runs(runner_id, outcome);

CREATE VIEW IF NOT EXISTS latest_replay_runs AS
SELECT * FROM (
    SELECT rr.*,
           row_number() OVER (
               PARTITION BY replay_id
               ORDER BY finished_utc DESC NULLS LAST, run_id DESC
           ) AS latest_rank
    FROM replay_runs AS rr
) WHERE latest_rank = 1;

CREATE VIEW IF NOT EXISTS latest_replay_runner_runs AS
SELECT * FROM (
    SELECT rr.*,
           row_number() OVER (
               PARTITION BY replay_id, runner_id
               ORDER BY finished_utc DESC NULLS LAST, run_id DESC
           ) AS latest_rank
    FROM replay_runs AS rr
) WHERE latest_rank = 1;

CREATE TRIGGER IF NOT EXISTS replay_runs_no_update
BEFORE UPDATE ON replay_runs BEGIN
    SELECT RAISE(ABORT, 'replay_runs is append-only');
END;
CREATE TRIGGER IF NOT EXISTS replay_runs_no_delete
BEFORE DELETE ON replay_runs BEGIN
    SELECT RAISE(ABORT, 'replay_runs is append-only');
END;

-- Corrections preserve immutable evidence while repairing a derived outcome
-- assigned by an older importer. The original replay_runs row, status, log,
-- and checksummed evidence directory remain untouched.
CREATE TABLE IF NOT EXISTS replay_run_corrections (
    evidence_key TEXT PRIMARY KEY REFERENCES replay_runs(evidence_key),
    corrected_outcome TEXT NOT NULL CHECK (corrected_outcome IN
        ('exact_eof','mismatch','crash','timeout','aborted','integrity_error','unknown')),
    reason TEXT NOT NULL,
    corrected_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;
CREATE TRIGGER IF NOT EXISTS replay_run_corrections_no_update
BEFORE UPDATE ON replay_run_corrections BEGIN
    SELECT RAISE(ABORT, 'replay run corrections are append-only');
END;
CREATE TRIGGER IF NOT EXISTS replay_run_corrections_no_delete
BEFORE DELETE ON replay_run_corrections BEGIN
    SELECT RAISE(ABORT, 'replay run corrections are append-only');
END;

CREATE TABLE IF NOT EXISTS work_items (
    work_id INTEGER PRIMARY KEY,
    work_key TEXT NOT NULL UNIQUE CHECK (length(work_key) = 64),
    operation TEXT NOT NULL CHECK (operation IN ('replay','convert')),
    replay_id INTEGER NOT NULL REFERENCES replays(replay_id),
    runner_id INTEGER REFERENCES runners(runner_id),
    conversion_protocol INTEGER,
    target_encoding TEXT,
    source_sha256 TEXT,
    priority INTEGER NOT NULL DEFAULT 0,
    created_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    CHECK (source_sha256 IS NULL OR length(source_sha256) = 64),
    CHECK ((operation = 'replay' AND runner_id IS NOT NULL)
        OR (operation = 'convert' AND conversion_protocol IS NOT NULL AND target_encoding IS NOT NULL))
) STRICT;

CREATE TABLE IF NOT EXISTS work_claims (
    work_id INTEGER PRIMARY KEY REFERENCES work_items(work_id),
    claim_token TEXT NOT NULL UNIQUE CHECK (length(claim_token) = 64),
    worker_id TEXT NOT NULL,
    claimed_utc TEXT NOT NULL,
    lease_until_utc TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS work_completions (
    work_id INTEGER PRIMARY KEY REFERENCES work_items(work_id),
    claim_token TEXT NOT NULL,
    completed_utc TEXT NOT NULL,
    outcome TEXT NOT NULL,
    evidence_key TEXT REFERENCES replay_runs(evidence_key)
) STRICT;

CREATE TABLE IF NOT EXISTS corpus_work_leases (
    corpus_work_id INTEGER PRIMARY KEY,
    corpus_id INTEGER NOT NULL REFERENCES corpora(corpus_id),
    operation TEXT NOT NULL CHECK (operation IN ('capture','convert','replay','transfer')),
    worker_id TEXT NOT NULL,
    host TEXT NOT NULL,
    audit_path TEXT,
    claim_token TEXT NOT NULL UNIQUE CHECK (length(claim_token) = 64),
    claimed_utc TEXT NOT NULL,
    heartbeat_utc TEXT NOT NULL,
    lease_until_utc TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'active'
        CHECK (state IN ('active','completed','failed','abandoned')),
    detail TEXT,
    finished_utc TEXT
) STRICT;
CREATE UNIQUE INDEX IF NOT EXISTS corpus_work_one_active
    ON corpus_work_leases(corpus_id,operation) WHERE state = 'active';

CREATE TRIGGER IF NOT EXISTS work_items_no_update BEFORE UPDATE ON work_items BEGIN
    SELECT RAISE(ABORT, 'work_items is append-only');
END;
CREATE TRIGGER IF NOT EXISTS work_items_no_delete BEFORE DELETE ON work_items BEGIN
    SELECT RAISE(ABORT, 'work_items is append-only');
END;
CREATE TRIGGER IF NOT EXISTS work_completions_no_update BEFORE UPDATE ON work_completions BEGIN
    SELECT RAISE(ABORT, 'work_completions is append-only');
END;
CREATE TRIGGER IF NOT EXISTS work_completions_no_delete BEFORE DELETE ON work_completions BEGIN
    SELECT RAISE(ABORT, 'work_completions is append-only');
END;
"""


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def connect(path: Path) -> sqlite3.Connection:
    path.parent.mkdir(parents=True, exist_ok=True)
    connection = sqlite3.connect(path, timeout=60)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA foreign_keys = ON")
    connection.execute("PRAGMA busy_timeout = 60000")
    connection.execute("PRAGMA journal_mode = WAL")
    connection.execute("PRAGMA synchronous = OFF")
    connection.executescript(SCHEMA)
    connection.execute(
        "INSERT OR IGNORE INTO schema_meta(key,value) VALUES('schema_version',?)",
        (str(SCHEMA_VERSION),),
    )
    version = connection.execute(
        "SELECT value FROM schema_meta WHERE key='schema_version'"
    ).fetchone()[0]
    if version == "2":
        connection.execute(
            "ALTER TABLE corpora ADD COLUMN corpus_status TEXT NOT NULL DEFAULT 'historical' "
            "CHECK (corpus_status IN ('active','historical','retired'))"
        )
        connection.execute("ALTER TABLE corpora ADD COLUMN retirement_reason TEXT")
        connection.execute("ALTER TABLE corpora ADD COLUMN retired_utc TEXT")
        connection.execute("ALTER TABLE corpora ADD COLUMN corpus_path TEXT")
        connection.execute(
            "UPDATE schema_meta SET value='3' WHERE key='schema_version'"
        )
        version = "3"
    if version in ("3", "4"):
        connection.execute(
            "UPDATE schema_meta SET value=? WHERE key='schema_version'",
            (str(SCHEMA_VERSION),),
        )
        version = str(SCHEMA_VERSION)
    if version != str(SCHEMA_VERSION):
        raise RuntimeError(f"unsupported replay database schema {version}")
    connection.commit()
    return connection


def parse_env(path: Path, allow_duplicates: bool = False) -> dict[str, str]:
    values: dict[str, str] = {}
    for number, raw in enumerate(path.read_text(errors="strict").splitlines(), 1):
        if not raw or raw.startswith("#"):
            continue
        if "=" not in raw:
            raise ValueError(f"{path}:{number}: malformed environment line")
        key, value = raw.split("=", 1)
        if key in values and not allow_duplicates:
            raise ValueError(f"{path}:{number}: duplicate key {key}")
        values[key] = value
    return values


def workspace_relative(value: str, workspace: Path | None) -> str:
    path = Path(value)
    if workspace is not None and path.is_absolute():
        try:
            return str(path.relative_to(workspace))
        except ValueError:
            pass
    return str(path)


def corpus_root_from_logical(logical_path: str) -> str | None:
    marker = "/traces/"
    if marker not in logical_path:
        return None
    return logical_path.split(marker, 1)[0]


def upsert_corpus(
    connection: sqlite3.Connection,
    root: str,
    seed_base: int | None = None,
    trace_schema: int | None = None,
    expected: int | None = None,
    campaign_sha: str | None = None,
) -> int:
    connection.execute(
        """INSERT INTO corpora(logical_root,seed_base,trace_schema,expected_replays,campaign_sha256)
           VALUES(?,?,?,?,?) ON CONFLICT(logical_root) DO UPDATE SET
             seed_base=coalesce(corpora.seed_base,excluded.seed_base),
             trace_schema=coalesce(corpora.trace_schema,excluded.trace_schema),
             expected_replays=coalesce(corpora.expected_replays,excluded.expected_replays),
             campaign_sha256=coalesce(corpora.campaign_sha256,excluded.campaign_sha256)""",
        (root, seed_base, trace_schema, expected, campaign_sha),
    )
    return int(
        connection.execute(
            "SELECT corpus_id FROM corpora WHERE logical_root=?", (root,)
        ).fetchone()[0]
    )


def upsert_replay(
    connection: sqlite3.Connection, logical: str, completion_marker: str | None
) -> int:
    root = corpus_root_from_logical(logical)
    corpus_id = upsert_corpus(connection, root) if root else None
    replay_key = sha256_bytes(f"logical-replay-v1\n{logical}\n".encode())
    connection.execute(
        """INSERT INTO replays(corpus_id,replay_key,logical_path,completion_marker)
           VALUES(?,?,?,?) ON CONFLICT(logical_path) DO UPDATE SET
             corpus_id=coalesce(replays.corpus_id,excluded.corpus_id),
             completion_marker=coalesce(replays.completion_marker,excluded.completion_marker)""",
        (corpus_id, replay_key, logical, completion_marker),
    )
    return int(
        connection.execute(
            "SELECT replay_id FROM replays WHERE logical_path=?", (logical,)
        ).fetchone()[0]
    )


def upsert_runner(connection: sqlite3.Connection, env: dict[str, str]) -> int:
    trust = env.get("RUNNER_BUNDLE_TRUST_SHA256", env.get("RUNNER_TRUST_SHA256"))
    if trust is None:
        raise ValueError("runner evidence has no trust identity")
    trust = trust.lower()
    raw = env["RUNNER_RAW_SHA256"].lower()
    identity_key = sha256_bytes(f"authenticated-runner-v1\n{trust}\n".encode())
    fields = (
        identity_key,
        "authenticated",
        trust,
        raw,
        env.get("RUNNER_BUNDLE_MANIFEST_SHA256"),
        env.get("RUNNER_LIB_MANIFEST_SHA256"),
        env.get("RUNNER_WRAPPER_SHA256"),
    )
    connection.execute(
        """INSERT INTO runners(identity_key,identity_kind,bundle_trust_sha256,raw_sha256,bundle_manifest_sha256,
                    library_manifest_sha256,wrapper_sha256)
           VALUES(?,?,?,?,?,?,?) ON CONFLICT(identity_key) DO NOTHING""",
        fields,
    )
    row = connection.execute(
        "SELECT * FROM runners WHERE identity_key=?", (identity_key,)
    ).fetchone()
    if row["raw_sha256"] != raw:
        raise ValueError(f"runner trust digest {trust} maps to conflicting raw binaries")
    return int(row["runner_id"])


def upsert_provisional_runner(
    connection: sqlite3.Connection, label: str, sha_prefix: str | None = None,
    raw_sha256: str | None = None,
) -> int:
    identity_key = sha256_bytes(
        f"provisional-runner-label-v1\nLABEL={label}\nPREFIX={sha_prefix or ''}\n".encode()
    )
    connection.execute(
        """INSERT OR IGNORE INTO runners(
             identity_key,identity_kind,runner_label,runner_sha_prefix,raw_sha256)
           VALUES(?,'provisional_label',?,?,?)""",
        (identity_key, label, sha_prefix, raw_sha256),
    )
    return int(connection.execute(
        "SELECT runner_id FROM runners WHERE identity_key=?", (identity_key,)
    ).fetchone()[0])


def upsert_legacy_replay(
    connection: sqlite3.Connection, namespace: str, legacy_key: str
) -> int:
    replay_key = sha256_bytes(
        f"legacy-replay-key-v1\nNAMESPACE={namespace}\nKEY={legacy_key}\n".encode()
    )
    connection.execute(
        """INSERT OR IGNORE INTO replays(
             replay_key,legacy_namespace,legacy_key) VALUES(?,?,?)""",
        (replay_key, namespace, legacy_key),
    )
    return int(connection.execute(
        "SELECT replay_id FROM replays WHERE replay_key=?", (replay_key,)
    ).fetchone()[0])


def verify_manifest(result: Path) -> str | None:
    manifest = result / "MANIFEST.sha256"
    if not manifest.is_file():
        return None
    for number, line in enumerate(manifest.read_text().splitlines(), 1):
        match = re.fullmatch(r"([0-9a-fA-F]{64}) [ *](.+)", line)
        if not match:
            raise ValueError(f"{manifest}:{number}: malformed checksum")
        expected, relative = match.groups()
        candidate = result / relative
        if not candidate.is_file() or sha256_file(candidate) != expected.lower():
            raise ValueError(f"{manifest}:{number}: checksum mismatch for {relative}")
    return sha256_file(manifest)


def native_extent(logical: str, workspace: Path | None) -> tuple[int, int] | None:
    logical_path = Path(logical)
    if not logical_path.is_absolute():
        if workspace is None:
            return None
        logical_path = workspace / logical_path
    native = Path(f"{logical_path}.parity.bitcode.zst")
    if not native.is_file() or native.stat().st_size < 36:
        return None
    with native.open("rb") as handle:
        handle.seek(-36, os.SEEK_END)
        footer = handle.read(36)
    if footer[:16] != b"RHPRTRACEFOOTER!":
        return None
    frame_count = int.from_bytes(footer[20:28], "little")
    final_frame = int.from_bytes(footer[28:36], "little")
    return frame_count, final_frame


def native_path(logical: str, workspace: Path | None) -> Path | None:
    path = Path(logical)
    if not path.is_absolute():
        if workspace is None:
            return None
        path = workspace / path
    return Path(f"{path}.parity.bitcode.zst")


def classify(status: str, command_status: int | None, marker_count: int, log: str) -> str:
    if status == "0" and command_status == 0 and marker_count == 1:
        return "exact_eof"
    if DIVERGENCE_RE.search(log) or "divergent frames" in log:
        return "mismatch"
    if status.startswith("aborted-") or command_status in (137, 143):
        return "aborted"
    if status.startswith("integrity-") or (command_status == 0 and marker_count != 1):
        return "integrity_error"
    if command_status == 124 or status == "124":
        return "timeout"
    if command_status not in (None, 0) or status.isdigit():
        return "crash"
    return "unknown"


def numeric(value: str | None) -> int | None:
    if value is None or not re.fullmatch(r"-?[0-9]+", value):
        return None
    return int(value)


def import_result(
    connection: sqlite3.Connection,
    result: Path,
    audit_root: Path,
    workspace: Path | None,
    host: str,
    fallback_env: dict[str, str] | None = None,
) -> bool:
    attestation = result / "attestation.env"
    status_path = result / "status"
    log_path = result / "log"
    if not log_path.is_file():
        log_path = result / "run.log"
    if not (attestation.is_file() and status_path.is_file() and log_path.is_file()):
        raise ValueError(f"incomplete replay evidence directory: {result}")
    manifest_sha = verify_manifest(result)
    env = parse_env(attestation)
    for key, value in (fallback_env or {}).items():
        env.setdefault(key, value)
    status_lines = status_path.read_text().splitlines()
    if len(status_lines) != 1 or not status_lines[0]:
        raise ValueError(f"{status_path}: expected one nonempty status line")
    status = status_lines[0]
    log_bytes = log_path.read_bytes()
    log_sha = sha256_bytes(log_bytes)
    if env.get("LOG_SHA256", log_sha).lower() != log_sha:
        raise ValueError(f"{log_path}: attested log hash mismatch")
    log = log_bytes.decode(errors="replace")
    trace_file = result / "trace.path"
    if trace_file.is_file():
        traces = trace_file.read_text().splitlines()
        if len(traces) != 1:
            raise ValueError(f"{trace_file}: expected exactly one trace path")
        logical = workspace_relative(traces[0], workspace)
    elif "LOGICAL_TRACE" in env:
        logical = workspace_relative(env["LOGICAL_TRACE"], workspace)
    else:
        raise ValueError(f"{result}: no logical trace identity")
    completion = env.get("COMPLETION_MARKER") or None
    if completion is not None:
        completion = workspace_relative(completion, workspace)
    runner_id = upsert_runner(connection, env)
    replay_id = upsert_replay(connection, logical, completion)
    command_status = numeric(
        env.get(
            "RUNNER_COMMAND_STATUS",
            env.get("RUNNER_STATUS", env.get("COMMAND_STATUS", env.get("RUN_STATUS"))),
        )
    )
    marker_count = numeric(
        env.get("EXACT_EOF_MARKER_COUNT", env.get("EXACT_SUCCESS_MARKER_COUNT"))
    )
    if marker_count is None:
        marker_count = sum(line == EOF_MARKER for line in log.splitlines())
    outcome = classify(status, command_status, marker_count, log)
    divergence = [int(value) for value in DIVERGENCE_RE.findall(log)]
    progress = divergence + [int(value) for value in HALTED_RE.findall(log)]
    progress += [int(value) for value in THROUGH_RE.findall(log)]
    divergence_frame = max(divergence) if divergence else None
    furthest_frame = max(progress) if progress else None
    matched_prefix_frames = recorded_frames = terminal_frame = None
    progress_precision = "universal_frame" if furthest_frame is not None else "unknown"
    if outcome == "exact_eof":
        native = native_path(logical, workspace)
        attested_native = env.get("NATIVE_SHA256_POST", env.get("NATIVE_SHA256_PRE"))
        extent = None
        if native is not None and native.is_file() and attested_native:
            if sha256_file(native) == attested_native.lower():
                extent = native_extent(logical, workspace)
        if extent is not None:
            recorded_frames, terminal_frame = extent
            matched_prefix_frames = recorded_frames
            furthest_frame = terminal_frame
            progress_precision = "exact"
        else:
            progress_precision = "exact_eof_unknown_extent"
    evidence_relative = str(result.relative_to(audit_root))
    evidence_seed = (
        "replay-run-v1\n"
        f"AUDIT={audit_root.resolve()}\n"
        f"EVIDENCE={evidence_relative}\n"
        f"MANIFEST={manifest_sha or log_sha}\n"
    )
    evidence_key = sha256_bytes(evidence_seed.encode())
    inserted = connection.execute(
        """INSERT OR IGNORE INTO replay_runs(
             evidence_key,replay_id,runner_id,run_kind,evidence_tier,outcome,result_status,
             command_status,exact_eof,eof_marker_count,furthest_frame,divergence_frame,
             matched_prefix_frames,recorded_frames,terminal_frame,progress_precision,
             started_utc,finished_utc,evidence_mtime_utc,timestamp_source,host,native_sha256_pre,native_sha256_post,
             completion_marker_sha256,log_sha256,evidence_manifest_sha256,audit_path,
             evidence_path,log_path,command,data_dir)
           VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)""",
        (
            evidence_key,
            replay_id,
            runner_id,
            "incremental_eof",
            "attested",
            outcome,
            status,
            command_status,
            int(outcome == "exact_eof"),
            marker_count,
            furthest_frame,
            divergence_frame,
            matched_prefix_frames,
            recorded_frames,
            terminal_frame,
            progress_precision,
            env.get("STARTED_UTC"),
            env.get("FINISHED_UTC"),
            None,
            "attested",
            host,
            env.get("NATIVE_SHA256_PRE"),
            env.get("NATIVE_SHA256_POST"),
            env.get("COMPLETION_MARKER_SHA256", env.get("MARKER_SHA256_POST")),
            log_sha,
            manifest_sha,
            str(audit_root.resolve()),
            str(result.resolve()),
            str(log_path.resolve()),
            env.get("COMMAND"),
            env.get("DATA_DIR"),
        ),
    ).rowcount
    return bool(inserted)


def import_audit(
    connection: sqlite3.Connection,
    audit: Path,
    workspace: Path | None,
    host: str,
) -> tuple[int, int]:
    fallback_env: dict[str, str] = {}
    for name in ("PROVENANCE.env", "provenance.env"):
        path = audit / name
        if path.is_file():
            fallback_env.update(parse_env(path))
    candidates = sorted(
        path.parent
        for path in audit.rglob("attestation.env")
        if (path.parent / "status").is_file()
        and ((path.parent / "log").is_file() or (path.parent / "run.log").is_file())
    )
    inserted = 0
    with connection:
        for result in candidates:
            inserted += import_result(
                connection, result, audit, workspace, host, fallback_env
            )
    provisional_inserted, provisional_seen = import_provisional_audit(
        connection, audit, workspace, host
    )
    return inserted + provisional_inserted, len(candidates) + provisional_seen


def provisional_runner_env(provenance: dict[str, str]) -> dict[str, str]:
    raw = provenance["runner_sha256"].lower()
    wrapper = provenance.get("runner_wrapper_sha256")
    manifest = provenance.get(
        "bundle_manifest_sha256", provenance.get("bundle_payload_manifest_sha256")
    )
    library = provenance.get("bundle_lib_manifest_sha256")
    identity_material = (
        "provisional-runner-deployment-v1\n"
        f"RAW={raw}\nWRAPPER={wrapper or 'unknown'}\n"
        f"MANIFEST={manifest or 'unknown'}\nLIB={library or 'unknown'}\n"
    )
    return {
        "RUNNER_RAW_SHA256": raw,
        "RUNNER_BUNDLE_TRUST_SHA256": sha256_bytes(identity_material.encode()),
        **({"RUNNER_WRAPPER_SHA256": wrapper} if wrapper else {}),
        **({"RUNNER_BUNDLE_MANIFEST_SHA256": manifest} if manifest else {}),
        **({"RUNNER_LIB_MANIFEST_SHA256": library} if library else {}),
    }


def import_provisional_audit(
    connection: sqlite3.Connection,
    audit: Path,
    workspace: Path | None,
    host: str,
) -> tuple[int, int]:
    provenance_path = audit / "SWEEP_PROVENANCE"
    snapshot_path = audit / "traces.snapshot"
    logs_dir = audit / "logs"
    if not (provenance_path.is_file() and snapshot_path.is_file() and logs_dir.is_dir()):
        return 0, 0
    provenance = parse_env(provenance_path)
    raw_runner = provenance.get("runner_sha256")
    runner_id = upsert_provisional_runner(
        connection, audit.name, raw_runner[:12] if raw_runner else None, raw_runner
    )
    corpus = Path(provenance["corpus"])
    by_log_name: dict[str, str] = {}
    for raw in snapshot_path.read_text().splitlines():
        logical = Path(raw)
        relative = logical.relative_to(corpus / "traces")
        key = "traces__" + str(relative).replace("/", "__") + ".log"
        if key in by_log_name:
            raise ValueError(f"{snapshot_path}: colliding provisional log key {key}")
        by_log_name[key] = workspace_relative(raw, workspace)
    inserted = seen = 0
    with connection:
        for log_path in sorted(path for path in logs_dir.iterdir() if path.is_file()):
            marker = ".log"
            offset = log_path.name.find(marker)
            if offset < 0:
                continue
            canonical_name = log_path.name[: offset + len(marker)]
            logical = by_log_name.get(canonical_name)
            if logical is None:
                raise ValueError(f"{log_path}: not present in frozen trace snapshot")
            seen += 1
            log_bytes = log_path.read_bytes()
            log_sha = sha256_bytes(log_bytes)
            log = log_bytes.decode(errors="replace")
            marker_count = sum(line == EOF_MARKER for line in log.splitlines())
            divergence = [int(value) for value in DIVERGENCE_RE.findall(log)]
            if marker_count == 1:
                outcome, status = "exact_eof", "0"
            elif divergence or "divergent frames" in log:
                outcome, status = "mismatch", "1"
            elif not log_bytes:
                outcome, status = "aborted", "interrupted-empty-log"
            elif re.search(r"panicked at|fatal runtime error|segmentation fault|aborted", log, re.I):
                outcome, status = "crash", "runner-crash"
            else:
                outcome, status = "aborted", "interrupted-provisional-log"
            replay_name = Path(logical).name.split("-session-", 1)[0]
            completion = str(Path(logical).parent / f"{replay_name}.complete")
            replay_id = upsert_replay(connection, logical, completion)
            divergence_frame = max(divergence) if divergence else None
            recorded_frames = terminal_frame = matched_prefix = None
            furthest_frame = divergence_frame
            precision = (
                "universal_frame"
                if divergence_frame is not None
                else "exact_eof_unknown_extent"
                if outcome == "exact_eof"
                else "unknown"
            )
            evidence_seed = (
                "replay-run-v1\n"
                f"AUDIT={audit.resolve()}\nEVIDENCE={log_path.relative_to(audit)}\n"
                f"MANIFEST={log_sha}\n"
            )
            mtime = log_path.stat().st_mtime
            evidence_mtime = datetime.datetime.fromtimestamp(
                mtime, datetime.timezone.utc
            ).isoformat().replace("+00:00", "Z")
            rowcount = connection.execute(
                """INSERT OR IGNORE INTO replay_runs(
                   evidence_key,replay_id,runner_id,run_kind,evidence_tier,outcome,
                   result_status,command_status,exact_eof,eof_marker_count,
                   furthest_frame,divergence_frame,matched_prefix_frames,recorded_frames,
                   terminal_frame,progress_precision,started_utc,finished_utc,host,
                   evidence_mtime_utc,timestamp_source,
                   native_sha256_pre,native_sha256_post,completion_marker_sha256,
                   log_sha256,evidence_manifest_sha256,audit_path,evidence_path,
                   log_path,command,data_dir)
                   VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)""",
                (
                    sha256_bytes(evidence_seed.encode()), replay_id, runner_id,
                    "provisional_eof", "provisional", outcome, status, None,
                    int(outcome == "exact_eof"), marker_count, furthest_frame,
                    divergence_frame, matched_prefix, recorded_frames, terminal_frame,
                    precision, None, None, host, evidence_mtime, "filesystem_mtime",
                    None, None, None, log_sha, None,
                    str(audit.resolve()), str(log_path.resolve()), str(log_path.resolve()),
                    "provisional parity sweep", provenance.get("data_dir"),
                ),
            ).rowcount
            inserted += rowcount
    return inserted, seen


def logical_from_legacy_log(log: str, workspace: Path | None) -> str | None:
    candidates = re.findall(r"(?:/[^\s\x1b]+)?\.jsonl\.zst(?:\.parity[^\s\x1b]*)?", log)
    for candidate in candidates:
        logical = candidate.split(".jsonl.zst", 1)[0] + ".jsonl.zst"
        marker = "/parity-save-replays/"
        if marker in logical:
            logical = "parity-save-replays/" + logical.split(marker, 1)[1]
        return workspace_relative(logical, workspace)
    return None


def legacy_runner_prefix(label: str) -> str | None:
    matches = re.findall(r"(?<![0-9a-f])([0-9a-f]{8,64})(?![0-9a-f])", label.lower())
    return matches[-1] if matches else None


def import_legacy_audit(
    connection: sqlite3.Connection,
    audit: Path,
    legacy_root: Path,
    namespace: str,
    workspace: Path | None,
    host: str,
) -> tuple[int, int]:
    logs_dir = audit / "logs"
    if not logs_dir.is_dir() or (audit / "SWEEP_PROVENANCE").is_file():
        return 0, 0
    runner_label = str(audit.relative_to(legacy_root))
    runner_id = upsert_provisional_runner(
        connection, runner_label, legacy_runner_prefix(runner_label)
    )
    inserted = seen = 0
    for log_path in sorted(path for path in logs_dir.iterdir() if path.is_file()):
        match = re.fullmatch(r"(.+)\.log(?:\.tmp\..+)?", log_path.name)
        if not match:
            continue
        seen += 1
        legacy_key = match.group(1)
        temporary = ".log.tmp." in log_path.name
        status_path = audit / "status" / f"{legacy_key}.status"
        if not status_path.is_file():
            status_path = audit / "status" / f"{legacy_key}.tsv"
        status = None
        status_label = None
        explicit_logical = None
        status_sha = "missing"
        if not temporary and status_path.is_file():
            status_lines = status_path.read_text(errors="replace").splitlines()
            if len(status_lines) == 1 and status_path.suffix == ".tsv":
                fields = status_lines[0].split("\t", 2)
                if len(fields) >= 2:
                    status_label = fields[0]
                    explicit_logical = workspace_relative(fields[1], workspace)
                    status = "0" if status_label in ("exact", "matched") else None
            elif len(status_lines) == 1:
                status = status_lines[0]
                status_label = status
            status_sha = sha256_file(status_path)
        log_bytes = log_path.read_bytes()
        log_sha = sha256_bytes(log_bytes)
        log = log_bytes.decode(errors="replace")
        logical = explicit_logical or logical_from_legacy_log(log, workspace)
        replay_id = (
            upsert_replay(connection, logical, None)
            if logical is not None
            else upsert_legacy_replay(connection, namespace, legacy_key)
        )
        marker_count = sum(line == EOF_MARKER for line in log.splitlines())
        command_status = numeric(status)
        divergence = [int(value) for value in DIVERGENCE_RE.findall(log)]
        if temporary:
            outcome, result_status = "aborted", "interrupted-publication"
        elif status == "0" and marker_count == 1:
            outcome, result_status = "exact_eof", status_label or status
        elif divergence or "divergent frames" in log or status_label == "divergent":
            outcome, result_status = "mismatch", status_label or status or "divergence"
        elif command_status == 124:
            outcome, result_status = "timeout", status_label or status
        elif re.search(r"panicked at|fatal runtime error|segmentation fault|aborted", log, re.I):
            outcome, result_status = "crash", status_label or status or "runner-crash"
        elif command_status not in (None, 0):
            outcome, result_status = "crash", status_label or status
        else:
            outcome, result_status = "aborted", status_label or status or "incomplete-legacy-evidence"
        divergence_frame = max(divergence) if divergence else None
        precision = "universal_frame" if divergence_frame is not None else (
            "exact_eof_unknown_extent" if outcome == "exact_eof" else "unknown"
        )
        evidence_seed = (
            "legacy-replay-run-v1\n"
            f"SOURCE_NAMESPACE={namespace}\n"
            f"AUDIT_REL={audit.relative_to(legacy_root)}\n"
            f"EVIDENCE_REL={log_path.relative_to(audit)}\n"
            f"LOG_SHA256={log_sha}\nSTATUS_SHA256={status_sha}\n"
        )
        evidence_mtime = utc_text(datetime.datetime.fromtimestamp(
            log_path.stat().st_mtime, datetime.timezone.utc
        ))
        inserted += connection.execute(
            """INSERT OR IGNORE INTO replay_runs(
               evidence_key,replay_id,runner_id,run_kind,evidence_tier,outcome,
               result_status,command_status,exact_eof,eof_marker_count,
               furthest_frame,divergence_frame,matched_prefix_frames,recorded_frames,
               terminal_frame,progress_precision,started_utc,finished_utc,host,
               evidence_mtime_utc,timestamp_source,native_sha256_pre,native_sha256_post,
               completion_marker_sha256,log_sha256,evidence_manifest_sha256,audit_path,
               evidence_path,log_path,command,data_dir)
               VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)""",
            (
                sha256_bytes(evidence_seed.encode()), replay_id, runner_id, "legacy_eof",
                "provisional", outcome, result_status, command_status,
                int(outcome == "exact_eof"), marker_count, divergence_frame,
                divergence_frame, None, None, None, precision, None, None, host,
                evidence_mtime, "filesystem_mtime", None, None, None, log_sha, None,
                str(audit.resolve()), str(log_path.resolve()), str(log_path.resolve()),
                "legacy parity sweep", None,
            ),
        ).rowcount
    return inserted, seen


def import_legacy_tree(
    connection: sqlite3.Connection,
    root: Path,
    namespace: str,
    workspace: Path | None,
    host: str,
) -> tuple[int, int, int]:
    inserted = seen = audits = 0
    with connection:
        for logs_dir in sorted(path for path in root.rglob("logs") if path.is_dir()):
            audit = logs_dir.parent
            if (audit / "SWEEP_PROVENANCE").is_file():
                add, observed = import_provisional_audit(
                    connection, audit, workspace, host
                )
            else:
                add, observed = import_legacy_audit(
                    connection, audit, root, namespace, workspace, host
                )
            if observed:
                inserted += add
                seen += observed
                audits += 1
    return inserted, seen, audits


def register_corpus(connection: sqlite3.Connection, corpus: Path, workspace: Path) -> int:
    campaign_path = corpus / "campaign.env"
    # Campaign files are operational journals and older controllers appended
    # updated values. Attestations remain strict; corpus metadata uses the last
    # recorded value for an intentionally repeated key.
    campaign = parse_env(campaign_path, allow_duplicates=True) if campaign_path.is_file() else {}
    root = workspace_relative(str(corpus.resolve()), workspace)
    corpus_id = upsert_corpus(
        connection,
        root,
        numeric(campaign.get("PARITY_INPUT_SEED_BASE")),
        numeric(campaign.get("PARITY_TRACE_SCHEMA")),
        numeric(campaign.get("EXPECTED_LOGICAL_REPLAYS")),
        sha256_file(campaign_path) if campaign_path.is_file() else None,
    )
    connection.execute(
        """UPDATE corpora SET corpus_status='active',corpus_path=?,
             retirement_reason=NULL,retired_utc=NULL WHERE corpus_id=?""",
        (str(corpus.resolve()), corpus_id),
    )
    connection.execute(
        """INSERT INTO corpus_locations(corpus_id,host,path,note) VALUES(?,?,?,?)
           ON CONFLICT(corpus_id,host,path) DO UPDATE SET
             note=excluded.note,observed_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now')""",
        (corpus_id, socket.gethostname(), str(corpus.resolve()), "registered corpus location"),
    )
    count = 0
    with connection:
        for marker in sorted((corpus / "traces").rglob("replay-*.complete")):
            stem = marker.name.removesuffix(".complete")
            artifacts = sorted(marker.parent.glob(f"{stem}-session-*.jsonl.zst"))
            artifacts += sorted(marker.parent.glob(f"{stem}-session-*.jsonl.zst.parity.bitcode.zst"))
            logicals = {str(path).removesuffix(".parity.bitcode.zst") for path in artifacts}
            if len(logicals) != 1:
                raise ValueError(f"{marker}: expected one logical recording, found {len(logicals)}")
            logical = workspace_relative(next(iter(logicals)), workspace)
            completion = workspace_relative(str(marker), workspace)
            replay_id = upsert_replay(connection, logical, completion)
            connection.execute(
                "UPDATE replays SET corpus_id=? WHERE replay_id=?", (corpus_id, replay_id)
            )
            count += 1
    return count


def retire_corpus(
    connection: sqlite3.Connection, logical_root: str, reason: str
) -> None:
    if not reason.strip():
        raise ValueError("retirement reason must not be empty")
    changed = connection.execute(
        """UPDATE corpora SET corpus_status='retired',retirement_reason=?,retired_utc=?
           WHERE logical_root=?""",
        (reason, utc_text(utc_now()), logical_root),
    ).rowcount
    if changed != 1:
        raise ValueError(f"unknown corpus: {logical_root}")


def activate_corpus(
    connection: sqlite3.Connection,
    logical_root: str,
    expected: int,
    location_host: str | None,
    location_path: str | None,
    note: str | None,
    operational_path: str | None,
) -> int:
    if expected < 0:
        raise ValueError("expected replay count must be nonnegative")
    corpus_id = upsert_corpus(connection, logical_root, expected=expected)
    connection.execute(
        """UPDATE corpora SET corpus_status='active',expected_replays=?,
             corpus_path=coalesce(?,corpus_path),
             retirement_reason=NULL,retired_utc=NULL WHERE corpus_id=?""",
        (expected, operational_path, corpus_id),
    )
    trace_prefix = logical_root + "/traces/"
    assigned = connection.execute(
        "UPDATE replays SET corpus_id=? WHERE logical_path LIKE ?",
        (corpus_id, trace_prefix + "%"),
    ).rowcount
    if assigned == 0:
        assigned = connection.execute(
            "UPDATE replays SET corpus_id=? WHERE corpus_id IS NULL AND logical_path LIKE ?",
            (corpus_id, logical_root + "/%"),
        ).rowcount
    if location_host or location_path:
        if not (location_host and location_path):
            raise ValueError("location host and path must be supplied together")
        connection.execute(
            """INSERT INTO corpus_locations(corpus_id,host,path,note) VALUES(?,?,?,?)
               ON CONFLICT(corpus_id,host,path) DO UPDATE SET
                 note=excluded.note,observed_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now')""",
            (corpus_id, location_host, location_path, note),
        )
    return assigned


def set_corpus_path(
    connection: sqlite3.Connection,
    logical_root: str,
    path: str,
    host: str,
    note: str | None,
) -> str:
    """Update a corpus's host-local artifact root without changing its status."""
    if not Path(path).is_absolute():
        raise ValueError("corpus path must be absolute")
    corpus = connection.execute(
        "SELECT corpus_id FROM corpora WHERE logical_root=?", (logical_root,)
    ).fetchone()
    if corpus is None:
        raise ValueError(f"unknown corpus: {logical_root}")
    corpus_id = int(corpus["corpus_id"])
    connection.execute(
        "UPDATE corpora SET corpus_path=? WHERE corpus_id=?", (path, corpus_id)
    )
    connection.execute(
        """INSERT INTO corpus_locations(corpus_id,host,path,note) VALUES(?,?,?,?)
           ON CONFLICT(corpus_id,host,path) DO UPDATE SET
             note=excluded.note,observed_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now')""",
        (corpus_id, host, path, note),
    )
    return path


def merge_corpora(
    connection: sqlite3.Connection,
    target_root: str,
    source_roots: list[str],
    expected: int,
    seed_base: int | None,
    trace_schema: int | None,
) -> dict[str, int]:
    if expected <= 0:
        raise ValueError("merged corpus expected count must be positive")
    if not source_roots or len(source_roots) != len(set(source_roots)):
        raise ValueError("merge requires distinct source corpora")
    if target_root in source_roots:
        raise ValueError("merge target must not also be a source")
    sources = connection.execute(
        f"""SELECT corpus_id,logical_root,corpus_status FROM corpora
            WHERE logical_root IN ({','.join('?' for _ in source_roots)})""",
        source_roots,
    ).fetchall()
    if len(sources) != len(source_roots):
        found = {row["logical_root"] for row in sources}
        missing = sorted(set(source_roots) - found)
        raise ValueError(f"unknown source corpora: {', '.join(missing)}")
    inactive = [row["logical_root"] for row in sources if row["corpus_status"] != "active"]
    if inactive:
        raise ValueError(f"source corpora are not active: {', '.join(inactive)}")
    source_ids = [int(row["corpus_id"]) for row in sources]
    placeholders = ",".join("?" for _ in source_ids)
    active_work = connection.execute(
        f"""SELECT count(*) FROM corpus_work_leases
            WHERE corpus_id IN ({placeholders}) AND state='active'""",
        source_ids,
    ).fetchone()[0]
    if active_work:
        raise ValueError("cannot merge corpora with active corpus-work leases")
    target_id = upsert_corpus(
        connection, target_root, seed_base=seed_base,
        trace_schema=trace_schema, expected=expected,
    )
    connection.execute(
        """UPDATE corpora SET corpus_status='active',expected_replays=?,seed_base=?,
             trace_schema=?,campaign_sha256=NULL,corpus_path=NULL,
             retirement_reason=NULL,retired_utc=NULL WHERE corpus_id=?""",
        (expected, seed_base, trace_schema, target_id),
    )
    final_members = connection.execute(
        f"SELECT count(*) FROM final_corpus_members WHERE corpus_id IN ({placeholders})",
        source_ids,
    ).fetchone()[0]
    if final_members != expected:
        raise ValueError(
            f"merged final membership is {final_members}, expected {expected}"
        )
    connection.execute(
        f"UPDATE final_corpus_members SET corpus_id=? WHERE corpus_id IN ({placeholders})",
        (target_id, *source_ids),
    )
    moved_replays = connection.execute(
        f"UPDATE replays SET corpus_id=? WHERE corpus_id IN ({placeholders})",
        (target_id, *source_ids),
    ).rowcount
    connection.execute(
        f"""INSERT OR IGNORE INTO corpus_locations(corpus_id,host,path,note,observed_utc)
            SELECT ?,host,path,'artifact root of merged corpus',observed_utc
            FROM corpus_locations WHERE corpus_id IN ({placeholders})""",
        (target_id, *source_ids),
    )
    reason = f"merged into {target_root}"
    connection.execute(
        f"""UPDATE corpora SET corpus_status='retired',retirement_reason=?,retired_utc=?
            WHERE corpus_id IN ({placeholders})""",
        (reason, utc_text(utc_now()), *source_ids),
    )
    return {
        "target_corpus_id": target_id,
        "source_corpora": len(source_ids),
        "final_members": int(final_members),
        "moved_replays": int(moved_replays),
    }


def absorb_corpora(
    connection: sqlite3.Connection,
    target_root: str,
    source_roots: list[str],
    expected: int,
) -> dict[str, int]:
    """Fold active source corpora into an existing active target corpus."""
    if expected <= 0:
        raise ValueError("absorbed corpus expected count must be positive")
    if not source_roots or len(source_roots) != len(set(source_roots)):
        raise ValueError("absorb requires distinct source corpora")
    if target_root in source_roots:
        raise ValueError("absorb target must not also be a source")
    target = connection.execute(
        "SELECT corpus_id,corpus_status FROM corpora WHERE logical_root=?",
        (target_root,),
    ).fetchone()
    if target is None:
        raise ValueError(f"unknown target corpus: {target_root}")
    if target["corpus_status"] != "active":
        raise ValueError(f"target corpus is not active: {target_root}")
    sources = connection.execute(
        f"""SELECT corpus_id,logical_root,corpus_status FROM corpora
            WHERE logical_root IN ({','.join('?' for _ in source_roots)})""",
        source_roots,
    ).fetchall()
    if len(sources) != len(source_roots):
        found = {row["logical_root"] for row in sources}
        missing = sorted(set(source_roots) - found)
        raise ValueError(f"unknown source corpora: {', '.join(missing)}")
    inactive = [row["logical_root"] for row in sources if row["corpus_status"] != "active"]
    if inactive:
        raise ValueError(f"source corpora are not active: {', '.join(inactive)}")
    target_id = int(target["corpus_id"])
    source_ids = [int(row["corpus_id"]) for row in sources]
    all_ids = [target_id, *source_ids]
    placeholders = ",".join("?" for _ in all_ids)
    active_work = connection.execute(
        f"""SELECT count(*) FROM corpus_work_leases
            WHERE corpus_id IN ({placeholders}) AND state='active'""",
        all_ids,
    ).fetchone()[0]
    if active_work:
        raise ValueError("cannot absorb corpora with active corpus-work leases")
    final_members = connection.execute(
        f"SELECT count(*) FROM final_corpus_members WHERE corpus_id IN ({placeholders})",
        all_ids,
    ).fetchone()[0]
    if final_members != expected:
        raise ValueError(
            f"absorbed final membership is {final_members}, expected {expected}"
        )
    source_placeholders = ",".join("?" for _ in source_ids)
    connection.execute(
        """UPDATE corpora SET expected_replays=?,retirement_reason=NULL,retired_utc=NULL
           WHERE corpus_id=?""",
        (expected, target_id),
    )
    connection.execute(
        f"UPDATE final_corpus_members SET corpus_id=? WHERE corpus_id IN ({source_placeholders})",
        (target_id, *source_ids),
    )
    moved_replays = connection.execute(
        f"UPDATE replays SET corpus_id=? WHERE corpus_id IN ({source_placeholders})",
        (target_id, *source_ids),
    ).rowcount
    connection.execute(
        f"""INSERT OR IGNORE INTO corpus_locations(corpus_id,host,path,note,observed_utc)
            SELECT ?,host,path,'artifact root of absorbed corpus',observed_utc
            FROM corpus_locations WHERE corpus_id IN ({source_placeholders})""",
        (target_id, *source_ids),
    )
    reason = f"absorbed into {target_root}"
    connection.execute(
        f"""UPDATE corpora SET corpus_status='retired',retirement_reason=?,retired_utc=?
            WHERE corpus_id IN ({source_placeholders})""",
        (reason, utc_text(utc_now()), *source_ids),
    )
    return {
        "target_corpus_id": target_id,
        "source_corpora": len(source_ids),
        "final_members": int(final_members),
        "moved_replays": int(moved_replays),
    }


def add_replay(connection: sqlite3.Connection, logical_path: str) -> int:
    return upsert_replay(connection, logical_path, None)


def import_final_snapshot(connection: sqlite3.Connection, snapshot: Path) -> dict[str, int]:
    keys = [line for line in snapshot.read_text().splitlines() if line]
    if len(keys) != len(set(keys)):
        raise ValueError(f"{snapshot}: duplicate replay key")
    replay_by_key: dict[str, sqlite3.Row] = {}
    for replay in connection.execute(
        "SELECT replay_id,corpus_id,logical_path FROM replays WHERE logical_path IS NOT NULL"
    ):
        key = replay["logical_path"].replace("/", "__")
        if key in replay_by_key:
            raise ValueError(f"ambiguous encoded replay key: {key}")
        replay_by_key[key] = replay
    missing = [key for key in keys if key not in replay_by_key]
    if missing:
        raise ValueError(
            f"{snapshot}: {len(missing)} keys have no replay identity; first: {missing[0]}"
        )
    grouped: dict[int, list[int]] = {}
    for key in keys:
        replay = replay_by_key[key]
        corpus_id = replay["corpus_id"]
        if corpus_id is None:
            logical_root = str(Path(replay["logical_path"]).parent)
            corpus_id = upsert_corpus(connection, logical_root)
            connection.execute(
                "UPDATE replays SET corpus_id=? WHERE replay_id=?",
                (corpus_id, replay["replay_id"]),
            )
        grouped.setdefault(int(corpus_id), []).append(int(replay["replay_id"]))
    source = str(snapshot.resolve())
    for corpus_id, replay_ids in grouped.items():
        connection.execute(
            "DELETE FROM final_corpus_members WHERE corpus_id=?", (corpus_id,)
        )
        connection.executemany(
            """INSERT INTO final_corpus_members(corpus_id,replay_id,source_ledger)
               VALUES(?,?,?)""",
            ((corpus_id, replay_id, source) for replay_id in replay_ids),
        )
        connection.execute(
            """UPDATE corpora SET corpus_status='active',expected_replays=?,
                 retirement_reason=NULL,retired_utc=NULL WHERE corpus_id=?""",
            (len(replay_ids), corpus_id),
        )
    return {
        connection.execute(
            "SELECT logical_root FROM corpora WHERE corpus_id=?", (corpus_id,)
        ).fetchone()[0]: len(replay_ids)
        for corpus_id, replay_ids in grouped.items()
    }


def set_current_runner(connection: sqlite3.Connection, trust: str) -> None:
    trust = trust.lower()
    runner = connection.execute(
        """SELECT runner_id FROM runners
           WHERE bundle_trust_sha256=? AND identity_kind='authenticated'""",
        (trust,),
    ).fetchone()
    if runner is None:
        raise ValueError(f"unknown authenticated runner: {trust}")
    connection.execute(
        """INSERT INTO schema_meta(key,value) VALUES('current_runner_trust_sha256',?)
           ON CONFLICT(key) DO UPDATE SET value=excluded.value""",
        (trust,),
    )


def has_attested_exact(
    connection: sqlite3.Connection,
    logical_path: str,
    runner_trust: str,
    native_sha256: str,
) -> bool:
    runner_trust = runner_trust.lower()
    native_sha256 = native_sha256.lower()
    if not re.fullmatch(r"[0-9a-f]{64}", runner_trust):
        raise ValueError("runner trust must be a lowercase SHA-256 digest")
    if not re.fullmatch(r"[0-9a-f]{64}", native_sha256):
        raise ValueError("native hash must be a lowercase SHA-256 digest")
    return bool(connection.execute(
        """SELECT EXISTS(
             SELECT 1 FROM replay_runs rr
             JOIN replays replay USING(replay_id)
             JOIN runners runner USING(runner_id)
             WHERE replay.logical_path=?
               AND runner.bundle_trust_sha256=?
               AND rr.evidence_tier='attested'
               AND rr.exact_eof=1
               AND rr.native_sha256_pre=?
               AND rr.native_sha256_post=?
           )""",
        (logical_path, runner_trust, native_sha256, native_sha256),
    ).fetchone()[0])


def exact_evidence_key(
    connection: sqlite3.Connection,
    logical_path: str,
    runner_trust: str,
    native_sha256: str,
) -> str | None:
    runner_trust = runner_trust.lower()
    native_sha256 = native_sha256.lower()
    if not has_attested_exact(connection, logical_path, runner_trust, native_sha256):
        return None
    row = connection.execute(
        """SELECT rr.evidence_key FROM replay_runs rr
           JOIN replays replay USING(replay_id)
           JOIN runners runner USING(runner_id)
           WHERE replay.logical_path=? AND runner.bundle_trust_sha256=?
             AND rr.evidence_tier='attested' AND rr.exact_eof=1
             AND rr.native_sha256_pre=? AND rr.native_sha256_post=?
           ORDER BY rr.finished_utc DESC NULLS LAST,rr.run_id DESC LIMIT 1""",
        (logical_path, runner_trust, native_sha256, native_sha256),
    ).fetchone()
    return row["evidence_key"] if row else None


def summary(connection: sqlite3.Connection) -> dict[str, object]:
    inventory = connection.execute("SELECT count(*) FROM replays").fetchone()[0]
    runs = connection.execute("SELECT count(*) FROM replay_runs").fetchone()[0]
    latest = {
        row["effective_outcome"]: row["amount"]
        for row in connection.execute(
            """SELECT coalesce(c.corrected_outcome,ranked.outcome) AS effective_outcome,
                      count(*) AS amount
               FROM (
                 SELECT rr.*,row_number() OVER (
                   PARTITION BY replay_id
                   ORDER BY finished_utc DESC NULLS LAST,run_id DESC
                 ) AS rank
                 FROM replay_runs rr
               ) ranked LEFT JOIN replay_run_corrections c USING(evidence_key)
               WHERE rank=1 GROUP BY effective_outcome"""
        )
    }
    runners = [
        dict(row)
        for row in connection.execute(
            """SELECT substr(r.bundle_trust_sha256,1,12) AS runner,
                      count(l.run_id) AS tested,
                      sum(coalesce(l.exact_eof,0)) AS exact
               FROM runners r LEFT JOIN latest_replay_runner_runs l USING(runner_id)
               GROUP BY r.runner_id ORDER BY r.first_seen_utc"""
        )
    ]
    return {"replays": inventory, "runs": runs, "latest": latest, "runners": runners}


def discover_external_activity(corpora: list[dict[str, object]]) -> dict[str, object]:
    corpus_by_path = {
        str(corpus["corpus_path"]): corpus for corpus in corpora if corpus.get("corpus_path")
    }
    conversions: dict[tuple[str, str], list[int]] = {}
    orchestrators: list[dict[str, object]] = []
    proc = Path("/proc")
    if not proc.is_dir():
        return {"conversions": [], "orchestrators": []}
    for process in proc.iterdir():
        if not process.name.isdigit():
            continue
        try:
            arguments = [part.decode(errors="replace") for part in
                         (process / "cmdline").read_bytes().split(b"\0") if part]
        except (OSError, PermissionError):
            continue
        joined = " ".join(arguments)
        shell_process = Path(arguments[0]).name in ("bash", "sh") if arguments else False
        if shell_process and "run_native_conversion_prepass.sh" in joined:
            corpus_path = next((arg for arg in arguments if arg in corpus_by_path), None)
            if corpus_path:
                audit_path = next(
                    (arg for arg in reversed(arguments)
                     if arg.startswith("/srv/") and "/audits/" in arg),
                    "unknown",
                )
                conversions.setdefault((corpus_path, audit_path), []).append(int(process.name))
        if shell_process and "run_schema16_existing_corpora_orchestrator.sh" in joined:
            audit_path = next(
                (arg for arg in reversed(arguments)
                 if arg.startswith("/srv/") and "/audits/" in arg),
                None,
            )
            state: dict[str, str] = {}
            if audit_path and (Path(audit_path) / "state.env").is_file():
                try:
                    state = parse_env(Path(audit_path) / "state.env", allow_duplicates=True)
                except (OSError, ValueError):
                    state = {}
            orchestrators.append({
                "pid": int(process.name), "audit_path": audit_path,
                "phase": state.get("PHASE", "unknown"),
                "managed_corpora": [
                    Path(argument).name for argument in arguments if argument in corpus_by_path
                ],
                "managed_paths": [argument for argument in arguments if argument in corpus_by_path],
            })
    conversion_rows = []
    for (corpus_path, audit_path), pids in conversions.items():
        conversion_rows.append({
            "corpus_path": corpus_path,
            "corpus": Path(corpus_path).name,
            "audit_path": audit_path,
            "processes": len(pids),
            "pids": sorted(pids),
        })
    return {"conversions": conversion_rows, "orchestrators": orchestrators}


def overview(connection: sqlite3.Connection) -> dict[str, object]:
    trust_row = connection.execute(
        "SELECT value FROM schema_meta WHERE key='current_runner_trust_sha256'"
    ).fetchone()
    runner = None
    if trust_row is not None:
        runner_row = connection.execute(
            """SELECT runner_id,bundle_trust_sha256,raw_sha256
               FROM runners WHERE bundle_trust_sha256=?""",
            (trust_row[0],),
        ).fetchone()
        runner = dict(runner_row) if runner_row else None

    artifact_roots = [
        (row["logical_root"], Path(row["corpus_path"]))
        for row in connection.execute(
            "SELECT logical_root,corpus_path FROM corpora WHERE corpus_path IS NOT NULL"
        )
    ]
    artifact_roots.sort(key=lambda item: len(item[0]), reverse=True)

    def physical_path(logical: str | None) -> Path | None:
        if not logical:
            return None
        logical_path = Path(logical)
        if logical_path.is_absolute():
            return logical_path
        for root, root_path in artifact_roots:
            if logical.startswith(root + "/"):
                return root_path / logical[len(root) + 1 :]
        return None

    corpora: list[dict[str, object]] = []
    for corpus_row in connection.execute(
        """SELECT * FROM corpora WHERE corpus_status='active'
           ORDER BY seed_base,logical_root"""
    ):
        corpus = dict(corpus_row)
        corpus["locations"] = [dict(row) for row in connection.execute(
            """SELECT host,path,note,observed_utc FROM corpus_locations
               WHERE corpus_id=? ORDER BY observed_utc DESC""",
            (corpus["corpus_id"],),
        )]
        has_final_members = connection.execute(
            "SELECT EXISTS(SELECT 1 FROM final_corpus_members WHERE corpus_id=?)",
            (corpus["corpus_id"],),
        ).fetchone()[0]
        if has_final_members:
            replay_rows = connection.execute(
                """SELECT r.replay_id,r.logical_path,r.completion_marker
                   FROM final_corpus_members f JOIN replays r USING(replay_id)
                   WHERE f.corpus_id=?""",
                (corpus["corpus_id"],),
            ).fetchall()
        else:
            replay_rows = connection.execute(
                "SELECT replay_id,logical_path,completion_marker FROM replays WHERE corpus_id=?",
                (corpus["corpus_id"],),
            ).fetchall()
        source_only = native_only = coexisting = missing = markers = 0
        for replay in replay_rows:
            logical = replay["logical_path"]
            source = physical_path(logical)
            source_exists = bool(source and source.is_file())
            native_exists = bool(source and Path(f"{source}.parity.bitcode.zst").is_file())
            if source_exists and native_exists:
                coexisting += 1
            elif source_exists:
                source_only += 1
            elif native_exists:
                native_only += 1
            else:
                missing += 1
            marker = replay["completion_marker"]
            if marker:
                marker_path = physical_path(marker)
                markers += int(bool(marker_path and marker_path.is_file()))
        outcomes: dict[str, int] = {}
        if runner is not None:
            outcomes = {
                row["outcome"]: row["amount"] for row in connection.execute(
                    """SELECT outcome,count(*) AS amount FROM (
                         SELECT coalesce(c.corrected_outcome,rr.outcome) AS outcome,
                                row_number() OVER (
                           PARTITION BY rr.replay_id
                           ORDER BY rr.finished_utc DESC NULLS LAST,rr.run_id DESC
                         ) AS rank
                         FROM replay_runs rr JOIN replays r USING(replay_id)
                         LEFT JOIN replay_run_corrections c USING(evidence_key)
                         WHERE r.corpus_id=? AND rr.runner_id=?
                           AND (?=0 OR EXISTS(
                             SELECT 1 FROM final_corpus_members f
                             WHERE f.corpus_id=r.corpus_id AND f.replay_id=r.replay_id
                           ))
                       ) WHERE rank=1 GROUP BY outcome""",
                    (corpus["corpus_id"], runner["runner_id"], has_final_members),
                )
            }
        exact = outcomes.get("exact_eof", 0)
        failed = sum(
            outcomes.get(outcome, 0)
            for outcome in ("mismatch", "crash", "timeout", "integrity_error", "unknown")
        )
        # An aborted attempt is not evidence about parity. It remains untested
        # and must be eligible for a later exact rerun.
        tested = exact + failed
        expected = corpus["expected_replays"] or len(replay_rows)
        corpus.update(
            registered=len(replay_rows), markers=markers, source_only=source_only,
            native_only=native_only, coexisting=coexisting, missing=missing,
            native_ready=native_only + coexisting, runner_outcomes=outcomes,
            current_exact=exact, current_failed=failed,
            current_aborted=outcomes.get("aborted", 0),
            current_untested=max(0, expected - tested), remaining=max(0, expected - exact),
            inventory_missing=max(0, expected - len(replay_rows)),
        )
        corpora.append(corpus)

    work = [dict(row) for row in connection.execute(
        """SELECT wi.operation,count(*) AS total,
                  sum(CASE WHEN done.work_id IS NOT NULL THEN 1 ELSE 0 END) AS completed,
                  sum(CASE WHEN done.work_id IS NULL AND claim.work_id IS NOT NULL THEN 1 ELSE 0 END) AS claimed,
                  sum(CASE WHEN done.work_id IS NULL AND claim.work_id IS NULL THEN 1 ELSE 0 END) AS queued
           FROM work_items wi LEFT JOIN work_completions done USING(work_id)
           LEFT JOIN work_claims claim USING(work_id)
           GROUP BY wi.operation ORDER BY wi.operation"""
    )]
    active_claims = [dict(row) for row in connection.execute(
        """SELECT wi.operation,wc.worker_id,wc.claimed_utc,wc.lease_until_utc,
                  r.logical_path
           FROM work_claims wc JOIN work_items wi USING(work_id)
           JOIN replays r USING(replay_id)
           WHERE wc.lease_until_utc > ? ORDER BY wc.lease_until_utc""",
        (utc_text(utc_now()),),
    )]
    corpus_work = [dict(row) for row in connection.execute(
        """SELECT c.logical_root,cwl.operation,cwl.worker_id,cwl.host,
                  cwl.audit_path,cwl.claimed_utc,cwl.heartbeat_utc,
                  cwl.lease_until_utc,cwl.detail
           FROM corpus_work_leases cwl JOIN corpora c USING(corpus_id)
           WHERE cwl.state='active' AND cwl.lease_until_utc > ?
           ORDER BY cwl.lease_until_utc,c.logical_root""",
        (utc_text(utc_now()),),
    )]
    hidden = {
        row["corpus_status"]: row["amount"] for row in connection.execute(
            """SELECT corpus_status,count(*) AS amount FROM corpora
               WHERE corpus_status<>'active' GROUP BY corpus_status"""
        )
    }
    external = discover_external_activity(corpora)
    converting_paths = {row["corpus_path"] for row in external["conversions"]}
    managed_paths = {
        path for orchestrator in external["orchestrators"]
        for path in orchestrator["managed_paths"]
    }
    leased_operations = {
        (row["logical_root"], row["operation"]): row for row in corpus_work
    }
    total_expected = sum(int(corpus["expected_replays"] or corpus["registered"]) for corpus in corpora)
    total_exact = sum(int(corpus["current_exact"]) for corpus in corpora)
    actions: list[str] = []
    restore_total = restore_corpora = 0
    restore_hosts: set[str] = set()
    if runner is None:
        actions.append("Set the authenticated current runner with set-current-runner.")
    for corpus in corpora:
        name = Path(str(corpus["logical_root"])).name
        leased_conversion = leased_operations.get((corpus["logical_root"], "convert"))
        leased_transfer = leased_operations.get((corpus["logical_root"], "transfer"))
        leased_replay = leased_operations.get((corpus["logical_root"], "replay"))
        leased_blocker = leased_conversion or leased_transfer
        if corpus["inventory_missing"]:
            corpus["next_state"] = "RESTORE"
            if leased_blocker is None:
                restore_total += int(corpus["inventory_missing"])
                restore_corpora += 1
        if corpus["missing"]:
            corpus["next_state"] = "RESTORE"
            if leased_blocker is None:
                restore_total += int(corpus["missing"])
                restore_corpora += int(not corpus["inventory_missing"])
                restore_hosts.update(location["host"] for location in corpus["locations"])
        conversion_remaining = int(corpus["registered"]) - int(corpus["native_ready"])
        if leased_transfer is not None:
            corpus["next_state"] = "TRANSFERRING"
            actions.append(
                f"Monitor leased transfer of {name} to {leased_transfer['host']} "
                f"by {leased_transfer['worker_id']}; do not duplicate it."
            )
        elif leased_conversion is not None:
            corpus["next_state"] = "CONVERTING"
            actions.append(
                f"Monitor leased conversion of {name} on {leased_conversion['host']} "
                f"by {leased_conversion['worker_id']}; do not duplicate it."
            )
        elif conversion_remaining > 0 and not corpus["missing"]:
            if corpus["corpus_path"] in converting_paths:
                corpus["next_state"] = "CONVERTING"
                actions.append(f"Monitor active conversion of {conversion_remaining} recordings in {name}; do not duplicate it.")
            elif corpus["corpus_path"] in managed_paths:
                corpus["next_state"] = "PENDING"
                actions.append(f"Conversion of {conversion_remaining} recordings in {name} is pending under the active orchestrator; do not duplicate it.")
            else:
                corpus["next_state"] = "CONVERT"
                actions.append(f"Convert {conversion_remaining} recordings in {name} to bitcode.")
        if corpus["current_failed"]:
            corpus["next_state"] = "FIX"
            actions.append(f"Investigate and rerun {corpus['current_failed']} failures in {name}.")
        if conversion_remaining == 0 and corpus["current_untested"]:
            if leased_replay is not None:
                corpus["next_state"] = "EOF"
                actions.append(
                    f"Monitor prioritized EOF testing of {corpus['current_untested']} "
                    f"recordings in {name} on {leased_replay['host']} by "
                    f"{leased_replay['worker_id']}; do not duplicate it."
                )
            elif corpus["corpus_path"] in managed_paths:
                corpus["next_state"] = "PENDING"
                actions.append(f"EOF testing of {corpus['current_untested']} recordings in {name} is pending under the active orchestrator.")
            else:
                corpus["next_state"] = "EOF"
                actions.append(f"EOF-test {corpus['current_untested']} recordings in {name}.")
        corpus.setdefault("next_state", "DONE" if not corpus["remaining"] else "BLOCKED")
    if restore_total:
        hosts = ", ".join(sorted(restore_hosts)) or "no recorded host"
        actions.insert(
            0,
            f"Restore or transfer {restore_total} final-set recordings across "
            f"{restore_corpora} RESTORE corpora from {hosts}; paths are in overview --json.",
        )
    # Replay validation is the product of conversion: it exposes parity bugs
    # and therefore takes precedence over producing more converted inventory.
    # Keep that policy in the overview so it remains a safe operator handoff.
    def action_priority(action: str) -> int:
        if action.startswith(("Set the authenticated", "Investigate and rerun")):
            return 0
        if "EOF" in action:
            return 1
        if action.startswith("Restore or transfer"):
            return 2
        if "transfer" in action.lower():
            return 3
        if "conversion" in action.lower() or action.startswith("Convert "):
            return 4
        return 5

    actions.sort(key=action_priority)
    if not actions and total_expected == total_exact:
        actions.append("Final set is exact at EOF on the current runner.")
    return {
        "generated_utc": utc_text(utc_now()),
        "current_runner": runner,
        "final_set": corpora,
        "totals": {"expected": total_expected, "current_exact": total_exact,
                   "remaining": total_expected - total_exact},
        "work": work,
        "active_claims": active_claims,
        "corpus_work": corpus_work,
        "external_activity": external,
        "next_actions": actions,
        "hidden_corpora": hidden,
        "history": summary(connection),
    }


def print_overview(report: dict[str, object]) -> None:
    runner = report["current_runner"]
    if runner:
        print(f"FINAL PARITY SET | runner={runner['bundle_trust_sha256'][:12]} raw={runner['raw_sha256'][:12]}")
    else:
        print("FINAL PARITY SET | runner=NOT CONFIGURED")
    print("corpus                                             state       ready       exact      failed  untested  remaining")
    for corpus in report["final_set"]:
        name = str(corpus["logical_root"])
        name = name.removeprefix("parity-save-replays/")
        name = name.removeprefix("parity-save-replays-legacy/")
        print(
            f"{name:<50} {corpus['next_state']:<10} {corpus['native_ready']:>5}/{corpus['registered']:<5} "
            f"{corpus['current_exact']:>5}/{corpus['expected_replays'] or corpus['registered']:<5} "
            f"{corpus['current_failed']:>7} {corpus['current_untested']:>9} {corpus['remaining']:>10}"
        )
    totals = report["totals"]
    print(f"TOTAL current-runner exact={totals['current_exact']}/{totals['expected']} remaining={totals['remaining']}")
    print("\nNEXT ACTIONS")
    for action in report["next_actions"]:
        print(f"- {action}")
    print("\nACTIVE WORK")
    external = report["external_activity"]
    if not report["work"] and not report["active_claims"] and not report["corpus_work"] and not external["conversions"] and not external["orchestrators"]:
        print("- No database-scheduled work or active claims.")
    for work in report["work"]:
        print(
            f"- {work['operation']}: total={work['total']} queued={work['queued']} "
            f"claimed={work['claimed']} completed={work['completed']}"
        )
    for claim in report["active_claims"]:
        print(f"- {claim['operation']} {claim['logical_path']} by {claim['worker_id']} until {claim['lease_until_utc']}")
    for lease in report["corpus_work"]:
        print(
            f"- leased {lease['operation']}: {lease['logical_root']} on {lease['host']} "
            f"by {lease['worker_id']} until {lease['lease_until_utc']} "
            f"audit={lease['audit_path'] or 'none'}"
        )
    for conversion in external["conversions"]:
        print(
            f"- external conversion active: {conversion['corpus']} "
            f"processes={conversion['processes']} audit={conversion['audit_path']}"
        )
    for orchestrator in external["orchestrators"]:
        print(
            f"- orchestrator pid={orchestrator['pid']} phase={orchestrator['phase']} "
            f"corpora={','.join(orchestrator['managed_corpora'])} audit={orchestrator['audit_path']}"
        )
    hidden = sum(report["hidden_corpora"].values())
    print(f"\nHistorical/retired corpora hidden: {hidden} (available via summary or overview --json)")


def utc_now() -> datetime.datetime:
    return datetime.datetime.now(datetime.timezone.utc)


def utc_text(value: datetime.datetime) -> str:
    return value.isoformat(timespec="milliseconds").replace("+00:00", "Z")


def add_work(
    connection: sqlite3.Connection,
    logical_path: str,
    operation: str,
    runner_trust: str | None,
    protocol: int | None,
    target_encoding: str | None,
    source_sha256: str | None,
    priority: int,
) -> int:
    replay = connection.execute(
        "SELECT replay_id,replay_key FROM replays WHERE logical_path=?", (logical_path,)
    ).fetchone()
    if replay is None:
        raise ValueError(f"unknown replay: {logical_path}")
    runner_id = None
    runner_identity = "none"
    if runner_trust is not None:
        runner = connection.execute(
            "SELECT runner_id,identity_key FROM runners WHERE bundle_trust_sha256=?",
            (runner_trust.lower(),),
        ).fetchone()
        if runner is None:
            raise ValueError(f"unknown authenticated runner: {runner_trust}")
        runner_id = int(runner["runner_id"])
        runner_identity = runner["identity_key"]
    if operation == "replay" and runner_id is None:
        raise ValueError("replay work requires --runner-trust")
    if operation == "convert" and (protocol is None or target_encoding is None):
        raise ValueError("conversion work requires --protocol and --target-encoding")
    material = (
        "replay-work-v1\n"
        f"OPERATION={operation}\nREPLAY={replay['replay_key']}\n"
        f"RUNNER={runner_identity}\nPROTOCOL={protocol}\nTARGET={target_encoding}\n"
        f"SOURCE={source_sha256}\n"
    )
    work_key = sha256_bytes(material.encode())
    connection.execute(
        """INSERT OR IGNORE INTO work_items(
             work_key,operation,replay_id,runner_id,conversion_protocol,target_encoding,
             source_sha256,priority) VALUES(?,?,?,?,?,?,?,?)""",
        (work_key, operation, replay["replay_id"], runner_id, protocol,
         target_encoding, source_sha256, priority),
    )
    return int(connection.execute(
        "SELECT work_id FROM work_items WHERE work_key=?", (work_key,)
    ).fetchone()[0])


def retry_resource_aborts(
    connection: sqlite3.Connection,
    runner_trust: str,
    audit_path: str,
) -> dict[str, int]:
    """Correct legacy SIGKILL/SIGTERM crashes and append retry work.

    Older distributed workers classified numeric 137/143 statuses as game
    crashes and completed their work items. Resource/controller signals are
    not parity evidence. Keep every immutable run and completion row, append
    an explicit outcome correction, and create a new digest-bound retry item.
    """
    rows = connection.execute(
        """SELECT rr.evidence_key,rr.command_status,wc.work_id,
                  wi.work_key,wi.operation,wi.replay_id,wi.runner_id,
                  wi.conversion_protocol,wi.target_encoding,wi.source_sha256,
                  wi.priority
           FROM replay_runs rr JOIN runners ru USING(runner_id)
           LEFT JOIN replay_run_corrections correction USING(evidence_key)
           LEFT JOIN work_completions wc USING(evidence_key)
           LEFT JOIN work_items wi USING(work_id)
           WHERE ru.bundle_trust_sha256=? AND rr.audit_path=?
             AND rr.outcome='crash' AND rr.command_status IN (137,143)
             AND rr.divergence_frame IS NULL AND rr.exact_eof=0
             AND correction.evidence_key IS NULL
           ORDER BY rr.run_id""",
        (runner_trust.lower(), str(Path(audit_path).resolve())),
    ).fetchall()
    corrected = requeued = 0
    with connection:
        for row in rows:
            reason = f"resource/controller signal {row['command_status']}; no parity divergence"
            connection.execute(
                """INSERT INTO replay_run_corrections(
                     evidence_key,corrected_outcome,reason) VALUES(?,?,?)""",
                (row["evidence_key"], "aborted", reason),
            )
            corrected += 1
            if row["work_id"] is None:
                continue
            retry_key = sha256_bytes(
                (
                    "replay-work-retry-v1\n"
                    f"PREVIOUS_WORK={row['work_key']}\n"
                    f"ABORTED_EVIDENCE={row['evidence_key']}\n"
                ).encode()
            )
            requeued += connection.execute(
                """INSERT OR IGNORE INTO work_items(
                     work_key,operation,replay_id,runner_id,conversion_protocol,
                     target_encoding,source_sha256,priority)
                   VALUES(?,?,?,?,?,?,?,?)""",
                (
                    retry_key,row["operation"],row["replay_id"],row["runner_id"],
                    row["conversion_protocol"],row["target_encoding"],
                    row["source_sha256"],max(row["priority"], 1000),
                ),
            ).rowcount
    return {"corrected": corrected, "requeued": requeued}


def enqueue_corpus_replay_work(
    connection: sqlite3.Connection,
    logical_root: str,
    runner_trust: str,
    priority: int,
    workspace: Path | None = None,
) -> dict[str, int]:
    """Create digest-bound replay work for every native artifact in a corpus.

    Artifact hashing happens before the write transaction so a large corpus
    does not hold the authoritative ledger's SQLite writer lock while it is
    being read from disk.
    """
    corpus = connection.execute(
        "SELECT corpus_id,corpus_path FROM corpora WHERE logical_root=?",
        (logical_root,),
    ).fetchone()
    if corpus is None:
        raise ValueError(f"unknown corpus: {logical_root}")
    if corpus["corpus_path"] is None and workspace is None:
        raise ValueError(f"corpus has no authoritative artifact path: {logical_root}")
    if workspace is not None:
        workspace = workspace.resolve(strict=True)
        if not workspace.is_dir():
            raise ValueError(f"workspace is not a directory: {workspace}")
    runner = connection.execute(
        "SELECT runner_id FROM runners WHERE bundle_trust_sha256=?",
        (runner_trust.lower(),),
    ).fetchone()
    if runner is None:
        raise ValueError(f"unknown authenticated runner: {runner_trust}")
    has_final_members = connection.execute(
        "SELECT EXISTS(SELECT 1 FROM final_corpus_members WHERE corpus_id=?)",
        (corpus["corpus_id"],),
    ).fetchone()[0]
    if has_final_members:
        rows = connection.execute(
            """SELECT r.logical_path,r.completion_marker FROM final_corpus_members f
               JOIN replays r USING(replay_id) WHERE f.corpus_id=?
               ORDER BY r.logical_path""",
            (corpus["corpus_id"],),
        ).fetchall()
    else:
        rows = connection.execute(
            """SELECT logical_path,completion_marker FROM replays
               WHERE corpus_id=? ORDER BY logical_path""",
            (corpus["corpus_id"],),
        ).fetchall()
    corpus_path = Path(corpus["corpus_path"]) if corpus["corpus_path"] else None
    artifacts: list[tuple[str, str]] = []
    missing = missing_marker = invalid_footer = 0
    for row in rows:
        logical = row["logical_path"]
        logical_path = Path(logical) if logical is not None else None
        if logical_path is None or logical_path.is_absolute() or ".." in logical_path.parts:
            raise ValueError(f"unsafe replay path: {logical}")
        if workspace is not None:
            physical = workspace / logical_path
        else:
            if not logical.startswith(logical_root + "/"):
                raise ValueError(f"replay is outside corpus root: {logical}")
            assert corpus_path is not None
            physical = corpus_path / logical[len(logical_root) + 1 :]
        native = Path(f"{physical}.parity.bitcode.zst")
        if not native.is_file():
            missing += 1
            continue
        marker_logical = row["completion_marker"]
        if marker_logical is not None:
            marker_path = Path(marker_logical)
            if marker_path.is_absolute() or ".." in marker_path.parts:
                raise ValueError(f"unsafe completion marker path: {marker_logical}")
            if workspace is not None:
                marker = workspace / marker_path
            else:
                if not marker_logical.startswith(logical_root + "/"):
                    raise ValueError(
                        f"completion marker is outside corpus root: {marker_logical}"
                    )
                assert corpus_path is not None
                marker = corpus_path / marker_logical[len(logical_root) + 1 :]
            if not marker.is_file():
                missing_marker += 1
                continue
        if native_extent(str(physical), None) is None:
            invalid_footer += 1
            continue
        artifacts.append((logical, sha256_file(native)))

    enqueued = skipped_exact = 0
    with connection:
        for logical, native_sha in artifacts:
            replay_id = connection.execute(
                "SELECT replay_id FROM replays WHERE logical_path=?", (logical,)
            ).fetchone()[0]
            exact = connection.execute(
                """SELECT EXISTS(SELECT 1 FROM replay_runs
                   WHERE replay_id=? AND runner_id=? AND exact_eof=1
                     AND native_sha256_pre=? AND native_sha256_post=?)""",
                (replay_id, runner["runner_id"], native_sha, native_sha),
            ).fetchone()[0]
            if exact:
                skipped_exact += 1
                continue
            add_work(
                connection, logical, "replay", runner_trust, None, None,
                native_sha, priority,
            )
            enqueued += 1
    return {
        "members": len(rows),
        "native_ready": len(artifacts),
        "enqueued": enqueued,
        "skipped_exact": skipped_exact,
        "missing_native": missing,
        "missing_marker": missing_marker,
        "invalid_footer": invalid_footer,
    }


def claim_work(
    connection: sqlite3.Connection,
    operation: str,
    worker_id: str,
    lease_seconds: int,
    runner_trust: str | None = None,
    logical_root: str | None = None,
) -> dict[str, object] | None:
    if lease_seconds <= 0:
        raise ValueError("lease seconds must be positive")
    now = utc_now()
    now_text = utc_text(now)
    lease_until = utc_text(now + datetime.timedelta(seconds=lease_seconds))
    connection.execute("BEGIN IMMEDIATE")
    try:
        row = connection.execute(
            """SELECT wi.*,r.logical_path,r.completion_marker,ru.bundle_trust_sha256
               FROM work_items wi JOIN replays r USING(replay_id)
               LEFT JOIN runners ru USING(runner_id)
               LEFT JOIN work_completions done USING(work_id)
               LEFT JOIN work_claims claim USING(work_id)
               WHERE wi.operation=? AND done.work_id IS NULL
                 AND (claim.work_id IS NULL OR claim.lease_until_utc <= ?)
                 AND (? IS NULL OR ru.bundle_trust_sha256=?)
                 AND (? IS NULL OR r.corpus_id=(
                       SELECT corpus_id FROM corpora WHERE logical_root=?))
               ORDER BY wi.priority DESC,wi.work_id LIMIT 1""",
            (
                operation,
                now_text,
                runner_trust,
                runner_trust.lower() if runner_trust else None,
                logical_root,
                logical_root,
            ),
        ).fetchone()
        if row is None:
            connection.commit()
            return None
        token = sha256_bytes(uuid.uuid4().bytes + os.urandom(32))
        connection.execute(
            """INSERT INTO work_claims(work_id,claim_token,worker_id,claimed_utc,lease_until_utc)
               VALUES(?,?,?,?,?) ON CONFLICT(work_id) DO UPDATE SET
                 claim_token=excluded.claim_token,worker_id=excluded.worker_id,
                 claimed_utc=excluded.claimed_utc,lease_until_utc=excluded.lease_until_utc""",
            (row["work_id"], token, worker_id, now_text, lease_until),
        )
        connection.commit()
        result = dict(row)
        result.update({"claim_token": token, "worker_id": worker_id,
                       "claimed_utc": now_text, "lease_until_utc": lease_until})
        return result
    except Exception:
        connection.rollback()
        raise


def complete_work(
    connection: sqlite3.Connection, token: str, outcome: str, evidence_key: str | None
) -> int:
    connection.execute("BEGIN IMMEDIATE")
    try:
        claim = connection.execute(
            """SELECT claim.work_id,wi.operation,wi.replay_id,wi.runner_id,
                      wi.source_sha256
               FROM work_claims claim JOIN work_items wi USING(work_id)
               WHERE claim.claim_token=?""",
            (token,),
        ).fetchone()
        if claim is None:
            raise ValueError("unknown or expired claim token")
        if evidence_key is not None:
            evidence = connection.execute(
                """SELECT replay_id,runner_id,outcome,native_sha256_pre
                   FROM replay_runs WHERE evidence_key=?""",
                (evidence_key,),
            ).fetchone()
            if evidence is None:
                raise ValueError("unknown replay evidence key")
            if claim["operation"] == "replay" and (
                evidence["replay_id"] != claim["replay_id"]
                or evidence["runner_id"] != claim["runner_id"]
            ):
                raise ValueError("replay evidence does not match claimed work")
            if evidence["outcome"] != outcome:
                raise ValueError("replay evidence outcome does not match completion")
            if (
                claim["operation"] == "replay"
                and claim["source_sha256"] is not None
                and evidence["native_sha256_pre"] != claim["source_sha256"]
            ):
                raise ValueError("replay evidence native digest does not match claimed work")
        connection.execute(
            """INSERT INTO work_completions(
                 work_id,claim_token,completed_utc,outcome,evidence_key) VALUES(?,?,?,?,?)""",
            (claim["work_id"], token, utc_text(utc_now()), outcome, evidence_key),
        )
        connection.execute("DELETE FROM work_claims WHERE claim_token=?", (token,))
        connection.commit()
        return int(claim["work_id"])
    except Exception:
        connection.rollback()
        raise


def renew_work(
    connection: sqlite3.Connection, token: str, lease_seconds: int
) -> dict[str, object]:
    """Extend the lease for the claim currently identified by *token*.

    A token stops being renewable as soon as another worker reclaims the item.
    This makes a short renewable lease safe for replay jobs whose duration is
    not known in advance: a stale worker can neither renew nor complete after
    its claim has been replaced.
    """
    if lease_seconds <= 0:
        raise ValueError("lease seconds must be positive")
    now = utc_now()
    now_text = utc_text(now)
    lease_until = utc_text(now + datetime.timedelta(seconds=lease_seconds))
    connection.execute("BEGIN IMMEDIATE")
    try:
        row = connection.execute(
            """SELECT work_id,worker_id FROM work_claims
               WHERE claim_token=?""",
            (token,),
        ).fetchone()
        if row is None:
            raise ValueError("unknown or superseded claim token")
        connection.execute(
            """UPDATE work_claims SET lease_until_utc=?
               WHERE work_id=? AND claim_token=?""",
            (lease_until, row["work_id"], token),
        )
        connection.commit()
        return {
            "work_id": int(row["work_id"]),
            "worker_id": row["worker_id"],
            "renewed_utc": now_text,
            "lease_until_utc": lease_until,
        }
    except Exception:
        connection.rollback()
        raise


def claim_corpus_work(
    connection: sqlite3.Connection,
    logical_root: str,
    operation: str,
    worker_id: str,
    host: str,
    audit_path: str | None,
    detail: str | None,
    lease_seconds: int,
) -> dict[str, object]:
    if lease_seconds <= 0:
        raise ValueError("lease seconds must be positive")
    now = utc_now()
    now_text = utc_text(now)
    lease_until = utc_text(now + datetime.timedelta(seconds=lease_seconds))
    connection.execute("BEGIN IMMEDIATE")
    try:
        corpus = connection.execute(
            "SELECT corpus_id FROM corpora WHERE logical_root=?", (logical_root,)
        ).fetchone()
        if corpus is None:
            raise ValueError(f"unknown corpus: {logical_root}")
        existing = connection.execute(
            """SELECT corpus_work_id,worker_id,host,lease_until_utc
               FROM corpus_work_leases
               WHERE corpus_id=? AND operation=? AND state='active'""",
            (corpus["corpus_id"], operation),
        ).fetchone()
        if existing is not None and existing["lease_until_utc"] > now_text:
            raise ValueError(
                f"corpus work already leased by {existing['worker_id']} on "
                f"{existing['host']} until {existing['lease_until_utc']}"
            )
        if existing is not None:
            connection.execute(
                """UPDATE corpus_work_leases
                   SET state='abandoned',finished_utc=? WHERE corpus_work_id=?""",
                (now_text, existing["corpus_work_id"]),
            )
        token = sha256_bytes(uuid.uuid4().bytes + os.urandom(32))
        cursor = connection.execute(
            """INSERT INTO corpus_work_leases(
                 corpus_id,operation,worker_id,host,audit_path,claim_token,
                 claimed_utc,heartbeat_utc,lease_until_utc,detail)
               VALUES(?,?,?,?,?,?,?,?,?,?)""",
            (corpus["corpus_id"], operation, worker_id, host, audit_path,
             token, now_text, now_text, lease_until, detail),
        )
        connection.commit()
        return {
            "corpus_work_id": int(cursor.lastrowid), "claim_token": token,
            "logical_root": logical_root, "operation": operation,
            "worker_id": worker_id, "host": host, "lease_until_utc": lease_until,
        }
    except Exception:
        connection.rollback()
        raise


def renew_corpus_work(
    connection: sqlite3.Connection, token: str, lease_seconds: int
) -> dict[str, str]:
    if lease_seconds <= 0:
        raise ValueError("lease seconds must be positive")
    now = utc_now()
    now_text = utc_text(now)
    lease_until = utc_text(now + datetime.timedelta(seconds=lease_seconds))
    changed = connection.execute(
        """UPDATE corpus_work_leases SET heartbeat_utc=?,lease_until_utc=?
           WHERE claim_token=? AND state='active'""",
        (now_text, lease_until, token),
    ).rowcount
    if changed != 1:
        raise ValueError("unknown or inactive corpus-work claim token")
    return {"heartbeat_utc": now_text, "lease_until_utc": lease_until}


def finish_corpus_work(
    connection: sqlite3.Connection, token: str, state: str, detail: str | None
) -> int:
    if state not in ("completed", "failed", "abandoned"):
        raise ValueError("corpus-work terminal state must be completed, failed, or abandoned")
    changed = connection.execute(
        """UPDATE corpus_work_leases SET state=?,detail=coalesce(?,detail),finished_utc=?
           WHERE claim_token=? AND state='active'""",
        (state, detail, utc_text(utc_now()), token),
    ).rowcount
    if changed != 1:
        raise ValueError("unknown or inactive corpus-work claim token")
    return changed


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    init = commands.add_parser("init")
    init.add_argument("database", type=Path)
    register = commands.add_parser("register-corpus")
    register.add_argument("database", type=Path)
    register.add_argument("corpus", type=Path)
    register.add_argument("--workspace", type=Path, required=True)
    one = commands.add_parser("import-result")
    one.add_argument("database", type=Path)
    one.add_argument("result", type=Path)
    one.add_argument("--audit-root", type=Path, required=True)
    one.add_argument("--workspace", type=Path)
    one.add_argument("--host", default=socket.gethostname())
    audit = commands.add_parser("import-audit")
    audit.add_argument("database", type=Path)
    audit.add_argument("audit", type=Path)
    audit.add_argument("--workspace", type=Path)
    audit.add_argument("--host", default=socket.gethostname())
    tree = commands.add_parser("import-tree")
    tree.add_argument("database", type=Path)
    tree.add_argument("audits", type=Path)
    tree.add_argument("--workspace", type=Path)
    tree.add_argument("--host", default=socket.gethostname())
    legacy = commands.add_parser("import-legacy-tree")
    legacy.add_argument("database", type=Path)
    legacy.add_argument("audits", type=Path)
    legacy.add_argument("--namespace", required=True)
    legacy.add_argument("--workspace", type=Path)
    legacy.add_argument("--host", default=socket.gethostname())
    report = commands.add_parser("summary")
    report.add_argument("database", type=Path)
    global_report = commands.add_parser("overview")
    global_report.add_argument("database", type=Path)
    global_report.add_argument("--json", action="store_true")
    current_runner = commands.add_parser("set-current-runner")
    current_runner.add_argument("database", type=Path)
    current_runner.add_argument("bundle_trust_sha256")
    exact = commands.add_parser("has-attested-exact")
    exact.add_argument("database", type=Path)
    exact.add_argument("logical_path")
    exact.add_argument("--runner-trust", required=True)
    exact.add_argument("--native-sha256", required=True)
    exact_key = commands.add_parser("exact-evidence-key")
    exact_key.add_argument("database", type=Path)
    exact_key.add_argument("logical_path")
    exact_key.add_argument("--runner-trust", required=True)
    exact_key.add_argument("--native-sha256", required=True)
    retire = commands.add_parser("retire-corpus")
    retire.add_argument("database", type=Path)
    retire.add_argument("logical_root")
    retire.add_argument("--reason", required=True)
    activate = commands.add_parser("activate-corpus")
    activate.add_argument("database", type=Path)
    activate.add_argument("logical_root")
    activate.add_argument("--expected", type=int, required=True)
    activate.add_argument("--location-host")
    activate.add_argument("--location-path")
    activate.add_argument("--note")
    activate.add_argument("--operational-path")
    set_path = commands.add_parser("set-corpus-path")
    set_path.add_argument("database", type=Path)
    set_path.add_argument("logical_root")
    set_path.add_argument("path")
    set_path.add_argument("--host", required=True)
    set_path.add_argument("--note")
    merge = commands.add_parser("merge-corpora")
    merge.add_argument("database", type=Path)
    merge.add_argument("target_root")
    merge.add_argument("source_roots", nargs="+")
    merge.add_argument("--expected", type=int, required=True)
    merge.add_argument("--seed-base", type=int)
    merge.add_argument("--trace-schema", type=int)
    absorb = commands.add_parser("absorb-corpora")
    absorb.add_argument("database", type=Path)
    absorb.add_argument("target_root")
    absorb.add_argument("source_roots", nargs="+")
    absorb.add_argument("--expected", type=int, required=True)
    add_replay_command = commands.add_parser("add-replay")
    add_replay_command.add_argument("database", type=Path)
    add_replay_command.add_argument("logical_path")
    final_snapshot = commands.add_parser("import-final-snapshot")
    final_snapshot.add_argument("database", type=Path)
    final_snapshot.add_argument("snapshot", type=Path)
    list_corpora = commands.add_parser("list-corpora")
    list_corpora.add_argument("database", type=Path)
    backup = commands.add_parser("backup")
    backup.add_argument("database", type=Path)
    backup.add_argument("destination", type=Path)
    add = commands.add_parser("add-work")
    add.add_argument("database", type=Path)
    add.add_argument("operation", choices=("replay", "convert"))
    add.add_argument("logical_path")
    add.add_argument("--runner-trust")
    add.add_argument("--protocol", type=int)
    add.add_argument("--target-encoding")
    add.add_argument("--source-sha256")
    add.add_argument("--priority", type=int, default=0)
    enqueue = commands.add_parser("enqueue-corpus-replays")
    enqueue.add_argument("database", type=Path)
    enqueue.add_argument("logical_root")
    enqueue.add_argument("--runner-trust", required=True)
    enqueue.add_argument("--priority", type=int, default=100)
    enqueue.add_argument("--workspace", type=Path)
    claim = commands.add_parser("claim-work")
    claim.add_argument("database", type=Path)
    claim.add_argument("operation", choices=("replay", "convert"))
    claim.add_argument("worker_id")
    claim.add_argument("--lease-seconds", type=int, default=3600)
    claim.add_argument("--runner-trust")
    claim.add_argument("--logical-root")
    renew = commands.add_parser("renew-work")
    renew.add_argument("database", type=Path)
    renew.add_argument("claim_token")
    renew.add_argument("--lease-seconds", type=int, default=3600)
    complete = commands.add_parser("complete-work")
    complete.add_argument("database", type=Path)
    complete.add_argument("claim_token")
    complete.add_argument("outcome")
    complete.add_argument("--evidence-key")
    retry_aborts = commands.add_parser("retry-resource-aborts")
    retry_aborts.add_argument("database", type=Path)
    retry_aborts.add_argument("--runner-trust", required=True)
    retry_aborts.add_argument("--audit-path", required=True)
    corpus_claim = commands.add_parser("claim-corpus-work")
    corpus_claim.add_argument("database", type=Path)
    corpus_claim.add_argument("logical_root")
    corpus_claim.add_argument("operation", choices=("capture", "convert", "replay", "transfer"))
    corpus_claim.add_argument("worker_id")
    corpus_claim.add_argument("--host", required=True)
    corpus_claim.add_argument("--audit-path")
    corpus_claim.add_argument("--detail")
    corpus_claim.add_argument("--lease-seconds", type=int, default=3600)
    corpus_renew = commands.add_parser("renew-corpus-work")
    corpus_renew.add_argument("database", type=Path)
    corpus_renew.add_argument("claim_token")
    corpus_renew.add_argument("--lease-seconds", type=int, default=3600)
    corpus_finish = commands.add_parser("finish-corpus-work")
    corpus_finish.add_argument("database", type=Path)
    corpus_finish.add_argument("claim_token")
    corpus_finish.add_argument("state", choices=("completed", "failed", "abandoned"))
    corpus_finish.add_argument("--detail")
    return result


def main() -> None:
    args = parser().parse_args()
    connection = connect(args.database)
    if args.command == "init":
        print(args.database)
    elif args.command == "register-corpus":
        count = register_corpus(connection, args.corpus.resolve(), args.workspace.resolve())
        print(json.dumps({"registered": count, "corpus": str(args.corpus)}))
    elif args.command == "import-result":
        with connection:
            inserted = import_result(
                connection,
                args.result.resolve(),
                args.audit_root.resolve(),
                args.workspace.resolve() if args.workspace else None,
                args.host,
            )
        evidence = connection.execute(
            "SELECT evidence_key FROM replay_runs WHERE evidence_path=?",
            (str(args.result.resolve()),),
        ).fetchone()
        print(json.dumps({
            "inserted": int(inserted),
            "result": str(args.result),
            "evidence_key": evidence["evidence_key"] if evidence else None,
        }))
    elif args.command == "import-audit":
        inserted, seen = import_audit(
            connection,
            args.audit.resolve(),
            args.workspace.resolve() if args.workspace else None,
            args.host,
        )
        print(json.dumps({"inserted": inserted, "seen": seen, "audit": str(args.audit)}))
    elif args.command == "import-tree":
        inserted = seen = audits = 0
        for audit in sorted(path for path in args.audits.iterdir() if path.is_dir()):
            add, observed = import_audit(
                connection,
                audit.resolve(),
                args.workspace.resolve() if args.workspace else None,
                args.host,
            )
            if observed:
                audits += 1
                inserted += add
                seen += observed
        print(json.dumps({"inserted": inserted, "seen": seen, "audits": audits}))
    elif args.command == "import-legacy-tree":
        inserted, seen, audits = import_legacy_tree(
            connection, args.audits.resolve(), args.namespace,
            args.workspace.resolve() if args.workspace else None, args.host,
        )
        print(json.dumps({"inserted": inserted, "seen": seen, "audits": audits}))
    elif args.command == "summary":
        print(json.dumps(summary(connection), indent=2, sort_keys=True))
    elif args.command == "overview":
        report = overview(connection)
        if args.json:
            print(json.dumps(report, indent=2, sort_keys=True))
        else:
            print_overview(report)
    elif args.command == "set-current-runner":
        with connection:
            set_current_runner(connection, args.bundle_trust_sha256)
        print(json.dumps({"current_runner": args.bundle_trust_sha256.lower()}))
    elif args.command == "has-attested-exact":
        print("1" if has_attested_exact(
            connection, args.logical_path, args.runner_trust, args.native_sha256
        ) else "0")
    elif args.command == "exact-evidence-key":
        print(json.dumps({"evidence_key": exact_evidence_key(
            connection, args.logical_path, args.runner_trust, args.native_sha256,
        )}, sort_keys=True))
    elif args.command == "retire-corpus":
        with connection:
            retire_corpus(connection, args.logical_root, args.reason)
        print(json.dumps({"retired": args.logical_root, "reason": args.reason}))
    elif args.command == "activate-corpus":
        with connection:
            assigned = activate_corpus(
                connection, args.logical_root, args.expected,
                args.location_host, args.location_path, args.note, args.operational_path,
            )
        print(json.dumps({"active": args.logical_root, "expected": args.expected,
                          "assigned": assigned}))
    elif args.command == "set-corpus-path":
        with connection:
            path = set_corpus_path(
                connection, args.logical_root, args.path, args.host, args.note,
            )
        print(json.dumps({"logical_root": args.logical_root, "corpus_path": path}))
    elif args.command == "add-replay":
        with connection:
            replay_id = add_replay(connection, args.logical_path)
        print(json.dumps({"replay_id": replay_id, "logical_path": args.logical_path}))
    elif args.command == "import-final-snapshot":
        with connection:
            imported = import_final_snapshot(connection, args.snapshot.resolve())
        print(json.dumps({"snapshot": str(args.snapshot), "corpora": imported}, sort_keys=True))
    elif args.command == "merge-corpora":
        with connection:
            merged = merge_corpora(
                connection, args.target_root, args.source_roots, args.expected,
                args.seed_base, args.trace_schema,
            )
        print(json.dumps(merged, sort_keys=True))
    elif args.command == "absorb-corpora":
        with connection:
            absorbed = absorb_corpora(
                connection, args.target_root, args.source_roots, args.expected,
            )
        print(json.dumps(absorbed, sort_keys=True))
    elif args.command == "list-corpora":
        print(json.dumps([dict(row) for row in connection.execute(
            """SELECT logical_root,corpus_status,seed_base,trace_schema,expected_replays,
                      corpus_path,retirement_reason,retired_utc
               FROM corpora ORDER BY corpus_status,seed_base,logical_root"""
        )], indent=2, sort_keys=True))
    elif args.command == "backup":
        if args.destination.exists():
            raise ValueError(f"backup destination already exists: {args.destination}")
        args.destination.parent.mkdir(parents=True, exist_ok=True)
        destination = sqlite3.connect(args.destination)
        try:
            connection.backup(destination)
        finally:
            destination.close()
        print(args.destination)
    elif args.command == "add-work":
        with connection:
            work_id = add_work(
                connection, args.logical_path, args.operation, args.runner_trust,
                args.protocol, args.target_encoding, args.source_sha256, args.priority,
            )
        print(json.dumps({"work_id": work_id}))
    elif args.command == "enqueue-corpus-replays":
        print(json.dumps(enqueue_corpus_replay_work(
            connection, args.logical_root, args.runner_trust, args.priority,
            args.workspace,
        ), sort_keys=True))
    elif args.command == "claim-work":
        print(json.dumps(claim_work(
            connection, args.operation, args.worker_id, args.lease_seconds,
            args.runner_trust, args.logical_root,
        ), sort_keys=True))
    elif args.command == "renew-work":
        print(json.dumps(renew_work(
            connection, args.claim_token, args.lease_seconds
        ), sort_keys=True))
    elif args.command == "complete-work":
        work_id = complete_work(connection, args.claim_token, args.outcome, args.evidence_key)
        print(json.dumps({"work_id": work_id, "completed": True}))
    elif args.command == "retry-resource-aborts":
        print(json.dumps(retry_resource_aborts(
            connection, args.runner_trust, args.audit_path,
        ), sort_keys=True))
    elif args.command == "claim-corpus-work":
        print(json.dumps(claim_corpus_work(
            connection, args.logical_root, args.operation, args.worker_id, args.host,
            args.audit_path, args.detail, args.lease_seconds,
        ), sort_keys=True))
    elif args.command == "renew-corpus-work":
        with connection:
            renewed = renew_corpus_work(connection, args.claim_token, args.lease_seconds)
        print(json.dumps(renewed, sort_keys=True))
    elif args.command == "finish-corpus-work":
        with connection:
            finish_corpus_work(connection, args.claim_token, args.state, args.detail)
        print(json.dumps({"completed": True, "state": args.state}, sort_keys=True))
    else:
        raise AssertionError(args.command)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, sqlite3.Error) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(2)
