use super::*;
use crate::test_support::{TestDeployment, demo_board, verified_output, verified_run};
use std::collections::BTreeSet;

const BOARD_ID: &str = "demo-standard-normal";
const MISSION_ID: &str = "Dem_Lei_MP";

async fn test_database() -> (tempfile::TempDir, Database) {
    let (directory, _config, database) = TestDeployment::new().migrate().await;
    (directory, database)
}

async fn register_test_identity(database: &Database, public_key: [u8; 32], username: &str) {
    let challenge = database
        .issue_challenge(
            ChallengePurpose::UsernameUpdate,
            public_key,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    database
        .apply_username_update(
            &challenge.id,
            challenge.nonce.into_bytes(),
            public_key,
            username,
        )
        .await
        .unwrap();
}

async fn insert_acceptance_sequence(database: &Database, created_at_ms: i64) {
    sqlx::query("INSERT INTO acceptance_sequences (created_at_ms) VALUES (?)")
        .bind(created_at_ms)
        .execute(database.fixture_pool())
        .await
        .unwrap();
}

struct SubmissionFixture {
    submission: NewSubmission,
    intent: SubmissionUploadIntent,
}

/// A registered uploader, a fresh submission challenge and the matching
/// upload projection for replay digest `[replay_byte; 32]`.
async fn submission_fixture(
    database: &Database,
    uploader: [u8; 32],
    replay_byte: u8,
) -> SubmissionFixture {
    if !database.identity_exists(&uploader).await.unwrap() {
        register_test_identity(database, uploader, "Robin").await;
    }
    let challenge = database
        .issue_challenge(
            ChallengePurpose::Submission,
            uploader,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    let id = uuid::Uuid::now_v7().to_string();
    let envelope_json = format!(
        "{{\"fixture\":{replay_byte},\"challenge\":\"{}\"}}",
        challenge.id
    );
    SubmissionFixture {
        submission: NewSubmission {
            id: id.clone(),
            upload_challenge_id: challenge.id.clone(),
            envelope_json: envelope_json.clone(),
            uploader_public_key: uploader,
            public_disclosure: "named_profile",
            board_id: BOARD_ID.to_owned(),
            mission_id: MISSION_ID.to_owned(),
            replay_sha256: [replay_byte; 32],
            replay_bytes: 4,
            replay_schema_version: robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            requested_metrics_json: "[\"original_score\",\"fastest_success\"]".to_owned(),
        },
        intent: SubmissionUploadIntent {
            proposed_submission_id: id,
            upload_challenge_id: challenge.id,
            upload_challenge_nonce: challenge.nonce.into_bytes(),
            upload_challenge_expires_at_ms: challenge.expires_at_ms,
            envelope_json,
            uploader_public_key: uploader,
            replay_sha256: [replay_byte; 32],
        },
    }
}

async fn acquire_upload_fixture(
    database: &Database,
    fixture: &SubmissionFixture,
) -> SubmissionUploadLease {
    match database
        .reserve_submission_upload(
            &fixture.intent,
            Duration::from_secs(30),
            Duration::from_secs(300),
        )
        .await
        .unwrap()
    {
        SubmissionUploadReservation::Acquired {
            lease,
            resume_uploaded: false,
        } => lease,
        other => panic!("unexpected test reservation result: {other:?}"),
    }
}

async fn insert_submission_fixture(
    database: &Database,
    fixture: &SubmissionFixture,
) -> SubmissionLifecycle {
    let lease = acquire_upload_fixture(database, fixture).await;
    database
        .mark_submission_upload_uploaded(&lease)
        .await
        .unwrap();
    database
        .finalize_submission_upload(&fixture.submission, &lease)
        .await
        .unwrap()
}

#[test]
fn issued_nonce_uses_protocol_hex_representation() {
    let issued = IssuedChallenge {
        id: "nonce-fixture".into(),
        nonce: robin_run_protocol::ChallengeNonce32::from_bytes([0xab; 32]),
        expires_at_ms: 123,
    };
    assert_eq!(
        serde_json::to_value(issued).unwrap(),
        serde_json::json!({
            "id": "nonce-fixture", "nonce": "ab".repeat(32), "expires_at_ms": 123,
        })
    );
}

#[tokio::test]
async fn count_columns_preserve_bounds_and_corruption_errors() {
    use sqlx::Connection;
    let mut connection = sqlx::SqliteConnection::connect(":memory:").await.unwrap();
    for value in [-1, 0, 65_535, 65_536, i64::MAX] {
        let row = sqlx::query("SELECT ? AS count")
            .bind(value)
            .fetch_one(&mut connection)
            .await
            .unwrap();
        let result = checked_count::<u16>(&row, "count", "count out of range");
        match u16::try_from(value) {
            Ok(expected) => assert_eq!(result.unwrap(), expected),
            Err(_) => assert!(
                matches!(result, Err(DbError::Corrupt(message)) if message == "count out of range")
            ),
        }
    }
}

#[tokio::test]
async fn text_filters_bind_values_and_reject_empty_allowlists() {
    use sqlx::Connection;
    let mut connection = sqlx::SqliteConnection::connect(":memory:").await.unwrap();
    for (allowed, expected) in [
        (vec![], 0_i64),
        (vec!["board-a".to_owned()], 1),
        (vec!["board-b".to_owned()], 0),
        (vec!["board-b".to_owned(), "board-a".to_owned()], 1),
    ] {
        let mut query = QueryBuilder::<Sqlite>::new(
            "WITH candidate AS (SELECT 'board-a' AS board_id) SELECT COUNT(*) FROM candidate WHERE ",
        );
        push_text_filter(&mut query, "candidate.board_id", &allowed);
        let actual: i64 = query
            .build_query_scalar()
            .fetch_one(&mut connection)
            .await
            .unwrap();
        assert_eq!(actual, expected);
    }
}

#[test]
#[ignore = "subprocess helper for the POSIX database lock regression"]
fn database_posix_lock_probe_child() {
    use rustix::fs::{FlockOperation, fcntl_lock};
    let path = std::env::var_os("ROBIN_TEST_POSIX_LOCK_PROBE_PATH").unwrap();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let error = fcntl_lock(&file, FlockOperation::NonBlockingLockExclusive)
        .expect_err("another process acquired the supposedly held database lock");
    assert!(matches!(
        error,
        rustix::io::Errno::AGAIN | rustix::io::Errno::ACCESS
    ));
}

#[tokio::test]
async fn database_leaf_identity_check_preserves_posix_locks() {
    use rustix::fs::{FlockOperation, fcntl_lock};

    let directory = tempfile::tempdir().unwrap();
    let parent = Arc::new(
        cap_std::fs::Dir::open_ambient_dir(directory.path(), cap_std::ambient_authority()).unwrap(),
    );
    let pinned = Arc::new(std::fs::File::create(directory.path().join("database")).unwrap());
    let holds_lock = || {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "db::tests::database_posix_lock_probe_child",
                "--ignored",
            ])
            .env(
                "ROBIN_TEST_POSIX_LOCK_PROBE_PATH",
                directory.path().join("database"),
            )
            .output()
            .unwrap();
        output.status.success()
    };
    fcntl_lock(&*pinned, FlockOperation::NonBlockingLockExclusive).unwrap();
    assert!(
        holds_lock(),
        "the test POSIX lock did not exclude another process"
    );
    verify_pinned_database_leaf(&parent, "database", &pinned)
        .await
        .unwrap();
    assert!(
        holds_lock(),
        "identity verification discarded the process's SQLite-style POSIX lock"
    );
    fcntl_lock(&*pinned, FlockOperation::Unlock).unwrap();

    std::fs::rename(
        directory.path().join("database"),
        directory.path().join("original"),
    )
    .unwrap();
    std::fs::write(directory.path().join("database"), b"replacement").unwrap();
    assert!(
        verify_pinned_database_leaf(&parent, "database", &pinned)
            .await
            .is_err()
    );
    std::fs::remove_file(directory.path().join("database")).unwrap();
    std::os::unix::fs::symlink("original", directory.path().join("database")).unwrap();
    assert!(
        verify_pinned_database_leaf(&parent, "database", &pinned)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn serving_connection_refuses_to_create_or_migrate_schema() {
    let directory = tempfile::tempdir().unwrap();
    let config = ServerConfig {
        database_path: directory.path().join("highscores.sqlite3"),
        ..Default::default()
    };
    assert!(Database::connect(&config).await.is_err());
    assert!(!config.database_path.exists());
    drop(Database::migrate(&config).await.unwrap());
    Database::connect(&config).await.unwrap();
}

#[test]
fn migration_chain_ends_with_the_ranked_protocol_v2_schema() {
    assert_eq!(CURRENT_SCHEMA_VERSION, 6);
    assert_eq!(MIGRATOR.migrations.len(), 6);
    assert_eq!(MIGRATOR.migrations[0].description.as_ref(), "initial");
    let v2 = &MIGRATOR.migrations[5];
    assert_eq!(v2.version, 6);
    assert_eq!(v2.description.as_ref(), "ranked protocol v2");
    assert!(
        !MIGRATOR.migrations[0]
            .sql
            .as_str()
            .contains("maintenance_write_leases"),
        "the immutable initial migration was rewritten"
    );
}

#[tokio::test]
async fn migrated_schema_drops_removed_concepts_and_keeps_security_indexes() {
    let (directory, database) = test_database().await;
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(database.fixture_pool())
        .await
        .unwrap();
    assert_eq!(journal_mode, "wal");
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(database.fixture_pool())
        .await
        .unwrap();
    assert_eq!(foreign_keys, 1);
    let objects = sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_master WHERE type IN ('table', 'view')",
    )
    .fetch_all(database.fixture_pool())
    .await
    .unwrap()
    .into_iter()
    .collect::<BTreeSet<_>>();
    for removed in [
        "campaign_objects",
        "competition_run_grants",
        "full_campaign_runs",
        "full_campaign_sessions",
        "full_campaign_participants",
        "full_campaign_metrics",
        "submission_campaign_objects",
        "verified_run_campaign_objects",
        "campaign_object_submission_references",
        "used_replay_session_geneses",
        "submission_participants",
        "submission_terminal_failures",
    ] {
        assert!(
            !objects.contains(removed),
            "schema still contains {removed}"
        );
    }
    for kept in [
        "identities",
        "username_history",
        "diagnostic_reports",
        "abuse_reports",
        "moderation_audit",
        "deletion_requests",
        "maintenance_write_leases",
        "replay_objects",
        "submissions",
        "verified_runs",
        "verified_run_metrics",
        "verified_run_achievements",
        "submission_upload_reservations",
    ] {
        assert!(objects.contains(kept), "schema lost {kept}");
    }
    let index: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'submissions_live_replay_idx'",
    )
    .fetch_one(database.fixture_pool())
    .await
    .unwrap();
    assert!(index.contains("UNIQUE") && index.contains("status = 'accepted'"));
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(directory.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o770
        );
    }
}

