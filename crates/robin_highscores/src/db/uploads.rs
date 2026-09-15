//! Upload reservations and durable finalization. Challenge consumption,
//! replay de-duplication and queue insertion stay in their own transactions.
use super::*;

fn check_reserved_intent(
    existing: &SqliteRow,
    intent: &SubmissionUploadIntent,
    envelope_sha256: &[u8; 32],
) -> Result<(), DbError> {
    if existing.try_get::<String, _>("envelope_json")? != intent.envelope_json
        || existing
            .try_get::<Vec<u8>, _>("envelope_sha256")?
            .as_slice()
            != envelope_sha256
        || existing
            .try_get::<Vec<u8>, _>("uploader_public_key")?
            .as_slice()
            != intent.uploader_public_key
        || existing.try_get::<Vec<u8>, _>("replay_sha256")?.as_slice() != intent.replay_sha256
    {
        return Err(DbError::SubmissionConflict);
    }
    Ok(())
}

async fn ensure_uploader_identity(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    uploader_public_key: &[u8; 32],
) -> Result<(), DbError> {
    let exists: i64 =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM identities WHERE public_key = ?)")
            .bind(uploader_public_key.as_slice())
            .fetch_one(&mut **tx)
            .await?;
    if exists == 0 {
        return Err(DbError::ResultInvariant(
            "uploader has no registered identity".to_owned(),
        ));
    }
    Ok(())
}

/// A replay that is pending or was ever accepted cannot be uploaded again, and
/// only one uncommitted upload of the same replay may be in flight.
async fn ensure_replay_is_not_live(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    replay_sha256: &[u8; 32],
    upload_challenge_id: &str,
) -> Result<(), DbError> {
    let live: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM submissions WHERE replay_sha256 = ? \
             AND (status = 'accepted' \
                  OR (status IN ('queued', 'verifying', 'retry_pending') \
                      AND tombstoned_at_ms IS NULL))) \
         OR EXISTS(SELECT 1 FROM submission_upload_reservations \
             WHERE replay_sha256 = ? AND state != 'committed' AND upload_challenge_id != ?)",
    )
    .bind(replay_sha256.as_slice())
    .bind(replay_sha256.as_slice())
    .bind(upload_challenge_id)
    .fetch_one(&mut **tx)
    .await?;
    if live != 0 {
        return Err(DbError::DuplicateReplay);
    }
    Ok(())
}

fn unique_violation_as(error: sqlx::Error, mapped: DbError) -> DbError {
    match error {
        sqlx::Error::Database(ref database_error) if database_error.is_unique_violation() => mapped,
        other => DbError::Sql(other),
    }
}

impl Database {
    /// Atomically consume a signed submission challenge and acquire the only
    /// lease which may ingest its replay bytes. Exact retries either resume
    /// the durable uploaded state, return the already-created lifecycle, or
    /// fail before the caller writes anything.
    pub async fn reserve_submission_upload(
        &self,
        intent: &SubmissionUploadIntent,
        lease_ttl: Duration,
        reservation_ttl: Duration,
    ) -> Result<SubmissionUploadReservation, DbError> {
        self.reserve_submission_upload_if_admitted(intent, lease_ttl, reservation_ttl, true)
            .await
    }

    /// The HTTP layer passes the capacity admission verdict sampled immediately
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
        let signed_expiry = i64::try_from(intent.upload_challenge_expires_at_ms)
            .map_err(|_| DbError::InvalidChallenge)?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        recover_upload_reservations_in(&mut tx, now).await?;

