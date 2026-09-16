use crate::config::ServerConfig;
use crate::identity::normalized_username;
use crate::model::{NewSubmission, SubmissionLifecycle, WorkerJob, now_epoch_ms};
use robin_run_protocol::{Digest32, OpaqueId, VerifiedAchievementEvaluationV1};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous,
};
use sqlx::{QueryBuilder, Row as _, Sqlite, SqlitePool};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
mod acceptance;
mod diagnostics;
pub use diagnostics::{DiagnosticPayload, DiagnosticSummary};
mod maintenance;
mod public_queries;
mod snapshot;
mod uploads;
mod worker;

pub use snapshot::{applied_schema_version, snapshot_database};

pub const CURRENT_SCHEMA_VERSION: i64 = robin_run_protocol::HIGHSCORES_DATABASE_SCHEMA_VERSION;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("database migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("system clock is before the Unix epoch")]
    Clock(#[from] std::time::SystemTimeError),
    #[error("not found")]
    NotFound,
    #[error("submission queue is full")]
    QueueFull,
    #[error("new submission storage admission is temporarily unavailable")]
    AdmissionUnavailable,
    #[error("upload reservation belongs to different immutable content")]
    SubmissionConflict,
    #[error("replay is already pending or verified")]
    DuplicateReplay,
    #[error("uploader already holds the maximum number of concurrent upload leases")]
    UploadConcurrencyLimit,
    #[error("upload lease expired or was replaced before the upload completed")]
    UploadLeaseLost,
    #[error("username update is not newer than the last accepted update")]
    UsernameUpdateSuperseded,
    #[error("worker does not hold the current submission lease")]
    LeaseLost,
    #[error("verifier result violates admission invariants: {0}")]
    ResultInvariant(String),
    #[error("stored database value is invalid: {0}")]
    Corrupt(String),
}

impl DbError {
    /// Stable, non-sensitive classification for operational logs. Error
    /// messages can contain SQLite details or verifier-derived diagnostics and
    /// must never be emitted by the public service or worker.
    pub const fn safe_log_code(&self) -> &'static str {
        match self {
            Self::Sql(_) => "database_io",
            Self::Migration(_) => "database_migration",
            Self::Clock(_) => "system_clock",
            Self::NotFound => "not_found",
            Self::QueueFull => "queue_full",
            Self::AdmissionUnavailable => "storage_admission_unavailable",
            Self::SubmissionConflict => "submission_conflict",
            Self::DuplicateReplay => "duplicate_replay",
            Self::UploadConcurrencyLimit => "upload_concurrency_limit",
            Self::UploadLeaseLost => "upload_lease_lost",
            Self::UsernameUpdateSuperseded => "username_update_superseded",
            Self::LeaseLost => "lease_lost",
            Self::ResultInvariant(_) => "result_invariant",
            Self::Corrupt(_) => "stored_data_corrupt",
        }
    }
}

/// Typed database operations. Raw SQL is not part of the production interface.
///
/// ```compile_fail,E0599
/// fn bypass_fencing(database: &robin_highscores::Database) {
///     let _ = database.pool();
/// }
/// ```
#[cfg_attr(
    not(feature = "test-support"),
    doc = "
The explicitly named fixture accessor is also absent without `test-support`:

```compile_fail,E0599
fn bypass_fencing(database: &robin_highscores::Database) {
    let _ = database.fixture_pool();
}
```

The pool itself remains private:

```compile_fail,E0616
fn bypass_fencing(database: &robin_highscores::Database) {
    let _ = &database.pool;
}
```
"
)]
#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
    max_pending_submissions: u32,
    max_concurrent_sensitive_writers: u64,
    max_concurrent_upload_writers: u64,
    max_concurrent_uploads_per_key: u32,
    /// Keep the exact parent, database inode and live WAL/SHM inodes pinned for
    /// the entire pool lifetime. The pool connects through the retained main
    /// file descriptor, never through the mutable configured pathname.
    _database_parent: Arc<cap_std::fs::Dir>,
    _database_file: Arc<std::fs::File>,
    _database_sidecars: Arc<Vec<std::fs::File>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintenanceWriteClass {
    ApiSensitive,
    ApiUpload,
    ApiMaintenance,
    Worker,
    Admin,
}

impl MaintenanceWriteClass {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ApiSensitive => "api_sensitive",
            Self::ApiUpload => "api_upload",
            Self::ApiMaintenance => "api_maintenance",
            Self::Worker => "worker",
            Self::Admin => "admin",
        }
    }
}