#[tokio::test]
async fn protocol_v2_migration_preserves_identities_and_diagnostics() {
    use sqlx::Connection as _;
    let mut connection = sqlx::SqliteConnection::connect("sqlite::memory:")
        .await
        .unwrap();
    for migration in &MIGRATOR.migrations[..5] {
        sqlx::raw_sql(migration.sql.as_str())
            .execute(&mut connection)
            .await
            .unwrap();
    }
    sqlx::query(
        "INSERT INTO identities (public_key, username, username_normalized, username_generation, \
             created_at_ms, updated_at_ms) VALUES (?, 'Robin', 'robin', 1, 1, 1)",
    )
    .bind([7_u8; 32].as_slice())
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO diagnostic_reports VALUES ('report', 1, ?, x'00', 1, 'zstd', 'bug', 'test')",
    )
    .bind([1_u8; 32].as_slice())
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO upload_challenges (id, nonce, purpose, public_key, generation, issued_at_ms, \
             expires_at_ms, offer_json) VALUES ('01234567890123456789', ?, 'submission', ?, 1, 1, 2, '{}')",
    )
    .bind([2_u8; 32].as_slice())
    .bind([7_u8; 32].as_slice())
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO campaign_objects (sha256, byte_length, created_at_ms) VALUES (?, 1, 1)",
    )
    .bind([3_u8; 32].as_slice())
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::raw_sql(MIGRATOR.migrations[5].sql.as_str())
        .execute(&mut connection)
        .await
        .unwrap();
    let identities: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM identities")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    let diagnostics: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM diagnostic_reports")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    let submission_challenges: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upload_challenges WHERE purpose = 'submission'")
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert_eq!((identities, diagnostics, submission_challenges), (1, 1, 0));
}