        let challenge = sqlx::query(
            "SELECT purpose, public_key, nonce, expires_at_ms, consumed_at_ms \
             FROM upload_challenges WHERE id = ?",
        )
        .bind(&intent.upload_challenge_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DbError::InvalidChallenge)?;
        let challenge_expires = challenge.try_get::<i64, _>("expires_at_ms")?;
        if challenge.try_get::<String, _>("purpose")? != ChallengePurpose::Submission.as_str()
            || challenge.try_get::<Vec<u8>, _>("public_key")?.as_slice()
                != intent.uploader_public_key
            || challenge.try_get::<Vec<u8>, _>("nonce")?.as_slice() != intent.upload_challenge_nonce
            || challenge_expires != signed_expiry
        {
            return Err(DbError::InvalidChallenge);
        }
        let reservation_expires = configured_reservation_expires.min(challenge_expires);
        let lease_expires = configured_lease_expires.min(reservation_expires);
        if lease_expires <= now {
            return Err(DbError::InvalidChallenge);
        }

        let existing = sqlx::query(
            "SELECT submission_id, envelope_json, envelope_sha256, uploader_public_key, \
                    replay_sha256, state, lease_expires_at_ms, reservation_expires_at_ms \
             FROM submission_upload_reservations WHERE upload_challenge_id = ?",
        )
        .bind(&intent.upload_challenge_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(existing) = existing {
            check_reserved_intent(&existing, intent, &envelope_sha256)?;
            let submission_id: String = existing.try_get("submission_id")?;
            let state: String = existing.try_get("state")?;
            if state == "committed" {
                let lifecycle = lifecycle_by_submission_id(&mut tx, &submission_id)
                    .await?
                    .ok_or_else(|| {
                        DbError::Corrupt(
                            "committed upload reservation has no live submission".to_owned(),
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
                && let Some(current_lease_expires) = current_lease_expires
                && current_lease_expires >= now
            {
                let retry_after_ms = u64::try_from(current_lease_expires - now + 1)
                    .map_err(|_| DbError::Corrupt("negative upload lease duration".to_owned()))?;
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
                },
                resume_uploaded,
            });
        }

        if challenge_expires < now
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
                (SELECT COUNT(*) FROM submissions \
                 WHERE status IN ('queued', 'verifying', 'retry_pending') \
                   AND tombstoned_at_ms IS NULL) + \
                (SELECT COUNT(*) FROM submission_upload_reservations \
                 WHERE state != 'committed' AND reservation_expires_at_ms >= ?)",
        )
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        if occupied >= i64::from(self.max_pending_submissions) {
            return Err(DbError::QueueFull);
        }
        ensure_uploader_identity(&mut tx, &intent.uploader_public_key).await?;
        ensure_replay_is_not_live(&mut tx, &intent.replay_sha256, &intent.upload_challenge_id)
            .await?;

        let lease_token = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO submission_upload_reservations (\
                upload_challenge_id, submission_id, envelope_json, envelope_sha256, \
                uploader_public_key, replay_sha256, state, lease_token, \
                lease_expires_at_ms, reservation_expires_at_ms, reserved_at_ms, updated_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, 'reserved', ?, ?, ?, ?, ?)",
        )
        .bind(&intent.upload_challenge_id)
        .bind(&intent.proposed_submission_id)
        .bind(&intent.envelope_json)
        .bind(envelope_sha256.as_slice())
        .bind(intent.uploader_public_key.as_slice())
        .bind(intent.replay_sha256.as_slice())
        .bind(&lease_token)
        .bind(lease_expires)
        .bind(reservation_expires)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_violation_as(error, DbError::DuplicateReplay))?;
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

    /// Release a failed partial stream for an exact retry. A process crash is
    /// handled by lease expiry/recovery.
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

