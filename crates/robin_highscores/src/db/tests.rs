use super::*;
use crate::test_support::{TestDeployment, demo_board, verified_output, verified_run};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

const BOARD_ID: &str = "demo-standard-normal";
const MISSION_ID: &str = "Dem_Lei_MP";

async fn test_database() -> (tempfile::TempDir, Database) {
    let (directory, _config, database) = TestDeployment::new().migrate().await;
    (directory, database)
}

/// Strictly increasing signed timestamps for fixture username updates.
fn next_signed_at() -> u64 {
    static CLOCK: AtomicU64 = AtomicU64::new(1_800_000_000_000);
    CLOCK.fetch_add(1, Ordering::SeqCst)
}

async fn register_test_identity(database: &Database, public_key: [u8; 32], username: &str) {
    database
        .apply_username_update(public_key, next_signed_at(), username)
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

/// A registered uploader and the upload projection of one signed request
/// (`request` distinguishes re-signed requests) for replay `[replay_byte; 32]`.
async fn submission_fixture(
    database: &Database,
    uploader: [u8; 32],
    replay_byte: u8,
    request: u32,
) -> SubmissionFixture {
    if !database.identity_exists(&uploader).await.unwrap() {
        register_test_identity(database, uploader, "Robin").await;
    }
    let id = uuid::Uuid::now_v7().to_string();
    let signed_request_json = format!(
        "{{\"fixture\":{replay_byte},\"uploader\":{},\"request\":{request}}}",
        uploader[0]
    );
    SubmissionFixture {
        submission: NewSubmission {
            id: id.clone(),
            signed_request_json: signed_request_json.clone(),
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
            signed_request_json,
            uploader_public_key: uploader,
            replay_sha256: [replay_byte; 32],
        },
    }
}

async fn reserve(
    database: &Database,
    intent: &SubmissionUploadIntent,
) -> Result<SubmissionUploadReservation, DbError> {
    database
        .reserve_submission_upload(intent, Duration::from_secs(30), Duration::from_secs(300))
        .await
}

async fn acquire_upload_fixture(
    database: &Database,
    fixture: &SubmissionFixture,
) -> SubmissionUploadLease {
    match reserve(database, &fixture.intent).await.unwrap() {
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
fn migration_chain_ends_with_the_signed_player_request_schema() {
    assert_eq!(CURRENT_SCHEMA_VERSION, 7);
    assert_eq!(MIGRATOR.migrations.len(), 7);
    assert_eq!(MIGRATOR.migrations[0].description.as_ref(), "initial");
    assert_eq!(
        MIGRATOR.migrations[5].description.as_ref(),
        "ranked protocol v2"
    );
    let signed = &MIGRATOR.migrations[6];
    assert_eq!(signed.version, 7);
    assert_eq!(signed.description.as_ref(), "signed player requests");
    assert!(
        !MIGRATOR.migrations[0]
            .sql
            .as_str()
            .contains("maintenance_write_leases"),
        "the immutable initial migration was rewritten"
    );
    assert!(
        MIGRATOR.migrations[5]
            .sql
            .as_str()
            .contains("upload_challenge_id TEXT NOT NULL UNIQUE"),
        "the deployed protocol V2 migration was rewritten"
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
        "upload_challenges",
        "challenge_generations",
        "submission_owner_status_challenges",
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
    let schema: String = sqlx::query_scalar(
        "SELECT group_concat(sql, char(10)) FROM sqlite_master WHERE sql IS NOT NULL",
    )
    .fetch_one(database.fixture_pool())
    .await
    .unwrap();
    assert!(
        !schema.contains("challenge"),
        "a challenge column or reference survived: {schema}"
    );
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

/// Apply one migration the way sqlx does: inside its own transaction.
async fn apply_migration(connection: &mut sqlx::SqliteConnection, index: usize) {
    use sqlx::Connection as _;
    let mut tx = connection.begin().await.unwrap();
    sqlx::raw_sql(MIGRATOR.migrations[index].sql.as_str())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn protocol_v2_migration_preserves_identities_and_diagnostics() {
    use sqlx::Connection as _;
    let options = SqliteConnectOptions::new()
        .filename(":memory:")
        .foreign_keys(true);
    let mut connection = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    for index in 0..5 {
        apply_migration(&mut connection, index).await;
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
        "INSERT INTO campaign_objects (sha256, byte_length, created_at_ms) VALUES (?, 1, 1)",
    )
    .bind([3_u8; 32].as_slice())
    .execute(&mut connection)
    .await
    .unwrap();
    for index in 5..7 {
        apply_migration(&mut connection, index).await;
    }
    let identities: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM identities")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    let diagnostics: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM diagnostic_reports")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!((identities, diagnostics), (1, 1));
}

/// 0007 must apply on top of a live 0006 database without losing players,
/// history, deletion receipts, queued or verified submissions, or an
/// in-flight uploaded reservation.
#[tokio::test]
async fn signed_request_migration_preserves_live_protocol_v2_rows() {
    use sqlx::Connection as _;
    let options = SqliteConnectOptions::new()
        .filename(":memory:")
        .foreign_keys(true);
    let mut connection = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    for index in 0..6 {
        apply_migration(&mut connection, index).await;
    }
    let key = [7_u8; 32];
    let replay = [3_u8; 32];
    let in_flight_replay = [4_u8; 32];
    for statement in [
        "INSERT INTO identities (public_key, username, username_normalized, username_generation, \
             created_at_ms, updated_at_ms) VALUES (?1, 'Robin', 'robin', 2, 1, 1)",
        "INSERT INTO upload_challenges (id, nonce, purpose, public_key, generation, issued_at_ms, \
             expires_at_ms) VALUES ('challenge-username-000001', ?2, 'username_update', ?1, 2, 1, 2)",
        "INSERT INTO upload_challenges (id, nonce, purpose, public_key, generation, issued_at_ms, \
             expires_at_ms, consumed_at_ms) \
             VALUES ('challenge-submission-00001', ?3, 'submission', ?1, 1, 1, 2, 1)",
        "INSERT INTO upload_challenges (id, nonce, purpose, public_key, generation, issued_at_ms, \
             expires_at_ms, consumed_at_ms) \
             VALUES ('challenge-submission-00002', ?4, 'submission', ?1, 2, 1, 99999999999999, 1)",
        "INSERT INTO username_history (public_key, challenge_id, generation, previous_username, \
             new_username, changed_at_ms) VALUES (?1, 'challenge-username-000001', 2, NULL, 'Robin', 1)",
        "INSERT INTO deletion_requests (id, challenge_id, owner_public_key, target_kind, target_id, \
             request_json, tombstoned_at_ms) \
             VALUES ('deletion-1', 'challenge-username-000001', ?1, 'run', 'run-gone', '{}', 5)",
        "INSERT INTO replay_objects (sha256, byte_length, created_at_ms) VALUES (?3, 4, 1)",
        "INSERT INTO submissions (id, upload_challenge_id, envelope_json, uploader_public_key, \
             public_disclosure, board_id, mission_id, replay_sha256, replay_bytes, \
             replay_schema_version, requested_metrics_json, status, next_attempt_at_ms, \
             created_at_ms, updated_at_ms) \
             VALUES ('submission-00000000000001', 'challenge-submission-00001', '{\"v\":2}', ?1, \
                     'named_profile', 'board', 'mission', ?3, 4, 1, '[]', 'accepted', 1, 1, 1)",
        "INSERT INTO acceptance_sequences (created_at_ms) VALUES (1)",
        "INSERT INTO verified_runs (id, submission_id, board_id, mission_id, edition, \
             recorded_engine_version, sim_config_json, max_concurrent_players, \
             participant_instance_count, starting_campaign_score, final_campaign_score, \
             original_score_delta, final_state_sha256, replay_frames, active_simulation_ticks, \
             ransom_collected, input_provenance_json, job_sha256, accepted_sequence, verified_at_ms) \
             VALUES ('run-000000000000000000001', 'submission-00000000000001', 'board', 'mission', \
                     'demo', 'engine', '{}', 1, 1, 0, 1, 1, ?3, 1, 1, 0, '{}', ?3, 1, 1)",
        "INSERT INTO verified_run_metrics (run_id, metric, value) \
             VALUES ('run-000000000000000000001', 'original_score', 1)",
        "INSERT INTO worker_events (submission_id, kind, worker_id, created_at_ms) \
             VALUES ('submission-00000000000001', 'accepted', 'worker', 1)",
        "INSERT INTO submission_upload_reservations (upload_challenge_id, submission_id, \
             envelope_json, envelope_sha256, uploader_public_key, replay_sha256, state, \
             reservation_expires_at_ms, reserved_at_ms, updated_at_ms, committed_at_ms) \
             VALUES ('challenge-submission-00001', 'submission-00000000000001', '{}', ?3, ?1, ?3, \
                     'committed', 10, 1, 1, 1)",
        "INSERT INTO submission_upload_reservations (upload_challenge_id, submission_id, \
             envelope_json, envelope_sha256, uploader_public_key, replay_sha256, state, lease_token, \
             lease_expires_at_ms, reservation_expires_at_ms, reserved_at_ms, updated_at_ms) \
             VALUES ('challenge-submission-00002', 'submission-00000000000002', '{}', ?4, ?1, ?4, \
                     'uploaded', 'lease-token-0000000000001', 50, 99999999999999, 1, 1)",
    ] {
        sqlx::query(statement)
            .bind(key.as_slice())
            .bind([9_u8; 32].as_slice())
            .bind(replay.as_slice())
            .bind(in_flight_replay.as_slice())
            .execute(&mut connection)
            .await
            .unwrap();
    }

    apply_migration(&mut connection, 6).await;

    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut connection)
        .await
        .unwrap();
    assert!(violations.is_empty(), "foreign keys dangle after 0007");
    let count = |sql: &'static str| sqlx::query_scalar::<_, i64>(sql);
    for (sql, expected) in [
        (
            "SELECT COUNT(*) FROM identities WHERE username_signed_at_unix_ms = 0",
            1,
        ),
        (
            "SELECT COUNT(*) FROM username_history WHERE signed_at_unix_ms = 0 AND generation = 2",
            1,
        ),
        (
            "SELECT COUNT(*) FROM deletion_requests WHERE signed_at_unix_ms = 0",
            1,
        ),
        (
            "SELECT COUNT(*) FROM submissions \
             WHERE status = 'accepted' AND signed_request_json = '{\"v\":2}'",
            1,
        ),
        ("SELECT COUNT(*) FROM verified_runs", 1),
        ("SELECT COUNT(*) FROM verified_run_metrics", 1),
        ("SELECT COUNT(*) FROM worker_events", 1),
        (
            "SELECT COUNT(*) FROM submission_upload_reservations \
             WHERE state = 'uploaded' AND submission_id = 'submission-00000000000002'",
            1,
        ),
        ("SELECT COUNT(*) FROM submission_upload_reservations", 1),
        (
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE name IN ('upload_challenges', 'challenge_generations', \
                            'submission_owner_status_challenges', 'submissions_v6_rows')",
            0,
        ),
    ] {
        assert_eq!(
            count(sql).fetch_one(&mut connection).await.unwrap(),
            expected,
            "{sql}"
        );
    }
    // The rebuilt unique live-replay index still guards the verified replay.
    assert!(
        sqlx::query(
            "INSERT INTO submissions (id, signed_request_json, uploader_public_key, \
                 public_disclosure, board_id, mission_id, replay_sha256, replay_bytes, \
                 replay_schema_version, requested_metrics_json, status, next_attempt_at_ms, \
                 created_at_ms, updated_at_ms) \
             VALUES ('submission-00000000000003', '{}', ?, 'anonymous', 'board', 'mission', ?, 4, \
                     1, '[]', 'queued', 1, 1, 1)",
        )
        .bind(key.as_slice())
        .bind(replay.as_slice())
        .execute(&mut connection)
        .await
        .is_err()
    );
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
async fn username_changes_are_audited_and_older_signed_updates_cannot_roll_back() {
    let (_directory, database) = test_database().await;
    let key = [42; 32];
    database
        .apply_username_update(key, 1_000, "Robin")
        .await
        .unwrap();
    database
        .apply_username_update(key, 2_000, "Locksley")
        .await
        .unwrap();
    // A replay of either earlier request, or of the latest one, is refused.
    for (signed_at, username) in [(1_000, "Robin"), (1_500, "Rollback"), (2_000, "Locksley")] {
        assert!(matches!(
            database
                .apply_username_update(key, signed_at, username)
                .await,
            Err(DbError::UsernameUpdateSuperseded)
        ));
    }
    let rows = sqlx::query(
        "SELECT previous_username, new_username, generation, signed_at_unix_ms \
         FROM username_history WHERE public_key = ? ORDER BY generation",
    )
    .bind(key.as_slice())
    .fetch_all(database.fixture_pool())
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<Option<String>, _>("previous_username"), None);
    assert_eq!(rows[1].get::<String, _>("previous_username"), "Robin");
    assert_eq!(
        (
            rows[1].get::<i64, _>("generation"),
            rows[1].get::<i64, _>("signed_at_unix_ms")
        ),
        (2, 2_000)
    );
    assert_eq!(
        database.public_identity(&key).await.unwrap().username,
        "Locksley"
    );
}

#[tokio::test]
async fn deletion_is_idempotent_owner_scoped_and_reuses_existing_tombstones() {
    let (_directory, database) = test_database().await;
    let owner = [9_u8; 32];
    let fixture = submission_fixture(&database, owner, 1, 0).await;
    let inserted = insert_submission_fixture(&database, &fixture).await;
    register_test_identity(&database, [10; 32], "Stranger").await;

    assert!(matches!(
        database
            .apply_deletion([10; 32], 1, "submission", &inserted.id, "{}", None)
            .await,
        Err(DbError::NotFound)
    ));
    let first = database
        .apply_deletion(
            owner,
            1,
            "submission",
            &inserted.id,
            "{}",
            Some(Duration::from_secs(60)),
        )
        .await
        .unwrap();
    let repeated = database
        .apply_deletion(
            owner,
            2,
            "submission",
            &inserted.id,
            "{\"again\":true}",
            Some(Duration::from_secs(120)),
        )
        .await
        .unwrap();
    assert_eq!(
        (
            repeated.id.as_str(),
            repeated.tombstoned_at_ms,
            repeated.purge_eligible_at_ms
        ),
        (
            first.id.as_str(),
            first.tombstoned_at_ms,
            first.purge_eligible_at_ms
        )
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM deletion_requests")
            .fetch_one(database.fixture_pool())
            .await
            .unwrap(),
        1
    );
    assert!(matches!(
        database.submission_lifecycle(&inserted.id).await,
        Err(DbError::NotFound)
    ));
    assert!(matches!(
        database
            .owner_submission_lifecycle(owner, &inserted.id)
            .await,
        Err(DbError::NotFound)
    ));
}

#[tokio::test]
async fn owner_status_hides_foreign_and_unknown_submissions_alike() {
    let (_directory, database) = test_database().await;
    let fixture = submission_fixture(&database, [9; 32], 1, 0).await;
    let inserted = insert_submission_fixture(&database, &fixture).await;
    assert_eq!(
        database
            .owner_submission_lifecycle([9; 32], &inserted.id)
            .await
            .unwrap()
            .id,
        inserted.id
    );
    for (key, id) in [
        ([10; 32], inserted.id.as_str()),
        ([9; 32], "missing-submission"),
    ] {
        assert!(matches!(
            database.owner_submission_lifecycle(key, id).await,
            Err(DbError::NotFound)
        ));
    }
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
async fn upload_reservation_is_exact_retryable_and_single_publish() {
    let (_directory, database) = test_database().await;
    let fixture = submission_fixture(&database, [9; 32], 1, 0).await;
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
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submission_upload_reservations")
            .fetch_one(database.fixture_pool())
            .await
            .unwrap(),
        0,
        "red admission must not reserve anything"
    );
    let lease = acquire_upload_fixture(&database, &fixture).await;
    assert!(matches!(
        reserve(&database, &fixture.intent).await.unwrap(),
        SubmissionUploadReservation::Busy { .. }
    ));
    assert!(database.abandon_submission_upload(&lease).await.unwrap());

    // A re-signed request (new signed_at, new canonical JSON) from the same
    // uploader resumes the abandoned reservation under its original ID.
    let resigned = submission_fixture(&database, [9; 32], 1, 1).await;
    let SubmissionUploadReservation::Acquired {
        lease: retry_lease,
        resume_uploaded: false,
    } = reserve(&database, &resigned.intent).await.unwrap()
    else {
        panic!("abandoned retry was not reacquired")
    };
    assert_eq!(retry_lease.submission_id, fixture.submission.id);
    database
        .mark_submission_upload_uploaded(&retry_lease)
        .await
        .unwrap();
    // The original request's projection no longer matches the reservation.
    let mut stale = fixture.submission.clone();
    stale.id = retry_lease.submission_id.clone();
    assert!(matches!(
        database
            .finalize_submission_upload(&stale, &retry_lease)
            .await,
        Err(DbError::SubmissionConflict)
    ));
    let mut finalized = resigned.submission.clone();
    finalized.id = retry_lease.submission_id.clone();
    let inserted = database
        .finalize_submission_upload(&finalized, &retry_lease)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submission_upload_reservations")
            .fetch_one(database.fixture_pool())
            .await
            .unwrap(),
        0,
        "finalization must remove its reservation"
    );
    let SubmissionUploadReservation::Existing { lifecycle } = database
        .reserve_submission_upload_if_admitted(
            &resigned.intent,
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
    // The original (now different) signed request is a duplicate replay.
    assert!(matches!(
        reserve(&database, &fixture.intent).await,
        Err(DbError::DuplicateReplay)
    ));
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
async fn per_uploader_concurrent_leases_are_capped() {
    let (_directory, _config, database) = TestDeployment::new()
        .configure(|_, config| config.max_concurrent_uploads_per_key = 2)
        .migrate()
        .await;
    let mut leases = Vec::new();
    for replay in 1..=2 {
        let fixture = submission_fixture(&database, [9; 32], replay, 0).await;
        leases.push(acquire_upload_fixture(&database, &fixture).await);
    }
    let third = submission_fixture(&database, [9; 32], 3, 0).await;
    assert!(matches!(
        reserve(&database, &third.intent).await,
        Err(DbError::UploadConcurrencyLimit)
    ));
    // Other uploaders are unaffected, and a released lease frees a slot.
    let other = submission_fixture(&database, [10; 32], 4, 0).await;
    acquire_upload_fixture(&database, &other).await;
    assert!(
        database
            .abandon_submission_upload(&leases[0])
            .await
            .unwrap()
    );
    acquire_upload_fixture(&database, &third).await;
}

#[tokio::test]
async fn a_pending_or_verified_replay_cannot_be_uploaded_again_by_anyone() {
    let (_directory, database) = test_database().await;
    let first = submission_fixture(&database, [9; 32], 1, 0).await;
    let lease = acquire_upload_fixture(&database, &first).await;
    // Another uploader cannot reserve the same replay while the first upload
    // is in flight...
    let second = submission_fixture(&database, [10; 32], 1, 0).await;
    assert!(matches!(
        reserve(&database, &second.intent).await,
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
        reserve(&database, &second.intent).await,
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
        reserve(&database, &second.intent).await.unwrap(),
        SubmissionUploadReservation::Acquired { .. }
    ));
}

#[tokio::test]
async fn uploaded_crash_recovery_reuses_the_canonical_submission_id() {
    let (_directory, database) = test_database().await;
    let fixture = submission_fixture(&database, [9; 32], 1, 0).await;
    let lease = acquire_upload_fixture(&database, &fixture).await;
    database
        .mark_submission_upload_uploaded(&lease)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE submission_upload_reservations \
         SET lease_expires_at_ms = reserved_at_ms + 1 WHERE submission_id = ?",
    )
    .bind(&lease.submission_id)
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
        panic!("uploaded recovery was not resumed while new admission was red")
    };
    assert_eq!(resumed.submission_id, fixture.submission.id);
    database
        .finalize_submission_upload(&fixture.submission, &resumed)
        .await
        .unwrap();
}

#[tokio::test]
async fn expired_abandoned_reservation_is_removed_and_frees_its_replay() {
    let (_directory, database) = test_database().await;
    let fixture = submission_fixture(&database, [9; 32], 1, 0).await;
    let lease = acquire_upload_fixture(&database, &fixture).await;
    assert!(database.abandon_submission_upload(&lease).await.unwrap());
    sqlx::query(
        "UPDATE submission_upload_reservations \
         SET reservation_expires_at_ms = reserved_at_ms + 1 WHERE submission_id = ?",
    )
    .bind(&lease.submission_id)
    .execute(database.fixture_pool())
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;
    assert!(database.recover_upload_reservations().await.unwrap() >= 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM submission_upload_reservations")
            .fetch_one(database.fixture_pool())
            .await
            .unwrap(),
        0
    );
    // Another uploader may now upload the same replay.
    let other = submission_fixture(&database, [10; 32], 1, 0).await;
    acquire_upload_fixture(&database, &other).await;
}

#[tokio::test]
async fn exhausted_infrastructure_failure_is_terminal_private_and_not_a_rejection() {
    let (_directory, database) = test_database().await;
    let fixture = submission_fixture(&database, [9; 32], 1, 0).await;
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
    let retry = submission_fixture(&database, [9; 32], 1, 1).await;
    acquire_upload_fixture(&database, &retry).await;
}

#[tokio::test]
async fn expired_rejected_submission_releases_its_replay_reference() {
    let (_directory, database) = test_database().await;
    let fixture = submission_fixture(&database, [9; 32], 1, 0).await;
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
    let fixture = submission_fixture(&database, [9; 32], 1, 0).await;
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
                board_ids: &visible,
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