/// Immutable storage projection of one authenticated upload. Serializable
/// data, not proof of authentication on its own.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionUploadIntent {
    pub proposed_submission_id: String,
    /// Canonical JSON of the exact signed submission.
    pub signed_request_json: String,
    pub uploader_public_key: [u8; 32],
    pub replay_sha256: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionUploadLease {
    pub submission_id: String,
    pub lease_token: String,
    pub lease_expires_at_ms: u64,
    pub reservation_expires_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum SubmissionUploadReservation {
    Acquired {
        lease: SubmissionUploadLease,
        resume_uploaded: bool,
    },
    Existing {
        lifecycle: SubmissionLifecycle,
    },
    Busy {
        retry_after_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicIdentity {
    pub public_key: [u8; 32],
    pub username: String,
}

/// Configured boards included in one leaderboard query.
#[derive(Debug, Clone)]
pub struct BoardQuery<'a> {
    pub board_ids: &'a [String],
    pub mission_id: &'a str,
    pub metric: robin_run_protocol::BoardMetricV1,
    pub max_concurrent_players: Option<u16>,
    pub player_public_key: Option<[u8; 32]>,
}

#[derive(Debug, Clone)]
pub struct BoardRow {
    pub rank: u64,
    pub run_id: String,
    pub metric_value: i64,
    pub original_score_delta: i64,
    pub active_simulation_ticks: u64,
    pub ransom_collected: u64,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u16,
    pub uploader: Option<PublicIdentity>,
    pub replay_sha256: [u8; 32],
    pub accepted_sequence: u64,
    pub verified_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct BoardCursor {
    pub metric_value: i64,
    pub accepted_sequence: i64,
    pub run_id: String,
}

#[derive(Debug, Clone)]
pub struct PlayerHistoryRecord {
    pub run_id: String,
    pub board_id: String,
    pub mission_id: String,
    pub original_score_delta: i64,
    pub active_simulation_ticks: u64,
    pub ransom_collected: u64,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u16,
    pub accepted_sequence: u64,
    pub verified_at_ms: u64,
    pub uploader: Option<PublicIdentity>,
}

#[derive(Debug, Clone)]
pub struct PlayerBestRecord {
    pub run_id: String,
    pub board_id: String,
    pub mission_id: String,
    pub metric: String,
    pub value: i64,
    pub max_concurrent_players: u16,
}

#[derive(Debug, Clone)]
pub struct PublicRunRecord {
    pub run_id: String,
    pub board_id: String,
    pub mission_id: String,
    pub recorded_engine_version: String,
    pub sim_config_json: String,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u16,
    pub starting_campaign_score: i32,
    pub final_campaign_score: i32,
    pub original_score_delta: i64,
    pub active_simulation_ticks: u64,
    pub ransom_collected: u64,
    pub verified_at_ms: u64,
    pub replay_sha256: [u8; 32],
    pub replay_bytes: u64,
    pub replay_schema_version: u32,
    pub uploader: Option<PublicIdentity>,
    pub achievements: Vec<(String, VerifiedAchievementEvaluationV1)>,
}

#[derive(Debug, Clone)]
pub struct DeletionRecord {
    pub id: String,
    pub tombstoned_at_ms: u64,
    pub purge_eligible_at_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ReplayGcCandidate {
    pub sha256: [u8; 32],
    pub byte_length: u64,
    pub claim_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModerationReportRecord {
    pub id: String,
    pub target_kind: String,
    pub target_id: String,
    pub category: String,
    pub detail: String,
    pub received_at_ms: u64,
    pub moderation_state: String,
    pub moderator_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModerationAuditRecord {
    pub id: u64,
    pub report_id: Option<String>,
    pub action: String,
    pub previous_state: Option<String>,
    pub new_state: Option<String>,
    pub detail: String,
    pub operator_id: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalCounts {
    pub queued_submissions: u64,
    pub active_upload_reservations: u64,
    pub abandoned_upload_reservations: u64,
    pub accepted_runs: u64,
    pub rejected_retained_submissions: u64,
    pub failed_retained_submissions: u64,
    pub open_abuse_reports: u64,
    pub replay_objects_live: u64,
    pub replay_objects_purging: u64,
}

impl Database {
    pub(crate) fn storage_volume(
        &self,
    ) -> Result<crate::storage_admission::StorageVolume, std::io::Error> {
        crate::storage_admission::StorageVolume::from_pinned_dir("database", &self._database_parent)
    }

    /// Open an already-migrated production database. Serving and worker
    /// processes deliberately never change the schema on startup.
    pub async fn connect(config: &ServerConfig) -> Result<Self, DbError> {
        Self::connect_inner(config, false).await
    }

    /// Explicit administrative migration entry point. This is intentionally
    /// not called by either long-running binary.
    pub async fn migrate(config: &ServerConfig) -> Result<Self, DbError> {
        Self::connect_inner(config, true).await
    }

    async fn connect_inner(config: &ServerConfig, migrate: bool) -> Result<Self, DbError> {
        let parent = config
            .database_path
            .parent()
            .ok_or_else(|| DbError::Corrupt("database path has no parent directory".to_owned()))?;
        let leaf = config
            .database_path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| DbError::Corrupt("database filename is not valid UTF-8".to_owned()))?
            .to_owned();
        {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(sqlx::Error::Io)?;
            let metadata = tokio::fs::symlink_metadata(parent)
                .await
                .map_err(sqlx::Error::Io)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(DbError::Corrupt(
                    "database directory must be a real directory".to_owned(),
                ));
            }
            set_private_permissions(parent, true).await?;
        }
        let pinned_parent_path = parent.to_owned();
        let database_parent = Arc::new(
            tokio::task::spawn_blocking(move || {
                crate::secure_fs::pin_private_root(&pinned_parent_path)
            })
            .await
            .map_err(|error| sqlx::Error::Io(std::io::Error::other(error)))?
            .map_err(sqlx::Error::Io)?,
        );
        let open_parent = Arc::clone(&database_parent);
        let open_leaf = leaf.clone();
        let database_file = Arc::new(
            tokio::task::spawn_blocking(move || {
                crate::secure_fs::open_private_database_file(
                    &open_parent,
                    std::path::Path::new(&open_leaf),
                    migrate,
                )
            })
            .await
            .map_err(|error| sqlx::Error::Io(std::io::Error::other(error)))?
            .map_err(sqlx::Error::Io)?,
        );
        let database_open_path = {
            use std::os::fd::AsRawFd as _;
            PathBuf::from(format!("/proc/self/fd/{}", database_file.as_raw_fd()))
        };
        let options = SqliteConnectOptions::new()
            .filename(database_open_path)
            .create_if_missing(false)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_millis(config.database_busy_timeout_ms));
        let pool = SqlitePoolOptions::new()
            .max_connections(16)
            .min_connections(0)
            .max_lifetime(None)
            .idle_timeout(None)
            .connect_with(options)
            .await?;
        if migrate {
            run_migrations(&pool).await?;
        } else {
            ensure_schema_current(&pool).await?;
        }
        verify_pinned_database_leaf(&database_parent, &leaf, &database_file).await?;
        let sidecars = pin_database_sidecars(&database_parent, &leaf).await?;
        Ok(Self {
            pool,
            max_pending_submissions: config.max_pending_submissions,
            max_concurrent_sensitive_writers: u64::try_from(config.max_concurrent_requests)
                .map_err(|_| {
                    DbError::ResultInvariant("sensitive writer limit overflows".to_owned())
                })?,
            max_concurrent_upload_writers: u64::try_from(config.max_concurrent_uploads).map_err(
                |_| DbError::ResultInvariant("upload writer limit overflows".to_owned()),
            )?,
            max_concurrent_uploads_per_key: config.max_concurrent_uploads_per_key,
            _database_parent: database_parent,
            _database_file: database_file,
            _database_sidecars: Arc::new(sidecars),
        })
    }

    /// Close the pool, waiting for checked-out connections to be returned.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub async fn health_check(&self) -> Result<(), DbError> {
        let value: i64 = sqlx::query_scalar("SELECT 1").fetch_one(&self.pool).await?;
        if value != 1 {
            return Err(DbError::Corrupt(
                "health query returned a non-one value".to_owned(),
            ));
        }
        Ok(())
    }

    pub async fn operational_counts(&self) -> Result<OperationalCounts, DbError> {
        let now = now_epoch_ms()?;
        let row = sqlx::query(
            "SELECT \
                (SELECT COUNT(*) FROM submissions WHERE status IN ('queued', 'verifying', 'retry_pending') AND tombstoned_at_ms IS NULL) AS queued_submissions, \
                (SELECT COUNT(*) FROM submission_upload_reservations WHERE state IN ('reserved','uploaded') AND reservation_expires_at_ms >= ?) AS active_upload_reservations, \
                (SELECT COUNT(*) FROM submission_upload_reservations WHERE state = 'abandoned' AND reservation_expires_at_ms >= ?) AS abandoned_upload_reservations, \
                (SELECT COUNT(*) FROM submissions WHERE status = 'accepted' AND tombstoned_at_ms IS NULL) AS accepted_runs, \
                (SELECT COUNT(*) FROM submissions WHERE status = 'rejected' AND tombstoned_at_ms IS NULL) AS rejected_retained_submissions, \
                (SELECT COUNT(*) FROM submissions WHERE status = 'failed' AND tombstoned_at_ms IS NULL) AS failed_retained_submissions, \
                (SELECT COUNT(*) FROM abuse_reports WHERE moderation_state IN ('open','reviewing')) AS open_abuse_reports, \
                (SELECT COUNT(*) FROM replay_objects WHERE purge_state = 'live') AS replay_objects_live, \
                (SELECT COUNT(*) FROM replay_objects WHERE purge_state = 'purging') AS replay_objects_purging",
        )
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        let count = |column: &str| -> Result<u64, DbError> {
            nonnegative_u64(row.try_get(column)?, column)
        };
        Ok(OperationalCounts {
            queued_submissions: count("queued_submissions")?,
            active_upload_reservations: count("active_upload_reservations")?,
            abandoned_upload_reservations: count("abandoned_upload_reservations")?,
            accepted_runs: count("accepted_runs")?,
            rejected_retained_submissions: count("rejected_retained_submissions")?,
            failed_retained_submissions: count("failed_retained_submissions")?,
            open_abuse_reports: count("open_abuse_reports")?,
            replay_objects_live: count("replay_objects_live")?,
            replay_objects_purging: count("replay_objects_purging")?,
        })
    }

    pub async fn accepted_sequence_watermark(&self) -> Result<u64, DbError> {
        let sequence: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(sequence), 0) FROM acceptance_sequences")
                .fetch_one(&self.pool)
                .await?;
        nonnegative_u64(sequence, "accepted_sequence_watermark")
    }

    pub async fn leaderboard_visibility_revision(&self) -> Result<u64, DbError> {
        let sequence: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence), 0) FROM leaderboard_visibility_events",
        )
        .fetch_one(&self.pool)
        .await?;
        nonnegative_u64(sequence, "leaderboard_visibility_revision")
    }

    /// Lifecycle of a live submission uploaded by `uploader_public_key`.
    /// Unknown, deleted and differently owned submissions are all `NotFound`,
    /// so a caller cannot distinguish them.
    pub async fn owner_submission_lifecycle(
        &self,
        uploader_public_key: [u8; 32],
        submission_id: &str,
    ) -> Result<SubmissionLifecycle, DbError> {
        let row = sqlx::query(
            "SELECT s.id, s.status, s.rejection_code, s.created_at_ms, s.updated_at_ms, \
                    r.id AS run_id \
             FROM submissions s LEFT JOIN verified_runs r ON r.submission_id = s.id \
             WHERE s.id = ? AND s.uploader_public_key = ? AND s.tombstoned_at_ms IS NULL",
        )
        .bind(submission_id)
        .bind(uploader_public_key.as_slice())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DbError::NotFound)?;
        lifecycle_from_row(row)
    }

    /// Tombstone an owned submission or run. Idempotent per owner and target:
    /// a repeated request (including a replay of the same signed request)
    /// returns the stored receipt without any further effect. A target that
    /// is already tombstoned for another reason is recorded with its existing
    /// tombstone.
    pub async fn apply_deletion(
        &self,
        public_key: [u8; 32],
        signed_at_unix_ms: u64,
        target_kind: &str,
        target_id: &str,
        request_json: &str,
        retention: Option<Duration>,
    ) -> Result<DeletionRecord, DbError> {
        let now = now_epoch_ms()?;
        let signed_at = i64::try_from(signed_at_unix_ms).map_err(|_| {
            DbError::ResultInvariant("signed timestamp does not fit SQLite INTEGER".to_owned())
        })?;
        let purge = retention
            .map(|duration| {
                i64::try_from(duration.as_millis())
                    .ok()
                    .and_then(|millis| now.checked_add(millis))
                    .ok_or_else(|| DbError::ResultInvariant("retention overflow".to_owned()))
            })
            .transpose()?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let existing = sqlx::query(
            "SELECT id, tombstoned_at_ms, purge_eligible_at_ms FROM deletion_requests \
             WHERE owner_public_key = ? AND target_kind = ? AND target_id = ?",
        )
        .bind(public_key.as_slice())
        .bind(target_kind)
        .bind(target_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(existing) = existing {
            tx.commit().await?;
            return deletion_record(
                existing.try_get("id")?,
                existing.try_get("tombstoned_at_ms")?,
                existing.try_get("purge_eligible_at_ms")?,
            );
        }
        let submission_id: Option<String> = match target_kind {
            "submission" => {
                sqlx::query_scalar(
                    "SELECT id FROM submissions WHERE id = ? AND uploader_public_key = ?",
                )
                .bind(target_id)
                .bind(public_key.as_slice())
                .fetch_optional(&mut *tx)
                .await?
            }
            "run" => {
                sqlx::query_scalar(
                    "SELECT r.submission_id FROM verified_runs r \
                     JOIN submissions s ON s.id = r.submission_id \
                     WHERE r.id = ? AND s.uploader_public_key = ?",
                )
                .bind(target_id)
                .bind(public_key.as_slice())
                .fetch_optional(&mut *tx)
                .await?
            }
            _ => {
                return Err(DbError::ResultInvariant(
                    "invalid deletion target".to_owned(),
                ));
            }
        };
        let submission_id = submission_id.ok_or(DbError::NotFound)?;
        let prior_tombstone = sqlx::query(
            "SELECT tombstoned_at_ms, purge_eligible_at_ms FROM submissions WHERE id = ?",
        )
        .bind(&submission_id)
        .fetch_one(&mut *tx)
        .await?;
        let (tombstoned_at, purge_eligible_at) = match prior_tombstone
            .try_get::<Option<i64>, _>("tombstoned_at_ms")?
        {
            Some(tombstoned_at) => (
                tombstoned_at,
                prior_tombstone.try_get::<Option<i64>, _>("purge_eligible_at_ms")?,
            ),
            None => {
                let was_public: i64 = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM verified_runs WHERE submission_id = ?)",
                )
                .bind(&submission_id)
                .fetch_one(&mut *tx)
                .await?;
                let changed = sqlx::query(
                    "UPDATE submissions SET tombstoned_at_ms = ?, purge_eligible_at_ms = ?, \
                        lease_owner = NULL, lease_expires_at_ms = NULL, \
                        status = CASE WHEN status = 'verifying' THEN 'retry_pending' ELSE status END, \
                        updated_at_ms = ? WHERE id = ? AND tombstoned_at_ms IS NULL",
                )
                .bind(now)
                .bind(purge)
                .bind(now)
                .bind(&submission_id)
                .execute(&mut *tx)
                .await?;
                if changed.rows_affected() != 1 {
                    return Err(DbError::Corrupt(
                        "untombstoned submission changed inside its deletion transaction"
                            .to_owned(),
                    ));
                }
                if was_public != 0 {
                    sqlx::query(
                        "INSERT INTO leaderboard_visibility_events (created_at_ms) VALUES (?)",
                    )
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                }
                (now, purge)
            }
        };
        let deletion_id = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO deletion_requests (id, owner_public_key, target_kind, target_id, \
                request_json, signed_at_unix_ms, tombstoned_at_ms, purge_eligible_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&deletion_id)
        .bind(public_key.as_slice())
        .bind(target_kind)
        .bind(target_id)
        .bind(request_json)
        .bind(signed_at)
        .bind(tombstoned_at)
        .bind(purge_eligible_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        deletion_record(deletion_id, tombstoned_at, purge_eligible_at)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_abuse_report(
        &self,
        target_kind: &str,
        target_id: &str,
        visible_board_ids: &[String],
        category: &str,
        detail: &str,
        reporter_ip_hash: [u8; 32],
        per_ip_limit: u32,
        per_key_limit: u32,
        per_target_limit: u32,
    ) -> Result<(String, u64), DbError> {
        let now = now_epoch_ms()?;
        let one_hour_ago = now.saturating_sub(60 * 60 * 1000);
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let quota_public_key: [u8; 32] = match target_kind {
            "run" => {
                let mut query = QueryBuilder::<Sqlite>::new(
                    "SELECT submission.uploader_public_key FROM verified_runs run \
                     JOIN submissions submission ON submission.id = run.submission_id \
                     WHERE submission.status = 'accepted' AND submission.tombstoned_at_ms IS NULL \
                       AND run.id = ",
                );
                query.push_bind(target_id);
                query.push(" AND ");
                push_text_filter(&mut query, "run.board_id", visible_board_ids);
                let key: Option<Vec<u8>> =
                    query.build_query_scalar().fetch_optional(&mut *tx).await?;
                fixed_32(key.ok_or(DbError::NotFound)?)?
            }
            "player" => {
                let key = hex::decode(target_id).map_err(|_| DbError::NotFound)?;
                let key = fixed_32(key)?;
                let exists: i64 = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM identities WHERE public_key = ?)",
                )
                .bind(key.as_slice())
                .fetch_one(&mut *tx)
                .await?;
                if exists == 0 {
                    return Err(DbError::NotFound);
                }
                key
            }
            _ => return Err(DbError::ResultInvariant("invalid report target".to_owned())),
        };
        let global: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM abuse_reports WHERE received_at_ms >= ?")
                .bind(one_hour_ago)
                .fetch_one(&mut *tx)
                .await?;
        let target_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM abuse_reports WHERE received_at_ms >= ? \
             AND target_kind = ? AND target_id = ?",
        )
        .bind(one_hour_ago)
        .bind(target_kind)
        .bind(target_id)
        .fetch_one(&mut *tx)
        .await?;
        let ip_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM abuse_reports WHERE received_at_ms >= ? \
             AND reporter_ip_hash = ?",
        )
        .bind(one_hour_ago)
        .bind(reporter_ip_hash.as_slice())
        .fetch_one(&mut *tx)
        .await?;
        let key_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM abuse_reports WHERE received_at_ms >= ? \
             AND quota_public_key = ?",
        )
        .bind(one_hour_ago)
        .bind(quota_public_key.as_slice())
        .fetch_one(&mut *tx)
        .await?;
        if global >= 1_000
            || target_count >= i64::from(per_target_limit)
            || ip_count >= i64::from(per_ip_limit)
            || key_count >= i64::from(per_key_limit)
        {
            return Err(DbError::QueueFull);
        }
        let id = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO abuse_reports \
             (id, target_kind, target_id, category, detail, received_at_ms, \
              reporter_ip_hash, quota_public_key, updated_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(target_kind)
        .bind(target_id)
        .bind(category)
        .bind(detail)
        .bind(now)
        .bind(reporter_ip_hash.as_slice())
        .bind(quota_public_key.as_slice())
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((
            id,
            u64::try_from(now)
                .map_err(|_| DbError::Corrupt("negative report timestamp".to_owned()))?,
        ))
    }

    pub async fn moderation_reports(
        &self,
        state: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ModerationReportRecord>, DbError> {
        if limit == 0 || limit > 500 {
            return Err(DbError::ResultInvariant(
                "moderation report limit must be in 1..=500".to_owned(),
            ));
        }
        if state
            .is_some_and(|value| !matches!(value, "open" | "reviewing" | "dismissed" | "actioned"))
        {
            return Err(DbError::ResultInvariant(
                "invalid moderation state".to_owned(),
            ));
        }
        let rows = sqlx::query(
            "SELECT id, target_kind, target_id, category, detail, received_at_ms, \
                    moderation_state, moderator_note FROM abuse_reports \
             WHERE (? IS NULL OR moderation_state = ?) \
             ORDER BY received_at_ms, id LIMIT ?",
        )
        .bind(state)
        .bind(state)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ModerationReportRecord {
                    id: row.try_get("id")?,
                    target_kind: row.try_get("target_kind")?,
                    target_id: row.try_get("target_id")?,
                    category: row.try_get("category")?,
                    detail: row.try_get("detail")?,
                    received_at_ms: nonnegative_u64(
                        row.try_get("received_at_ms")?,
                        "received_at_ms",
                    )?,
                    moderation_state: row.try_get("moderation_state")?,
                    moderator_note: row.try_get("moderator_note")?,
                })
            })
            .collect()
    }

    pub async fn moderate_report(
        &self,
        report_id: &str,
        new_state: &str,
        detail: &str,
        operator_id: &str,
    ) -> Result<(), DbError> {
        if !matches!(new_state, "reviewing" | "dismissed" | "actioned")
            || detail.is_empty()
            || detail.len() > 4_000
            || operator_id.is_empty()
            || operator_id.len() > 128
        {
            return Err(DbError::ResultInvariant(
                "invalid moderation action".to_owned(),
            ));
        }
        let now = now_epoch_ms()?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let previous: String =
            sqlx::query_scalar("SELECT moderation_state FROM abuse_reports WHERE id = ?")
                .bind(report_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(DbError::NotFound)?;
        if matches!(previous.as_str(), "dismissed" | "actioned") && previous != new_state {
            return Err(DbError::ResultInvariant(
                "closed moderation reports cannot be reopened or rewritten".to_owned(),
            ));
        }
        sqlx::query(
            "UPDATE abuse_reports SET moderation_state = ?, moderator_note = ?, updated_at_ms = ? \
             WHERE id = ?",
        )
        .bind(new_state)
        .bind(detail)
        .bind(now)
        .bind(report_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO moderation_audit \
             (report_id, action, previous_state, new_state, detail, operator_id, created_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(report_id)
        .bind(new_state)
        .bind(previous)
        .bind(new_state)
        .bind(detail)
        .bind(operator_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn moderation_audit(
        &self,
        report_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ModerationAuditRecord>, DbError> {
        if limit == 0 || limit > 500 {
            return Err(DbError::ResultInvariant(
                "moderation audit limit must be in 1..=500".to_owned(),
            ));
        }
        let rows = sqlx::query(
            "SELECT id, report_id, action, previous_state, new_state, detail, operator_id, \
                    created_at_ms FROM moderation_audit WHERE (? IS NULL OR report_id = ?) \
             ORDER BY id DESC LIMIT ?",
        )
        .bind(report_id)
        .bind(report_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ModerationAuditRecord {
                    id: nonnegative_u64(row.try_get("id")?, "moderation audit id")?,
                    report_id: row.try_get("report_id")?,
                    action: row.try_get("action")?,
                    previous_state: row.try_get("previous_state")?,
                    new_state: row.try_get("new_state")?,
                    detail: row.try_get("detail")?,
                    operator_id: row.try_get("operator_id")?,
                    created_at_ms: nonnegative_u64(row.try_get("created_at_ms")?, "created_at_ms")?,
                })
            })
            .collect()
    }

    /// Apply an authenticated username update signed at `signed_at_unix_ms`.
    /// The update must be signed strictly later than the last accepted one for
    /// the key, so replaying an older (or the same) signed update within its
    /// freshness window cannot roll a newer name back.
    pub async fn apply_username_update(
        &self,
        public_key: [u8; 32],
        signed_at_unix_ms: u64,
        username: &str,
    ) -> Result<(), DbError> {
        let now = now_epoch_ms()?;
        let signed_at = i64::try_from(signed_at_unix_ms).map_err(|_| {
            DbError::ResultInvariant("signed timestamp does not fit SQLite INTEGER".to_owned())
        })?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let prior = sqlx::query(
            "SELECT username, username_generation, username_signed_at_unix_ms \
             FROM identities WHERE public_key = ?",
        )
        .bind(public_key.as_slice())
        .fetch_optional(&mut *tx)
        .await?;
        let generation = match &prior {
            Some(row) => {
                if row.try_get::<i64, _>("username_signed_at_unix_ms")? >= signed_at {
                    return Err(DbError::UsernameUpdateSuperseded);
                }
                row.try_get::<i64, _>("username_generation")?
                    .checked_add(1)
                    .ok_or_else(|| DbError::Corrupt("username generation overflow".to_owned()))?
            }
            None => 1,
        };
        let previous_username = prior
            .as_ref()
            .map(|row| row.try_get::<String, _>("username"))
            .transpose()?;
        sqlx::query(
            "INSERT INTO identities \
             (public_key, username, username_normalized, username_generation, \
              username_signed_at_unix_ms, created_at_ms, updated_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(public_key) DO UPDATE SET \
                 username = excluded.username, \
                 username_normalized = excluded.username_normalized, \
                 username_generation = excluded.username_generation, \
                 username_signed_at_unix_ms = excluded.username_signed_at_unix_ms, \
                 updated_at_ms = excluded.updated_at_ms",
        )
        .bind(public_key.as_slice())
        .bind(username)
        .bind(normalized_username(username))
        .bind(generation)
        .bind(signed_at)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO username_history \
             (public_key, generation, signed_at_unix_ms, previous_username, new_username, \
              changed_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(public_key.as_slice())
        .bind(generation)
        .bind(signed_at)
        .bind(previous_username)
        .bind(username)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn identity_exists(&self, public_key: &[u8; 32]) -> Result<bool, DbError> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM identities WHERE public_key = ?)",
        )
        .bind(public_key.as_slice())
        .fetch_one(&self.pool)
        .await?
            != 0)
    }

    pub async fn public_identity(&self, public_key: &[u8; 32]) -> Result<PublicIdentity, DbError> {
        let row = sqlx::query("SELECT username FROM identities WHERE public_key = ?")
            .bind(public_key.as_slice())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        Ok(PublicIdentity {
            public_key: *public_key,
            username: row.try_get("username")?,
        })
    }

    pub async fn submission_lifecycle(&self, id: &str) -> Result<SubmissionLifecycle, DbError> {
        let row = sqlx::query(
            "SELECT s.id, s.status, s.rejection_code, s.created_at_ms, s.updated_at_ms, \
                    r.id AS run_id \
             FROM submissions s LEFT JOIN verified_runs r ON r.submission_id = s.id \
             WHERE s.id = ? AND s.tombstoned_at_ms IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DbError::NotFound)?;
        lifecycle_from_row(row)
    }
}

// Library unit fixtures use cfg(test); cross-crate integration and binary
// fixtures must explicitly opt in. Normal server/worker/admin builds expose no
// raw pool accessor and retain the narrow operations and shutdown API.
#[cfg(any(test, feature = "test-support"))]
impl Database {
    /// Raw access solely for corruption/concurrency fixtures.
    ///
    /// This bypasses the typed interface. Never enable `test-support` in deployments.
    pub fn fixture_pool(&self) -> &SqlitePool {
        &self.pool
    }
}

async fn recover_upload_reservations_in(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    now: i64,
) -> Result<u64, DbError> {
    let abandoned = sqlx::query(
        "UPDATE submission_upload_reservations \
         SET state = 'abandoned', lease_token = NULL, lease_expires_at_ms = NULL, \
             abandoned_at_ms = ?, updated_at_ms = ? \
         WHERE state = 'reserved' AND lease_expires_at_ms < ?",
    )
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    // Expired reservations are bounded durable state: remove them so an
    // abandoned upload cannot pin its replay hash forever.
    let expired = sqlx::query(
        "DELETE FROM submission_upload_reservations WHERE reservation_expires_at_ms < ?",
    )
    .bind(now)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    abandoned
        .checked_add(expired)
        .ok_or_else(|| DbError::Corrupt("upload recovery count overflow".to_owned()))
}

fn deletion_record(
    id: String,
    tombstoned_at_ms: i64,
    purge_eligible_at_ms: Option<i64>,
) -> Result<DeletionRecord, DbError> {
    Ok(DeletionRecord {
        id,
        tombstoned_at_ms: nonnegative_u64(tombstoned_at_ms, "tombstoned_at_ms")?,
        purge_eligible_at_ms: purge_eligible_at_ms
            .map(|value| nonnegative_u64(value, "purge_eligible_at_ms"))
            .transpose()?,
    })
}

async fn verify_pinned_database_leaf(
    parent: &Arc<cap_std::fs::Dir>,
    leaf: &str,
    pinned: &Arc<std::fs::File>,
) -> Result<(), DbError> {
    let opened_parent = Arc::clone(parent);
    let opened_leaf = leaf.to_owned();
    let current = tokio::task::spawn_blocking(move || -> std::io::Result<std::fs::File> {
        // Closing any ordinary descriptor for this inode would discard
        // SQLite's process-wide POSIX locks, even on another thread. An
        // O_PATH descriptor can authenticate the leaf without that close
        // side effect. Keep the same beneath/no-symlink path confinement.
        let fd = crate::secure_fs::open_beneath_no_symlinks(
            &*opened_parent,
            std::path::Path::new(&opened_leaf),
            rustix::fs::OFlags::PATH | rustix::fs::OFlags::CLOEXEC,
        )
        .map_err(std::io::Error::from)?;
        let file = std::fs::File::from(fd);
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other("database is not a regular file"));
        }
        Ok(file)
    })
    .await
    .map_err(|error| sqlx::Error::Io(std::io::Error::other(error)))?
    .map_err(sqlx::Error::Io)?;
    {
        use std::os::unix::fs::MetadataExt as _;
        let expected = pinned.metadata().map_err(sqlx::Error::Io)?;
        let actual = current.metadata().map_err(sqlx::Error::Io)?;
        if (actual.dev(), actual.ino()) != (expected.dev(), expected.ino()) {
            return Err(DbError::Corrupt(
                "database leaf changed while the connection pool was opening".to_owned(),
            ));
        }
    }
    Ok(())
}

