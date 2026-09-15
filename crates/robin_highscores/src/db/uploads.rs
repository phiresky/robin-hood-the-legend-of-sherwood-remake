//! Upload reservations and durable finalization. Replay de-duplication, the
//! per-uploader concurrency cap and queue insertion stay in their own
//! transactions.
use super::*;

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

/// A replay that is pending or was ever accepted cannot be uploaded again.
/// The byte-identical signed request that created that submission is an exact
/// retry and observes its lifecycle; any other request is a duplicate.
async fn live_submission_for_replay(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    replay_sha256: &[u8; 32],
    signed_request_json: &str,
) -> Result<Option<SubmissionLifecycle>, DbError> {
    let row = sqlx::query(
        "SELECT s.id, s.status, s.rejection_code, s.created_at_ms, s.updated_at_ms, \
                s.signed_request_json, r.id AS run_id \
         FROM submissions s LEFT JOIN verified_runs r ON r.submission_id = s.id \
         WHERE s.replay_sha256 = ? \
           AND (s.status = 'accepted' \
                OR (s.status IN ('queued', 'verifying', 'retry_pending') \
                    AND s.tombstoned_at_ms IS NULL)) \
         LIMIT 1",
    )
    .bind(replay_sha256.as_slice())
    .fetch_optional(&mut **tx)
    .await?;
    match row {
        None => Ok(None),
        Some(row) if row.try_get::<String, _>("signed_request_json")? == signed_request_json => {
            lifecycle_from_row(row).map(Some)
        }
        Some(_) => Err(DbError::DuplicateReplay),
    }
}