#[tokio::test]
async fn migration_refuses_a_tampered_canonical_schema_checksum() {
    use sqlx::Connection as _;
    let directory = tempfile::tempdir().unwrap();
    let config = ServerConfig {
        database_path: directory.path().join("tampered.sqlite3"),
        ..Default::default()
    };
    drop(Database::migrate(&config).await.unwrap());

    let options = SqliteConnectOptions::new().filename(&config.database_path);
    let mut connection = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET checksum = zeroblob(48) WHERE version = 1")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();

    assert!(matches!(
        Database::migrate(&config).await,
        Err(DbError::Migration(
            sqlx::migrate::MigrateError::VersionMismatch(1)
        ))
    ));
}

#[tokio::test]
async fn symlinked_database_path_and_ancestor_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target.sqlite3");
    std::fs::write(&target, []).unwrap();
    let link = directory.path().join("database.sqlite3");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let config = ServerConfig {
        database_path: link,
        ..Default::default()
    };
    assert!(Database::connect(&config).await.is_err());

    let real = directory.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let link = directory.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let config = ServerConfig {
        database_path: link.join("highscores.sqlite3"),
        ..Default::default()
    };
    assert!(Database::connect(&config).await.is_err());
}

#[tokio::test]
async fn pinned_database_and_wal_survive_ancestor_swap_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let live = directory.path().join("live");
    let config = ServerConfig {
        database_path: live.join("data/highscores.sqlite3"),
        ..Default::default()
    };
    let database = Database::migrate(&config).await.unwrap();
    insert_acceptance_sequence(&database, 101).await;

    let displaced = directory.path().join("displaced");
    tokio::fs::rename(&live, &displaced).await.unwrap();
    tokio::fs::create_dir_all(config.database_path.parent().unwrap())
        .await
        .unwrap();
    let sentinels = [
        (config.database_path.clone(), b"main-sentinel".as_slice()),
        (
            PathBuf::from(format!("{}-wal", config.database_path.display())),
            b"wal-sentinel".as_slice(),
        ),
        (
            PathBuf::from(format!("{}-shm", config.database_path.display())),
            b"shm-sentinel".as_slice(),
        ),
    ];
    for (path, bytes) in &sentinels {
        tokio::fs::write(path, bytes).await.unwrap();
    }

    insert_acceptance_sequence(&database, 102).await;
    database.fixture_pool().close().await;
    drop(database);
    for (path, bytes) in &sentinels {
        assert_eq!(tokio::fs::read(path).await.unwrap(), *bytes);
    }

    let mut displaced_config = config.clone();
    displaced_config.database_path = displaced.join("data/highscores.sqlite3");
    let reopened = Database::connect(&displaced_config).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM acceptance_sequences")
            .fetch_one(reopened.fixture_pool())
            .await
            .unwrap(),
        2
    );
    reopened.fixture_pool().close().await;
}