/// Apply every reviewed, append-only migration.
async fn run_migrations(pool: &SqlitePool) -> Result<(), DbError> {
    let mut connection = pool.acquire().await?;
    MIGRATOR.run_direct(None, &mut *connection, false).await?;
    drop(connection);
    ensure_schema_current(pool).await
}

async fn ensure_schema_current(pool: &SqlitePool) -> Result<(), DbError> {
    let rows =
        sqlx::query("SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .map_err(|error| match error {
                sqlx::Error::Database(database) if database.message().contains("no such table") => {
                    sqlx::Error::Protocol(
                        "database is not migrated; run `robin-highscores-admin migrate`".to_owned(),
                    )
                }
                other => other,
            })?;
    let current = rows.last().map(|row| row.get::<i64, _>("version"));
    let checksums_match = rows
        .iter()
        .zip(MIGRATOR.migrations.iter())
        .all(|(row, migration)| {
            row.get::<i64, _>("version") == migration.version
                && row.get::<Vec<u8>, _>("checksum").as_slice() == migration.checksum.as_ref()
        });
    if rows.len() != usize::try_from(CURRENT_SCHEMA_VERSION).expect("small schema version")
        || current != Some(CURRENT_SCHEMA_VERSION)
        || rows.iter().any(|row| !row.get::<bool, _>("success"))
        || !checksums_match
    {
        return Err(DbError::Corrupt(format!(
            "database schema is not current (expected version {CURRENT_SCHEMA_VERSION}); run the explicit migration command"
        )));
    }
    Ok(())
}

