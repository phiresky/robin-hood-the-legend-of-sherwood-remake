//! Upload reservations and durable finalization. Challenge consumption, replay identity, and predecessor checks stay in their original transactions.
//! No independent pool, repository transaction, or fence is created here.
use super::*;

async fn register_uploaded_artifacts(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    submission: &NewSubmission,
    now: i64,
) -> Result<(i64, i64), DbError> {
    let replay_bytes = i64::try_from(submission.replay_bytes).map_err(|_| {
        DbError::ResultInvariant("replay length does not fit SQLite INTEGER".to_owned())
    })?;
    let starting_campaign_bytes =
        i64::try_from(submission.starting_campaign_bytes).map_err(|_| {
            DbError::ResultInvariant(
                "starting campaign length does not fit SQLite INTEGER".to_owned(),
            )
        })?;
    sqlx::query(
        "INSERT INTO replay_objects (sha256, byte_length, created_at_ms) VALUES (?, ?, ?) \
         ON CONFLICT(sha256) DO UPDATE SET \
             purged_at_ms = NULL, purge_state = 'live', purge_token = NULL, \
             purge_claimed_at_ms = NULL \
         WHERE replay_objects.purge_state = 'purged'",
    )
    .bind(submission.replay_sha256.as_slice())
    .bind(replay_bytes)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    let replay_object =
        sqlx::query("SELECT byte_length, purge_state FROM replay_objects WHERE sha256 = ?")
            .bind(submission.replay_sha256.as_slice())
            .fetch_one(&mut **tx)
            .await?;
    let stored_replay_bytes: i64 = replay_object.try_get("byte_length")?;
    if stored_replay_bytes != replay_bytes {
        return Err(DbError::ResultInvariant(
            "content-addressed replay length conflicts with existing object".to_owned(),
        ));
    }
    if replay_object.try_get::<String, _>("purge_state")? != "live" {
        return Err(DbError::QueueFull);
    }

    sqlx::query(
        "INSERT INTO campaign_objects (sha256, byte_length, created_at_ms) VALUES (?, ?, ?) \
         ON CONFLICT(sha256) DO UPDATE SET purge_state = 'live', purge_token = NULL, \
             purge_claimed_at_ms = NULL, purged_at_ms = NULL \
         WHERE campaign_objects.byte_length = excluded.byte_length \
           AND campaign_objects.purge_state = 'purged'",
    )
    .bind(submission.starting_campaign_sha256.as_slice())
    .bind(starting_campaign_bytes)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    let starting_object =
        sqlx::query("SELECT byte_length, purge_state FROM campaign_objects WHERE sha256 = ?")
            .bind(submission.starting_campaign_sha256.as_slice())
            .fetch_one(&mut **tx)
            .await?;
    if starting_object.try_get::<i64, _>("byte_length")? != starting_campaign_bytes
        || starting_object.try_get::<String, _>("purge_state")? != "live"
    {
        return Err(DbError::ResultInvariant(
            "uploaded starting campaign identity is not live and exact".to_owned(),
        ));
    }

    Ok((replay_bytes, starting_campaign_bytes))
}
async fn ensure_participant_identities(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    participants: &[crate::model::ParticipantClaim],
) -> Result<(), DbError> {
    for participant in participants {
        let exists: i64 =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM identities WHERE public_key = ?)")
                .bind(participant.public_key.as_slice())
                .fetch_one(&mut **tx)
                .await?;
        if exists == 0 {
            return Err(DbError::ResultInvariant(format!(
                "participant seat {} has no registered identity",
                participant.seat
            )));
        }
    }
    Ok(())
}
impl Database {
    /// Atomically consume a signed submission challenge and acquire the only
    /// lease which may ingest its artifact bytes. Exact retries either resume
    /// the durable uploaded state, return the already-created lifecycle, or
    /// fail before the caller reads another multipart field.
    pub async fn reserve_submission_upload(
        &self,
        intent: &SubmissionUploadIntent,
        lease_ttl: Duration,
        reservation_ttl: Duration,
    ) -> Result<SubmissionUploadReservation, DbError> {
        self.reserve_submission_upload_if_admitted(intent, lease_ttl, reservation_ttl, true)
            .await
    }