#[tokio::test]
async fn username_changes_are_append_only_audited_and_old_challenges_cannot_roll_back() {
    let (_directory, database) = test_database().await;
    let key = [42; 32];
    let rollback = database
        .issue_challenge(
            ChallengePurpose::UsernameUpdate,
            key,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    for username in ["Robin", "Locksley"] {
        register_test_identity(&database, key, username).await;
    }
    assert!(matches!(
        database
            .apply_username_update(&rollback.id, rollback.nonce.into_bytes(), key, "Rollback")
            .await,
        Err(DbError::InvalidChallenge)
    ));
    let rows = sqlx::query(
        "SELECT previous_username, new_username FROM username_history \
         WHERE public_key = ? ORDER BY generation",
    )
    .bind(key.as_slice())
    .fetch_all(database.fixture_pool())
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<Option<String>, _>("previous_username"), None);
    assert_eq!(rows[1].get::<String, _>("previous_username"), "Robin");
    assert_eq!(
        database.public_identity(&key).await.unwrap().username,
        "Locksley"
    );
}

#[tokio::test]
async fn abuse_report_quotas_are_atomic_per_ip_key_and_target() {
    let (_directory, database) = test_database().await;
    let key = [43; 32];
    register_test_identity(&database, key, "Marian").await;
    database
        .insert_abuse_report(
            "player",
            &hex::encode(key),
            &[],
            "other",
            "first",
            [1; 32],
            1,
            10,
            10,
        )
        .await
        .unwrap();
    assert!(matches!(
        database
            .insert_abuse_report(
                "player",
                &hex::encode(key),
                &[],
                "other",
                "second",
                [1; 32],
                1,
                10,
                10
            )
            .await,
        Err(DbError::QueueFull)
    ));
}