async fn set_private_permissions(path: &std::path::Path, directory: bool) -> Result<(), DbError> {
    use rustix::fs::OFlags;
    use std::os::unix::fs::PermissionsExt as _;
    let mut flags = OFlags::RDONLY | OFlags::CLOEXEC;
    if directory {
        flags |= OFlags::DIRECTORY;
    }
    let fd = crate::secure_fs::open_ambient_no_symlinks(path, flags)
        .map_err(std::io::Error::from)
        .map_err(|error| DbError::Sql(sqlx::Error::Io(error)))?;
    let file = std::fs::File::from(fd);
    let metadata = file
        .metadata()
        .map_err(|error| DbError::Sql(sqlx::Error::Io(error)))?;
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err(DbError::Corrupt(format!(
            "database path has the wrong file type: {}",
            path.display()
        )));
    }
    let mode = if directory {
        crate::secure_fs::SHARED_PRIVATE_DIRECTORY_MODE
    } else {
        crate::secure_fs::SHARED_MUTABLE_FILE_MODE
    };
    if metadata.permissions().mode() & 0o7777 != mode {
        file.set_permissions(std::fs::Permissions::from_mode(mode))
            .map_err(|error| DbError::Sql(sqlx::Error::Io(error)))?;
    }
    Ok(())
}