/// Enforce the per-uploader cap on uploads holding a live lease. `except`
/// excludes the reservation being reacquired.
async fn ensure_uploader_concurrency(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    uploader_public_key: &[u8; 32],
    now: i64,
    maximum: u32,
    except: Option<&str>,
) -> Result<(), DbError> {
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM submission_upload_reservations \
         WHERE uploader_public_key = ? AND state IN ('reserved', 'uploaded') \
           AND lease_expires_at_ms >= ? AND submission_id != ?",
    )
    .bind(uploader_public_key.as_slice())
    .bind(now)
    .bind(except.unwrap_or(""))
    .fetch_one(&mut **tx)
    .await?;
    if active >= i64::from(maximum) {
        return Err(DbError::UploadConcurrencyLimit);
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
    /// Acquire the only lease which may ingest this replay's bytes. Exact
    /// retries return the already-created lifecycle; a re-signed retry by the
    /// same uploader resumes its abandoned or durably uploaded reservation.
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
    /// before this transaction. Completed exact retries and uploaded
    /// reservations remain observable while admission is red, but no new
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
        let lease_expires = now
            .checked_add(lease_ms)
            .ok_or_else(|| DbError::Corrupt("upload lease expiry overflow".to_owned()))?;
        let reservation_expires = now
            .checked_add(reservation_ms)
            .ok_or_else(|| DbError::Corrupt("upload reservation expiry overflow".to_owned()))?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        recover_upload_reservations_in(&mut tx, now).await?;

        if let Some(lifecycle) =
            live_submission_for_replay(&mut tx, &intent.replay_sha256, &intent.signed_request_json)
                .await?
        {
            tx.commit().await?;
            return Ok(SubmissionUploadReservation::Existing { lifecycle });
        }

        let existing = sqlx::query(
            "SELECT submission_id, uploader_public_key, state, lease_expires_at_ms, \
                    reservation_expires_at_ms \
             FROM submission_upload_reservations WHERE replay_sha256 = ?",
        )
        .bind(intent.replay_sha256.as_slice())
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(existing) = existing {
            if existing
                .try_get::<Vec<u8>, _>("uploader_public_key")?
                .as_slice()
                != intent.uploader_public_key
            {
                return Err(DbError::DuplicateReplay);
            }
            let submission_id: String = existing.try_get("submission_id")?;
            let state: String = existing.try_get("state")?;
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
            ensure_uploader_concurrency(
                &mut tx,
                &intent.uploader_public_key,
                now,
                self.max_concurrent_uploads_per_key,
                Some(&submission_id),
            )
            .await?;
            let stored_reservation_expires: i64 = existing.try_get("reservation_expires_at_ms")?;
            let next_lease_expires = lease_expires.min(stored_reservation_expires);
            if next_lease_expires <= now {
                return Err(DbError::UploadLeaseLost);
            }
            let lease_token = uuid::Uuid::now_v7().to_string();
            let next_state = if resume_uploaded {
                "uploaded"
            } else {
                "reserved"
            };
            let changed = sqlx::query(
                "UPDATE submission_upload_reservations \
                 SET state = ?, lease_token = ?, lease_expires_at_ms = ?, updated_at_ms = ?, \
                     abandoned_at_ms = NULL, signed_request_json = ? \
                 WHERE submission_id = ? AND state = ?",
            )
            .bind(next_state)
            .bind(&lease_token)
            .bind(next_lease_expires)
            .bind(now)
            .bind(&intent.signed_request_json)
            .bind(&submission_id)
            .bind(&state)
            .execute(&mut *tx)
            .await?;
            if changed.rows_affected() != 1 {
                return Err(DbError::UploadLeaseLost);
            }
            tx.commit().await?;
            return Ok(SubmissionUploadReservation::Acquired {
                lease: SubmissionUploadLease {
                    submission_id,
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

        if !admission_available {
            return Err(DbError::AdmissionUnavailable);
        }
        let occupied: i64 = sqlx::query_scalar(
            "SELECT \
                (SELECT COUNT(*) FROM submissions \
                 WHERE status IN ('queued', 'verifying', 'retry_pending') \
                   AND tombstoned_at_ms IS NULL) + \
                (SELECT COUNT(*) FROM submission_upload_reservations \
                 WHERE reservation_expires_at_ms >= ?)",
        )
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        if occupied >= i64::from(self.max_pending_submissions) {
            return Err(DbError::QueueFull);
        }
        ensure_uploader_concurrency(
            &mut tx,
            &intent.uploader_public_key,
            now,
            self.max_concurrent_uploads_per_key,
            None,
        )
        .await?;
        ensure_uploader_identity(&mut tx, &intent.uploader_public_key).await?;

        let lease_token = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO submission_upload_reservations (\
                submission_id, signed_request_json, uploader_public_key, replay_sha256, state, \
                lease_token, lease_expires_at_ms, reservation_expires_at_ms, reserved_at_ms, \
                updated_at_ms) \
             VALUES (?, ?, ?, ?, 'reserved', ?, ?, ?, ?, ?)",
        )
        .bind(&intent.proposed_submission_id)
        .bind(&intent.signed_request_json)
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
        tx.commit().await?;
        Ok(SubmissionUploadReservation::Acquired {
            lease: SubmissionUploadLease {
                submission_id: intent.proposed_submission_id.clone(),
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
             WHERE submission_id = ? AND state = 'reserved' \
               AND lease_token = ? AND lease_expires_at_ms >= ? \
               AND reservation_expires_at_ms >= ?",
        )
        .bind(now)
        .bind(&lease.submission_id)
        .bind(&lease.lease_token)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(DbError::UploadLeaseLost);
        }
        Ok(())
    }

    /// Release a failed partial stream for a retry. A process crash is
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
             WHERE submission_id = ? AND state IN ('reserved', 'uploaded') \
               AND lease_token = ?",
        )
        .bind(now)
        .bind(now)
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

    /// Atomically publish a replay already durably stored under an upload
    /// reservation. The reservation lease is the only authority to create the
    /// submission; it is deleted in the same transaction.
    pub(crate) async fn finalize_submission_upload(
        &self,
        submission: &NewSubmission,
        lease: &SubmissionUploadLease,
    ) -> Result<SubmissionLifecycle, DbError> {
        let now = now_epoch_ms()?;
        if submission.id != lease.submission_id {
            return Err(DbError::SubmissionConflict);
        }
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let reservation = sqlx::query(
            "SELECT signed_request_json, uploader_public_key, replay_sha256, state, lease_token, \
                    lease_expires_at_ms, reservation_expires_at_ms \
             FROM submission_upload_reservations WHERE submission_id = ?",
        )
        .bind(&submission.id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DbError::UploadLeaseLost)?;
        if reservation.try_get::<String, _>("state")? != "uploaded"
            || reservation
                .try_get::<Option<String>, _>("lease_token")?
                .as_deref()
                != Some(lease.lease_token.as_str())
            || reservation
                .try_get::<Option<i64>, _>("lease_expires_at_ms")?
                .is_none_or(|expires| expires < now)
            || reservation.try_get::<i64, _>("reservation_expires_at_ms")? < now
        {
            return Err(DbError::UploadLeaseLost);
        }
        if reservation.try_get::<String, _>("signed_request_json")?
            != submission.signed_request_json
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
        if live_submission_for_replay(
            &mut tx,
            &submission.replay_sha256,
            &submission.signed_request_json,
        )
        .await?
        .is_some()
        {
            return Err(DbError::DuplicateReplay);
        }

        let replay_bytes = i64::try_from(submission.replay_bytes).map_err(|_| {
            DbError::ResultInvariant("replay length does not fit SQLite INTEGER".to_owned())
        })?;
        register_replay_object_in(&mut tx, &submission.replay_sha256, replay_bytes, now).await?;

        sqlx::query(
            "INSERT INTO submissions (\
                id, signed_request_json, uploader_public_key, public_disclosure, \
                board_id, mission_id, replay_sha256, replay_bytes, replay_schema_version, \
                requested_metrics_json, status, next_attempt_at_ms, created_at_ms, updated_at_ms\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'queued', ?, ?, ?)",
        )
        .bind(&submission.id)
        .bind(&submission.signed_request_json)
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
            "DELETE FROM submission_upload_reservations \
             WHERE submission_id = ? AND state = 'uploaded' AND lease_token = ?",
        )
        .bind(&submission.id)
        .bind(&lease.lease_token)
        .execute(&mut *tx)
        .await?;
        if committed.rows_affected() != 1 {
            return Err(DbError::UploadLeaseLost);
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
