//! Artifact garbage collection and backup/write coordination. This module uses the same pool and process fence as request and worker operations.
//! No independent pool, repository transaction, or fence is created here.
use super::*;

impl Database {
    pub async fn claim_replay_gc_candidates(
        &self,
        now_unix_ms: u64,
        orphan_before_unix_ms: u64,
        limit: u32,
    ) -> Result<Vec<ReplayGcCandidate>, DbError> {
        if limit == 0 || limit > 1_000 {
            return Err(DbError::ResultInvariant(
                "replay GC limit must be in 1..=1000".to_owned(),
            ));
        }
        let now = i64::try_from(now_unix_ms)
            .map_err(|_| DbError::ResultInvariant("GC timestamp does not fit i64".to_owned()))?;
        let orphan_before = i64::try_from(orphan_before_unix_ms).map_err(|_| {
            DbError::ResultInvariant("orphan cutoff timestamp does not fit i64".to_owned())
        })?;
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT ro.sha256, ro.byte_length FROM replay_objects ro \
             WHERE ro.purge_state = 'live' \
               AND NOT EXISTS (SELECT 1 FROM maintenance_locks lock \
                               WHERE lock.name = 'backup' AND lock.expires_at_ms > ?) \
               AND NOT EXISTS (SELECT 1 FROM submissions live \
                               WHERE live.replay_sha256 = ro.sha256 \
                                 AND live.tombstoned_at_ms IS NULL) \
               AND ((EXISTS (SELECT 1 FROM submissions historical \
                            WHERE historical.replay_sha256 = ro.sha256) \
                     AND NOT EXISTS (SELECT 1 FROM submissions retained \
                                     WHERE retained.replay_sha256 = ro.sha256 \
                                       AND (retained.purge_eligible_at_ms IS NULL \
                                            OR retained.purge_eligible_at_ms > ?))) \
                    OR (NOT EXISTS (SELECT 1 FROM submissions any_submission \
                                   WHERE any_submission.replay_sha256 = ro.sha256) \
                        AND ro.created_at_ms <= ?)) \
             ORDER BY ro.created_at_ms, ro.sha256 LIMIT ?",
        )
        .bind(now)
        .bind(now)
        .bind(orphan_before)
        .bind(i64::from(limit))
        .fetch_all(&mut *tx)
        .await?;
        let mut claimed = Vec::with_capacity(rows.len());
        for row in rows {
            let sha256 = fixed_32(row.try_get("sha256")?)?;
            let token = uuid::Uuid::now_v7().to_string();
            let changed = sqlx::query(
                "UPDATE replay_objects SET purge_state = 'purging', purge_token = ?, \
                     purge_claimed_at_ms = ? \
                 WHERE sha256 = ? AND purge_state = 'live' \
                   AND NOT EXISTS (SELECT 1 FROM maintenance_locks lock \
                                   WHERE lock.name = 'backup' AND lock.expires_at_ms > ?) \
                   AND NOT EXISTS (SELECT 1 FROM submissions live \
                                   WHERE live.replay_sha256 = replay_objects.sha256 \
                                     AND live.tombstoned_at_ms IS NULL)",
            )
            .bind(&token)
            .bind(now)
            .bind(sha256.as_slice())
            .bind(now)
            .execute(&mut *tx)
            .await?;
            if changed.rows_affected() == 1 {
                claimed.push(ReplayGcCandidate {
                    sha256,
                    byte_length: nonnegative_u64(row.try_get("byte_length")?, "byte_length")?,
                    claim_token: token,
                });
            }
        }
        tx.commit().await?;
        Ok(claimed)
    }

    pub async fn expire_rejected_submissions(
        &self,
        rejected_before_unix_ms: u64,
        tombstoned_at_unix_ms: u64,
    ) -> Result<u64, DbError> {
        let rejected_before = i64::try_from(rejected_before_unix_ms).map_err(|_| {
            DbError::ResultInvariant("rejected retention cutoff does not fit i64".to_owned())
        })?;
        let tombstoned_at = i64::try_from(tombstoned_at_unix_ms).map_err(|_| {
            DbError::ResultInvariant("tombstone timestamp does not fit i64".to_owned())
        })?;
        let purge_eligible_at = tombstoned_at.checked_add(1).ok_or_else(|| {
            DbError::ResultInvariant("rejected purge timestamp overflows".to_owned())
        })?;
        let changed = sqlx::query(
            "UPDATE submissions SET tombstoned_at_ms = ?, purge_eligible_at_ms = ?, \
                 updated_at_ms = ? \
             WHERE (status = 'rejected' \
                    OR EXISTS (SELECT 1 FROM submission_terminal_failures f \
                               WHERE f.submission_id = submissions.id)) \
               AND tombstoned_at_ms IS NULL AND updated_at_ms <= ?",
        )
        .bind(tombstoned_at)
        .bind(purge_eligible_at)
        .bind(tombstoned_at)
        .bind(rejected_before)
        .execute(&self.pool)
        .await?;
        Ok(changed.rows_affected())
    }

    /// Return durable in-progress purge claims. A crashed collector resumes
    /// the same token-specific quarantine operation; it must never reset a
    /// stale claim to `live`, because an old collector could still wake and
    /// delete a newly referenced object.
    pub async fn claimed_replay_purges(
        &self,
        limit: u32,
    ) -> Result<Vec<ReplayGcCandidate>, DbError> {
        if limit == 0 || limit > 1_000 {
            return Err(DbError::ResultInvariant(
                "replay GC limit must be in 1..=1000".to_owned(),
            ));
        }
        let now = now_epoch_ms()?;
        let rows = sqlx::query(
            "SELECT sha256, byte_length, purge_token FROM replay_objects \
             WHERE purge_state = 'purging' \
               AND NOT EXISTS (SELECT 1 FROM maintenance_locks lock \
                   WHERE lock.name = 'backup' AND lock.expires_at_ms > ?) \
             ORDER BY purge_claimed_at_ms, sha256 LIMIT ?",
        )
        .bind(now)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ReplayGcCandidate {
                    sha256: fixed_32(row.try_get("sha256")?)?,
                    byte_length: nonnegative_u64(row.try_get("byte_length")?, "byte_length")?,
                    claim_token: row.try_get("purge_token")?,
                })
            })
            .collect()
    }

    pub async fn finish_replay_purge(
        &self,
        digest: &[u8; 32],
        claim_token: &str,
        now_unix_ms: u64,
    ) -> Result<bool, DbError> {
        let now = i64::try_from(now_unix_ms)
            .map_err(|_| DbError::ResultInvariant("purge timestamp does not fit i64".to_owned()))?;
        let changed = sqlx::query(
            "UPDATE replay_objects SET purged_at_ms = ?, purge_state = 'purged', \
                 purge_token = NULL, purge_claimed_at_ms = NULL \
             WHERE sha256 = ? AND purge_state = 'purging' AND purge_token = ?",
        )
        .bind(now)
        .bind(digest.as_slice())
        .bind(claim_token)
        .execute(&self.pool)
        .await?;
        Ok(changed.rows_affected() == 1)
    }

    pub async fn claim_campaign_gc_candidates(
        &self,
        now_unix_ms: u64,
        orphan_before_unix_ms: u64,
        limit: u32,
    ) -> Result<Vec<CampaignGcCandidate>, DbError> {
        if limit == 0 || limit > 1_000 {
            return Err(DbError::ResultInvariant(
                "campaign GC limit must be in 1..=1000".to_owned(),
            ));
        }
        let now = i64::try_from(now_unix_ms)
            .map_err(|_| DbError::ResultInvariant("GC timestamp exceeds i64".to_owned()))?;
        let orphan_before = i64::try_from(orphan_before_unix_ms)
            .map_err(|_| DbError::ResultInvariant("GC cutoff exceeds i64".to_owned()))?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let rows = sqlx::query(
            "SELECT object.sha256, object.byte_length FROM campaign_objects object \
             WHERE object.purge_state = 'live' \
               AND NOT EXISTS (SELECT 1 FROM maintenance_locks lock \
                               WHERE lock.name = 'backup' AND lock.expires_at_ms > ?) \
               AND NOT EXISTS (SELECT 1 FROM campaign_object_submission_references ref \
                   JOIN submissions submission ON submission.id = ref.submission_id \
                   WHERE ref.sha256 = object.sha256 AND submission.tombstoned_at_ms IS NULL) \
               AND ((EXISTS (SELECT 1 FROM campaign_object_submission_references ref \
                            WHERE ref.sha256 = object.sha256) \
                     AND NOT EXISTS (SELECT 1 FROM campaign_object_submission_references ref \
                         JOIN submissions submission ON submission.id = ref.submission_id \
                         WHERE ref.sha256 = object.sha256 \
                           AND (submission.purge_eligible_at_ms IS NULL \
                                OR submission.purge_eligible_at_ms > ?))) \
                    OR (NOT EXISTS (SELECT 1 FROM campaign_object_submission_references ref \
                                   WHERE ref.sha256 = object.sha256) \
                        AND object.created_at_ms <= ?)) \
             ORDER BY object.created_at_ms, object.sha256 LIMIT ?",
        )
        .bind(now)
        .bind(now)
        .bind(orphan_before)
        .bind(i64::from(limit))
        .fetch_all(&mut *tx)
        .await?;
        let mut claimed = Vec::with_capacity(rows.len());
        for row in rows {
            let sha256 = fixed_32(row.try_get("sha256")?)?;
            let token = uuid::Uuid::now_v7().to_string();
            let changed = sqlx::query(
                "UPDATE campaign_objects SET purge_state = 'purging', purge_token = ?, \
                     purge_claimed_at_ms = ? WHERE sha256 = ? AND purge_state = 'live' \
                     AND NOT EXISTS (SELECT 1 FROM maintenance_locks lock \
                         WHERE lock.name = 'backup' AND lock.expires_at_ms > ?) \
                     AND NOT EXISTS (SELECT 1 FROM campaign_object_submission_references ref \
                         JOIN submissions submission ON submission.id = ref.submission_id \
                         WHERE ref.sha256 = campaign_objects.sha256 \
                           AND submission.tombstoned_at_ms IS NULL)",
            )
            .bind(&token)
            .bind(now)
            .bind(sha256.as_slice())
            .bind(now)
            .execute(&mut *tx)
            .await?;
            if changed.rows_affected() == 1 {
                claimed.push(CampaignGcCandidate {
                    sha256,
                    byte_length: nonnegative_u64(row.try_get("byte_length")?, "byte_length")?,
                    claim_token: token,
                });
            }
        }
        tx.commit().await?;
        Ok(claimed)
    }

    pub async fn claimed_campaign_purges(
        &self,
        limit: u32,
    ) -> Result<Vec<CampaignGcCandidate>, DbError> {
        if limit == 0 || limit > 1_000 {
            return Err(DbError::ResultInvariant(
                "campaign GC limit must be in 1..=1000".to_owned(),
            ));
        }
        let now = now_epoch_ms()?;
        let rows = sqlx::query(
            "SELECT sha256, byte_length, purge_token FROM campaign_objects \
             WHERE purge_state = 'purging' \
               AND NOT EXISTS (SELECT 1 FROM maintenance_locks lock \
                   WHERE lock.name = 'backup' AND lock.expires_at_ms > ?) \
             ORDER BY purge_claimed_at_ms, sha256 LIMIT ?",
        )
        .bind(now)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(CampaignGcCandidate {
                    sha256: fixed_32(row.try_get("sha256")?)?,
                    byte_length: nonnegative_u64(row.try_get("byte_length")?, "byte_length")?,
                    claim_token: row.try_get("purge_token")?,
                })
            })
            .collect()
    }

    pub async fn finish_campaign_purge(
        &self,
        digest: &[u8; 32],
        claim_token: &str,
        now_unix_ms: u64,
    ) -> Result<bool, DbError> {
        let now = i64::try_from(now_unix_ms)
            .map_err(|_| DbError::ResultInvariant("purge timestamp exceeds i64".to_owned()))?;
        let changed = sqlx::query(
            "UPDATE campaign_objects SET purged_at_ms = ?, purge_state = 'purged', \
                 purge_token = NULL, purge_claimed_at_ms = NULL \
             WHERE sha256 = ? AND purge_state = 'purging' AND purge_token = ?",
        )
        .bind(now)
        .bind(digest.as_slice())
        .bind(claim_token)
        .execute(&self.pool)
        .await?;
        Ok(changed.rows_affected() == 1)
    }

    /// Admit one bounded state-mutating request only while no backup has
    /// closed the writer gate. The lease is durable across processes so the
    /// scheduled admin and secret-isolated worker share one authority.
    pub async fn acquire_maintenance_write_lease(
        &self,
        writer_class: MaintenanceWriteClass,
        owner: &str,
        ttl: Duration,
    ) -> Result<String, DbError> {
        if owner.is_empty() || owner.len() > 128 {
            return Err(DbError::ResultInvariant(
                "invalid maintenance writer owner".to_owned(),
            ));
        }
        let now = now_epoch_ms()?;
        let expires = now
            .checked_add(i64::try_from(ttl.as_millis()).map_err(|_| {
                DbError::ResultInvariant("maintenance writer TTL exceeds i64".to_owned())
            })?)
            .ok_or_else(|| {
                DbError::ResultInvariant("maintenance writer expiry overflow".to_owned())
            })?;
        let token = uuid::Uuid::now_v7().to_string();
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("DELETE FROM maintenance_write_leases WHERE expires_at_ms <= ?")
            .bind(now)
            .execute(&mut *tx)
            .await?;
        let backup_active: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM maintenance_locks \
             WHERE name = 'backup' AND expires_at_ms > ?)",
        )
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        if backup_active != 0 {
            return Err(DbError::QueueFull);
        }
        let class_limit = match writer_class {
            MaintenanceWriteClass::ApiSensitive => self.max_concurrent_sensitive_writers,
            MaintenanceWriteClass::ApiUpload => self.max_concurrent_upload_writers,
            MaintenanceWriteClass::ApiMaintenance
            | MaintenanceWriteClass::Worker
            | MaintenanceWriteClass::Admin => 1,
        };
        let active_class: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM maintenance_write_leases \
             WHERE writer_class = ? AND expires_at_ms > ?",
        )
        .bind(writer_class.as_str())
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        if u64::try_from(active_class).map_err(|_| {
            DbError::Corrupt("maintenance writer class count is negative".to_owned())
        })? >= class_limit
        {
            return Err(DbError::QueueFull);
        }
        sqlx::query(
            "INSERT INTO maintenance_write_leases \
             (token, writer_class, owner, acquired_at_ms, expires_at_ms) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&token)
        .bind(writer_class.as_str())
        .bind(owner)
        .bind(now)
        .bind(expires)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(token)
    }

    pub async fn release_maintenance_write_lease(&self, token: &str) -> Result<bool, DbError> {
        let changed = sqlx::query("DELETE FROM maintenance_write_leases WHERE token = ?")
            .bind(token)
            .execute(&self.pool)
            .await?;
        Ok(changed.rows_affected() == 1)
    }

    pub async fn refresh_maintenance_write_lease(
        &self,
        token: &str,
        ttl: Duration,
    ) -> Result<bool, DbError> {
        let now = now_epoch_ms()?;
        let expires = now
            .checked_add(i64::try_from(ttl.as_millis()).map_err(|_| {
                DbError::ResultInvariant("maintenance writer TTL exceeds i64".to_owned())
            })?)
            .ok_or_else(|| {
                DbError::ResultInvariant("maintenance writer expiry overflow".to_owned())
            })?;
        let changed = sqlx::query(
            "UPDATE maintenance_write_leases SET expires_at_ms = ? \
             WHERE token = ? AND expires_at_ms > ?",
        )
        .bind(expires)
        .bind(token)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(changed.rows_affected() == 1)
    }

    pub async fn backup_lock_active(&self) -> Result<bool, DbError> {
        let now = now_epoch_ms()?;
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM maintenance_locks \
             WHERE name = 'backup' AND expires_at_ms > ?)",
        )
        .bind(now)
        .fetch_one(&self.pool)
        .await?
            != 0)
    }

    pub async fn active_maintenance_write_lease_count(&self) -> Result<u64, DbError> {
        let now = now_epoch_ms()?;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM maintenance_write_leases WHERE expires_at_ms > ?",
        )
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        u64::try_from(count).map_err(|_| {
            DbError::ResultInvariant("maintenance writer count is negative".to_owned())
        })
    }

    pub async fn acquire_backup_lock(&self, owner: &str, ttl: Duration) -> Result<String, DbError> {
        if owner.is_empty() || owner.len() > 128 {
            return Err(DbError::ResultInvariant("invalid backup owner".to_owned()));
        }
        let now = now_epoch_ms()?;
        let expires = now
            .checked_add(
                i64::try_from(ttl.as_millis()).map_err(|_| {
                    DbError::ResultInvariant("backup lock TTL exceeds i64".to_owned())
                })?,
            )
            .ok_or_else(|| DbError::ResultInvariant("backup lock expiry overflow".to_owned()))?;
        let token = uuid::Uuid::now_v7().to_string();
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("DELETE FROM maintenance_locks WHERE expires_at_ms <= ?")
            .bind(now)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM maintenance_write_leases WHERE expires_at_ms <= ?")
            .bind(now)
            .execute(&mut *tx)
            .await?;
        let purge_in_progress: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM replay_objects WHERE purge_state = 'purging') \
                 OR EXISTS(SELECT 1 FROM campaign_objects WHERE purge_state = 'purging')",
        )
        .fetch_one(&mut *tx)
        .await?;
        if purge_in_progress != 0 {
            return Err(DbError::QueueFull);
        }
        let inserted = sqlx::query(
            "INSERT INTO maintenance_locks \
             (name, token, owner, acquired_at_ms, expires_at_ms) VALUES ('backup', ?, ?, ?, ?) \
             ON CONFLICT(name) DO NOTHING",
        )
        .bind(&token)
        .bind(owner)
        .bind(now)
        .bind(expires)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() != 1 {
            return Err(DbError::QueueFull);
        }
        tx.commit().await?;
        Ok(token)
    }

    pub async fn refresh_backup_lock(&self, token: &str, ttl: Duration) -> Result<bool, DbError> {
        let now = now_epoch_ms()?;
        let expires = now
            .checked_add(
                i64::try_from(ttl.as_millis()).map_err(|_| {
                    DbError::ResultInvariant("backup lock TTL exceeds i64".to_owned())
                })?,
            )
            .ok_or_else(|| DbError::ResultInvariant("backup lock expiry overflow".to_owned()))?;
        let changed = sqlx::query(
            "UPDATE maintenance_locks SET expires_at_ms = ? \
             WHERE name = 'backup' AND token = ? AND expires_at_ms > ?",
        )
        .bind(expires)
        .bind(token)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(changed.rows_affected() == 1)
    }

    pub async fn release_backup_lock(&self, token: &str) -> Result<bool, DbError> {
        let changed =
            sqlx::query("DELETE FROM maintenance_locks WHERE name = 'backup' AND token = ?")
                .bind(token)
                .execute(&self.pool)
                .await?;
        Ok(changed.rows_affected() == 1)
    }

    /// Reconcile an outcome-uncertain backup-gate deletion while the caller
    /// still owns the exclusive runtime fence. Unlike `backup_lock_active`,
    /// this checks the exact token even after its TTL has elapsed.
    pub async fn backup_lock_token_present(&self, token: &str) -> Result<bool, DbError> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM maintenance_locks WHERE name = 'backup' AND token = ?)",
        )
        .bind(token)
        .fetch_one(&self.pool)
        .await?
            != 0)
    }

    pub async fn online_backup_to(&self, destination: &std::path::Path) -> Result<(), DbError> {
        if tokio::fs::try_exists(destination)
            .await
            .map_err(sqlx::Error::Io)?
        {
            return Err(DbError::ResultInvariant(
                "backup database destination already exists".to_owned(),
            ));
        }
        let destination = destination
            .to_str()
            .ok_or_else(|| DbError::ResultInvariant("backup path is not UTF-8".to_owned()))?;
        sqlx::query("VACUUM INTO ?")
            .bind(destination)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
