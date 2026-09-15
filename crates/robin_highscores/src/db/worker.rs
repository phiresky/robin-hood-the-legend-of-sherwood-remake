//! Worker lease and terminal transitions. Every transition requires the live lease.
use super::*;

impl Database {
    pub async fn lease_next(
        &self,
        worker_id: &str,
        lease_duration: Duration,
    ) -> Result<Option<WorkerJob>, DbError> {
        if worker_id.is_empty() || worker_id.len() > 128 {
            return Err(DbError::ResultInvariant("invalid worker ID".to_owned()));
        }
        let now = now_epoch_ms()?;
        let lease_ms = i64::try_from(lease_duration.as_millis())
            .map_err(|_| DbError::ResultInvariant("worker lease is too long".to_owned()))?;
        let lease_expires = now
            .checked_add(lease_ms)
            .ok_or_else(|| DbError::ResultInvariant("worker lease overflow".to_owned()))?;

        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query(
            "UPDATE submissions SET status = 'retry_pending', lease_owner = NULL, \
                lease_expires_at_ms = NULL, next_attempt_at_ms = ?, updated_at_ms = ? \
             WHERE status = 'verifying' AND lease_expires_at_ms < ?",
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let row = sqlx::query(
            "SELECT id, board_id, mission_id, replay_sha256, replay_bytes, \
                    replay_schema_version, attempts \
             FROM submissions \
             WHERE status IN ('queued', 'retry_pending') AND tombstoned_at_ms IS NULL \
                 AND next_attempt_at_ms <= ? \
             ORDER BY created_at_ms, id LIMIT 1",
        )
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(None);
        };
        let id: String = row.try_get("id")?;
        let claimed = sqlx::query(
            "UPDATE submissions SET status = 'verifying', attempts = attempts + 1, \
                lease_owner = ?, lease_expires_at_ms = ?, updated_at_ms = ? \
             WHERE id = ? AND tombstoned_at_ms IS NULL \
                 AND status IN ('queued', 'retry_pending')",
        )
        .bind(worker_id)
        .bind(lease_expires)
        .bind(now)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        if claimed.rows_affected() != 1 {
            return Err(DbError::Corrupt(
                "pending submission changed inside its immediate lease transaction".to_owned(),
            ));
        }
        insert_worker_event(&mut tx, &id, "leased", worker_id, "", now).await?;
        let attempts = nonnegative_u64(row.try_get("attempts")?, "attempts")?;
        let job = WorkerJob {
            submission_id: id,
            board_id: row.try_get("board_id")?,
            mission_id: row.try_get("mission_id")?,
            replay_sha256: fixed_32(row.try_get("replay_sha256")?)?,
            replay_bytes: nonnegative_u64(row.try_get("replay_bytes")?, "replay_bytes")?,
            replay_schema_version: checked_count(
                &row,
                "replay_schema_version",
                "replay schema version exceeds u32",
            )?,
            attempts: u32::try_from(attempts + 1)
                .map_err(|_| DbError::Corrupt("attempt count exceeds u32".to_owned()))?,
        };
        tx.commit().await?;
        Ok(Some(job))
    }

    pub async fn retry_job(
        &self,
        submission_id: &str,
        worker_id: &str,
        retry_after: Duration,
        private_detail: &str,
    ) -> Result<(), DbError> {
        let now = now_epoch_ms()?;
        let retry_ms = i64::try_from(retry_after.as_millis())
            .map_err(|_| DbError::ResultInvariant("retry delay is too long".to_owned()))?;
        let next = now
            .checked_add(retry_ms)
            .ok_or_else(|| DbError::ResultInvariant("retry time overflow".to_owned()))?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        ensure_worker_lease(&mut tx, submission_id, worker_id, now).await?;
        sqlx::query(
            "UPDATE submissions SET status = 'retry_pending', next_attempt_at_ms = ?, \
                lease_owner = NULL, lease_expires_at_ms = NULL, updated_at_ms = ? WHERE id = ?",
        )
        .bind(next)
        .bind(now)
        .bind(submission_id)
        .execute(&mut *tx)
        .await?;
        insert_worker_event(
            &mut tx,
            submission_id,
            "retry",
            worker_id,
            private_detail,
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// End a submission after its bounded verifier-infrastructure retry policy
    /// is exhausted. This is intentionally distinct from a replay rejection:
    /// no public rejection code is written and private worker diagnostics are
    /// never projected by the API.
    pub async fn fail_job(
        &self,
        submission_id: &str,
        worker_id: &str,
        private_detail: &str,
    ) -> Result<(), DbError> {
        if private_detail.is_empty() || private_detail.len() > 2000 {
            return Err(DbError::ResultInvariant(
                "private infrastructure failure detail must contain 1..=2000 bytes".to_owned(),
            ));
        }
        let now = now_epoch_ms()?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        ensure_worker_lease(&mut tx, submission_id, worker_id, now).await?;
        let changed = sqlx::query(
            "UPDATE submissions SET status = 'failed', failure_detail = ?, lease_owner = NULL, \
                 lease_expires_at_ms = NULL, updated_at_ms = ? \
             WHERE id = ? AND status = 'verifying'",
        )
        .bind(private_detail)
        .bind(now)
        .bind(submission_id)
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(DbError::LeaseLost);
        }
        insert_worker_event(
            &mut tx,
            submission_id,
            "failed",
            worker_id,
            private_detail,
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn reject_job(
        &self,
        submission_id: &str,
        worker_id: &str,
        rejection_code: &str,
        detail_code: Option<&str>,
    ) -> Result<(), DbError> {
        if !is_public_rejection_code(rejection_code) {
            return Err(DbError::ResultInvariant(format!(
                "unknown public rejection code `{rejection_code}`"
            )));
        }
        if detail_code.is_some_and(|detail| detail.is_empty() || detail.len() > 128) {
            return Err(DbError::ResultInvariant(
                "rejection detail code must contain 1..=128 bytes".to_owned(),
            ));
        }
        let now = now_epoch_ms()?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        ensure_worker_lease(&mut tx, submission_id, worker_id, now).await?;
        sqlx::query(
            "UPDATE submissions SET status = 'rejected', rejection_code = ?, \
                rejection_detail = ?, lease_owner = NULL, lease_expires_at_ms = NULL, \
                updated_at_ms = ? WHERE id = ?",
        )
        .bind(rejection_code)
        .bind(detail_code)
        .bind(now)
        .bind(submission_id)
        .execute(&mut *tx)
        .await?;
        insert_worker_event(
            &mut tx,
            submission_id,
            "rejected",
            worker_id,
            detail_code.unwrap_or(rejection_code),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
