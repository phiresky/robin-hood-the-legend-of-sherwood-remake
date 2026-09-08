"""Reusable immutable ledger schema (including append-only evidence triggers)."""
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

-- A reblock changes the byte identity of a native trace without changing its
-- decoded record stream.  Keep that proof separate from replay execution
-- evidence: a migrated artifact is not a second replay run.
CREATE TABLE IF NOT EXISTS native_reblock_audits (
    audit_id INTEGER PRIMARY KEY,
    audit_key TEXT NOT NULL UNIQUE CHECK (length(audit_key) = 64),
    audit_path TEXT NOT NULL UNIQUE,
    evidence_manifest_sha256 TEXT NOT NULL CHECK (length(evidence_manifest_sha256) = 64),
    runner_bundle_trust_sha256 TEXT NOT NULL CHECK (length(runner_bundle_trust_sha256) = 64),
    runner_raw_sha256 TEXT NOT NULL CHECK (length(runner_raw_sha256) = 64),
    native_paths_sha256 TEXT NOT NULL CHECK (length(native_paths_sha256) = 64),
    before_manifest_sha256 TEXT NOT NULL CHECK (length(before_manifest_sha256) = 64),
    after_manifest_sha256 TEXT NOT NULL CHECK (length(after_manifest_sha256) = 64),
    artifact_count INTEGER NOT NULL CHECK (artifact_count > 0),
    completed_utc TEXT NOT NULL,
    imported_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;
CREATE TRIGGER IF NOT EXISTS native_reblock_audits_no_update
BEFORE UPDATE ON native_reblock_audits BEGIN
    SELECT RAISE(ABORT, 'native_reblock_audits is append-only');
END;
CREATE TRIGGER IF NOT EXISTS native_reblock_audits_no_delete
BEFORE DELETE ON native_reblock_audits BEGIN
    SELECT RAISE(ABORT, 'native_reblock_audits is append-only');
END;

CREATE TABLE IF NOT EXISTS native_artifact_lineage (
    lineage_id INTEGER PRIMARY KEY,
    lineage_key TEXT NOT NULL UNIQUE CHECK (length(lineage_key) = 64),
    replay_id INTEGER NOT NULL REFERENCES replays(replay_id),
    audit_id INTEGER NOT NULL REFERENCES native_reblock_audits(audit_id),
    source_sha256 TEXT NOT NULL CHECK (length(source_sha256) = 64),
    target_sha256 TEXT NOT NULL CHECK (length(target_sha256) = 64),
    CHECK (source_sha256 <> target_sha256),
    UNIQUE(audit_id,replay_id),
    UNIQUE(replay_id,source_sha256,target_sha256)
) STRICT;
CREATE INDEX IF NOT EXISTS native_artifact_lineage_target
    ON native_artifact_lineage(replay_id,target_sha256);
CREATE INDEX IF NOT EXISTS native_artifact_lineage_source
    ON native_artifact_lineage(replay_id,source_sha256);
CREATE TRIGGER IF NOT EXISTS native_artifact_lineage_no_update
BEFORE UPDATE ON native_artifact_lineage BEGIN
    SELECT RAISE(ABORT, 'native_artifact_lineage is append-only');
END;
CREATE TRIGGER IF NOT EXISTS native_artifact_lineage_no_delete
BEFORE DELETE ON native_artifact_lineage BEGIN
    SELECT RAISE(ABORT, 'native_artifact_lineage is append-only');
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
    corpus_id INTEGER NOT NULL REFERENCES corpora(corpus_id),
    save_group TEXT NOT NULL,
    stripe_key TEXT NOT NULL,
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

-- A native reblock changes the artifact digest bound into queued replay work.
-- Preserve the obsolete work row, but make its terminal replacement explicit
-- rather than pretending that the obsolete digest was executed.
CREATE TABLE IF NOT EXISTS work_supersessions (
    work_id INTEGER PRIMARY KEY REFERENCES work_items(work_id),
    lineage_id INTEGER NOT NULL REFERENCES native_artifact_lineage(lineage_id),
    replacement_work_id INTEGER NOT NULL REFERENCES work_items(work_id),
    superseded_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    reason TEXT NOT NULL,
    CHECK (work_id <> replacement_work_id)
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
    SELECT CASE WHEN
      NEW.work_key IS NOT OLD.work_key OR NEW.operation IS NOT OLD.operation
      OR NEW.replay_id IS NOT OLD.replay_id OR NEW.runner_id IS NOT OLD.runner_id
      OR NEW.conversion_protocol IS NOT OLD.conversion_protocol
      OR NEW.target_encoding IS NOT OLD.target_encoding
      OR NEW.source_sha256 IS NOT OLD.source_sha256 OR NEW.priority IS NOT OLD.priority
      OR NEW.created_utc IS NOT OLD.created_utc
      OR NEW.corpus_id IS NULL
      OR NEW.corpus_id IS NOT (SELECT corpus_id FROM replays WHERE replay_id=NEW.replay_id)
      OR NEW.save_group IS NOT (SELECT
           rtrim(rtrim(logical_path,replace(logical_path,'/','')),'/')
           FROM replays WHERE replay_id=NEW.replay_id)
      OR NEW.stripe_key IS NOT (SELECT
           substr(logical_path,length(rtrim(logical_path,replace(logical_path,'/','')))+1)
           FROM replays WHERE replay_id=NEW.replay_id)
      THEN RAISE(ABORT, 'work_items is append-only') END;
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
CREATE TRIGGER IF NOT EXISTS work_completions_not_superseded
BEFORE INSERT ON work_completions
WHEN EXISTS(SELECT 1 FROM work_supersessions WHERE work_id=NEW.work_id) BEGIN
    SELECT RAISE(ABORT, 'cannot complete superseded work');
END;
CREATE TRIGGER IF NOT EXISTS work_supersessions_valid
BEFORE INSERT ON work_supersessions WHEN NOT EXISTS (
    SELECT 1
    FROM work_items obsolete,work_items replacement,native_artifact_lineage lineage
    WHERE obsolete.work_id=NEW.work_id
      AND replacement.work_id=NEW.replacement_work_id
      AND lineage.lineage_id=NEW.lineage_id
      AND obsolete.operation='replay' AND replacement.operation='replay'
      AND obsolete.replay_id=lineage.replay_id
      AND replacement.replay_id=lineage.replay_id
      AND obsolete.runner_id IS replacement.runner_id
      AND replacement.source_sha256=lineage.target_sha256
      AND NOT EXISTS (
        SELECT 1 FROM work_completions done WHERE done.work_id=obsolete.work_id
      )
      AND EXISTS (
        WITH RECURSIVE ancestors(sha256) AS (
          VALUES(lineage.target_sha256)
          UNION
          SELECT prior.source_sha256
          FROM native_artifact_lineage prior
          JOIN ancestors ON ancestors.sha256=prior.target_sha256
          WHERE prior.replay_id=lineage.replay_id
        )
        SELECT 1 FROM ancestors
        WHERE sha256=obsolete.source_sha256
          AND sha256<>lineage.target_sha256
      )
) BEGIN
    SELECT RAISE(ABORT, 'invalid work supersession lineage or replacement');
END;
CREATE TRIGGER IF NOT EXISTS work_supersessions_no_update
BEFORE UPDATE ON work_supersessions BEGIN
    SELECT RAISE(ABORT, 'work_supersessions is append-only');
END;
CREATE TRIGGER IF NOT EXISTS work_supersessions_no_delete
BEFORE DELETE ON work_supersessions BEGIN
    SELECT RAISE(ABORT, 'work_supersessions is append-only');
END;

"""


