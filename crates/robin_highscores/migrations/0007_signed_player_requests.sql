-- Signed player requests: every player operation (submission, username
-- update, deletion, private submission status) is an Ed25519-signed claim
-- with `signed_at_unix_ms` instead of a server-issued one-use challenge. All
-- challenge storage is dropped. Existing identities, username history,
-- deletion requests, submissions, verified runs and in-flight upload
-- reservations are preserved.

-- Rebuilding `submissions` drops rows that `verified_runs` and
-- `worker_events` reference. Defer foreign-key checks to COMMIT, by which
-- point every parent row has been re-inserted under the same table name.
PRAGMA defer_foreign_keys = ON;

-- Upload reservations were keyed by their challenge. They are now keyed by
-- the proposed submission ID with at most one uncommitted upload per replay;
-- finalization deletes the row instead of keeping a committed tombstone.
CREATE TABLE submission_upload_reservations_v7(
  submission_id TEXT PRIMARY KEY NOT NULL CHECK(length(submission_id) BETWEEN 20 AND 64),
  signed_request_json TEXT NOT NULL CHECK(json_valid(signed_request_json)),
  uploader_public_key BLOB NOT NULL CHECK(length(uploader_public_key) = 32),
  replay_sha256 BLOB NOT NULL UNIQUE CHECK(length(replay_sha256) = 32),
  state TEXT NOT NULL CHECK(state IN('reserved', 'uploaded', 'abandoned')),
  lease_token TEXT CHECK(lease_token IS NULL OR length(lease_token) BETWEEN 20 AND 64),
  lease_expires_at_ms INTEGER,
  reservation_expires_at_ms INTEGER NOT NULL,
  reserved_at_ms INTEGER NOT NULL CHECK(reserved_at_ms >= 0),
  updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= reserved_at_ms),
  abandoned_at_ms INTEGER,
  CHECK(reservation_expires_at_ms > reserved_at_ms),
  CHECK(lease_expires_at_ms IS NULL OR lease_expires_at_ms > reserved_at_ms),
  CHECK(abandoned_at_ms IS NULL OR abandoned_at_ms >= reserved_at_ms),
  CHECK((state IN('reserved', 'uploaded')
AND lease_token IS NOT NULL
AND lease_expires_at_ms IS NOT NULL
AND abandoned_at_ms IS NULL)
OR(state = 'abandoned'
AND lease_token IS NULL
AND lease_expires_at_ms IS NULL
AND abandoned_at_ms IS NOT NULL))
) STRICT;
INSERT INTO submission_upload_reservations_v7
  (submission_id, signed_request_json, uploader_public_key, replay_sha256, state, lease_token,
   lease_expires_at_ms, reservation_expires_at_ms, reserved_at_ms, updated_at_ms, abandoned_at_ms)
SELECT submission_id, envelope_json, uploader_public_key, replay_sha256, state, lease_token,
       lease_expires_at_ms, reservation_expires_at_ms, reserved_at_ms, updated_at_ms, abandoned_at_ms
FROM submission_upload_reservations
WHERE state != 'committed';
DROP TABLE submission_upload_reservations;
ALTER TABLE submission_upload_reservations_v7 RENAME TO submission_upload_reservations;
CREATE INDEX submission_upload_reservations_expiry_idx
ON submission_upload_reservations(reservation_expires_at_ms);
CREATE INDEX submission_upload_reservations_uploader_idx
ON submission_upload_reservations(uploader_public_key, lease_expires_at_ms)
WHERE state IN('reserved', 'uploaded');

-- Submissions lose their challenge reference. The signed document column is
-- renamed: an identical retry of a live submission returns that submission.
CREATE TABLE submissions_v6_rows AS SELECT * FROM submissions;
DROP TABLE submissions;
CREATE TABLE submissions(
  id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 20 AND 64),
  signed_request_json TEXT NOT NULL CHECK(json_valid(signed_request_json)),
  uploader_public_key BLOB NOT NULL CHECK(length(uploader_public_key) = 32),
  public_disclosure TEXT NOT NULL CHECK(public_disclosure IN('named_profile', 'anonymous')),
  board_id TEXT NOT NULL CHECK(length(board_id) BETWEEN 1 AND 128),
  mission_id TEXT NOT NULL CHECK(length(mission_id) BETWEEN 1 AND 256),
  replay_sha256 BLOB NOT NULL CHECK(length(replay_sha256) = 32),
  replay_bytes INTEGER NOT NULL CHECK(replay_bytes > 0),
  replay_schema_version INTEGER NOT NULL CHECK(replay_schema_version > 0),
  requested_metrics_json TEXT NOT NULL CHECK(json_valid(requested_metrics_json)),
  status TEXT NOT NULL CHECK(status IN('queued', 'verifying', 'retry_pending', 'accepted', 'rejected', 'failed')),
  rejection_code TEXT,
  rejection_detail TEXT CHECK(rejection_detail IS NULL OR length(rejection_detail) <= 128),
  failure_detail TEXT CHECK(failure_detail IS NULL OR length(failure_detail) BETWEEN 1 AND 2000),
  attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
  next_attempt_at_ms INTEGER NOT NULL CHECK(next_attempt_at_ms >= 0),
  lease_owner TEXT,
  lease_expires_at_ms INTEGER,
  created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
  updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms),
  tombstoned_at_ms INTEGER,
  purge_eligible_at_ms INTEGER,
  FOREIGN KEY(replay_sha256) REFERENCES replay_objects(sha256),
  CHECK((status = 'rejected') = (rejection_code IS NOT NULL)),
  CHECK(status = 'rejected' OR rejection_detail IS NULL),
  CHECK((status = 'failed') = (failure_detail IS NOT NULL)),
  CHECK((status = 'verifying') = (lease_owner IS NOT NULL AND lease_expires_at_ms IS NOT NULL)),
  CHECK(tombstoned_at_ms IS NULL OR tombstoned_at_ms >= created_at_ms),
  CHECK(tombstoned_at_ms IS NOT NULL OR purge_eligible_at_ms IS NULL),
  CHECK(purge_eligible_at_ms IS NULL OR purge_eligible_at_ms > tombstoned_at_ms)
) STRICT;
INSERT INTO submissions
  (id, signed_request_json, uploader_public_key, public_disclosure, board_id, mission_id,
   replay_sha256, replay_bytes, replay_schema_version, requested_metrics_json, status,
   rejection_code, rejection_detail, failure_detail, attempts, next_attempt_at_ms, lease_owner,
   lease_expires_at_ms, created_at_ms, updated_at_ms, tombstoned_at_ms, purge_eligible_at_ms)
