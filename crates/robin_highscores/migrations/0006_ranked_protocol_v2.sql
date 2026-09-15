-- Ranked protocol V2: boards come from server configuration, uploads are a
-- signed replay plus a one-use upload challenge, and the verifier resimulates
-- against raw game content. Every manifest, competition, campaign-chain,
-- full-campaign, session-genesis, participant co-signing and campaign-object
-- table is removed.
--
-- Submission and verified-run data are NOT preserved: at the time of this
-- migration the production database held 0 submissions and 0 verified runs.
-- Identities, username history, deletion requests, abuse reports, moderation
-- audit, diagnostics, maintenance leases and replay objects are kept. Any
-- replay object left without a submission becomes an ordinary age-gated GC
-- orphan.

DROP VIEW IF EXISTS campaign_object_submission_references;

DROP TABLE verified_run_campaign_objects;
DROP TABLE submission_campaign_objects;
DROP TABLE campaign_objects;
DROP TABLE full_campaign_sessions;
DROP TABLE full_campaign_participants;
DROP TABLE full_campaign_metrics;
DROP TABLE full_campaign_runs;
DROP TABLE competition_run_grants;
DROP TABLE used_replay_session_geneses;
DROP TABLE submission_participants;
DROP TABLE submission_terminal_failures;
DROP TABLE submission_upload_reservations;
DROP TABLE verified_run_metrics;
DROP TABLE verified_run_achievements;
DROP TABLE worker_events;

-- `submissions` and `verified_runs` reference each other. Clear the cycle
-- before the implicit DELETE performed by DROP TABLE.
UPDATE submissions SET predecessor_run_id = NULL;
DELETE FROM verified_runs;
DELETE FROM submissions;
DROP TABLE verified_runs;
DROP TABLE submissions;

-- V1 submission challenges carried server-built offers; none can be redeemed.
DELETE FROM upload_challenges WHERE purpose = 'submission';
DELETE FROM challenge_generations WHERE purpose = 'submission';

CREATE TABLE submissions(
  id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 20 AND 64),
  upload_challenge_id TEXT NOT NULL UNIQUE,
  envelope_json TEXT NOT NULL CHECK(json_valid(envelope_json)),
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
  FOREIGN KEY(upload_challenge_id) REFERENCES upload_challenges(id),
  FOREIGN KEY(replay_sha256) REFERENCES replay_objects(sha256),
  CHECK((status = 'rejected') = (rejection_code IS NOT NULL)),
  CHECK(status = 'rejected' OR rejection_detail IS NULL),
  CHECK((status = 'failed') = (failure_detail IS NOT NULL)),
  CHECK((status = 'verifying') = (lease_owner IS NOT NULL AND lease_expires_at_ms IS NOT NULL)),
  CHECK(tombstoned_at_ms IS NULL OR tombstoned_at_ms >= created_at_ms),
  CHECK(tombstoned_at_ms IS NOT NULL OR purge_eligible_at_ms IS NULL),
  CHECK(purge_eligible_at_ms IS NULL OR purge_eligible_at_ms > tombstoned_at_ms)
) STRICT;
CREATE INDEX submissions_queue_idx ON submissions(status, next_attempt_at_ms, created_at_ms);
CREATE INDEX submissions_replay_idx ON submissions(replay_sha256);
CREATE INDEX submissions_uploader_idx ON submissions(uploader_public_key, created_at_ms);
-- A replay that is pending or was ever accepted cannot be submitted again by
-- anyone. Rejected and infrastructure-failed replays may be retried.
CREATE UNIQUE INDEX submissions_live_replay_idx ON submissions(replay_sha256)
WHERE status = 'accepted'
   OR (status IN('queued', 'verifying', 'retry_pending') AND tombstoned_at_ms IS NULL);