async fn ensure_worker_lease(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    submission_id: &str,
    worker_id: &str,
    now: i64,
) -> Result<(), DbError> {
    let valid: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM submissions \
         WHERE id = ? AND tombstoned_at_ms IS NULL AND status = 'verifying' \
             AND lease_owner = ? AND lease_expires_at_ms >= ?)",
    )
    .bind(submission_id)
    .bind(worker_id)
    .bind(now)
    .fetch_one(&mut **tx)
    .await?;
    if valid == 0 {
        return Err(DbError::LeaseLost);
    }
    Ok(())
}

async fn insert_worker_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    submission_id: &str,
    kind: &str,
    worker_id: &str,
    detail: &str,
    now: i64,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO worker_events \
         (submission_id, kind, worker_id, detail, created_at_ms) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(submission_id)
    .bind(kind)
    .bind(worker_id)
    .bind(detail)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn lifecycle_from_row(row: SqliteRow) -> Result<SubmissionLifecycle, DbError> {
    Ok(SubmissionLifecycle {
        id: row.try_get("id")?,
        state: crate::model::SubmissionState::from_columns(
            row.try_get::<String, _>("status")?.as_str(),
            row.try_get("run_id")?,
            row.try_get("rejection_code")?,
        )
        .map_err(DbError::Corrupt)?,
        created_at_ms: nonnegative_u64(row.try_get("created_at_ms")?, "created_at_ms")?,
        updated_at_ms: nonnegative_u64(row.try_get("updated_at_ms")?, "updated_at_ms")?,
    })
}