    /// Atomically publish a replay already durably stored by an upload
    /// reservation. The reservation is the only authority to create the
    /// submission: the one-use challenge was consumed before storage writes.
    pub(crate) async fn finalize_submission_upload(
        &self,
        submission: &NewSubmission,
        lease: &SubmissionUploadLease,
    ) -> Result<SubmissionLifecycle, DbError> {
        let now = now_epoch_ms()?;
        if submission.id != lease.submission_id
            || submission.upload_challenge_id != lease.upload_challenge_id
        {
            return Err(DbError::SubmissionConflict);
        }
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let reservation = sqlx::query(
            "SELECT submission_id, envelope_json, envelope_sha256, uploader_public_key, \
                    replay_sha256, state, lease_token, lease_expires_at_ms, \
                    reservation_expires_at_ms \
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
                .try_get::<Vec<u8>, _>("uploader_public_key")?
                .as_slice()
                != submission.uploader_public_key
            || reservation
                .try_get::<Vec<u8>, _>("replay_sha256")?
                .as_slice()
                != submission.replay_sha256
        {
            return Err(DbError::SubmissionConflict);
        }
        let reservation_state: String = reservation.try_get("state")?;
        if reservation_state == "committed" {
            let lifecycle = lifecycle_by_submission_id(&mut tx, &submission.id)
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
        let consumed: Option<Option<i64>> = sqlx::query_scalar(
            "SELECT consumed_at_ms FROM upload_challenges WHERE id = ? AND purpose = 'submission'",
        )
        .bind(&submission.upload_challenge_id)
        .fetch_optional(&mut *tx)
        .await?;
        if !matches!(consumed, Some(Some(_))) {
            return Err(DbError::Corrupt(
                "upload reservation is detached from its consumed challenge".to_owned(),
            ));
        }

        let pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM submissions \
             WHERE status IN ('queued', 'verifying', 'retry_pending') AND tombstoned_at_ms IS NULL",
        )
        .fetch_one(&mut *tx)
        .await?;
        if pending >= i64::from(self.max_pending_submissions) {
            return Err(DbError::QueueFull);
        }
        ensure_uploader_identity(&mut tx, &submission.uploader_public_key).await?;
        ensure_replay_is_not_live(
            &mut tx,
            &submission.replay_sha256,
            &submission.upload_challenge_id,
        )
        .await?;

        let replay_bytes = i64::try_from(submission.replay_bytes).map_err(|_| {
            DbError::ResultInvariant("replay length does not fit SQLite INTEGER".to_owned())
        })?;
        register_replay_object_in(&mut tx, &submission.replay_sha256, replay_bytes, now).await?;

        sqlx::query(
            "INSERT INTO submissions (\
                id, upload_challenge_id, envelope_json, uploader_public_key, public_disclosure, \
                board_id, mission_id, replay_sha256, replay_bytes, replay_schema_version, \
                requested_metrics_json, status, next_attempt_at_ms, created_at_ms, updated_at_ms\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'queued', ?, ?, ?)",
        )
        .bind(&submission.id)
        .bind(&submission.upload_challenge_id)
        .bind(&submission.envelope_json)
        .bind(submission.uploader_public_key.as_slice())
        .bind(submission.public_disclosure)
        .bind(&submission.board_id)
        .bind(&submission.mission_id)
        .bind(submission.replay_sha256.as_slice())
        .bind(replay_bytes)
        .bind(i64::from(submission.replay_schema_version))
        .bind(&submission.requested_metrics_json)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_violation_as(error, DbError::DuplicateReplay))?;
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
        tx.commit().await?;
        let now = nonnegative_u64(now, "submission timestamp")?;
        Ok(SubmissionLifecycle {
            id: submission.id.clone(),
            state: crate::model::SubmissionState::Queued,
            created_at_ms: now,
            updated_at_ms: now,
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
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        register_replay_object_in(&mut tx, digest, byte_length, now).await?;
        tx.commit().await?;
        Ok(())
    }
}

async fn register_replay_object_in(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    digest: &[u8; 32],
    byte_length: i64,
    now: i64,
) -> Result<(), DbError> {
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
    .execute(&mut **tx)
    .await?;
    let row = sqlx::query("SELECT byte_length, purge_state FROM replay_objects WHERE sha256 = ?")
        .bind(digest.as_slice())
        .fetch_one(&mut **tx)
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