CREATE TABLE submission_upload_reservations(
  upload_challenge_id TEXT PRIMARY KEY NOT NULL,
  submission_id TEXT NOT NULL UNIQUE CHECK(length(submission_id) BETWEEN 20 AND 64),
  envelope_json TEXT NOT NULL CHECK(json_valid(envelope_json)),
  envelope_sha256 BLOB NOT NULL CHECK(length(envelope_sha256) = 32),
  uploader_public_key BLOB NOT NULL CHECK(length(uploader_public_key) = 32),
  replay_sha256 BLOB NOT NULL CHECK(length(replay_sha256) = 32),
  state TEXT NOT NULL CHECK(state IN('reserved', 'uploaded', 'abandoned', 'committed')),
  lease_token TEXT CHECK(lease_token IS NULL OR length(lease_token) BETWEEN 20 AND 64),
  lease_expires_at_ms INTEGER,
  reservation_expires_at_ms INTEGER NOT NULL,
  reserved_at_ms INTEGER NOT NULL CHECK(reserved_at_ms >= 0),
  updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= reserved_at_ms),
  abandoned_at_ms INTEGER,
  committed_at_ms INTEGER,
  FOREIGN KEY(upload_challenge_id) REFERENCES upload_challenges(id) ON DELETE CASCADE,
  CHECK(reservation_expires_at_ms > reserved_at_ms),
  CHECK(lease_expires_at_ms IS NULL OR lease_expires_at_ms > reserved_at_ms),
  CHECK(abandoned_at_ms IS NULL OR abandoned_at_ms >= reserved_at_ms),
  CHECK(committed_at_ms IS NULL OR committed_at_ms >= reserved_at_ms),
  CHECK((state IN('reserved', 'uploaded')
AND lease_token IS NOT NULL
AND lease_expires_at_ms IS NOT NULL
AND abandoned_at_ms IS NULL
AND committed_at_ms IS NULL)
OR(state = 'abandoned'
AND lease_token IS NULL
AND lease_expires_at_ms IS NULL
AND abandoned_at_ms IS NOT NULL
AND committed_at_ms IS NULL)
OR(state = 'committed'
AND lease_token IS NULL
AND lease_expires_at_ms IS NULL
AND abandoned_at_ms IS NULL
AND committed_at_ms IS NOT NULL))
) STRICT;
CREATE INDEX submission_upload_reservations_expiry_idx
ON submission_upload_reservations(reservation_expires_at_ms)
WHERE state != 'committed';
CREATE INDEX submission_upload_reservations_lease_idx
ON submission_upload_reservations(lease_expires_at_ms)
WHERE state IN('reserved', 'uploaded');
-- Only one uncommitted upload of the same replay can be in flight.
CREATE UNIQUE INDEX submission_upload_reservations_replay_idx
ON submission_upload_reservations(replay_sha256)
WHERE state != 'committed';

CREATE TABLE verified_runs(
  id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 20 AND 64),
  submission_id TEXT NOT NULL UNIQUE,
  board_id TEXT NOT NULL CHECK(length(board_id) BETWEEN 1 AND 128),
  mission_id TEXT NOT NULL CHECK(length(mission_id) BETWEEN 1 AND 256),
  edition TEXT NOT NULL CHECK(edition IN('demo', 'full')),
  recorded_engine_version TEXT NOT NULL CHECK(length(recorded_engine_version) BETWEEN 1 AND 64),
  sim_config_json TEXT NOT NULL CHECK(json_valid(sim_config_json)),
  max_concurrent_players INTEGER NOT NULL CHECK(max_concurrent_players BETWEEN 1 AND 4),
  participant_instance_count INTEGER NOT NULL CHECK(participant_instance_count BETWEEN max_concurrent_players AND 1024),
  starting_campaign_score INTEGER NOT NULL,
  final_campaign_score INTEGER NOT NULL,
  original_score_delta INTEGER NOT NULL CHECK(original_score_delta BETWEEN 0 AND 4294967295),
  final_state_sha256 BLOB NOT NULL CHECK(length(final_state_sha256) = 32),
  replay_frames INTEGER NOT NULL CHECK(replay_frames > 0),
  active_simulation_ticks INTEGER NOT NULL CHECK(active_simulation_ticks >= 0),
  ransom_collected INTEGER NOT NULL CHECK(ransom_collected >= 0),
  input_provenance_json TEXT NOT NULL CHECK(json_valid(input_provenance_json)),
  job_sha256 BLOB NOT NULL CHECK(length(job_sha256) = 32),
  accepted_sequence INTEGER NOT NULL UNIQUE CHECK(accepted_sequence > 0),
  verified_at_ms INTEGER NOT NULL CHECK(verified_at_ms >= 0),
  FOREIGN KEY(submission_id) REFERENCES submissions(id)
) STRICT;
CREATE INDEX verified_runs_board_idx
ON verified_runs(board_id, mission_id, max_concurrent_players, accepted_sequence);

CREATE TABLE verified_run_metrics(
  run_id TEXT NOT NULL,
  metric TEXT NOT NULL CHECK(metric IN('original_score', 'fastest_success')),
  value INTEGER NOT NULL CHECK((metric = 'original_score' AND value BETWEEN 0 AND 4294967295)
OR(metric = 'fastest_success' AND value >= 0)),
  PRIMARY KEY(run_id, metric),
  FOREIGN KEY(run_id) REFERENCES verified_runs(id) ON DELETE CASCADE
) STRICT;
CREATE INDEX verified_metrics_board_idx ON verified_run_metrics(metric, value, run_id);

CREATE TABLE verified_run_achievements(
  run_id TEXT NOT NULL,
  achievement_id TEXT NOT NULL CHECK(length(achievement_id) BETWEEN 1 AND 128),
  evaluation TEXT NOT NULL CHECK(evaluation IN('unverifiable', 'not_earned', 'earned')),
  evidence_json TEXT NOT NULL CHECK(json_valid(evidence_json)),
  PRIMARY KEY(run_id, achievement_id),
  FOREIGN KEY(run_id) REFERENCES verified_runs(id) ON DELETE CASCADE
) STRICT;

CREATE TABLE worker_events(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  submission_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN('leased', 'retry', 'accepted', 'rejected', 'failed')),
  worker_id TEXT NOT NULL CHECK(length(worker_id) BETWEEN 1 AND 128),
  detail TEXT,
  created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
  FOREIGN KEY(submission_id) REFERENCES submissions(id)
) STRICT;