fn fixed_32(bytes: Vec<u8>) -> Result<[u8; 32], DbError> {
    bytes
        .try_into()
        .map_err(|_| DbError::Corrupt("expected a 32-byte digest".to_owned()))
}

/// `column IN (...)` over bound values; an empty allowlist matches nothing.
fn push_text_filter(query: &mut QueryBuilder<Sqlite>, column: &'static str, values: &[String]) {
    if values.is_empty() {
        query.push("0");
        return;
    }
    query.push(column).push(" IN (");
    let mut separated = query.separated(", ");
    for value in values {
        separated.push_bind(value.clone());
    }
    query.push(")");
}

fn nonnegative_u64(value: i64, field: &str) -> Result<u64, DbError> {
    u64::try_from(value).map_err(|_| DbError::Corrupt(format!("{field} is negative")))
}

/// Decode a SQLite INTEGER into a bounded public count. SQL type/column errors
/// remain SQLx errors instead of being disguised as an absent or zero count.
fn checked_count<T: TryFrom<i64>>(
    row: &SqliteRow,
    column: &str,
    corruption: &'static str,
) -> Result<T, DbError> {
    T::try_from(row.try_get::<i64, _>(column)?).map_err(|_| DbError::Corrupt(corruption.to_owned()))
}

/// Decode the optional named uploader of a run row selected with the
/// `public_disclosure`, `uploader_public_key` and joined `username` columns.
fn decode_uploader(row: &SqliteRow) -> Result<Option<PublicIdentity>, DbError> {
    let disclosure: String = row.try_get("public_disclosure")?;
    match disclosure.as_str() {
        "anonymous" => Ok(None),
        "named_profile" => Ok(Some(PublicIdentity {
            public_key: fixed_32(row.try_get("uploader_public_key")?)?,
            username: row
                .try_get::<Option<String>, _>("username")?
                .ok_or_else(|| {
                    DbError::Corrupt("named uploader has no registered identity".to_owned())
                })?,
        })),
        other => Err(DbError::Corrupt(format!(
            "invalid stored public disclosure `{other}`"
        ))),
    }
}