SELECT id, envelope_json, uploader_public_key, public_disclosure, board_id, mission_id,
       replay_sha256, replay_bytes, replay_schema_version, requested_metrics_json, status,
       rejection_code, rejection_detail, failure_detail, attempts, next_attempt_at_ms, lease_owner,
       lease_expires_at_ms, created_at_ms, updated_at_ms, tombstoned_at_ms, purge_eligible_at_ms
FROM submissions_v6_rows;
DROP TABLE submissions_v6_rows;
CREATE INDEX submissions_queue_idx ON submissions(status, next_attempt_at_ms, created_at_ms);
CREATE INDEX submissions_replay_idx ON submissions(replay_sha256);
CREATE INDEX submissions_uploader_idx ON submissions(uploader_public_key, created_at_ms);
-- A replay that is pending or was ever accepted cannot be submitted again by
-- anyone. Rejected and infrastructure-failed replays may be retried.
CREATE UNIQUE INDEX submissions_live_replay_idx ON submissions(replay_sha256)
WHERE status = 'accepted'
   OR (status IN('queued', 'verifying', 'retry_pending') AND tombstoned_at_ms IS NULL);

-- Username history referenced the challenge that authorized each change. V1
-- rows carry no signed timestamp and are recorded with 0.
CREATE TABLE username_history_v7(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  public_key BLOB NOT NULL CHECK(length(public_key) = 32),
  generation INTEGER NOT NULL CHECK(generation > 0),
  signed_at_unix_ms INTEGER NOT NULL CHECK(signed_at_unix_ms >= 0),
  previous_username TEXT,
  new_username TEXT NOT NULL CHECK(length(new_username) BETWEEN 1 AND 48),
  changed_at_ms INTEGER NOT NULL CHECK(changed_at_ms >= 0),
  FOREIGN KEY(public_key) REFERENCES identities(public_key)
) STRICT;
INSERT INTO username_history_v7
  (id, public_key, generation, signed_at_unix_ms, previous_username, new_username, changed_at_ms)
SELECT id, public_key, generation, 0, previous_username, new_username, changed_at_ms
FROM username_history;
DROP TABLE username_history;
ALTER TABLE username_history_v7 RENAME TO username_history;
CREATE INDEX username_history_key_idx ON username_history(public_key, changed_at_ms, id);

-- Deletion requests referenced their challenge. Deletion is now idempotent
-- per owner and target: a repeated signed request returns the stored receipt.
CREATE TABLE deletion_requests_v7(
  id TEXT PRIMARY KEY NOT NULL,
  owner_public_key BLOB NOT NULL CHECK(length(owner_public_key) = 32),
  target_kind TEXT NOT NULL CHECK(target_kind IN('submission', 'run')),
  target_id TEXT NOT NULL,
  request_json TEXT NOT NULL CHECK(json_valid(request_json)),
  signed_at_unix_ms INTEGER NOT NULL CHECK(signed_at_unix_ms >= 0),
  tombstoned_at_ms INTEGER NOT NULL CHECK(tombstoned_at_ms >= 0),
  purge_eligible_at_ms INTEGER,
  UNIQUE(owner_public_key, target_kind, target_id),
  FOREIGN KEY(owner_public_key) REFERENCES identities(public_key),
  CHECK(purge_eligible_at_ms IS NULL OR purge_eligible_at_ms > tombstoned_at_ms)
) STRICT;
INSERT INTO deletion_requests_v7
  (id, owner_public_key, target_kind, target_id, request_json, signed_at_unix_ms,
   tombstoned_at_ms, purge_eligible_at_ms)
SELECT id, owner_public_key, target_kind, target_id, request_json, 0,
       tombstoned_at_ms, purge_eligible_at_ms
FROM deletion_requests;
DROP TABLE deletion_requests;
ALTER TABLE deletion_requests_v7 RENAME TO deletion_requests;

-- A username update must be signed strictly later than the last accepted
-- one, so a replayed older request cannot roll the name back.
ALTER TABLE identities ADD COLUMN username_signed_at_unix_ms INTEGER NOT NULL DEFAULT 0
  CHECK(username_signed_at_unix_ms >= 0);

DROP TABLE submission_owner_status_challenges;
DROP TABLE challenge_generations;
DROP TABLE upload_challenges;
