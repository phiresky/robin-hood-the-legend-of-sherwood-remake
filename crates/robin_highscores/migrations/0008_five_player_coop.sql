-- Widen co-op rankings to all five supported seats. Preserve accepted runs
-- and their child records when SQLite rebuilds the constrained parent table.
PRAGMA defer_foreign_keys = ON;

CREATE TABLE verified_runs_v8(
  id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 20 AND 64),
  submission_id TEXT NOT NULL UNIQUE,
  board_id TEXT NOT NULL CHECK(length(board_id) BETWEEN 1 AND 128),
  mission_id TEXT NOT NULL CHECK(length(mission_id) BETWEEN 1 AND 256),
  edition TEXT NOT NULL CHECK(edition IN('demo', 'full')),
  recorded_engine_version TEXT NOT NULL CHECK(length(recorded_engine_version) BETWEEN 1 AND 64),
  sim_config_json TEXT NOT NULL CHECK(json_valid(sim_config_json)),
  max_concurrent_players INTEGER NOT NULL CHECK(max_concurrent_players BETWEEN 1 AND 5),
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

INSERT INTO verified_runs_v8 SELECT * FROM verified_runs;
CREATE TEMP TABLE co_op_metrics_backup AS SELECT * FROM verified_run_metrics;
CREATE TEMP TABLE co_op_achievements_backup AS SELECT * FROM verified_run_achievements;
-- DROP cascades to metrics and achievements; both are restored below.
DROP TABLE verified_runs;
ALTER TABLE verified_runs_v8 RENAME TO verified_runs;
CREATE INDEX verified_runs_board_idx
ON verified_runs(board_id, mission_id, max_concurrent_players, accepted_sequence);
INSERT INTO verified_run_metrics SELECT * FROM co_op_metrics_backup;
INSERT INTO verified_run_achievements SELECT * FROM co_op_achievements_backup;
DROP TABLE co_op_metrics_backup;
DROP TABLE co_op_achievements_backup;