const fn achievement_evaluation_name(value: VerifiedAchievementEvaluationV1) -> &'static str {
    match value {
        VerifiedAchievementEvaluationV1::Unverifiable => "unverifiable",
        VerifiedAchievementEvaluationV1::NotEarned => "not_earned",
        VerifiedAchievementEvaluationV1::Earned => "earned",
    }
}

fn achievement_evaluation(value: &str) -> Result<VerifiedAchievementEvaluationV1, DbError> {
    match value {
        "unverifiable" => Ok(VerifiedAchievementEvaluationV1::Unverifiable),
        "not_earned" => Ok(VerifiedAchievementEvaluationV1::NotEarned),
        "earned" => Ok(VerifiedAchievementEvaluationV1::Earned),
        other => Err(DbError::Corrupt(format!(
            "unknown stored achievement evaluation `{other}`"
        ))),
    }
}

fn is_public_rejection_code(value: &str) -> bool {
    value
        .parse::<robin_run_protocol::VerificationRejectionCodeV1>()
        .is_ok()
}

fn canonical_json_string(value: &(impl Serialize + ?Sized)) -> Result<String, DbError> {
    String::from_utf8(
        robin_run_protocol::canonical_json_bytes(value)
            .map_err(|error| DbError::ResultInvariant(error.to_string()))?,
    )
    .map_err(|error| DbError::ResultInvariant(error.to_string()))
}