    /// The HTTP layer passes the capacity/backup verdict sampled immediately
    /// before this transaction. Committed and active exact retries remain
    /// observable while admission is red, but no challenge is consumed and no
    /// artifact-writing reservation is acquired until it is green again.
    pub async fn reserve_submission_upload_if_admitted(
        &self,
        intent: &SubmissionUploadIntent,
        lease_ttl: Duration,
        reservation_ttl: Duration,
        admission_available: bool,
    ) -> Result<SubmissionUploadReservation, DbError> {
        let now = now_epoch_ms()?;
        let lease_ms = i64::try_from(lease_ttl.as_millis())
            .map_err(|_| DbError::Corrupt("upload lease TTL does not fit i64".to_owned()))?;
        let reservation_ms = i64::try_from(reservation_ttl.as_millis())
            .map_err(|_| DbError::Corrupt("upload reservation TTL does not fit i64".to_owned()))?;
        if lease_ms <= 0 || reservation_ms < lease_ms {
            return Err(DbError::Corrupt(
                "upload reservation TTL must cover its lease".to_owned(),
            ));
        }
        let configured_lease_expires = now
            .checked_add(lease_ms)
            .ok_or_else(|| DbError::Corrupt("upload lease expiry overflow".to_owned()))?;
        let configured_reservation_expires = now
            .checked_add(reservation_ms)
            .ok_or_else(|| DbError::Corrupt("upload reservation expiry overflow".to_owned()))?;
        let envelope_sha256 = Digest32::digest_bytes(intent.envelope_json.as_bytes()).into_bytes();
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        recover_upload_reservations_in(&mut tx, now).await?;

        let challenge = sqlx::query(
            "SELECT purpose, public_key, expires_at_ms, consumed_at_ms, offer_json, public_metadata_json \
             FROM upload_challenges WHERE id = ?",
        )
        .bind(&intent.upload_challenge_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DbError::InvalidChallenge)?;
        let offer_json = challenge
            .try_get::<Option<String>, _>("offer_json")?
            .ok_or_else(|| DbError::Corrupt("submission challenge has no offer".to_owned()))?;
        let public_metadata_json = challenge
            .try_get::<Option<String>, _>("public_metadata_json")?
            .ok_or_else(|| {
                DbError::Corrupt("submission challenge has no metadata snapshot".to_owned())
            })?;
        let signed_offer_expires = challenge.try_get::<i64, _>("expires_at_ms")?;
        let reservation_expires = configured_reservation_expires.min(signed_offer_expires);
        let lease_expires = configured_lease_expires.min(reservation_expires);
        if lease_expires <= now {
            return Err(DbError::InvalidChallenge);
        }
        if challenge.try_get::<String, _>("purpose")? != ChallengePurpose::Submission.as_str()
            || offer_json != intent.offer_json
            || challenge.try_get::<Vec<u8>, _>("public_key")?.as_slice()
                != intent.session_genesis_host_public_key
        {
            return Err(DbError::InvalidChallenge);
        }

        let existing = sqlx::query(
            "SELECT submission_id, envelope_json, envelope_sha256, controller_public_key, \
                    session_genesis_sha256, session_genesis_host_public_key, replay_session_id, \
                    session_genesis_host_nonce, state, lease_expires_at_ms, \
                    reservation_expires_at_ms \
             FROM submission_upload_reservations WHERE upload_challenge_id = ?",
        )
        .bind(&intent.upload_challenge_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(existing) = existing {
            if existing.try_get::<String, _>("envelope_json")? != intent.envelope_json
                || existing
                    .try_get::<Vec<u8>, _>("envelope_sha256")?
                    .as_slice()
                    != envelope_sha256
                || existing
                    .try_get::<Vec<u8>, _>("controller_public_key")?
                    .as_slice()
                    != intent.controller_public_key
                || existing
                    .try_get::<Vec<u8>, _>("session_genesis_sha256")?
                    .as_slice()
                    != intent.session_genesis_sha256
                || existing
                    .try_get::<Vec<u8>, _>("session_genesis_host_public_key")?
                    .as_slice()
                    != intent.session_genesis_host_public_key
                || existing
                    .try_get::<Vec<u8>, _>("replay_session_id")?
                    .as_slice()
                    != intent.replay_session_id
                || existing
                    .try_get::<Vec<u8>, _>("session_genesis_host_nonce")?
                    .as_slice()
                    != intent.session_genesis_host_nonce
            {
                return Err(DbError::SubmissionConflict);
            }
            let submission_id: String = existing.try_get("submission_id")?;
            let state: String = existing.try_get("state")?;
            if state == "committed" {
                let lifecycle = lifecycle_by_submission_id(&mut tx, &submission_id)
                    .await?
                    .ok_or_else(|| {
                        DbError::Corrupt(
                            "committed upload reservation has no submission".to_owned(),
                        )
                    })?;
                tx.commit().await?;
                return Ok(SubmissionUploadReservation::Existing { lifecycle });
            }
            let stored_reservation_expires: i64 = existing.try_get("reservation_expires_at_ms")?;
            if stored_reservation_expires < now {
                return Err(DbError::InvalidChallenge);
            }
            let current_lease_expires: Option<i64> = existing.try_get("lease_expires_at_ms")?;
            if matches!(state.as_str(), "reserved" | "uploaded")
                && current_lease_expires.is_some_and(|expires| expires >= now)
            {
                let retry_after_ms =
                    u64::try_from(current_lease_expires.expect("checked as present") - now + 1)
                        .map_err(|_| {
                            DbError::Corrupt("negative upload lease duration".to_owned())
                        })?;
                tx.commit().await?;
                return Ok(SubmissionUploadReservation::Busy { retry_after_ms });
            }
            let resume_uploaded = state == "uploaded";
            if !resume_uploaded && state != "abandoned" && state != "reserved" {
                return Err(DbError::Corrupt(
                    "upload reservation has an invalid recoverable state".to_owned(),
                ));
            }
            if !resume_uploaded && !admission_available {
                return Err(DbError::AdmissionUnavailable);
            }
            let lease_token = uuid::Uuid::now_v7().to_string();
            let next_lease_expires = lease_expires.min(stored_reservation_expires);
            if next_lease_expires <= now {
                return Err(DbError::InvalidChallenge);
            }
            let next_state = if resume_uploaded {
                "uploaded"
            } else {
                "reserved"
            };
            let changed = sqlx::query(
                "UPDATE submission_upload_reservations \
                 SET state = ?, lease_token = ?, lease_expires_at_ms = ?, updated_at_ms = ?, \
                     abandoned_at_ms = NULL \
                 WHERE upload_challenge_id = ? AND state = ?",
            )
            .bind(next_state)
            .bind(&lease_token)
            .bind(next_lease_expires)
            .bind(now)
            .bind(&intent.upload_challenge_id)
            .bind(&state)
            .execute(&mut *tx)
            .await?;
            if changed.rows_affected() != 1 {
                return Err(DbError::InvalidChallenge);
            }
            tx.commit().await?;
            return Ok(SubmissionUploadReservation::Acquired {
                lease: SubmissionUploadLease {
                    submission_id,
                    upload_challenge_id: intent.upload_challenge_id.clone(),
                    lease_token,
                    lease_expires_at_ms: nonnegative_u64(
                        next_lease_expires,
                        "lease_expires_at_ms",
                    )?,
                    reservation_expires_at_ms: nonnegative_u64(
                        stored_reservation_expires,
                        "reservation_expires_at_ms",
                    )?,
                    offer_json,
                    public_metadata_json,
                },
                resume_uploaded,
            });
        }

        if challenge.try_get::<i64, _>("expires_at_ms")? < now
            || challenge
                .try_get::<Option<i64>, _>("consumed_at_ms")?
                .is_some()
        {
            return Err(DbError::InvalidChallenge);
        }
        if !admission_available {
            return Err(DbError::AdmissionUnavailable);
        }
        let occupied: i64 = sqlx::query_scalar(
            "SELECT \
                (SELECT COUNT(*) FROM submissions s \
                 WHERE s.status IN ('queued', 'verifying', 'retry_pending') \
                   AND s.tombstoned_at_ms IS NULL \
                   AND NOT EXISTS (SELECT 1 FROM submission_terminal_failures f \
                                   WHERE f.submission_id = s.id)) + \
                (SELECT COUNT(*) FROM submission_upload_reservations r \
                 WHERE r.state != 'committed' AND r.reservation_expires_at_ms >= ?)",
        )
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        if occupied >= i64::from(self.max_pending_submissions) {
            return Err(DbError::QueueFull);
        }
        ensure_participant_identities(&mut tx, &intent.participants).await?;
        let genesis_used: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM used_replay_session_geneses \
             WHERE (host_public_key = ? AND replay_session_id = ? AND host_nonce = ?) \
                OR session_genesis_sha256 = ?)",
        )
        .bind(intent.session_genesis_host_public_key.as_slice())
        .bind(intent.replay_session_id.as_slice())
        .bind(intent.session_genesis_host_nonce.as_slice())
        .bind(intent.session_genesis_sha256.as_slice())
        .fetch_one(&mut *tx)
        .await?;
        if genesis_used != 0 {
            return Err(DbError::SubmissionConflict);
        }

