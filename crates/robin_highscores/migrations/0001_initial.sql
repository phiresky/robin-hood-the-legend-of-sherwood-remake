-- Canonical first production schema. Ranked storage starts here and accepts
-- no alternative Rust-server schema.
PRAGMA foreign_keys = ON;

CREATE TABLE identities(
  public_key BLOB PRIMARY KEY NOT NULL CHECK(length(public_key) = 32),
  username TEXT NOT NULL CHECK(length(username) BETWEEN 1 AND 48),
  username_normalized TEXT NOT NULL CHECK(length(username_normalized) BETWEEN 1 AND 48),
  username_generation INTEGER NOT NULL CHECK(username_generation > 0),
  created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
  updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
) STRICT;
CREATE TABLE upload_challenges(
  id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 20 AND 64),
  nonce BLOB NOT NULL UNIQUE CHECK(length(nonce) = 32),
  purpose TEXT NOT NULL CHECK(purpose IN('submission', 'username_update', 'deletion')),
  public_key BLOB NOT NULL CHECK(length(public_key) = 32),
  generation INTEGER NOT NULL CHECK(generation > 0),
  issued_at_ms INTEGER NOT NULL CHECK(issued_at_ms >= 0),
  expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms > issued_at_ms),
  offer_json TEXT CHECK(offer_json IS NULL OR json_valid(offer_json)),
  public_metadata_json TEXT CHECK(public_metadata_json IS NULL OR json_valid(public_metadata_json)),
  consumed_at_ms INTEGER,
  CHECK(consumed_at_ms IS NULL OR consumed_at_ms >= issued_at_ms)
) STRICT;
CREATE TABLE challenge_generations(
  public_key BLOB NOT NULL CHECK(length(public_key) = 32),
  purpose TEXT NOT NULL CHECK(purpose IN('submission', 'username_update', 'deletion')),
  generation INTEGER NOT NULL CHECK(generation >= 0),
  PRIMARY KEY(public_key, purpose)
) STRICT;
CREATE INDEX upload_challenges_expiry_idx
ON upload_challenges(
  expires_at_ms
) WHERE consumed_at_ms IS NULL;
CREATE TABLE replay_objects(
  sha256 BLOB PRIMARY KEY NOT NULL CHECK(length(sha256) = 32),
  byte_length INTEGER NOT NULL CHECK(byte_length > 0),
  created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
  purged_at_ms INTEGER,
  purge_state TEXT NOT NULL DEFAULT 'live' CHECK(purge_state IN('live', 'purging', 'purged')),
  purge_token TEXT,
  purge_claimed_at_ms INTEGER,
  CHECK(purged_at_ms IS NULL OR purged_at_ms >= created_at_ms),
  CHECK((purge_state = 'purging') =(purge_token IS NOT NULL)),
  CHECK((purge_state = 'purging') =(purge_claimed_at_ms IS NOT NULL)),
  CHECK((purge_state = 'purged') =(purged_at_ms IS NOT NULL))
) STRICT;
CREATE TABLE submissions(
  id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 20 AND 64),
  upload_challenge_id TEXT NOT NULL UNIQUE,
  offer_json TEXT NOT NULL CHECK(json_valid(offer_json)),
  envelope_json TEXT NOT NULL CHECK(json_valid(envelope_json)),
  signatures_json TEXT NOT NULL CHECK(json_valid(signatures_json)),
  public_metadata_json TEXT NOT NULL CHECK(json_valid(public_metadata_json)),
  replay_sha256 BLOB NOT NULL CHECK(length(replay_sha256) = 32),
  replay_bytes INTEGER NOT NULL CHECK(replay_bytes > 0),
  build_manifest_id BLOB NOT NULL CHECK(length(build_manifest_id) = 32),
  content_manifest_id BLOB NOT NULL CHECK(length(content_manifest_id) = 32),
  config_id BLOB NOT NULL CHECK(length(config_id) = 32),
  ruleset_id BLOB NOT NULL CHECK(length(ruleset_id) = 32),
  mission_id TEXT NOT NULL CHECK(length(mission_id) BETWEEN 1 AND 128),
  scope_kind TEXT NOT NULL CHECK(scope_kind IN('individual_level', 'campaign')),
  starting_campaign_sha256 BLOB NOT NULL CHECK(length(starting_campaign_sha256) = 32),
  starting_state_json TEXT NOT NULL CHECK(json_valid(starting_state_json)),
  campaign_chain_id TEXT,
  predecessor_run_id TEXT,
  competition_manifest_id BLOB CHECK(competition_manifest_id IS NULL OR length(competition_manifest_id) = 32),
  requested_metrics_json TEXT NOT NULL CHECK(json_valid(requested_metrics_json)),
  participant_claims_json TEXT NOT NULL CHECK(json_valid(participant_claims_json)),
  max_concurrent_players INTEGER NOT NULL CHECK(max_concurrent_players BETWEEN 1 AND 4),
  participant_instance_count INTEGER NOT NULL CHECK(participant_instance_count BETWEEN max_concurrent_players AND 1024),
  session_genesis_sha256 BLOB NOT NULL CHECK(length(session_genesis_sha256) = 32),
  status TEXT NOT NULL CHECK(status IN('queued', 'verifying', 'retry_pending', 'accepted', 'rejected')),
  rejection_code TEXT,
  attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
  verification_request_sha256 BLOB CHECK(verification_request_sha256 IS NULL OR length(verification_request_sha256) = 32),
  verification_request_json TEXT CHECK(verification_request_json IS NULL OR json_valid(verification_request_json)),
  next_attempt_at_ms INTEGER NOT NULL CHECK(next_attempt_at_ms >= 0),
  lease_owner TEXT,
  lease_expires_at_ms INTEGER,
  created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
  updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms),
  tombstoned_at_ms INTEGER,
  purge_eligible_at_ms INTEGER,
  campaign_content_manifest_id BLOB
  CHECK(campaign_content_manifest_id IS NULL OR length(campaign_content_manifest_id) = 32),
  starting_campaign_bytes INTEGER NOT NULL CHECK(starting_campaign_bytes > 0),
  controller_public_key BLOB NOT NULL CHECK(length(controller_public_key) = 32),
  canonical_campaign_state_json TEXT NOT NULL CHECK(json_valid(canonical_campaign_state_json)),
  verifier_job_route_json TEXT
  CHECK(verifier_job_route_json IS NULL OR json_valid(verifier_job_route_json)),
  verifier_job_config_sha256 BLOB
  CHECK(verifier_job_config_sha256 IS NULL OR length(verifier_job_config_sha256) = 32),
  verifier_policy_manifest_sha256 BLOB
  CHECK(verifier_policy_manifest_sha256 IS NULL OR length(verifier_policy_manifest_sha256) = 32),
  FOREIGN KEY(upload_challenge_id) REFERENCES upload_challenges(id),
  FOREIGN KEY(replay_sha256) REFERENCES replay_objects(sha256),
  FOREIGN KEY(predecessor_run_id) REFERENCES verified_runs(id),
  CHECK((status = 'rejected') =(rejection_code IS NOT NULL)),
  CHECK((verification_request_sha256 IS NULL) =(verification_request_json IS NULL)),
  CHECK((status = 'verifying') =(lease_owner IS NOT NULL AND lease_expires_at_ms IS NOT NULL)),
  CHECK((scope_kind = 'campaign') = (campaign_content_manifest_id IS NOT NULL)),
  CHECK(tombstoned_at_ms IS NULL OR tombstoned_at_ms >= created_at_ms),
  CHECK(tombstoned_at_ms IS NOT NULL OR purge_eligible_at_ms IS NULL),
  CHECK(purge_eligible_at_ms IS NULL OR purge_eligible_at_ms > tombstoned_at_ms)
) STRICT;
CREATE INDEX submissions_queue_idx
ON submissions(
  status,
  next_attempt_at_ms,
  created_at_ms
);
CREATE INDEX submissions_replay_idx ON submissions(replay_sha256);
CREATE UNIQUE INDEX accepted_campaign_predecessor_idx
ON submissions(
  predecessor_run_id
)
WHERE predecessor_run_id IS NOT NULL AND status = 'accepted';
CREATE TABLE used_replay_session_geneses(
  host_public_key BLOB NOT NULL CHECK(length(host_public_key) = 32),
  replay_session_id BLOB NOT NULL CHECK(length(replay_session_id) = 32),
  host_nonce BLOB NOT NULL CHECK(length(host_nonce) = 32),
  session_genesis_sha256 BLOB NOT NULL UNIQUE CHECK(length(session_genesis_sha256) = 32),
  submission_id TEXT NOT NULL UNIQUE,
  consumed_at_ms INTEGER NOT NULL CHECK(consumed_at_ms >= 0),
  PRIMARY KEY(host_public_key, replay_session_id, host_nonce),
  FOREIGN KEY(submission_id) REFERENCES submissions(id) ON DELETE CASCADE
) STRICT;
CREATE TABLE submission_participants(
  submission_id TEXT NOT NULL,
  seat INTEGER NOT NULL CHECK(seat BETWEEN 0 AND 3),
  participant_instance_id BLOB NOT NULL CHECK(length(participant_instance_id) = 32),
  public_key BLOB NOT NULL CHECK(length(public_key) = 32),
  public_disclosure TEXT NOT NULL
  DEFAULT 'named_profile'
  CHECK(public_disclosure IN('named_profile', 'anonymous')),
  PRIMARY KEY(submission_id, participant_instance_id),
  UNIQUE(submission_id, public_key),
  FOREIGN KEY(submission_id) REFERENCES submissions(id) ON DELETE CASCADE,
  FOREIGN KEY(public_key) REFERENCES identities(public_key)
) STRICT;
CREATE TABLE verified_run_metrics(
  run_id TEXT NOT NULL,
  metric TEXT NOT NULL CHECK(metric IN('original_score', 'fastest_success')),
  value INTEGER NOT NULL CHECK((metric = 'original_score' AND value BETWEEN 0 AND 4294967295)
OR(metric = 'fastest_success' AND value >= 0)),
  PRIMARY KEY(run_id, metric),
  FOREIGN KEY(run_id) REFERENCES verified_runs(id) ON DELETE CASCADE
) STRICT;
CREATE INDEX verified_metrics_board_idx
ON verified_run_metrics(
  metric,
  value,
  run_id
);
CREATE TABLE verified_run_achievements(
  run_id TEXT NOT NULL,
  achievement_id TEXT NOT NULL CHECK(length(achievement_id) BETWEEN 1 AND 128),
  earned INTEGER NOT NULL CHECK(earned IN(0, 1)),
  evidence_json TEXT NOT NULL CHECK(json_valid(evidence_json)),
  evaluation TEXT NOT NULL
  DEFAULT 'unverifiable'
  CHECK(evaluation IN('unverifiable', 'not_earned', 'earned')),
  PRIMARY KEY(run_id, achievement_id),
  FOREIGN KEY(run_id) REFERENCES verified_runs(id) ON DELETE CASCADE
) STRICT;
CREATE TABLE full_campaign_sessions(
  full_campaign_run_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
  run_id TEXT NOT NULL UNIQUE,
  PRIMARY KEY(full_campaign_run_id, ordinal),
  FOREIGN KEY(full_campaign_run_id) REFERENCES full_campaign_runs(id) ON DELETE CASCADE,
  FOREIGN KEY(run_id) REFERENCES verified_runs(id)
) STRICT;
CREATE TABLE full_campaign_participants(
  full_campaign_run_id TEXT NOT NULL,
  public_key BLOB NOT NULL CHECK(length(public_key) = 32),
  PRIMARY KEY(full_campaign_run_id, public_key),
  UNIQUE(full_campaign_run_id, public_key),
  FOREIGN KEY(full_campaign_run_id) REFERENCES full_campaign_runs(id) ON DELETE CASCADE,
  FOREIGN KEY(public_key) REFERENCES identities(public_key)
) STRICT;
CREATE TABLE full_campaign_metrics(
  full_campaign_run_id TEXT NOT NULL,
  metric TEXT NOT NULL CHECK(metric IN('original_score', 'fastest_success')),
  value INTEGER NOT NULL CHECK((metric = 'original_score' AND value BETWEEN 0 AND 4294967295)
OR(metric = 'fastest_success' AND value >= 0)),
  PRIMARY KEY(full_campaign_run_id, metric),
  FOREIGN KEY(full_campaign_run_id) REFERENCES full_campaign_runs(id) ON DELETE CASCADE
) STRICT;
CREATE INDEX full_campaign_metrics_board_idx
ON full_campaign_metrics(
  metric,
  value,
  full_campaign_run_id
);
CREATE TABLE acceptance_sequences(
  sequence INTEGER PRIMARY KEY AUTOINCREMENT,
  created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
) STRICT;
CREATE TABLE leaderboard_visibility_events(
  sequence INTEGER PRIMARY KEY AUTOINCREMENT,
  created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
) STRICT;
CREATE TABLE worker_events(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  submission_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN('leased', 'retry', 'accepted', 'rejected')),
  worker_id TEXT NOT NULL CHECK(length(worker_id) BETWEEN 1 AND 128),
  detail TEXT,
  created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
  FOREIGN KEY(submission_id) REFERENCES submissions(id)
) STRICT;
CREATE TABLE deletion_requests(
  id TEXT PRIMARY KEY NOT NULL,
  challenge_id TEXT NOT NULL UNIQUE,
  owner_public_key BLOB NOT NULL CHECK(length(owner_public_key) = 32),
  target_kind TEXT NOT NULL CHECK(target_kind IN('submission', 'run')),
  target_id TEXT NOT NULL,
  request_json TEXT NOT NULL CHECK(json_valid(request_json)),
  tombstoned_at_ms INTEGER NOT NULL CHECK(tombstoned_at_ms >= 0),
  purge_eligible_at_ms INTEGER,
  FOREIGN KEY(challenge_id) REFERENCES upload_challenges(id),
  FOREIGN KEY(owner_public_key) REFERENCES identities(public_key),
  CHECK(purge_eligible_at_ms IS NULL OR purge_eligible_at_ms > tombstoned_at_ms)
) STRICT;
CREATE TABLE abuse_reports(
  id TEXT PRIMARY KEY NOT NULL,
  target_kind TEXT NOT NULL CHECK(target_kind IN('run', 'player')),
  target_id TEXT NOT NULL,
  category TEXT NOT NULL CHECK(category IN('suspected_cheating', 'offensive_identity', 'privacy', 'copyright', 'other')),
  detail TEXT NOT NULL CHECK(length(detail) BETWEEN 1 AND 2000),
  received_at_ms INTEGER NOT NULL CHECK(received_at_ms >= 0),
  moderation_state TEXT NOT NULL DEFAULT 'open' CHECK(moderation_state IN('open', 'reviewing', 'dismissed', 'actioned'))
  ,
  reporter_ip_hash BLOB CHECK(reporter_ip_hash IS NULL OR length(reporter_ip_hash) = 32),
  quota_public_key BLOB CHECK(quota_public_key IS NULL OR length(quota_public_key) = 32),
  updated_at_ms INTEGER,
  moderator_note TEXT CHECK(moderator_note IS NULL OR length(moderator_note) <= 4000)
) STRICT;
CREATE INDEX abuse_reports_rate_idx ON abuse_reports(
  received_at_ms,
  target_kind,
  target_id
);
CREATE TABLE username_history(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  public_key BLOB NOT NULL CHECK(length(public_key) = 32),
  challenge_id TEXT NOT NULL UNIQUE,
  generation INTEGER NOT NULL CHECK(generation > 0),
  previous_username TEXT,
  new_username TEXT NOT NULL CHECK(length(new_username) BETWEEN 1 AND 48),
  changed_at_ms INTEGER NOT NULL CHECK(changed_at_ms >= 0),
  FOREIGN KEY(public_key) REFERENCES identities(public_key),
  FOREIGN KEY(challenge_id) REFERENCES upload_challenges(id)
) STRICT;
CREATE INDEX username_history_key_idx
ON username_history(
  public_key,
  changed_at_ms,
  id
);
CREATE TABLE campaign_objects(
  sha256 BLOB PRIMARY KEY NOT NULL CHECK(length(sha256) = 32),
  byte_length INTEGER NOT NULL CHECK(byte_length > 0),
  created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
  purge_state TEXT NOT NULL DEFAULT 'live' CHECK(purge_state IN('live', 'purging', 'purged')),
  purge_token TEXT,
  purge_claimed_at_ms INTEGER,
  purged_at_ms INTEGER,
  CHECK((purge_state = 'purging') =(purge_token IS NOT NULL)),
  CHECK((purge_state = 'purging') =(purge_claimed_at_ms IS NOT NULL)),
  CHECK((purge_state = 'purged') =(purged_at_ms IS NOT NULL))
) STRICT;
CREATE TABLE verified_run_campaign_objects(
  run_id TEXT NOT NULL,
  role TEXT NOT NULL CHECK(role IN('starting', 'final')),
  sha256 BLOB NOT NULL CHECK(length(sha256) = 32),
  PRIMARY KEY(run_id, role),
  FOREIGN KEY(run_id) REFERENCES verified_runs(id) ON DELETE CASCADE,
  FOREIGN KEY(sha256) REFERENCES campaign_objects(sha256)
) STRICT;
CREATE INDEX verified_run_campaign_objects_digest_idx
ON verified_run_campaign_objects(
  sha256
);
CREATE INDEX abuse_reports_ip_rate_idx
ON abuse_reports(
  reporter_ip_hash,
  received_at_ms
)
WHERE reporter_ip_hash IS NOT NULL;
CREATE INDEX abuse_reports_key_rate_idx
ON abuse_reports(
  quota_public_key,
  received_at_ms
)
WHERE quota_public_key IS NOT NULL;
CREATE TABLE moderation_audit(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  report_id TEXT,
  action TEXT NOT NULL CHECK(action IN('reviewing', 'dismissed', 'actioned', 'run_tombstoned', 'player_renamed')),
  previous_state TEXT,
  new_state TEXT,
  detail TEXT NOT NULL CHECK(length(detail) BETWEEN 1 AND 4000),
  operator_id TEXT NOT NULL CHECK(length(operator_id) BETWEEN 1 AND 128),
  created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
  FOREIGN KEY(report_id) REFERENCES abuse_reports(id)
) STRICT;
CREATE INDEX moderation_audit_report_idx
ON moderation_audit(
  report_id,
  created_at_ms,
  id
);
CREATE TABLE maintenance_locks(
  name TEXT PRIMARY KEY NOT NULL CHECK(name IN('backup')),
  token TEXT NOT NULL UNIQUE CHECK(length(token) BETWEEN 20 AND 64),
  owner TEXT NOT NULL CHECK(length(owner) BETWEEN 1 AND 128),
  acquired_at_ms INTEGER NOT NULL CHECK(acquired_at_ms >= 0),
  expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms > acquired_at_ms)
) STRICT;
CREATE TABLE submission_terminal_failures(
  submission_id TEXT PRIMARY KEY NOT NULL,
  code TEXT NOT NULL CHECK(code = 'verification_infrastructure'),
  request_artifact_sha256 BLOB CHECK(request_artifact_sha256 IS NULL OR length(request_artifact_sha256) = 32),
  private_detail TEXT NOT NULL CHECK(length(private_detail) BETWEEN 1 AND 2000),
  failed_at_ms INTEGER NOT NULL CHECK(failed_at_ms >= 0),
  FOREIGN KEY(submission_id) REFERENCES submissions(id) ON DELETE CASCADE
) STRICT;
CREATE TABLE submission_campaign_objects(
  submission_id TEXT NOT NULL,
  role TEXT NOT NULL CHECK(role = 'starting'),
  sha256 BLOB NOT NULL CHECK(length(sha256) = 32),
  PRIMARY KEY(submission_id, role),
  FOREIGN KEY(submission_id) REFERENCES submissions(id) ON DELETE CASCADE,
  FOREIGN KEY(sha256) REFERENCES campaign_objects(sha256)
) STRICT;
CREATE INDEX submission_campaign_objects_digest_idx
ON submission_campaign_objects(
  sha256
);
CREATE TABLE submission_owner_status_challenges(
  id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 20 AND 64),
  nonce BLOB NOT NULL UNIQUE CHECK(length(nonce) = 32),
  controller_public_key BLOB NOT NULL CHECK(length(controller_public_key) = 32),
  submission_id TEXT NOT NULL CHECK(length(submission_id) BETWEEN 20 AND 64),
  issued_at_ms INTEGER NOT NULL CHECK(issued_at_ms >= 0),
  expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms > issued_at_ms),
  consumed_at_ms INTEGER,
  CHECK(consumed_at_ms IS NULL OR consumed_at_ms >= issued_at_ms)
) STRICT;
CREATE INDEX submission_owner_status_challenges_expiry_idx
ON submission_owner_status_challenges(
  expires_at_ms
)
WHERE consumed_at_ms IS NULL;
CREATE TABLE verified_runs(
  id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 20 AND 64),
  submission_id TEXT NOT NULL UNIQUE,
  verifier_build_id BLOB NOT NULL CHECK(length(verifier_build_id) = 32),
  build_manifest_id BLOB NOT NULL CHECK(length(build_manifest_id) = 32),
  content_manifest_id BLOB NOT NULL CHECK(length(content_manifest_id) = 32),
  config_id BLOB NOT NULL CHECK(length(config_id) = 32),
  ruleset_id BLOB NOT NULL CHECK(length(ruleset_id) = 32),
  mission_id TEXT NOT NULL CHECK(length(mission_id) BETWEEN 1 AND 128),
  scope_kind TEXT NOT NULL CHECK(scope_kind IN('individual_level', 'campaign')),
  competition_manifest_id BLOB CHECK(competition_manifest_id IS NULL OR length(competition_manifest_id) = 32),
  starting_campaign_sha256 BLOB NOT NULL CHECK(length(starting_campaign_sha256) = 32),
  final_campaign_sha256 BLOB NOT NULL CHECK(length(final_campaign_sha256) = 32),
  campaign_chain_id TEXT,
  predecessor_run_id TEXT,
  final_state_sha256 BLOB NOT NULL CHECK(length(final_state_sha256) = 32),
  result_sha256 BLOB NOT NULL CHECK(length(result_sha256) = 32),
  verification_request_sha256 BLOB NOT NULL CHECK(length(verification_request_sha256) = 32),
  verification_result_json TEXT NOT NULL CHECK(json_valid(verification_result_json)),
  public_verification_request_sha256 BLOB NOT NULL
  CHECK(length(public_verification_request_sha256) = 32),
  public_verification_request_json TEXT NOT NULL CHECK(json_valid(public_verification_request_json)),
  public_verification_result_sha256 BLOB NOT NULL
  CHECK(length(public_verification_result_sha256) = 32),
  public_verification_result_json TEXT NOT NULL CHECK(json_valid(public_verification_result_json)),
  public_projection_binding_json TEXT NOT NULL CHECK(json_valid(public_projection_binding_json)),
  input_provenance_json TEXT NOT NULL CHECK(json_valid(input_provenance_json)),
  terminal_outcome TEXT NOT NULL CHECK(terminal_outcome = 'won'),
  replay_frames INTEGER NOT NULL CHECK(replay_frames > 0),
  diagnostics_json TEXT NOT NULL CHECK(json_valid(diagnostics_json)),
  original_score_delta INTEGER NOT NULL CHECK(original_score_delta BETWEEN 0 AND 4294967295),
  active_simulation_ticks INTEGER NOT NULL CHECK(active_simulation_ticks >= 0),
  ransom_collected INTEGER NOT NULL CHECK(ransom_collected >= 0),
  starting_campaign_score INTEGER NOT NULL,
  final_campaign_score INTEGER NOT NULL,
  campaign_session_kind TEXT CHECK(campaign_session_kind IN('field_mission', 'headquarters')),
  campaign_session_ordinal INTEGER,
  campaign_hq_sequence INTEGER,
  campaign_complete_evidence_sha256 BLOB CHECK(campaign_complete_evidence_sha256 IS NULL OR length(campaign_complete_evidence_sha256) = 32),
  max_concurrent_players INTEGER NOT NULL CHECK(max_concurrent_players BETWEEN 1 AND 4),
  participant_instance_count INTEGER NOT NULL CHECK(participant_instance_count BETWEEN max_concurrent_players AND 1024),
  named_participant_instance_count INTEGER NOT NULL CHECK(named_participant_instance_count >= 0),
  anonymous_participant_instance_count INTEGER NOT NULL CHECK(anonymous_participant_instance_count >= 0),
  accepted_sequence INTEGER NOT NULL UNIQUE CHECK(accepted_sequence > 0),
  verified_at_ms INTEGER NOT NULL CHECK(verified_at_ms >= 0),
  campaign_content_manifest_id BLOB
  CHECK(campaign_content_manifest_id IS NULL OR length(campaign_content_manifest_id) = 32),
  starting_campaign_bytes INTEGER NOT NULL CHECK(starting_campaign_bytes > 0),
  final_campaign_bytes INTEGER NOT NULL CHECK(final_campaign_bytes > 0),
  canonical_campaign_state_json TEXT NOT NULL CHECK(json_valid(canonical_campaign_state_json)),
  FOREIGN KEY(submission_id) REFERENCES submissions(id),
  FOREIGN KEY(predecessor_run_id) REFERENCES verified_runs(id),
  CHECK(named_participant_instance_count + anonymous_participant_instance_count = participant_instance_count),
  CHECK((scope_kind = 'campaign') = (campaign_content_manifest_id IS NOT NULL)),
  CHECK((scope_kind = 'campaign') =(campaign_session_kind IS NOT NULL AND campaign_session_ordinal IS NOT NULL)),
  CHECK(campaign_session_ordinal IS NULL OR campaign_session_ordinal BETWEEN 0 AND 4095),
  CHECK((campaign_session_kind = 'headquarters') =(campaign_hq_sequence IS NOT NULL)),
  CHECK(campaign_hq_sequence IS NULL OR campaign_hq_sequence > 0)
) STRICT;
CREATE INDEX verified_runs_board_idx ON verified_runs(
  scope_kind,
  mission_id,
  ruleset_id,
  competition_manifest_id,
  max_concurrent_players,
  verified_at_ms
);
CREATE TABLE full_campaign_runs(
  id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 20 AND 64),
  chain_id TEXT NOT NULL UNIQUE CHECK(length(chain_id) BETWEEN 1 AND 64),
  terminal_run_id TEXT NOT NULL UNIQUE,
  aggregate_request_sha256 BLOB NOT NULL CHECK(length(aggregate_request_sha256) = 32),
  aggregate_sha256 BLOB NOT NULL UNIQUE CHECK(length(aggregate_sha256) = 32),
  aggregate_json TEXT NOT NULL CHECK(json_valid(aggregate_json)),
  aggregate_request_json TEXT NOT NULL CHECK(json_valid(aggregate_request_json)),
  public_aggregate_request_sha256 BLOB NOT NULL CHECK(length(public_aggregate_request_sha256) = 32),
  public_aggregate_request_json TEXT NOT NULL CHECK(json_valid(public_aggregate_request_json)),
  public_aggregate_result_sha256 BLOB NOT NULL CHECK(length(public_aggregate_result_sha256) = 32),
  public_aggregate_result_json TEXT NOT NULL CHECK(json_valid(public_aggregate_result_json)),
  public_projection_binding_json TEXT NOT NULL CHECK(json_valid(public_projection_binding_json)),
  campaign_complete_evidence_sha256 BLOB NOT NULL CHECK(length(campaign_complete_evidence_sha256) = 32),
  config_id BLOB NOT NULL CHECK(length(config_id) = 32),
  ruleset_id BLOB NOT NULL CHECK(length(ruleset_id) = 32),
  competition_manifest_id BLOB CHECK(competition_manifest_id IS NULL OR length(competition_manifest_id) = 32),
  starting_campaign_sha256 BLOB NOT NULL CHECK(length(starting_campaign_sha256) = 32),
  final_campaign_sha256 BLOB NOT NULL CHECK(length(final_campaign_sha256) = 32),
  starting_campaign_score INTEGER NOT NULL,
  final_campaign_score INTEGER NOT NULL,
  active_simulation_ticks INTEGER NOT NULL CHECK(active_simulation_ticks >= 0),
  ransom_collected INTEGER NOT NULL CHECK(ransom_collected >= 0),
  max_concurrent_players INTEGER NOT NULL CHECK(max_concurrent_players BETWEEN 1 AND 4),
  participant_instance_count INTEGER NOT NULL CHECK(participant_instance_count >= max_concurrent_players),
  named_participant_instance_count INTEGER NOT NULL CHECK(named_participant_instance_count >= 0),
  anonymous_participant_instance_count INTEGER NOT NULL CHECK(anonymous_participant_instance_count >= 0),
  accepted_sequence INTEGER NOT NULL UNIQUE CHECK(accepted_sequence > 0),
  verified_at_ms INTEGER NOT NULL CHECK(verified_at_ms >= 0),
  tombstoned_at_ms INTEGER,
  campaign_content_manifest_id BLOB NOT NULL CHECK(length(campaign_content_manifest_id) = 32),
  starting_campaign_bytes INTEGER NOT NULL CHECK(starting_campaign_bytes > 0),
  final_campaign_bytes INTEGER NOT NULL CHECK(final_campaign_bytes > 0),
  canonical_campaign_state_json TEXT NOT NULL CHECK(json_valid(canonical_campaign_state_json)),
  FOREIGN KEY(terminal_run_id) REFERENCES verified_runs(id),
  CHECK(named_participant_instance_count + anonymous_participant_instance_count = participant_instance_count),
  CHECK(tombstoned_at_ms IS NULL OR tombstoned_at_ms >= verified_at_ms),
  CHECK(final_campaign_score >= starting_campaign_score)
) STRICT;
CREATE INDEX full_campaign_board_idx ON full_campaign_runs(
  ruleset_id,
  competition_manifest_id,
  max_concurrent_players,
  verified_at_ms
);
CREATE TABLE submission_upload_reservations(
  upload_challenge_id TEXT PRIMARY KEY NOT NULL,
  submission_id TEXT NOT NULL UNIQUE CHECK(length(submission_id) BETWEEN 20 AND 64),
  envelope_json TEXT NOT NULL CHECK(json_valid(envelope_json)),
  envelope_sha256 BLOB NOT NULL CHECK(length(envelope_sha256) = 32),
  controller_public_key BLOB NOT NULL CHECK(length(controller_public_key) = 32),
  session_genesis_host_public_key BLOB NOT NULL
  CHECK(length(session_genesis_host_public_key) = 32),
  replay_session_id BLOB NOT NULL CHECK(length(replay_session_id) = 32),
  session_genesis_host_nonce BLOB NOT NULL CHECK(length(session_genesis_host_nonce) = 32),
  session_genesis_sha256 BLOB NOT NULL UNIQUE CHECK(length(session_genesis_sha256) = 32),
  state TEXT NOT NULL CHECK(state IN('reserved', 'uploaded', 'abandoned', 'committed')),
  lease_token TEXT CHECK(lease_token IS NULL OR length(lease_token) BETWEEN 20 AND 64),
  lease_expires_at_ms INTEGER,
  reservation_expires_at_ms INTEGER NOT NULL,
  reserved_at_ms INTEGER NOT NULL CHECK(reserved_at_ms >= 0),
  updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= reserved_at_ms),
  abandoned_at_ms INTEGER,
  committed_at_ms INTEGER,
  FOREIGN KEY(upload_challenge_id) REFERENCES upload_challenges(id) ON DELETE CASCADE,
  UNIQUE(session_genesis_host_public_key, replay_session_id, session_genesis_host_nonce),
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
ON submission_upload_reservations(
  reservation_expires_at_ms
)
WHERE state != 'committed';
CREATE INDEX submission_upload_reservations_lease_idx
ON submission_upload_reservations(
  lease_expires_at_ms
)
WHERE state IN(
  'reserved',
  'uploaded'
);
CREATE INDEX submissions_campaign_state_config_idx
ON submissions(
  config_id,
  canonical_campaign_state_json
);
CREATE INDEX verified_runs_campaign_state_config_idx
ON verified_runs(
  config_id,
  canonical_campaign_state_json
);
CREATE INDEX full_campaign_runs_campaign_state_config_idx
ON full_campaign_runs(
  config_id,
  canonical_campaign_state_json
);
CREATE TABLE competition_run_grants(
  id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 20 AND 64),
  nonce BLOB NOT NULL UNIQUE CHECK(length(nonce) = 32),
  host_public_key BLOB NOT NULL CHECK(length(host_public_key) = 32),
  competition_manifest_id BLOB NOT NULL CHECK(length(competition_manifest_id) = 32),
  ranked_session_sha256 BLOB NOT NULL CHECK(length(ranked_session_sha256) = 32),
  request_sha256 BLOB NOT NULL UNIQUE CHECK(length(request_sha256) = 32),
  replay_session_id BLOB NOT NULL CHECK(length(replay_session_id) = 32),
  request_json TEXT NOT NULL CHECK(json_valid(request_json)),
  grant_json TEXT NOT NULL CHECK(json_valid(grant_json)),
  admitted_at_ms INTEGER NOT NULL CHECK(admitted_at_ms > 0),
  expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms > admitted_at_ms),
  competition_ends_at_ms INTEGER NOT NULL CHECK(competition_ends_at_ms > expires_at_ms),
  invalidated_at_ms INTEGER,
  consumed_at_ms INTEGER,
  completed_at_ms INTEGER,
  upload_challenge_id TEXT UNIQUE,
  CHECK(invalidated_at_ms IS NULL OR invalidated_at_ms >= admitted_at_ms),
  CHECK(consumed_at_ms IS NULL OR consumed_at_ms >= admitted_at_ms),
  CHECK(completed_at_ms IS NULL OR(consumed_at_ms IS NOT NULL AND completed_at_ms >= consumed_at_ms)),
  CHECK((consumed_at_ms IS NULL) =(upload_challenge_id IS NULL)),
  CHECK(NOT(invalidated_at_ms IS NOT NULL AND consumed_at_ms IS NOT NULL)),
  FOREIGN KEY(host_public_key) REFERENCES identities(public_key),
  FOREIGN KEY(upload_challenge_id) REFERENCES upload_challenges(id)
) STRICT;
CREATE UNIQUE INDEX competition_run_grants_session_idx
ON competition_run_grants(
  host_public_key,
  competition_manifest_id,
  replay_session_id
);
CREATE INDEX competition_run_grants_live_idx
ON competition_run_grants(
  host_public_key,
  competition_manifest_id,
  expires_at_ms
)
WHERE invalidated_at_ms IS NULL AND consumed_at_ms IS NULL;
CREATE INDEX competition_run_grants_incomplete_idx
ON competition_run_grants(
  host_public_key,
  competition_manifest_id,
  expires_at_ms
)
WHERE invalidated_at_ms IS NULL AND completed_at_ms IS NULL;
CREATE INDEX submissions_verifier_authority_idx ON submissions(
  verifier_policy_manifest_sha256,
  verifier_job_config_sha256
) WHERE verifier_job_config_sha256 IS NOT NULL;
CREATE VIEW campaign_object_submission_references AS
    SELECT submission_id, sha256 FROM submission_campaign_objects
    UNION
    SELECT run.submission_id, object.sha256
      FROM verified_run_campaign_objects object
      JOIN verified_runs run ON run.id = object.run_id
/* campaign_object_submission_references(submission_id,sha256) */;