fn opaque_id(value: String) -> Result<OpaqueId, DbError> {
    OpaqueId::new(value).map_err(|error| DbError::Corrupt(error.to_string()))
}

fn digest_from_row(row: &SqliteRow, column: &str) -> Result<Digest32, DbError> {
    Ok(Digest32::from_bytes(fixed_32(row.try_get(column)?)?))
}

async fn pin_database_sidecars(
    database_parent: &Arc<cap_std::fs::Dir>,
    leaf: &str,
) -> Result<Vec<std::fs::File>, DbError> {
    let mut sidecars = Vec::new();
    for suffix in ["-wal", "-shm"] {
        let sidecar_name = format!("{leaf}{suffix}");
        let sidecar_parent = Arc::clone(database_parent);
        match tokio::task::spawn_blocking(move || {
            crate::secure_fs::open_regular_file(
                &sidecar_parent,
                std::path::Path::new(&sidecar_name),
            )
        })
        .await
        .map_err(|error| sqlx::Error::Io(std::io::Error::other(error)))?
        {
            Ok(file) => {
                use std::os::unix::fs::PermissionsExt as _;
                if file
                    .metadata()
                    .map_err(sqlx::Error::Io)?
                    .permissions()
                    .mode()
                    & 0o777
                    != crate::secure_fs::SHARED_MUTABLE_FILE_MODE
                {
                    file.set_permissions(std::fs::Permissions::from_mode(
                        crate::secure_fs::SHARED_MUTABLE_FILE_MODE,
                    ))
                    .map_err(sqlx::Error::Io)?;
                }
                sidecars.push(file);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(DbError::Sql(sqlx::Error::Io(error))),
        }
    }
    Ok(sidecars)
}

#[cfg(test)]
mod tests;