        let lease_token = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO submission_upload_reservations (\
                upload_challenge_id, submission_id, envelope_json, envelope_sha256, \
                controller_public_key, session_genesis_host_public_key, replay_session_id, \
                session_genesis_host_nonce, session_genesis_sha256, state, lease_token, \
                lease_expires_at_ms, reservation_expires_at_ms, reserved_at_ms, updated_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'reserved', ?, ?, ?, ?, ?)",
        )
        .bind(&intent.upload_challenge_id)
        .bind(&intent.proposed_submission_id)
        .bind(&intent.envelope_json)
        .bind(envelope_sha256.as_slice())
        .bind(intent.controller_public_key.as_slice())
        .bind(intent.session_genesis_host_public_key.as_slice())
        .bind(intent.replay_session_id.as_slice())
        .bind(intent.session_genesis_host_nonce.as_slice())
        .bind(intent.session_genesis_sha256.as_slice())
        .bind(&lease_token)
        .bind(lease_expires)
        .bind(reservation_expires)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| match error {
            sqlx::Error::Database(ref database_error) if database_error.is_unique_violation() => {
                DbError::SubmissionConflict
            }
            other => DbError::Sql(other),
        })?;
        let consumed = sqlx::query(
            "UPDATE upload_challenges SET consumed_at_ms = ? \
             WHERE id = ? AND purpose = 'submission' AND consumed_at_ms IS NULL",
        )
        .bind(now)
        .bind(&intent.upload_challenge_id)
        .execute(&mut *tx)
        .await?;
        if consumed.rows_affected() != 1 {
            return Err(DbError::InvalidChallenge);
        }
        tx.commit().await?;
        Ok(SubmissionUploadReservation::Acquired {
            lease: SubmissionUploadLease {
                submission_id: intent.proposed_submission_id.clone(),
                upload_challenge_id: intent.upload_challenge_id.clone(),
                lease_token,
                lease_expires_at_ms: nonnegative_u64(lease_expires, "lease_expires_at_ms")?,
                reservation_expires_at_ms: nonnegative_u64(
                    reservation_expires,
                    "reservation_expires_at_ms",
                )?,
                offer_json,
                public_metadata_json,
            },
            resume_uploaded: false,
        })
    }

    pub async fn mark_submission_upload_uploaded(
        &self,
        lease: &SubmissionUploadLease,
    ) -> Result<(), DbError> {
        let now = now_epoch_ms()?;
        let changed = sqlx::query(
            "UPDATE submission_upload_reservations SET state = 'uploaded', updated_at_ms = ? \
             WHERE upload_challenge_id = ? AND submission_id = ? AND state = 'reserved' \
               AND lease_token = ? AND lease_expires_at_ms >= ? \
               AND reservation_expires_at_ms >= ?",
        )
        .bind(now)
        .bind(&lease.upload_challenge_id)
        .bind(&lease.submission_id)
        .bind(&lease.lease_token)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(DbError::InvalidChallenge);
        }
        Ok(())
    }

    /// Release a failed partial stream or a corrupt uploaded artifact set for
    /// an exact retry. A process crash is handled by lease expiry/recovery.
    pub async fn abandon_submission_upload(
        &self,
        lease: &SubmissionUploadLease,
    ) -> Result<bool, DbError> {
        let now = now_epoch_ms()?;
        let changed = sqlx::query(
            "UPDATE submission_upload_reservations \
             SET state = 'abandoned', lease_token = NULL, lease_expires_at_ms = NULL, \
                 abandoned_at_ms = ?, updated_at_ms = ? \
             WHERE upload_challenge_id = ? AND submission_id = ? \
               AND state IN ('reserved', 'uploaded') \
               AND lease_token = ?",
        )
        .bind(now)
        .bind(now)
        .bind(&lease.upload_challenge_id)
        .bind(&lease.submission_id)
        .bind(&lease.lease_token)
        .execute(&self.pool)
        .await?;
        Ok(changed.rows_affected() == 1)
    }

    pub async fn recover_upload_reservations(&self) -> Result<u64, DbError> {
        let now = now_epoch_ms()?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let changed = recover_upload_reservations_in(&mut tx, now).await?;
        tx.commit().await?;
        Ok(changed)
    }

    /// Atomically publish artifacts already durably stored by an upload
    /// reservation. The reservation is the only authority to create the
    /// submission: the one-use challenge was consumed before storage writes.
    pub(crate) async fn finalize_submission_upload(
        &self,
        submission: &NewSubmission,
        lease: &SubmissionUploadLease,
    ) -> Result<SubmissionLifecycle, DbError> {
        let now = now_epoch_ms()?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;

        if submission.id != lease.submission_id
            || submission.upload_challenge_id != lease.upload_challenge_id
            || submission.offer_json != lease.offer_json
            || submission.public_metadata_json != lease.public_metadata_json
        {
            return Err(DbError::SubmissionConflict);
        }

        let reservation = sqlx::query(
            "SELECT submission_id, envelope_json, envelope_sha256, controller_public_key, \
                    session_genesis_host_public_key, replay_session_id, \
                    session_genesis_host_nonce, session_genesis_sha256, state, lease_token, \
                    lease_expires_at_ms, reservation_expires_at_ms \
             FROM submission_upload_reservations WHERE upload_challenge_id = ?",
        )
        .bind(&submission.upload_challenge_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DbError::InvalidChallenge)?;
        let envelope_sha256 = Digest32::digest_bytes(submission.envelope_json.as_bytes());
        if reservation.try_get::<String, _>("submission_id")? != submission.id
            || reservation.try_get::<String, _>("envelope_json")? != submission.envelope_json
            || reservation
                .try_get::<Vec<u8>, _>("envelope_sha256")?
                .as_slice()
                != envelope_sha256.as_bytes()
            || reservation
                .try_get::<Vec<u8>, _>("controller_public_key")?
                .as_slice()
                != submission.controller_public_key
            || reservation
                .try_get::<Vec<u8>, _>("session_genesis_host_public_key")?
                .as_slice()
                != submission.session_genesis_host_public_key
            || reservation
                .try_get::<Vec<u8>, _>("replay_session_id")?
                .as_slice()
                != submission.replay_session_id
            || reservation
                .try_get::<Vec<u8>, _>("session_genesis_host_nonce")?
                .as_slice()
                != submission.session_genesis_host_nonce
            || reservation
                .try_get::<Vec<u8>, _>("session_genesis_sha256")?
                .as_slice()
                != submission.session_genesis_sha256
        {
            return Err(DbError::SubmissionConflict);
        }
        let reservation_state: String = reservation.try_get("state")?;
        if reservation_state == "committed" {
            let lifecycle = self
                .existing_submission_by_challenge(&mut tx, submission)
                .await?
                .ok_or_else(|| {
                    DbError::Corrupt("committed upload reservation has no submission".to_owned())
                })?;
            tx.commit().await?;
            return Ok(lifecycle);
        }
        if reservation_state != "uploaded"
            || reservation
                .try_get::<Option<String>, _>("lease_token")?
                .as_deref()
                != Some(lease.lease_token.as_str())
            || reservation
                .try_get::<Option<i64>, _>("lease_expires_at_ms")?
                .is_none_or(|expires| expires < now)
            || reservation.try_get::<i64, _>("reservation_expires_at_ms")? < now
        {
            return Err(DbError::InvalidChallenge);
        }

        let challenge = sqlx::query(
            "SELECT purpose, consumed_at_ms, offer_json, public_metadata_json \
             FROM upload_challenges WHERE id = ?",
        )
        .bind(&submission.upload_challenge_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DbError::InvalidChallenge)?;
        if challenge.try_get::<String, _>("purpose")? != ChallengePurpose::Submission.as_str()
            || challenge
                .try_get::<Option<i64>, _>("consumed_at_ms")?
                .is_none()
            || challenge
                .try_get::<Option<String>, _>("offer_json")?
                .as_deref()
                != Some(submission.offer_json.as_str())
            || challenge
                .try_get::<Option<String>, _>("public_metadata_json")?
                .as_deref()
                != Some(submission.public_metadata_json.as_str())
        {
            return Err(DbError::Corrupt(
                "upload reservation is detached from its consumed challenge".to_owned(),
            ));
        }

        let pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM submissions s \
             WHERE status IN ('queued', 'verifying', 'retry_pending') \
                 AND tombstoned_at_ms IS NULL \
                 AND NOT EXISTS (SELECT 1 FROM submission_terminal_failures f \
                                 WHERE f.submission_id = s.id)",
        )
        .fetch_one(&mut *tx)
        .await?;
        if pending >= i64::from(self.max_pending_submissions) {
            return Err(DbError::QueueFull);
        }

        ensure_participant_identities(&mut tx, &submission.participants).await?;

        let (replay_bytes, starting_campaign_bytes) =
            register_uploaded_artifacts(&mut tx, submission, now).await?;

        let inserted_submission = sqlx::query(
            "INSERT INTO submissions (\
                id, upload_challenge_id, offer_json, envelope_json, signatures_json, \
                public_metadata_json, \
                replay_sha256, replay_bytes, build_manifest_id, \
                content_manifest_id, campaign_content_manifest_id, config_id, ruleset_id, mission_id, scope_kind, \
                starting_campaign_sha256, starting_campaign_bytes, controller_public_key, \
                canonical_campaign_state_json, starting_state_json, campaign_chain_id, \
                predecessor_run_id, competition_manifest_id, requested_metrics_json, \
                participant_claims_json, max_concurrent_players, participant_instance_count, \
                session_genesis_sha256, status, next_attempt_at_ms, \
                created_at_ms, updated_at_ms\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, \
                       ?, ?, ?, ?, ?, ?, ?, 'queued', ?, ?, ?) ON CONFLICT(upload_challenge_id) DO NOTHING",
        )
        .bind(&submission.id)
        .bind(&submission.upload_challenge_id)
        .bind(&submission.offer_json)
        .bind(&submission.envelope_json)
        .bind(&submission.signatures_json)
        .bind(&submission.public_metadata_json)
        .bind(submission.replay_sha256.as_slice())
        .bind(replay_bytes)
        .bind(submission.build_manifest_id.as_slice())
        .bind(submission.content_manifest_id.as_slice())
        .bind(
            submission
                .campaign_content_manifest_id
                .as_ref()
                .map(|digest| digest.as_slice()),
        )
        .bind(submission.config_id.as_slice())
        .bind(submission.ruleset_id.as_slice())
        .bind(&submission.mission_id)
        .bind(&submission.scope_kind)
        .bind(submission.starting_campaign_sha256.as_slice())
        .bind(starting_campaign_bytes)
        .bind(submission.controller_public_key.as_slice())
        .bind(&submission.canonical_campaign_state_json)
        .bind(&submission.starting_state_json)
        .bind(&submission.campaign_chain_id)
        .bind(&submission.predecessor_run_id)
        .bind(
            submission
                .competition_manifest_id
                .as_ref()
                .map(|digest| digest.as_slice()),
        )
        .bind(&submission.requested_metrics_json)
        .bind(&submission.participant_claims_json)
        .bind(i64::from(submission.max_concurrent_players))
        .bind(i64::from(submission.participant_instance_count))
        .bind(submission.session_genesis_sha256.as_slice())
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        if inserted_submission.rows_affected() == 0 {
            let existing = self
                .existing_submission_by_challenge(&mut tx, submission)
                .await?
                .ok_or_else(|| {
                    DbError::Corrupt(
                        "submission challenge conflict disappeared during insertion".to_owned(),
                    )
                })?;
            tx.commit().await?;
            return Ok(existing);
        }
        sqlx::query(
            "INSERT INTO submission_campaign_objects (submission_id, role, sha256) \
             VALUES (?, 'starting', ?)",
        )
        .bind(&submission.id)
        .bind(submission.starting_campaign_sha256.as_slice())
        .execute(&mut *tx)
        .await?;
        for participant in &submission.participants {
            sqlx::query(
                "INSERT INTO submission_participants \
                 (submission_id, seat, participant_instance_id, public_key, public_disclosure) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&submission.id)
            .bind(i64::from(participant.seat))
            .bind(participant.participant_instance_id.as_slice())
            .bind(participant.public_key.as_slice())
            .bind(&participant.public_disclosure)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO used_replay_session_geneses \
             (host_public_key, replay_session_id, host_nonce, session_genesis_sha256, \
              submission_id, consumed_at_ms) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(submission.session_genesis_host_public_key.as_slice())
        .bind(submission.replay_session_id.as_slice())
        .bind(submission.session_genesis_host_nonce.as_slice())
        .bind(submission.session_genesis_sha256.as_slice())
        .bind(&submission.id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| match error {
            sqlx::Error::Database(ref database_error) if database_error.is_unique_violation() => {
                DbError::SubmissionConflict
            }
            other => DbError::Sql(other),
        })?;
        let committed = sqlx::query(
            "UPDATE submission_upload_reservations \
             SET state = 'committed', lease_token = NULL, lease_expires_at_ms = NULL, \
                 committed_at_ms = ?, updated_at_ms = ? \
             WHERE upload_challenge_id = ? AND submission_id = ? AND state = 'uploaded' \
               AND lease_token = ?",
        )
        .bind(now)
        .bind(now)
        .bind(&submission.upload_challenge_id)
        .bind(&submission.id)
        .bind(&lease.lease_token)
        .execute(&mut *tx)
        .await?;
        if committed.rows_affected() != 1 {
            return Err(DbError::InvalidChallenge);
        }
        // Completion is distinct from exchanging the grant for an offer. In
        // the reserved-upload flow the challenge may be consumed before body
        // streaming begins, so only the durable submission insertion opens
        // this host/competition lane for another attempt.
        let completed_grant = sqlx::query(
            "UPDATE competition_run_grants SET completed_at_ms = ? \
             WHERE upload_challenge_id = ? AND consumed_at_ms IS NOT NULL \
               AND completed_at_ms IS NULL",
        )
        .bind(now)
        .bind(&submission.upload_challenge_id)
        .execute(&mut *tx)
        .await?;
        let expected_grant = submission.competition_manifest_id.is_some();
        if (completed_grant.rows_affected() == 1) != expected_grant {
            return Err(DbError::ResultInvariant(
                "competition submission completion did not match its one-use run grant".into(),
            ));
        }
        tx.commit().await?;
        Ok(SubmissionLifecycle {
            id: submission.id.clone(),
            state: crate::model::SubmissionState::Queued,
            created_at_ms: now as u64,
            updated_at_ms: now as u64,
        })
    }

    pub async fn register_replay_object(
        &self,
        digest: &[u8; 32],
        byte_length: u64,
    ) -> Result<(), DbError> {
        let now = now_epoch_ms()?;
        let byte_length = i64::try_from(byte_length).map_err(|_| {
            DbError::ResultInvariant("replay length does not fit SQLite INTEGER".to_owned())
        })?;
        sqlx::query(
            "INSERT INTO replay_objects (sha256, byte_length, created_at_ms) VALUES (?, ?, ?) \
             ON CONFLICT(sha256) DO UPDATE SET \
                 purged_at_ms = NULL, purge_state = 'live', purge_token = NULL, \
                 purge_claimed_at_ms = NULL \
             WHERE replay_objects.purge_state = 'purged'",
        )
        .bind(digest.as_slice())
        .bind(byte_length)
        .bind(now)
        .execute(&self.pool)
        .await?;
        let row =
            sqlx::query("SELECT byte_length, purge_state FROM replay_objects WHERE sha256 = ?")
                .bind(digest.as_slice())
                .fetch_one(&self.pool)
                .await?;
        if row.try_get::<i64, _>("byte_length")? != byte_length {
            return Err(DbError::ResultInvariant(
                "content-addressed replay length conflicts with existing object".to_owned(),
            ));
        }
        if row.try_get::<String, _>("purge_state")? != "live" {
            return Err(DbError::QueueFull);
        }
        Ok(())
    }

    pub(super) async fn existing_submission_by_challenge(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        submission: &NewSubmission,
    ) -> Result<Option<SubmissionLifecycle>, DbError> {
        let row = sqlx::query(
            "SELECT s.id, CASE WHEN f.submission_id IS NULL THEN s.status ELSE 'failed' END AS status, \
                    s.rejection_code, s.created_at_ms, s.updated_at_ms, \
                    s.envelope_json, s.replay_sha256, s.replay_bytes, \
                    s.starting_campaign_sha256, s.starting_campaign_bytes, \
                    s.controller_public_key, s.canonical_campaign_state_json, r.id AS run_id \
             FROM submissions s LEFT JOIN verified_runs r ON r.submission_id = s.id \
             LEFT JOIN submission_terminal_failures f ON f.submission_id = s.id \
             WHERE s.upload_challenge_id = ? AND s.tombstoned_at_ms IS NULL",
        )
        .bind(&submission.upload_challenge_id)
        .fetch_optional(&mut **tx)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        if row.try_get::<String, _>("envelope_json")? != submission.envelope_json
            || row.try_get::<Vec<u8>, _>("replay_sha256")?.as_slice() != submission.replay_sha256
            || row.try_get::<i64, _>("replay_bytes")?
                != i64::try_from(submission.replay_bytes).map_err(|_| {
                    DbError::ResultInvariant("replay length does not fit SQLite INTEGER".to_owned())
                })?
            || row
                .try_get::<Vec<u8>, _>("starting_campaign_sha256")?
                .as_slice()
                != submission.starting_campaign_sha256
            || row.try_get::<i64, _>("starting_campaign_bytes")?
                != i64::try_from(submission.starting_campaign_bytes).map_err(|_| {
                    DbError::ResultInvariant(
                        "starting campaign length does not fit SQLite INTEGER".to_owned(),
                    )
                })?
            || row
                .try_get::<Vec<u8>, _>("controller_public_key")?
                .as_slice()
                != submission.controller_public_key.as_slice()
            || row
                .try_get::<String, _>("canonical_campaign_state_json")?
                .as_str()
                != submission.canonical_campaign_state_json.as_str()
        {
            return Err(DbError::SubmissionConflict);
        }
        Ok(Some(lifecycle_from_row(row)?))
    }
}