#[tokio::test]
async fn writer_heartbeats_and_class_limits_match_the_capacity_model() {
    let (_directory, _config, database) = TestDeployment::new()
        .configure(|_, config| {
            config.max_concurrent_requests = 2;
            config.max_concurrent_uploads = 2;
        })
        .migrate()
        .await;
    let mut leases = Vec::new();
    for (class, count) in [
        (MaintenanceWriteClass::ApiSensitive, 2),
        (MaintenanceWriteClass::ApiUpload, 2),
        (MaintenanceWriteClass::ApiMaintenance, 1),
        (MaintenanceWriteClass::Worker, 1),
        (MaintenanceWriteClass::Admin, 1),
    ] {
        for ordinal in 0..count {
            leases.push(
                database
                    .acquire_maintenance_write_lease(
                        class,
                        &format!("{class:?}-{ordinal}"),
                        Duration::from_secs(60),
                    )
                    .await
                    .unwrap(),
            );
        }
        assert!(matches!(
            database
                .acquire_maintenance_write_lease(
                    class,
                    &format!("{class:?}-overflow"),
                    Duration::from_secs(60),
                )
                .await,
            Err(DbError::QueueFull)
        ));
    }
    assert_eq!(
        database
            .active_maintenance_write_lease_count()
            .await
            .unwrap(),
        7
    );
    assert!(
        database
            .refresh_maintenance_write_lease(&leases[0], Duration::from_secs(60))
            .await
            .unwrap()
    );
    for lease in leases {
        assert!(
            database
                .release_maintenance_write_lease(&lease)
                .await
                .unwrap()
        );
    }
    assert_eq!(
        database
            .active_maintenance_write_lease_count()
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn purpose_quotas_reserve_submission_challenge_capacity() {
    let (_directory, _config, database) = TestDeployment::new()
        .configure(|_, config| config.max_pending_submissions = 2)
        .migrate()
        .await;
    for key in [[1_u8; 32], [2_u8; 32]] {
        database
            .issue_challenge(
                ChallengePurpose::UsernameUpdate,
                key,
                Duration::from_secs(60),
            )
            .await
            .unwrap();
    }
    assert!(matches!(
        database
            .issue_challenge(
                ChallengePurpose::UsernameUpdate,
                [3_u8; 32],
                Duration::from_secs(60)
            )
            .await,
        Err(DbError::QueueFull)
    ));
    database
        .issue_challenge(
            ChallengePurpose::Submission,
            [4_u8; 32],
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    assert!(
        database
            .issue_challenge(
                ChallengePurpose::OwnerStatus,
                [4_u8; 32],
                Duration::from_secs(60)
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn concurrent_challenge_issuance_has_unique_monotonic_generations() {
    let (_directory, database) = test_database().await;
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let database = database.clone();
        tasks.push(tokio::spawn(async move {
            database
                .issue_challenge(
                    ChallengePurpose::UsernameUpdate,
                    [8; 32],
                    Duration::from_secs(60),
                )
                .await
                .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let row = sqlx::query(
        "SELECT COUNT(*) AS total, COUNT(DISTINCT generation) AS distinct_total, \
                MAX(generation) AS maximum \
         FROM upload_challenges WHERE public_key = ? AND purpose = 'username_update'",
    )
    .bind([8; 32].as_slice())
    .fetch_one(database.fixture_pool())
    .await
    .unwrap();
    assert_eq!(row.get::<i64, _>("total"), 16);
    assert_eq!(row.get::<i64, _>("distinct_total"), 16);
    assert_eq!(row.get::<i64, _>("maximum"), 16);
}

#[tokio::test]
async fn upload_reservation_is_exact_retryable_and_single_publish() {
    let (_directory, database) = test_database().await;
    let fixture = submission_fixture(&database, [9; 32], 1).await;
    assert!(matches!(
        database
            .reserve_submission_upload_if_admitted(
                &fixture.intent,
                Duration::from_secs(30),
                Duration::from_secs(300),
                false,
            )
            .await,
        Err(DbError::AdmissionUnavailable)
    ));
    assert_eq!(
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT consumed_at_ms FROM upload_challenges WHERE id = ?"
        )
        .bind(&fixture.intent.upload_challenge_id)
        .fetch_one(database.fixture_pool())
        .await
        .unwrap(),
        None,
        "red admission must not consume the one-use challenge"
    );
    let lease = acquire_upload_fixture(&database, &fixture).await;
    assert!(matches!(
        database
            .reserve_submission_upload(
                &fixture.intent,
                Duration::from_secs(30),
                Duration::from_secs(300)
            )
            .await
            .unwrap(),
        SubmissionUploadReservation::Busy { .. }
    ));
    let mut conflicting = fixture.intent.clone();
    conflicting.envelope_json = "{\"immutable\":false}".to_owned();
    assert!(matches!(
        database
            .reserve_submission_upload(
                &conflicting,
                Duration::from_secs(30),
                Duration::from_secs(300)
            )
            .await,
        Err(DbError::SubmissionConflict)
    ));
    let mut wrong_nonce = fixture.intent.clone();
    wrong_nonce.upload_challenge_nonce[0] ^= 1;
    assert!(matches!(
        database
            .reserve_submission_upload(
                &wrong_nonce,
                Duration::from_secs(30),
                Duration::from_secs(300)
            )
            .await,
        Err(DbError::InvalidChallenge)
    ));
    assert!(database.abandon_submission_upload(&lease).await.unwrap());

    let mut retry_intent = fixture.intent.clone();
    retry_intent.proposed_submission_id = uuid::Uuid::now_v7().to_string();
    let SubmissionUploadReservation::Acquired {
        lease: retry_lease,
        resume_uploaded: false,
    } = database
        .reserve_submission_upload(
            &retry_intent,
            Duration::from_secs(30),
            Duration::from_secs(300),
        )
        .await
        .unwrap()
    else {
        panic!("abandoned exact retry was not reacquired")
    };
    assert_eq!(retry_lease.submission_id, fixture.submission.id);
    database
        .mark_submission_upload_uploaded(&retry_lease)
        .await
        .unwrap();
    let inserted = database
        .finalize_submission_upload(&fixture.submission, &retry_lease)
        .await
        .unwrap();
    let SubmissionUploadReservation::Existing { lifecycle } = database
        .reserve_submission_upload_if_admitted(
            &retry_intent,
            Duration::from_secs(30),
            Duration::from_secs(300),
            false,
        )
        .await
        .unwrap()
    else {
        panic!("completed exact retry did not return its lifecycle")
    };
    assert_eq!(lifecycle.id, inserted.id);
    assert!(
        database
            .lease_next("only-worker", Duration::from_secs(30))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        database
            .lease_next("no-duplicate-worker", Duration::from_secs(30))
            .await
            .unwrap()
            .is_none(),
        "an exact retry must not create a second verifier job"
    );
}

#[tokio::test]
async fn a_pending_or_verified_replay_cannot_be_uploaded_again_by_anyone() {
    let (_directory, database) = test_database().await;
    let first = submission_fixture(&database, [9; 32], 1).await;
    let lease = acquire_upload_fixture(&database, &first).await;
    // A second challenge (any uploader) for the same replay cannot reserve
    // while the first upload is in flight...
    let second = submission_fixture(&database, [10; 32], 1).await;
    assert!(matches!(
        database
            .reserve_submission_upload(
                &second.intent,
                Duration::from_secs(30),
                Duration::from_secs(300)
            )
            .await,
        Err(DbError::DuplicateReplay)
    ));
    database
        .mark_submission_upload_uploaded(&lease)
        .await
        .unwrap();
    database
        .finalize_submission_upload(&first.submission, &lease)
        .await
        .unwrap();
    // ...nor once it is queued.
    assert!(matches!(
        database
            .reserve_submission_upload(
                &second.intent,
                Duration::from_secs(30),
                Duration::from_secs(300)
            )
            .await,
        Err(DbError::DuplicateReplay)
    ));
    // A rejected replay may be retried.
    let job = database
        .lease_next("worker", Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    database
        .reject_job(
            &job.submission_id,
            "worker",
            "state_hash_mismatch",
            Some("frame_9"),
        )
        .await
        .unwrap();
    assert!(matches!(
        database
            .reserve_submission_upload(
                &second.intent,
                Duration::from_secs(30),
                Duration::from_secs(300)
            )
            .await
            .unwrap(),
        SubmissionUploadReservation::Acquired { .. }
    ));
}

#[tokio::test]
async fn uploaded_crash_recovery_reuses_the_canonical_submission_id() {
    let (_directory, database) = test_database().await;
    let fixture = submission_fixture(&database, [9; 32], 1).await;
    let lease = acquire_upload_fixture(&database, &fixture).await;
    database
        .mark_submission_upload_uploaded(&lease)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE submission_upload_reservations \
         SET lease_expires_at_ms = reserved_at_ms + 1 WHERE upload_challenge_id = ?",
    )
    .bind(&fixture.intent.upload_challenge_id)
    .execute(database.fixture_pool())
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;
    database.recover_upload_reservations().await.unwrap();
    let SubmissionUploadReservation::Acquired {
        lease: resumed,
        resume_uploaded: true,
    } = database
        .reserve_submission_upload_if_admitted(
            &fixture.intent,
            Duration::from_secs(30),
            Duration::from_secs(300),
            false,
        )
        .await
        .unwrap()
    else {
        panic!("uploaded exact recovery was not resumed while new admission was red")
    };
    assert_eq!(resumed.submission_id, fixture.submission.id);
    database
        .finalize_submission_upload(&fixture.submission, &resumed)
        .await
        .unwrap();
}

#[tokio::test]
async fn expired_abandoned_reservation_and_challenge_are_bounded() {
    let (_directory, database) = test_database().await;
    let fixture = submission_fixture(&database, [9; 32], 1).await;
    let lease = acquire_upload_fixture(&database, &fixture).await;
    assert!(database.abandon_submission_upload(&lease).await.unwrap());
    sqlx::query(
        "UPDATE submission_upload_reservations \
         SET reservation_expires_at_ms = reserved_at_ms + 1 WHERE upload_challenge_id = ?",
    )
    .bind(&fixture.intent.upload_challenge_id)
    .execute(database.fixture_pool())
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;
    assert!(database.recover_upload_reservations().await.unwrap() >= 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM upload_challenges WHERE id = ?")
            .bind(&fixture.intent.upload_challenge_id)
            .fetch_one(database.fixture_pool())
            .await
            .unwrap(),
        0
    );
    assert!(matches!(
        database
            .reserve_submission_upload(
                &fixture.intent,
                Duration::from_secs(30),
                Duration::from_secs(300)
            )
            .await,
        Err(DbError::InvalidChallenge)
    ));
}

#[tokio::test]
async fn exhausted_infrastructure_failure_is_terminal_private_and_not_a_rejection() {
    let (_directory, database) = test_database().await;
    let fixture = submission_fixture(&database, [9; 32], 1).await;
    let inserted = insert_submission_fixture(&database, &fixture).await;
    let job = database
        .lease_next("worker-1", Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.submission_id, inserted.id);
    assert_eq!(job.board_id, BOARD_ID);
    database
        .fail_job(&inserted.id, "worker-1", "private verifier I/O failure")
        .await
        .unwrap();
    let lifecycle = database.submission_lifecycle(&inserted.id).await.unwrap();
    assert_eq!(lifecycle.state, crate::model::SubmissionState::Failed);
    assert!(
        database
            .lease_next("worker-2", Duration::from_secs(60))
            .await
            .unwrap()
            .is_none(),
        "terminal infrastructure failures must never be leased again"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT failure_detail FROM submissions WHERE id = ?")
            .bind(&inserted.id)
            .fetch_one(database.fixture_pool())
            .await
            .unwrap(),
        "private verifier I/O failure"
    );
    // A failed replay may be submitted again.
    let retry = submission_fixture(&database, [9; 32], 1).await;
    acquire_upload_fixture(&database, &retry).await;
}

#[tokio::test]
async fn expired_rejected_submission_releases_its_replay_reference() {
    let (_directory, database) = test_database().await;
    let fixture = submission_fixture(&database, [9; 32], 1).await;
    let inserted = insert_submission_fixture(&database, &fixture).await;
    let job = database
        .lease_next("worker", Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    database
        .reject_job(&job.submission_id, "worker", "malformed_replay", None)
        .await
        .unwrap();
    assert!(
        database
            .reject_job(&job.submission_id, "worker", "build_not_allowed", None)
            .await
            .is_err()
    );
    let rejected_at: i64 = sqlx::query_scalar("SELECT updated_at_ms FROM submissions WHERE id = ?")
        .bind(&inserted.id)
        .fetch_one(database.fixture_pool())
        .await
        .unwrap();
    let tombstoned_at = u64::try_from(rejected_at).unwrap() + 1;
    assert_eq!(
        database
            .expire_rejected_submissions(u64::try_from(rejected_at).unwrap(), tombstoned_at)
            .await
            .unwrap(),
        1
    );
    let candidates = database
        .claim_replay_gc_candidates(tombstoned_at + 1, tombstoned_at, 10)
        .await
        .unwrap();
    assert_eq!(
        candidates
            .into_iter()
            .map(|candidate| candidate.sha256)
            .collect::<Vec<_>>(),
        [fixture.submission.replay_sha256]
    );
}

#[tokio::test]
async fn accepted_run_is_published_and_bound_to_its_job_board_and_replay() {
    let (_directory, database) = test_database().await;
    let board = demo_board(BOARD_ID, &[MISSION_ID]);
    let fixture = submission_fixture(&database, [9; 32], 1).await;
    let inserted = insert_submission_fixture(&database, &fixture).await;
    let job = database
        .lease_next("worker", Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    let job_sha256 = Digest32::digest_bytes(b"job");
    let replay_sha256 = Digest32::from_bytes(fixture.submission.replay_sha256);
    let output = verified_output(job_sha256, replay_sha256, verified_run(10, 60, 90));

    // Wrong job digest, replay, outcome and wrapped scores never publish.
    assert!(matches!(
        database
            .accept_job(
                &job.submission_id,
                "worker",
                Digest32::digest_bytes(b"other"),
                &board,
                &output
            )
            .await,
        Err(DbError::ResultInvariant(_))
    ));
    let wrong_replay = verified_output(
        job_sha256,
        Digest32::from_bytes([2; 32]),
        verified_run(10, 60, 90),
    );
    assert!(
        database
            .accept_job(
                &job.submission_id,
                "worker",
                job_sha256,
                &board,
                &wrong_replay
            )
            .await
            .is_err()
    );
    let mut lost = verified_run(10, 60, 90);
    lost.outcome = robin_run_protocol::TerminalOutcomeV1::Lost;
    assert!(
        database
            .accept_job(
                &job.submission_id,
                "worker",
                job_sha256,
                &board,
                &verified_output(job_sha256, replay_sha256, lost)
            )
            .await
            .is_err()
    );
    let other_board = demo_board("other-board", &[MISSION_ID]);
    assert!(
        database
            .accept_job(
                &job.submission_id,
                "worker",
                job_sha256,
                &other_board,
                &output
            )
            .await
            .is_err()
    );
    assert!(matches!(
        database
            .accept_job(
                &job.submission_id,
                "another-worker",
                job_sha256,
                &board,
                &output
            )
            .await,
        Err(DbError::LeaseLost)
    ));

    let run_id = database
        .accept_job(&job.submission_id, "worker", job_sha256, &board, &output)
        .await
        .unwrap();
    let lifecycle = database.submission_lifecycle(&inserted.id).await.unwrap();
    assert_eq!(
        lifecycle.state,
        crate::model::SubmissionState::Accepted {
            run_id: OpaqueId::new(run_id.clone()).unwrap()
        }
    );
    let visible = vec![BOARD_ID.to_owned()];
    let run = database.public_run(&run_id, &visible).await.unwrap();
    assert_eq!(run.original_score_delta, 50);
    assert_eq!(run.active_simulation_ticks, 90);
    assert_eq!(run.uploader.as_ref().unwrap().username, "Robin");
    assert_eq!(run.achievements.len(), 24);
    assert!(matches!(
        database.public_run(&run_id, &[]).await,
        Err(DbError::NotFound)
    ));
    assert_eq!(
        database.replay_for_run(&run_id, &visible).await.unwrap(),
        (fixture.submission.replay_sha256, 4)
    );
    let rows = database
        .leaderboard_rows(
            &BoardQuery {
                board_id: BOARD_ID,
                mission_id: MISSION_ID,
                metric: robin_run_protocol::BoardMetricV1::FastestSuccess,
                max_concurrent_players: Some(1),
                player_public_key: Some([9; 32]),
            },
            None,
            10,
            database.accepted_sequence_watermark().await.unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].rank, rows[0].metric_value), (1, 90));
}

#[test]
fn ranked_score_rejects_signed_wraparound() {
    use super::acceptance::validate_ranked_score;
    assert!(validate_ranked_score(100, 150, 50).is_ok());
    assert!(validate_ranked_score(-100, -50, 50).is_ok());
    assert!(validate_ranked_score(i32::MAX, i32::MIN, 1).is_err());
    assert!(validate_ranked_score(100, 99, u32::MAX as i64).is_err());
    assert!(validate_ranked_score(100, 150, 49).is_err());
}

#[test]
fn stored_achievement_evaluation_is_lossless_and_exhaustive() {
    for evaluation in [
        VerifiedAchievementEvaluationV1::Unverifiable,
        VerifiedAchievementEvaluationV1::NotEarned,
        VerifiedAchievementEvaluationV1::Earned,
    ] {
        assert_eq!(
            achievement_evaluation(achievement_evaluation_name(evaluation)).unwrap(),
            evaluation
        );
    }
    assert!(matches!(
        achievement_evaluation("false"),
        Err(DbError::Corrupt(_))
    ));
}
