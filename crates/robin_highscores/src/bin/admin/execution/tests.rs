use super::super::capacity::estimate_backup_space;
use super::super::capacity::round_up_to_allocation;
use super::super::cleanup::cleanup_failed_partial_backup;
use super::super::filesystem::acquire_backup_operation_lock;
use super::super::filesystem::copy_open_file_sync;
use super::super::filesystem::require_pinned_file_name;
use super::super::filesystem::revalidate_pinned_regular_path;
use super::super::filesystem::set_private_directory;
use super::super::filesystem::sync_cap_directory;
use super::super::filesystem::write_private_file;
use super::super::fixtures::remove_test_database_sidecars;
use super::super::fixtures::test_restore_sources;
use super::super::fixtures::write_test_release_manifest;
use super::super::policy::SYSTEMD_UNIT_FILES;
use super::super::verification::BackupManifestExpectation;
use super::super::verification::pin_transaction_backup_from_status;
use super::super::verification::verify_backup;
use super::super::verification::verify_backup_pinned_offline_with_expected;
use super::super::verification::verify_backup_pinned_with_expected;
use super::super::verification::verify_backup_pinned_with_expected_and_hook;
use super::super::verification::verify_transaction_backup_from_root_pinned;
use super::*;
use robin_highscores::Database;
use robin_highscores::ServerConfig;
use robin_highscores::backup::BackupManifestV4 as BackupManifest;
use robin_highscores::backup::BackupSpaceEstimateV1;
use robin_highscores::backup::BackupStatusV4;
use robin_highscores::backup::BackupVerificationReceiptV2;
use robin_highscores::backup::load_backup_release_identity_oob_file;
use robin_highscores::backup::parse_backup_id;
use robin_run_protocol::canonical_json_bytes;
use sqlx::sqlite::SqliteConnectOptions;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

async fn assert_backup_completion_ownership(mode: &'static str) {
    use std::sync::Arc;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_owned();
    let mut config = ServerConfig::default();
    config.database_path = root.join("live.sqlite3");
    let database = Database::migrate(&config).await.unwrap();
    let runtime = database.runtime_fence().clone();
    let source = root.join("source");
    std::fs::write(&source, b"physical backup bytes").unwrap();
    let partial = root.join(".test.partial/payload");
    let published = root.join("published.marker");
    let operation_root = root.clone();
    let operation_path = partial.clone();
    let publication_path = published.clone();
    let operation_database = database.clone();
    let (registered, wait_registered) = tokio::sync::oneshot::channel();
    let (release, wait_release) = std::sync::mpsc::channel();
    let (release_queue, wait_release_queue) = std::sync::mpsc::channel();
    let refresh_seen = Arc::new(tokio::sync::Notify::new());
    let notify_refresh = Arc::clone(&refresh_seen);
    let replacement_ready = Arc::new(tokio::sync::Notify::new());
    let wait_replacement = Arc::clone(&replacement_ready);
    let mut caller = tokio::spawn(async move {
        run_owned_backup(async move {
            let _operation_lock = acquire_backup_operation_lock(&operation_root)?;
            let (token, fence) = acquire_backup_write_authority(&operation_database).await?;
            let operation_token = token.clone();
            let body_database = operation_database.clone();
            let refresh_database = operation_database.clone();
            let refresh_token = token.clone();
            let result = run_with_backup_lock_heartbeat_using(
                async move {
                    // On the one-blocking-thread runtime this occupies its only
                    // thread before the registered physical copy is enqueued.
                    let blocker = if mode == "queued" {
                        let (started, wait_started) = tokio::sync::oneshot::channel();
                        let blocker = tokio::task::spawn_blocking(move || {
                            started.send(()).unwrap();
                            wait_release_queue
                                .recv_timeout(Duration::from_secs(10))
                                .unwrap();
                        });
                        wait_started.await?;
                        Some(blocker)
                    } else {
                        None
                    };
                    let job = robin_highscores::physical_work::spawn_blocking(move || {
                        wait_release.recv_timeout(Duration::from_secs(10)).unwrap();
                        copy_open_file_sync(std::fs::File::open(source)?, &operation_path)
                    });
                    registered.send(operation_token.clone()).unwrap();
                    if mode == "operation_error" || mode == "queued" {
                        drop(job);
                        drop(blocker);
                        anyhow::bail!("injected backup operation error");
                    }
                    if mode == "operation_panic" {
                        drop(job);
                        panic!("injected backup operation panic");
                    }
                    job.await??;
                    if mode == "heartbeat_then_panic" {
                        panic!("backup operation panicked after heartbeat loss");
                    }
                    // This is the same exact-token barrier retained immediately
                    // before real installation/status publication.
                    refresh_backup_lock(&body_database, &operation_token).await?;
                    std::fs::write(publication_path, b"authorized")?;
                    Ok(())
                },
                Duration::from_millis(10),
                move || {
                    let notify = Arc::clone(&notify_refresh);
                    let wait_replacement = Arc::clone(&wait_replacement);
                    let database = refresh_database.clone();
                    let token = refresh_token.clone();
                    async move {
                        let result = match mode {
                            "heartbeat_error" | "heartbeat_then_panic" => {
                                Err(anyhow::anyhow!("injected backup heartbeat error"))
                            }
                            "heartbeat_panic" => {
                                notify.notify_one();
                                panic!("injected backup heartbeat panic")
                            }
                            "replaced_token" => {
                                wait_replacement.notified().await;
                                refresh_backup_lock(&database, &token).await
                            }
                            _ => refresh_backup_lock(&database, &token).await,
                        };
                        notify.notify_one();
                        result
                    }
                },
            )
            .await;
            let released = release_backup_gate_and_close_pool_under_exclusive_fence(
                &operation_database,
                &token,
                &fence,
            )
            .await?;
            if mode != "replaced_token" {
                anyhow::ensure!(released, "backup gate disappeared");
            }
            result
        })
        .await
    });
    let token = tokio::time::timeout(Duration::from_secs(5), wait_registered)
        .await
        .unwrap()
        .unwrap();
    if mode == "replaced_token" {
        assert!(database.release_backup_lock(&token).await.unwrap());
        database
            .acquire_backup_lock("replacement", BACKUP_LOCK_TTL)
            .await
            .unwrap();
        replacement_ready.notify_one();
    }
    if mode.starts_with("heartbeat_") || mode == "replaced_token" {
        tokio::time::timeout(Duration::from_secs(5), refresh_seen.notified())
            .await
            .unwrap();
    }
    if mode == "caller_cancelled" {
        caller.abort();
        assert!((&mut caller).await.unwrap_err().is_cancelled());
    } else {
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut caller)
                .await
                .is_err(),
            "backup owner returned before physical mutation completed"
        );
    }
    assert!(!partial.exists());
    assert!(runtime.try_lock_exclusive_quiescence().unwrap().is_none());
    assert!(runtime.try_lock_exclusive_admission().unwrap().is_none());
    assert!(
        acquire_backup_operation_lock(&root).is_err(),
        "another backup could clean the active partial"
    );
    assert!(database.backup_lock_active().await.unwrap());
    release.send(()).unwrap();
    if mode == "queued" {
        release_queue.send(()).unwrap();
    }
    if mode != "caller_cancelled" {
        let result = tokio::time::timeout(Duration::from_secs(5), caller)
            .await
            .unwrap()
            .unwrap();
        if mode == "success" {
            result.unwrap();
        } else {
            let error = format!("{:#}", result.unwrap_err());
            let expected = match mode {
                "heartbeat_error" | "heartbeat_then_panic" => "injected backup heartbeat error",
                "heartbeat_panic" => "backup lock refresh panicked",
                "operation_error" | "queued" => "injected backup operation error",
                "operation_panic" => "panicked after physical-work drain",
                "replaced_token" => "backup lock expired or was lost",
                _ => unreachable!(),
            };
            assert!(error.contains(expected), "{error}");
        }
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if runtime.try_lock_exclusive_quiescence().unwrap().is_some()
                && acquire_backup_operation_lock(&root).is_ok()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(std::fs::read(partial).unwrap(), b"physical backup bytes");
    if mode == "replaced_token" {
        assert!(!published.exists());
    }
}

#[tokio::test]
async fn backup_owner_drains_physical_work() {
    for mode in [
        "success",
        "heartbeat_error",
        "operation_error",
        "caller_cancelled",
        "replaced_token",
    ] {
        assert_backup_completion_ownership(mode).await;
    }
}

#[test]
fn backup_owner_drains_queued_physical_work() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap()
        .block_on(assert_backup_completion_ownership("queued"));
}

#[tokio::test]
#[ignore = "requires explicit LLVM backend for actual unwind/destructor execution"]
async fn backup_owner_unwind_drains_physical_work() {
    for mode in ["operation_panic", "heartbeat_panic", "heartbeat_then_panic"] {
        assert_backup_completion_ownership(mode).await;
    }
}

async fn assert_scrub_connection_closes(panic: bool) {
    let directory = tempfile::tempdir().unwrap();
    let mut config = ServerConfig::default();
    config.database_path = directory.path().join("snapshot.sqlite3");
    let database = Database::migrate(&config).await.unwrap();
    database.close_fenced().await.unwrap();
    let error = scrub_transient_backup_state_with_hook(&config.database_path, 1, || {
        if panic {
            panic!("injected scrub panic with active transaction")
        }
        anyhow::bail!("injected scrub error with active transaction")
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains(if panic {
        "scrub panicked"
    } else {
        "injected scrub error"
    }));
    assert!(
        !config
            .database_path
            .with_extension("sqlite3-journal")
            .exists()
    );
    let options = SqliteConnectOptions::new()
        .filename(&config.database_path)
        .busy_timeout(Duration::ZERO);
    let mut reopened = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    sqlx::query("BEGIN EXCLUSIVE")
        .execute(&mut reopened)
        .await
        .unwrap();
    sqlx::query("ROLLBACK")
        .execute(&mut reopened)
        .await
        .unwrap();
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn backup_owner_closes_destination_sqlite_on_error() {
    assert_scrub_connection_closes(false).await;
}

#[tokio::test]
async fn backup_owner_drains_source_sql_before_partial_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = ServerConfig::default();
    config.database_path = directory.path().join("source.sqlite3");
    let database = Database::migrate(&config).await.unwrap();
    let partial = directory
        .path()
        .join(format!(".backup-v4-1-{}.partial", "a".repeat(32)));
    std::fs::create_dir(&partial).unwrap();
    set_private_directory(&partial).await.unwrap();
    let connection = database.pool().acquire().await.unwrap();
    let cleanup = cleanup_failed_partial_backup(
        &database,
        directory.path(),
        &partial,
        anyhow::anyhow!("original backup failure"),
    );
    tokio::pin!(cleanup);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut cleanup)
            .await
            .is_err()
    );
    assert!(
        partial.exists(),
        "partial removed before source SQL returned to idle"
    );
    drop(connection);
    let error = tokio::time::timeout(Duration::from_secs(5), cleanup)
        .await
        .unwrap();
    assert_eq!(error.to_string(), "original backup failure");
    assert!(!partial.exists());
    database.close_fenced().await.unwrap();
}

#[tokio::test]
#[ignore = "requires explicit LLVM backend for actual unwind/destructor execution"]
async fn backup_owner_unwind_closes_destination_sqlite() {
    assert_scrub_connection_closes(true).await;
}

#[tokio::test]
async fn uncertain_gate_release_keeps_exclusive_fence_through_pool_close() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = ServerConfig::default();
    config.database_path = directory.path().join("highscores.sqlite3");
    let database = Database::migrate(&config).await.unwrap();
    let token = database
        .acquire_backup_lock("release-error-test", Duration::from_secs(60))
        .await
        .unwrap();
    let exclusive = acquire_exclusive_backup_database_fence(&database, &token)
        .await
        .unwrap();
    let held_connection = database.pool().acquire().await.unwrap();
    let release_connection = std::sync::Arc::new(tokio::sync::Notify::new());
    let holder_release = release_connection.clone();
    let holder = tokio::spawn(async move {
        holder_release.notified().await;
        drop(held_connection);
    });
    let release_attempted = std::sync::Arc::new(tokio::sync::Notify::new());
    let cleanup_release_attempted = release_attempted.clone();
    let cleanup_database = database.clone();
    let cleanup_token = token.clone();
    let release_database = database.clone();
    let release_token = token.clone();
    let cleanup = tokio::spawn(async move {
        close_pool_under_exclusive_fence_after_release(
            &cleanup_database,
            &cleanup_token,
            &exclusive,
            async move {
                assert!(release_database.release_backup_lock(&release_token).await?);
                cleanup_release_attempted.notify_one();
                Err(robin_highscores::db::DbError::Corrupt(
                    "injected outcome-uncertain post-delete failure".to_owned(),
                ))
            },
        )
        .await
    });
    release_attempted.notified().await;
    assert!(
        database
            .runtime_fence()
            .try_lock_exclusive_quiescence()
            .unwrap()
            .is_none(),
        "cleanup dropped EX while a checked-out SQLx connection remained"
    );
    release_connection.notify_one();
    holder.await.unwrap();
    let error = cleanup.await.unwrap().unwrap_err();
    assert!(format!("{error:#}").contains("exact token is absent after reconciliation"));
    let runtime = database.runtime_fence().clone();
    let admission = runtime.try_lock_exclusive_admission().unwrap().unwrap();
    let quiescence = runtime.try_lock_exclusive_quiescence().unwrap().unwrap();
    runtime
        .validate_exclusive_pair(&admission, &quiescence)
        .unwrap();
}

#[tokio::test]
async fn killed_worker_lease_drains_under_retained_exclusive_fence() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let directory = tempfile::tempdir().unwrap();
    let mut config = ServerConfig::default();
    config.database_path = directory.path().join("highscores.sqlite3");
    let database = Database::migrate(&config).await.unwrap();
    let killed_worker_lease = database
        .acquire_maintenance_write_lease(
            robin_highscores::db::MaintenanceWriteClass::Worker,
            "killed-worker",
            Duration::from_millis(250),
        )
        .await
        .unwrap();
    let live_worker_operation = database
        .runtime_fence()
        .acquire_one_off_shared()
        .await
        .unwrap();
    let backup_lock = database
        .acquire_backup_lock("stale-lease-test", Duration::from_secs(1))
        .await
        .unwrap();
    let initial_backup_expiry: i64 = sqlx::query_scalar(
        "SELECT expires_at_ms FROM maintenance_locks WHERE name = 'backup' AND token = ?",
    )
    .bind(&backup_lock)
    .fetch_one(database.pool())
    .await
    .unwrap();

    let exclusive_acquisition = acquire_exclusive_backup_database_fence(&database, &backup_lock);
    tokio::pin!(exclusive_acquisition);
    tokio::select! {
        _ = &mut exclusive_acquisition => panic!("EX bypassed a live worker operation"),
        () = tokio::time::sleep(Duration::from_millis(50)) => {}
    }
    assert_eq!(
        database
            .active_maintenance_write_lease_count()
            .await
            .unwrap(),
        1,
        "live worker lease vanished while its kernel fence was retained"
    );
    drop(live_worker_operation);
    let exclusive = exclusive_acquisition.await.unwrap();
    assert!(
        database
            .runtime_fence()
            .try_acquire_one_off_shared()
            .unwrap()
            .is_none(),
        "EX admission must reject every new database join during TTL recovery"
    );
    let snapshot_started = AtomicBool::new(false);
    let drain = run_with_backup_lock_heartbeat(&database, &backup_lock, async {
        wait_for_maintenance_writers_with_timing(
            &database,
            &backup_lock,
            Duration::from_secs(2),
            Duration::from_millis(10),
        )
        .await?;
        exclusive.revalidate()?;
        anyhow::ensure!(
            database.active_maintenance_write_lease_count().await? == 0,
            "stale worker lease remained active after its TTL drain"
        );
        snapshot_started.store(true, Ordering::SeqCst);
        Ok::<_, anyhow::Error>(())
    });
    tokio::pin!(drain);
    tokio::select! {
        result = &mut drain => panic!("snapshot crossed a live killed-worker lease: {result:?}"),
        () = tokio::time::sleep(Duration::from_millis(50)) => {}
    }
    assert!(
        !snapshot_started.load(Ordering::SeqCst),
        "snapshot began before the killed worker lease TTL elapsed"
    );
    let refreshed_backup_expiry: i64 = sqlx::query_scalar(
        "SELECT expires_at_ms FROM maintenance_locks WHERE name = 'backup' AND token = ?",
    )
    .bind(&backup_lock)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert!(
        refreshed_backup_expiry > initial_backup_expiry,
        "backup gate heartbeat did not advance during stale-lease recovery"
    );
    drain.await.unwrap();
    assert!(snapshot_started.load(Ordering::SeqCst));
    assert_eq!(
        database
            .active_maintenance_write_lease_count()
            .await
            .unwrap(),
        0
    );
    let stale_expiry: i64 =
        sqlx::query_scalar("SELECT expires_at_ms FROM maintenance_write_leases WHERE token = ?")
            .bind(&killed_worker_lease)
            .fetch_one(database.pool())
            .await
            .unwrap();
    assert!(stale_expiry <= robin_highscores::model::now_epoch_ms().unwrap());
    assert!(
        release_backup_gate_and_close_pool_under_exclusive_fence(
            &database,
            &backup_lock,
            &exclusive,
        )
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn verified_backup_install_is_noreplace_and_reports_parent_sync_failure() {
    let directory = tempfile::tempdir().unwrap();
    let racing_partial = directory.path().join("partial-race");
    let racing_complete = directory.path().join("complete-race");
    tokio::fs::create_dir(&racing_partial).await.unwrap();
    write_private_file(&racing_partial.join("new"), b"new")
        .await
        .unwrap();
    tokio::fs::create_dir(&racing_complete).await.unwrap();
    write_private_file(&racing_complete.join("winner"), b"winner")
        .await
        .unwrap();
    assert!(
        install_verified_partial_with(&racing_partial, &racing_complete, || Ok(())).is_err(),
        "an independently installed completed backup must win the publication race"
    );
    assert!(racing_partial.join("new").is_file());
    assert_eq!(
        tokio::fs::read(racing_complete.join("winner"))
            .await
            .unwrap(),
        b"winner"
    );

    let partial = directory.path().join("partial-parent-sync");
    let complete = directory.path().join("complete-parent-sync");
    tokio::fs::create_dir_all(partial.join("nested"))
        .await
        .unwrap();
    write_private_file(&partial.join("nested/bytes"), b"durable bytes")
        .await
        .unwrap();
    let outcome = install_verified_partial_with(&partial, &complete, || {
        anyhow::bail!("injected parent fsync failure")
    })
    .unwrap();
    assert!(matches!(
        outcome,
        BackupInstallOutcome::InstalledButParentSyncFailed(_)
    ));
    assert!(!partial.exists());
    assert_eq!(
        tokio::fs::read(complete.join("nested/bytes"))
            .await
            .unwrap(),
        b"durable bytes"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn durability_sync_rejects_a_symlink_in_the_verified_tree() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let partial = directory.path().join("partial");
    let complete = directory.path().join("complete");
    tokio::fs::create_dir(&partial).await.unwrap();
    write_private_file(&directory.path().join("outside"), b"outside")
        .await
        .unwrap();
    symlink(
        directory.path().join("outside"),
        partial.join("substituted"),
    )
    .unwrap();
    assert!(install_verified_partial_with(&partial, &complete, || Ok(())).is_err());
    assert!(partial.exists());
    assert!(!complete.exists());
}

#[cfg(unix)]
#[test]
fn status_publication_is_one_atomic_owner_only_file_with_typed_failures() {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let root = tempfile::tempdir().unwrap();
    let status_parent = root.path().join("status");
    std::fs::create_dir(&status_parent).unwrap();
    std::fs::set_permissions(&status_parent, std::fs::Permissions::from_mode(0o700)).unwrap();
    let status = status_parent.join("backup-status.json");
    std::fs::write(&status, b"old-envelope").unwrap();
    std::fs::set_permissions(&status, std::fs::Permissions::from_mode(0o400)).unwrap();

    assert!(
        publish_private_atomic_with_hooks(
            &status,
            b"pre-rename-failure",
            sync_cap_directory,
            || anyhow::bail!("injected pre-rename failure"),
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&status).unwrap(), b"old-envelope");
    assert!(
        std::fs::read_dir(&status_parent)
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp"))
    );

    let durability = publish_private_atomic_with(&status, b"new-envelope", |_| {
        anyhow::bail!("injected parent fsync failure")
    })
    .unwrap();
    assert!(matches!(
        durability,
        StatusPublicationOutcome::PublishedButParentSyncFailed(_)
    ));
    assert_eq!(std::fs::read(&status).unwrap(), b"new-envelope");

    let identity = publish_private_atomic_with(&status, b"authenticated-envelope", |_| {
        std::fs::remove_file(&status)?;
        std::fs::write(&status, b"substituted")?;
        std::fs::set_permissions(&status, std::fs::Permissions::from_mode(0o400))?;
        Ok(())
    })
    .unwrap();
    assert!(matches!(
        identity,
        StatusPublicationOutcome::PublishedButIdentityUncertain(_)
    ));

    let parent_mode_race = publish_private_atomic_with(&status, b"mode-race", |_| {
        std::fs::set_permissions(&status_parent, std::fs::Permissions::from_mode(0o777))?;
        Ok(())
    })
    .unwrap();
    assert!(matches!(
        parent_mode_race,
        StatusPublicationOutcome::PublishedButIdentityUncertain(_)
    ));
    std::fs::set_permissions(&status_parent, std::fs::Permissions::from_mode(0o700)).unwrap();

    let published = publish_private_atomic(&status, b"canonical-envelope").unwrap();
    assert!(matches!(published, StatusPublicationOutcome::Published));
    assert_eq!(std::fs::read(&status).unwrap(), b"canonical-envelope");
    let metadata = std::fs::metadata(&status).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o400);
    assert_eq!(metadata.nlink(), 1);
    assert_eq!(metadata.uid(), rustix::process::geteuid().as_raw());
    std::fs::set_permissions(&status_parent, std::fs::Permissions::from_mode(0o750)).unwrap();
    assert!(publish_private_atomic(&status, b"rejected").is_err());
    assert_eq!(std::fs::read(&status).unwrap(), b"canonical-envelope");
}

#[cfg(unix)]
#[test]
fn status_publication_detects_parent_replacement_after_install() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = tempfile::tempdir().unwrap();
    let status_parent = root.path().join("status");
    let detached_parent = root.path().join("detached-status");
    std::fs::create_dir(&status_parent).unwrap();
    std::fs::set_permissions(&status_parent, std::fs::Permissions::from_mode(0o700)).unwrap();
    let status = status_parent.join("backup-status.json");
    let outcome = publish_private_atomic_with(&status, b"envelope", |_| {
        std::fs::rename(&status_parent, &detached_parent)?;
        std::fs::create_dir(&status_parent)?;
        std::fs::set_permissions(&status_parent, std::fs::Permissions::from_mode(0o700))?;
        Ok(())
    })
    .unwrap();
    assert!(matches!(
        outcome,
        StatusPublicationOutcome::PublishedButIdentityUncertain(_)
    ));
    assert!(!status.exists());
    assert_eq!(
        std::fs::read(detached_parent.join("backup-status.json")).unwrap(),
        b"envelope"
    );
}

#[tokio::test]
async fn publication_is_authenticated_atomic_and_keeps_a_complete_backup() {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    let configuration = directory.path().join("configuration");
    let release_manifest = directory.path().join("vps-release-manifest-v2.json");
    let backup_root = data.join("backups");
    let api_secrets = data.join("api-secrets");
    let status_root = data.join("status");
    let status_path = status_root.join("backup-status.json");
    tokio::fs::create_dir_all(&data).await.unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).unwrap();
    tokio::fs::create_dir(&status_root).await.unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&status_root, std::fs::Permissions::from_mode(0o700)).unwrap();
    tokio::fs::create_dir(&api_secrets).await.unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&api_secrets, std::fs::Permissions::from_mode(0o700)).unwrap();
    tokio::fs::create_dir(&configuration).await.unwrap();
    write_private_file(&configuration.join("server.toml"), b"bind = 'loopback'\n")
        .await
        .unwrap();
    write_private_file(
        &configuration.join("api-moderation.token"),
        b"private-token",
    )
    .await
    .unwrap();
    let release_identity = write_test_release_manifest(&release_manifest).await;

    let mut config = ServerConfig::default();
    config.database_path = data.join("highscores.sqlite3");
    config.replay_directory = data.join("replays");
    config.campaign_state_directory = data.join("campaigns");
    config.cursor_secret_path = data.join("cursor-hmac.key");
    config.backup_authority_hmac_secret_path = api_secrets.join("backup-authority-hmac.key");
    config.competition_run_grant_secret_path = data.join("competition-run-grant.key");
    config.run_preflight_grant_secret_path = data.join("run-preflight-grant.key");
    config.moderation_bearer_token_path = Some(configuration.join("api-moderation.token"));
    config.backup_manifest_path = Some(status_path.clone());
    config.release_manifest_path = Some(release_manifest.clone());
    config.maximum_backup_age_hours = Some(32);
    config.minimum_storage_free_bytes = 64 * 1024 * 1024;
    write_private_file(&config.cursor_secret_path, &[0x31; 32])
        .await
        .unwrap();
    write_private_file(&config.backup_authority_hmac_secret_path, &[0x31; 32])
        .await
        .unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(
        &config.backup_authority_hmac_secret_path,
        std::fs::Permissions::from_mode(0o400),
    )
    .unwrap();
    write_private_file(&config.competition_run_grant_secret_path, &[0x32; 32])
        .await
        .unwrap();
    write_private_file(&config.run_preflight_grant_secret_path, &[0x33; 32])
        .await
        .unwrap();
    Database::migrate(&config).await.unwrap();
    let restore_sources = test_restore_sources(&config, directory.path()).await;
    assert!(
        backup_and_publish_status(
            &config,
            &release_manifest,
            &release_identity,
            &backup_root,
            &status_path,
            2,
            &restore_sources,
        )
        .await
        .is_err(),
        "backup authority must reject a missing production backup root"
    );
    assert!(
        !backup_root.exists(),
        "backup authority must not provision a missing production backup root"
    );
    tokio::fs::create_dir(&backup_root).await.unwrap();
    #[cfg(unix)]
    {
        std::fs::set_permissions(&backup_root, std::fs::Permissions::from_mode(0o750)).unwrap();
        assert!(
            backup_and_publish_status(
                &config,
                &release_manifest,
                &release_identity,
                &backup_root,
                &status_path,
                2,
                &restore_sources,
            )
            .await
            .is_err(),
            "backup authority must reject a misprovisioned backup root"
        );
        assert_eq!(
            std::fs::metadata(&backup_root)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o750,
            "backup authority must not repair production root metadata"
        );
        std::fs::set_permissions(&backup_root, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let admission_partial = backup_root.join(format!(".backup-v4-3-{}.partial", "c".repeat(32)));
    tokio::fs::create_dir(&admission_partial).await.unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&admission_partial, std::fs::Permissions::from_mode(0o700)).unwrap();
    write_private_file(&admission_partial.join("must-remain"), b"pre-admission")
        .await
        .unwrap();
    #[cfg(unix)]
    {
        std::fs::set_permissions(&status_root, std::fs::Permissions::from_mode(0o750)).unwrap();
        assert!(
            backup_and_publish_status(
                &config,
                &release_manifest,
                &release_identity,
                &backup_root,
                &status_path,
                2,
                &restore_sources,
            )
            .await
            .is_err(),
            "a non-0700 status authority parent must fail before cleanup"
        );
        assert!(admission_partial.join("must-remain").is_file());
        std::fs::set_permissions(&status_root, std::fs::Permissions::from_mode(0o700)).unwrap();

        std::fs::write(&status_path, b"{}").unwrap();
        std::fs::set_permissions(&status_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            backup_and_publish_status(
                &config,
                &release_manifest,
                &release_identity,
                &backup_root,
                &status_path,
                2,
                &restore_sources,
            )
            .await
            .is_err(),
            "a non-0400 status authority must fail before cleanup"
        );
        assert!(admission_partial.join("must-remain").is_file());
        std::fs::remove_file(&status_path).unwrap();
    }
    let mutable_estimate = estimate_backup_space(
        &config,
        &release_identity,
        &backup_root,
        &status_path,
        &restore_sources,
    )
    .await
    .unwrap();
    let estimate_bytes = canonical_json_bytes(&mutable_estimate).unwrap();
    assert_eq!(
        serde_json::from_slice::<BackupSpaceEstimateV1>(&estimate_bytes).unwrap(),
        mutable_estimate,
        "deployment evidence must round-trip as one canonical typed document"
    );
    assert!(
        mutable_estimate.manifest_logical_upper_bound_bytes > 0
            && mutable_estimate.status_temp_logical_upper_bound_bytes
                < mutable_estimate.manifest_logical_upper_bound_bytes
            && mutable_estimate.status_temp_logical_upper_bound_bytes
                <= u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES).unwrap(),
        "the estimator must size the full payload manifest and compact status independently"
    );
    assert_eq!(
        mutable_estimate.concurrent_database_margin_bytes,
        round_up_to_allocation(
            robin_highscores::storage_admission::maximum_capacity_demand_bytes(&config)
                .unwrap()
                .database,
            mutable_estimate.allocation_granularity_bytes,
        )
        .unwrap()
    );
    assert_eq!(
        mutable_estimate.concurrent_object_margin_bytes,
        u64::try_from(config.max_concurrent_uploads).unwrap()
            * round_up_to_allocation(
                config.max_replay_bytes,
                mutable_estimate.allocation_granularity_bytes,
            )
            .unwrap()
            + (u64::try_from(config.max_concurrent_uploads).unwrap() + 1)
                * round_up_to_allocation(
                    config.max_campaign_bytes,
                    mutable_estimate.allocation_granularity_bytes,
                )
                .unwrap(),
        "every concurrently admitted replay/campaign pair must fit after the scan"
    );
    assert_eq!(
        mutable_estimate.restore_source_map_count,
        u64::try_from(restore_sources.len()).unwrap()
    );
    assert_eq!(
        mutable_estimate.required_scratch_bytes,
        mutable_estimate.dense_payload_bytes
            + mutable_estimate.directory_and_entry_overhead_bytes
            + mutable_estimate.manifest_allocation_upper_bound_bytes
            + mutable_estimate.status_temp_allocation_upper_bound_bytes
            + mutable_estimate.concurrent_object_margin_bytes
            + mutable_estimate.concurrent_database_margin_bytes,
        "one scratch generation must include exact documents plus bounded in-flight growth"
    );
    let mut insufficient = mutable_estimate.clone();
    insufficient.observed_available_bytes = insufficient.required_available_bytes - 1;
    assert!(insufficient.ensure_available().is_err());
    let mut inode_pressure = mutable_estimate.clone();
    inode_pressure.observed_available_inode_count = inode_pressure.required_inode_count - 1;
    assert!(inode_pressure.ensure_available().is_err());
    assert_eq!(
        std::fs::read_dir(&backup_root).unwrap().count(),
        1,
        "capacity rejection must not create anything beyond the pre-admission partial"
    );
    assert!(admission_partial.join("must-remain").is_file());
    let immutable_release_bytes = directory.path().join("immutable-release");
    tokio::fs::create_dir_all(immutable_release_bytes.join("static/datadir"))
        .await
        .unwrap();
    write_private_file(
        &immutable_release_bytes.join("static/datadir/not-a-backup-input"),
        &vec![0x5a; 128 * 1024],
    )
    .await
    .unwrap();
    config.manifest_directory = Some(immutable_release_bytes.join("manifests"));
    let estimate_with_immutable_release = estimate_backup_space(
        &config,
        &release_identity,
        &backup_root,
        &status_path,
        &restore_sources,
    )
    .await
    .unwrap();
    config.manifest_directory = None;
    let contemporaneous_without_immutable_release = estimate_backup_space(
        &config,
        &release_identity,
        &backup_root,
        &status_path,
        &restore_sources,
    )
    .await
    .unwrap();
    assert_eq!(
        estimate_with_immutable_release.required_scratch_bytes,
        contemporaneous_without_immutable_release.required_scratch_bytes,
        "release/static/datadir and manifest roots must not consume mutable backup capacity"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let unit_root = directory.path().join("installed-user-units");
        tokio::fs::create_dir(unit_root.join("default.target.wants"))
            .await
            .unwrap();
        tokio::fs::create_dir(unit_root.join("timers.target.wants"))
            .await
            .unwrap();
        symlink(
            unit_root.join("robin-highscores.target"),
            unit_root
                .join("default.target.wants")
                .join("robin-highscores.target"),
        )
        .unwrap();
        symlink(
            unit_root.join("robin-highscores-backup.timer"),
            unit_root
                .join("timers.target.wants")
                .join("robin-highscores-backup.timer"),
        )
        .unwrap();
        write_private_file(
            &unit_root.join("unrelated.service"),
            b"must not be archived",
        )
        .await
        .unwrap();
    }

    assert!(!status_path.exists());
    assert!(
        backup_and_publish_status(
            &config,
            &release_manifest,
            &release_identity,
            &backup_root,
            &status_path,
            0,
            &restore_sources,
        )
        .await
        .is_err()
    );
    assert!(!status_path.exists());

    let stale_partial = backup_root.join(format!(".backup-v4-1-{}.partial", "a".repeat(32)));
    tokio::fs::create_dir_all(stale_partial.join("restore/state"))
        .await
        .unwrap();
    write_private_file(&stale_partial.join("restore/state/interrupted"), b"sigkill")
        .await
        .unwrap();
    #[cfg(unix)]
    {
        std::fs::set_permissions(&stale_partial, std::fs::Permissions::from_mode(0o500)).unwrap();
        std::fs::set_permissions(
            stale_partial.join("restore"),
            std::fs::Permissions::from_mode(0o500),
        )
        .unwrap();
        std::fs::set_permissions(
            stale_partial.join("restore/state"),
            std::fs::Permissions::from_mode(0o500),
        )
        .unwrap();

        let key_path = config.backup_authority_hmac_secret_path.clone();
        let displaced_key = api_secrets.join("backup-authority-hmac.displaced");
        let swap_key_path = key_path.clone();
        let swap_displaced_key = displaced_key.clone();
        assert!(
            backup_and_publish_status_with_limit_and_publisher_and_hooks(
                &config,
                &release_manifest,
                &release_identity,
                &backup_root,
                &status_path,
                2,
                &restore_sources,
                robin_highscores::backup::MAX_BACKUP_STATUS_BYTES,
                publish_private_atomic,
                move || {
                    std::fs::rename(&swap_key_path, &swap_displaced_key)?;
                    std::fs::write(&swap_key_path, [0x41; 32])?;
                    std::fs::set_permissions(
                        &swap_key_path,
                        std::fs::Permissions::from_mode(0o400),
                    )?;
                    Ok(())
                },
                || Ok(()),
            )
            .await
            .is_err(),
            "a pathname replacement of the pinned key must fail before backup installation"
        );
        assert!(!status_path.exists());
        assert!(
            std::fs::read_dir(&backup_root).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("backup-v4-")
            }),
            "a key swap before install must not leave a completed generation"
        );
        std::fs::remove_file(&key_path).unwrap();
        std::fs::rename(&displaced_key, &key_path).unwrap();

        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let key_mutator = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&key_path)
            .unwrap();
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o400)).unwrap();
        let mutation_handle = key_mutator.try_clone().unwrap();
        assert!(
            backup_and_publish_status_with_limit_and_publisher_and_hooks(
                &config,
                &release_manifest,
                &release_identity,
                &backup_root,
                &status_path,
                2,
                &restore_sources,
                robin_highscores::backup::MAX_BACKUP_STATUS_BYTES,
                publish_private_atomic,
                || Ok(()),
                move || {
                    use std::os::unix::fs::FileExt as _;
                    mutation_handle.write_all_at(&[0x42; 32], 0)?;
                    mutation_handle.sync_all()?;
                    Ok(())
                },
            )
            .await
            .is_err(),
            "an in-place key mutation must fail before status publication"
        );
        assert!(!status_path.exists());
        assert!(
            std::fs::read_dir(&backup_root).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("backup-v4-")
            }),
            "a key mutation before status publication must remove the unreferenced generation"
        );
        {
            use std::os::unix::fs::FileExt as _;
            key_mutator.write_all_at(&[0x31; 32], 0).unwrap();
            key_mutator.sync_all().unwrap();
        }
    }

    let first = backup_and_publish_status(
        &config,
        &release_manifest,
        &release_identity,
        &backup_root,
        &status_path,
        2,
        &restore_sources,
    )
    .await
    .unwrap();
    assert!(!stale_partial.exists());
    assert!(!admission_partial.exists());
    assert!(first.is_dir());
    let status_bytes = tokio::fs::read(&status_path).await.unwrap();
    let status: BackupStatusV4 = serde_json::from_slice(&status_bytes).unwrap();
    assert_eq!(canonical_json_bytes(&status).unwrap(), status_bytes);
    status.verify(&[0x31; 32]).unwrap();
    assert_eq!(status.release_identity, release_identity);
    assert_eq!(Path::new(&status.backup_directory), first);
    assert!(!status_root.join("backup-manifest.json").exists());
    verify_backup(&first).await.unwrap();
    let first_manifest: BackupManifest = serde_json::from_slice(
        &tokio::fs::read(first.join("backup-manifest.json"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        status.backup_manifest_sha256,
        first_manifest.sha256().unwrap()
    );
    assert_eq!(
        status.database_schema_version,
        first_manifest.database_schema_version
    );
    assert_eq!(status.file_count, first_manifest.files.len() as u64);
    assert_eq!(status.total_bytes, first_manifest.total_bytes().unwrap());
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd as _;

        let backup_directory_file = std::fs::File::open(&first).unwrap();
        let status_file = std::fs::File::open(&status_path).unwrap();
        let release_file = std::fs::File::open(&release_manifest).unwrap();
        assert_eq!(
            load_backup_release_identity_oob_file(release_file)
                .await
                .unwrap(),
            release_identity
        );
        let release_hardlink = directory.path().join("release-manifest-hardlink.json");
        std::fs::hard_link(&release_manifest, &release_hardlink).unwrap();
        assert!(
            load_backup_release_identity_oob_file(std::fs::File::open(&release_manifest).unwrap())
                .await
                .is_err(),
            "a hard-linked release authority must be rejected"
        );
        std::fs::remove_file(release_hardlink).unwrap();
        std::fs::set_permissions(&release_manifest, std::fs::Permissions::from_mode(0o400))
            .unwrap();
        assert!(
            load_backup_release_identity_oob_file(std::fs::File::open(&release_manifest).unwrap())
                .await
                .is_err(),
            "a release authority with the wrong mode must be rejected"
        );
        std::fs::set_permissions(&release_manifest, std::fs::Permissions::from_mode(0o440))
            .unwrap();
        let wrong_release_name = directory.path().join("not-the-release-manifest.json");
        std::fs::copy(&release_manifest, &wrong_release_name).unwrap();
        std::fs::set_permissions(&wrong_release_name, std::fs::Permissions::from_mode(0o440))
            .unwrap();
        assert!(
            require_pinned_file_name(
                &std::fs::File::open(wrong_release_name).unwrap(),
                "vps-release-manifest-v2.json",
            )
            .is_err(),
            "the out-of-band release descriptor must retain its canonical filename"
        );
        let verification = verify_backup_pinned_with_expected(
            u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
            u32::try_from(status_file.as_raw_fd()).unwrap(),
            &first_manifest.sha256().unwrap(),
            &release_identity,
            &backup_root,
            &status_path,
            false,
        )
        .await
        .unwrap();
        let receipt = &(*verification.receipt());
        receipt.validate().unwrap();
        assert_eq!(receipt.backup_id, status.backup_id);
        assert_eq!(receipt.backup_directory, status.backup_directory);
        let receipt_bytes = canonical_json_bytes(&receipt).unwrap();
        assert!(!receipt_bytes.ends_with(b"\n"));
        assert_eq!(
            canonical_json_bytes(
                &serde_json::from_slice::<BackupVerificationReceiptV2>(&receipt_bytes).unwrap()
            )
            .unwrap(),
            receipt_bytes,
            "successful verifier stdout is an exact canonical receipt"
        );
        let explicit_receipt = receipt.clone();
        drop(verification);

        let historical = verify_backup_pinned_offline_with_expected(
            u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
            &first_manifest.sha256().unwrap(),
            &release_identity,
            &[0x31; 32],
            &backup_root,
            false,
        )
        .await
        .unwrap();
        assert!(
            (*historical.receipt()).current_status.is_none(),
            "historical verification must be independent of singleton latest status"
        );
        drop(historical);

        let backup_root_file = std::fs::File::open(&backup_root).unwrap();
        let (transaction_verification, transaction_root_guard) =
            verify_transaction_backup_from_root_pinned(
                u32::try_from(backup_root_file.as_raw_fd()).unwrap(),
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                &release_identity,
                &[0x31; 32],
                &backup_root,
                &status_path,
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            (*transaction_verification.receipt()),
            explicit_receipt,
            "transaction mode derives exactly the authenticated status manifest digest"
        );
        drop(transaction_verification);
        drop(transaction_root_guard);

        let wrong_root = directory.path().join("wrong-parent/backups");
        std::fs::create_dir_all(&wrong_root).unwrap();
        std::fs::set_permissions(&wrong_root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let wrong_root_file = std::fs::File::open(&wrong_root).unwrap();
        assert!(
            pin_transaction_backup_from_status(
                u32::try_from(wrong_root_file.as_raw_fd()).unwrap(),
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                &backup_root,
                &status_path,
            )
            .is_err(),
            "a same-basename backup root outside the canonical authority must be rejected"
        );

        let malicious_status_root = directory.path().join("malicious-path-status");
        std::fs::create_dir(&malicious_status_root).unwrap();
        let malicious_status_path = malicious_status_root.join("backup-status.json");
        let mut malicious_status = status.clone();
        malicious_status.backup_directory = directory
            .path()
            .join("outside")
            .join(&status.backup_id)
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            &malicious_status_path,
            canonical_json_bytes(&malicious_status).unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(
            &malicious_status_path,
            std::fs::Permissions::from_mode(0o400),
        )
        .unwrap();
        let malicious_status_file = std::fs::File::open(&malicious_status_path).unwrap();
        assert!(
            pin_transaction_backup_from_status(
                u32::try_from(backup_root_file.as_raw_fd()).unwrap(),
                u32::try_from(malicious_status_file.as_raw_fd()).unwrap(),
                &backup_root,
                &malicious_status_path,
            )
            .is_err(),
            "an embedded backup path outside canonical root/ID must never be opened"
        );

        let canonical_lock = backup_root.join(".backup-operation.lock");
        let displaced_lock = backup_root.join(".displaced-backup-operation.lock");
        let second_lock = std::sync::Mutex::new(None);
        assert!(
            verify_backup_pinned_with_expected_and_hook(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                BackupManifestExpectation::Explicit(&first_manifest.sha256().unwrap()),
                &release_identity,
                &backup_root,
                &status_path,
                false,
                || {
                    std::fs::rename(&canonical_lock, &displaced_lock)?;
                    *second_lock.lock().unwrap() =
                        Some(acquire_backup_operation_lock(&backup_root)?);
                    Ok(())
                },
            )
            .await
            .is_err(),
            "a replaced lock domain must not yield a receipt"
        );
        let replacement_lock = second_lock.into_inner().unwrap();
        assert!(
            replacement_lock.is_some(),
            "the replacement inode demonstrates a second concurrently acquirable lock domain"
        );
        drop(replacement_lock);
        std::fs::remove_file(&canonical_lock).unwrap();
        std::fs::rename(&displaced_lock, &canonical_lock).unwrap();

        let displaced_status = status_root.join("displaced-backup-status.json");
        assert!(
            verify_backup_pinned_with_expected_and_hook(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                BackupManifestExpectation::Explicit(&first_manifest.sha256().unwrap()),
                &release_identity,
                &backup_root,
                &status_path,
                false,
                || {
                    std::fs::rename(&status_path, &displaced_status)?;
                    std::fs::write(&status_path, &status_bytes)?;
                    std::fs::set_permissions(&status_path, std::fs::Permissions::from_mode(0o400))?;
                    Ok(())
                },
            )
            .await
            .is_err(),
            "a concurrently replaced status path must not yield a receipt"
        );
        std::fs::remove_file(&status_path).unwrap();
        std::fs::rename(&displaced_status, &status_path).unwrap();

        let displaced_backup = backup_root.join("displaced-complete-backup");
        assert!(
            verify_backup_pinned_with_expected_and_hook(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                BackupManifestExpectation::Explicit(&first_manifest.sha256().unwrap()),
                &release_identity,
                &backup_root,
                &status_path,
                false,
                || {
                    std::fs::rename(&first, &displaced_backup)?;
                    std::fs::create_dir(&first)?;
                    std::fs::set_permissions(&first, std::fs::Permissions::from_mode(0o700))?;
                    Ok(())
                },
            )
            .await
            .is_err(),
            "a concurrently pruned or replaced backup path must not yield a receipt"
        );
        std::fs::remove_dir(&first).unwrap();
        std::fs::rename(&displaced_backup, &first).unwrap();

        for (relative, expected_mode, label) in [
            ("restore/state/cursor-hmac.key", 0o600, "payload"),
            ("backup-manifest.json", 0o600, "manifest"),
            (
                "backup-verification-envelope.json",
                0o400,
                "verification envelope",
            ),
        ] {
            let original = first.join(relative);
            let displaced = backup_root.join(format!("displaced-{}", relative.replace('/', "-")));
            let bytes = std::fs::read(&original).unwrap();
            assert!(
                verify_backup_pinned_with_expected_and_hook(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    BackupManifestExpectation::Explicit(&first_manifest.sha256().unwrap()),
                    &release_identity,
                    &backup_root,
                    &status_path,
                    false,
                    || {
                        std::fs::rename(&original, &displaced)?;
                        std::fs::write(&original, &bytes)?;
                        std::fs::set_permissions(
                            &original,
                            std::fs::Permissions::from_mode(expected_mode),
                        )?;
                        Ok(())
                    },
                )
                .await
                .is_err(),
                "an identical-byte {label} inode substitution must not yield a receipt"
            );
            std::fs::remove_file(&original).unwrap();
            std::fs::rename(&displaced, &original).unwrap();
        }

        let empty_directory = first.join("replays");
        assert_eq!(std::fs::read_dir(&empty_directory).unwrap().count(), 0);
        let displaced_empty_directory = backup_root.join("displaced-empty-replays");
        assert!(
            verify_backup_pinned_with_expected_and_hook(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                BackupManifestExpectation::Explicit(&first_manifest.sha256().unwrap()),
                &release_identity,
                &backup_root,
                &status_path,
                false,
                || {
                    std::fs::rename(&empty_directory, &displaced_empty_directory)?;
                    std::fs::create_dir(&empty_directory)?;
                    std::fs::set_permissions(
                        &empty_directory,
                        std::fs::Permissions::from_mode(0o700),
                    )?;
                    Ok(())
                },
            )
            .await
            .is_err(),
            "an identical empty-directory inode substitution must not yield a receipt"
        );
        std::fs::remove_dir(&empty_directory).unwrap();
        std::fs::rename(&displaced_empty_directory, &empty_directory).unwrap();

        let original_database = first.join("highscores.sqlite3");
        let displaced_database = backup_root.join("displaced-backup-database");
        let prepared_database = backup_root.join("prepared-backup-database");
        std::fs::copy(&original_database, &prepared_database).unwrap();
        std::fs::set_permissions(&prepared_database, std::fs::Permissions::from_mode(0o600))
            .unwrap();
        let prepared_url = format!("sqlite://{}", prepared_database.display());
        let mut prepared_connection = sqlx::SqliteConnection::connect(&prepared_url)
            .await
            .unwrap();
        sqlx::query("PRAGMA user_version = 17")
            .execute(&mut prepared_connection)
            .await
            .unwrap();
        prepared_connection.close().await.unwrap();
        remove_test_database_sidecars(&prepared_database).await;
        assert!(
            verify_backup_pinned_with_expected_and_hook(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                BackupManifestExpectation::Explicit(&first_manifest.sha256().unwrap()),
                &release_identity,
                &backup_root,
                &status_path,
                false,
                || {
                    std::fs::rename(&original_database, &displaced_database)?;
                    std::fs::rename(&prepared_database, &original_database)?;
                    Ok(())
                },
            )
            .await
            .is_err(),
            "a different-byte valid-schema database substitution must not yield a receipt"
        );
        std::fs::remove_file(&original_database).unwrap();
        std::fs::rename(&displaced_database, &original_database).unwrap();

        let release_guard = std::fs::File::open(&release_manifest).unwrap();
        let displaced_release = directory.path().join("displaced-release-manifest.json");
        let release_bytes = std::fs::read(&release_manifest).unwrap();
        std::fs::rename(&release_manifest, &displaced_release).unwrap();
        std::fs::write(&release_manifest, &release_bytes).unwrap();
        std::fs::set_permissions(&release_manifest, std::fs::Permissions::from_mode(0o440))
            .unwrap();
        assert!(
            revalidate_pinned_regular_path(
                &release_guard,
                &release_manifest,
                0o440,
                None,
                "out-of-band release manifest",
            )
            .is_err(),
            "a concurrently replaced release authority must not precede receipt stdout"
        );
        std::fs::remove_file(&release_manifest).unwrap();
        std::fs::rename(displaced_release, &release_manifest).unwrap();

        assert!(
            verify_backup_pinned_with_expected(
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                &first_manifest.sha256().unwrap(),
                &release_identity,
                &backup_root,
                &status_path,
                false,
            )
            .await
            .is_err(),
            "a regular file cannot substitute the pinned backup directory"
        );
        assert!(
            verify_backup_pinned_with_expected(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                &first_manifest.sha256().unwrap(),
                &release_identity,
                &backup_root,
                &status_path,
                false,
            )
            .await
            .is_err(),
            "a directory cannot substitute the pinned status envelope"
        );

        let wrong_name_path = status_root.join("not-backup-status.json");
        tokio::fs::write(&wrong_name_path, &status_bytes)
            .await
            .unwrap();
        std::fs::set_permissions(&wrong_name_path, std::fs::Permissions::from_mode(0o400)).unwrap();
        let wrong_name_file = std::fs::File::open(&wrong_name_path).unwrap();
        assert!(
            verify_backup_pinned_with_expected(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(wrong_name_file.as_raw_fd()).unwrap(),
                &first_manifest.sha256().unwrap(),
                &release_identity,
                &backup_root,
                &wrong_name_path,
                false,
            )
            .await
            .is_err(),
            "a differently named status descriptor must be rejected"
        );

        let wrong_mode_root = directory.path().join("wrong-mode-status");
        tokio::fs::create_dir(&wrong_mode_root).await.unwrap();
        let wrong_mode_path = wrong_mode_root.join("backup-status.json");
        tokio::fs::write(&wrong_mode_path, &status_bytes)
            .await
            .unwrap();
        std::fs::set_permissions(&wrong_mode_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let wrong_mode_file = std::fs::File::open(&wrong_mode_path).unwrap();
        assert!(
            verify_backup_pinned_with_expected(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(wrong_mode_file.as_raw_fd()).unwrap(),
                &first_manifest.sha256().unwrap(),
                &release_identity,
                &backup_root,
                &wrong_mode_path,
                false,
            )
            .await
            .is_err(),
            "a non-0400 status descriptor must be rejected"
        );

        let noncanonical_root = directory.path().join("noncanonical-status");
        tokio::fs::create_dir(&noncanonical_root).await.unwrap();
        let noncanonical_path = noncanonical_root.join("backup-status.json");
        let mut noncanonical_bytes = status_bytes.clone();
        noncanonical_bytes.push(b'\n');
        tokio::fs::write(&noncanonical_path, noncanonical_bytes)
            .await
            .unwrap();
        std::fs::set_permissions(&noncanonical_path, std::fs::Permissions::from_mode(0o400))
            .unwrap();
        let noncanonical_file = std::fs::File::open(&noncanonical_path).unwrap();
        assert!(
            verify_backup_pinned_with_expected(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(noncanonical_file.as_raw_fd()).unwrap(),
                &first_manifest.sha256().unwrap(),
                &release_identity,
                &backup_root,
                &noncanonical_path,
                false,
            )
            .await
            .is_err(),
            "a noncanonical status document must be rejected"
        );

        let same_name_root = directory.path().join("substituted-root");
        let same_name_directory = same_name_root.join(&status.backup_id);
        tokio::fs::create_dir_all(&same_name_directory)
            .await
            .unwrap();
        std::fs::set_permissions(&same_name_directory, std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let same_name_file = std::fs::File::open(&same_name_directory).unwrap();
        assert!(
            verify_backup_pinned_with_expected(
                u32::try_from(same_name_file.as_raw_fd()).unwrap(),
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                &first_manifest.sha256().unwrap(),
                &release_identity,
                &backup_root,
                &status_path,
                false,
            )
            .await
            .is_err(),
            "matching basenames cannot substitute a different pinned backup root"
        );

        let deleted_status_root = directory.path().join("deleted-status");
        tokio::fs::create_dir(&deleted_status_root).await.unwrap();
        let deleted_status_path = deleted_status_root.join("backup-status.json");
        tokio::fs::write(&deleted_status_path, &status_bytes)
            .await
            .unwrap();
        std::fs::set_permissions(&deleted_status_path, std::fs::Permissions::from_mode(0o400))
            .unwrap();
        let deleted_status_file = std::fs::File::open(&deleted_status_path).unwrap();
        tokio::fs::remove_file(&deleted_status_path).await.unwrap();
        assert!(
            verify_backup_pinned_with_expected(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(deleted_status_file.as_raw_fd()).unwrap(),
                &first_manifest.sha256().unwrap(),
                &release_identity,
                &backup_root,
                &deleted_status_path,
                false,
            )
            .await
            .is_err(),
            "an unlinked status-envelope descriptor has no durable path identity"
        );

        let status_hardlink = status_root.join("backup-status-hardlink");
        std::fs::hard_link(&status_path, &status_hardlink).unwrap();
        assert!(
            verify_backup_pinned_with_expected(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                &first_manifest.sha256().unwrap(),
                &release_identity,
                &backup_root,
                &status_path,
                false,
            )
            .await
            .is_err(),
            "a hard-linked status authority must be rejected"
        );
        std::fs::remove_file(status_hardlink).unwrap();

        let bad_status_root = directory.path().join("bad-status");
        tokio::fs::create_dir(&bad_status_root).await.unwrap();
        let bad_status_path = bad_status_root.join("backup-status.json");
        let bad_status = BackupStatusV4::new_authenticated(
            status.backup_id.clone(),
            status.backup_directory.clone(),
            first_manifest.clone(),
            &[0x99; 32],
        )
        .unwrap();
        tokio::fs::write(&bad_status_path, canonical_json_bytes(&bad_status).unwrap())
            .await
            .unwrap();
        std::fs::set_permissions(&bad_status_path, std::fs::Permissions::from_mode(0o400)).unwrap();
        let bad_status_file = std::fs::File::open(&bad_status_path).unwrap();
        assert!(
            verify_backup_pinned_with_expected(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(bad_status_file.as_raw_fd()).unwrap(),
                &first_manifest.sha256().unwrap(),
                &release_identity,
                &backup_root,
                &status_path,
                false,
            )
            .await
            .is_err(),
            "the status descriptor must come from the exact trusted parent"
        );
        assert!(
            verify_backup_pinned_with_expected(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(bad_status_file.as_raw_fd()).unwrap(),
                &first_manifest.sha256().unwrap(),
                &release_identity,
                &backup_root,
                &bad_status_path,
                false,
            )
            .await
            .is_err(),
            "a status envelope authenticated by a different key must be rejected"
        );

        let mut wrong_schema_manifest = first_manifest.clone();
        wrong_schema_manifest.database_schema_version += 1;
        assert!(
            BackupStatusV4::new_authenticated(
                status.backup_id.clone(),
                status.backup_directory.clone(),
                wrong_schema_manifest,
                &[0x31; 32],
            )
            .is_err(),
            "status authoring must reject a database/release schema mismatch"
        );

        assert!(
            verify_backup_pinned_with_expected(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                &"ab".repeat(32),
                &release_identity,
                &backup_root,
                &status_path,
                false,
            )
            .await
            .is_err(),
            "the pinned payload must match the independently expected manifest digest"
        );

        let mut wrong_release = release_identity.clone();
        wrong_release.source_commit = "abcdef0123456789abcdef0123456789abcdef01".to_owned();
        assert!(
            verify_backup_pinned_with_expected(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                &first_manifest.sha256().unwrap(),
                &wrong_release,
                &backup_root,
                &status_path,
                false,
            )
            .await
            .is_err(),
            "the pinned payload must match the out-of-band release identity"
        );

        std::fs::set_permissions(&first, std::fs::Permissions::from_mode(0o750)).unwrap();
        assert!(
            verify_backup_pinned_with_expected(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                &first_manifest.sha256().unwrap(),
                &release_identity,
                &backup_root,
                &status_path,
                false,
            )
            .await
            .is_err(),
            "a non-0700 pinned backup root must be rejected"
        );
        std::fs::set_permissions(&first, std::fs::Permissions::from_mode(0o700)).unwrap();

        let missing_path = first.join("restore/state/cursor-hmac.key");
        let hidden_path = first.join("restore/state/cursor-hmac.key.missing");
        std::fs::rename(&missing_path, &hidden_path).unwrap();
        assert!(
            verify_backup_pinned_with_expected(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                &first_manifest.sha256().unwrap(),
                &release_identity,
                &backup_root,
                &status_path,
                false,
            )
            .await
            .is_err(),
            "a missing pinned payload object must be rejected"
        );
        std::fs::rename(hidden_path, missing_path).unwrap();

        let unexpected_path = first.join("restore/state/unexpected");
        write_private_file(&unexpected_path, b"unexpected")
            .await
            .unwrap();
        assert!(
            verify_backup_pinned_with_expected(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                &first_manifest.sha256().unwrap(),
                &release_identity,
                &backup_root,
                &status_path,
                false,
            )
            .await
            .is_err(),
            "an unexpected pinned payload object must be rejected"
        );
        std::fs::remove_file(unexpected_path).unwrap();
    }
    let archived_units = first_manifest
        .files
        .iter()
        .filter(|file| file.relative_path.starts_with("restore/systemd/user/"))
        .map(|file| file.relative_path.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(archived_units.len(), SYSTEMD_UNIT_FILES.len());
    assert!(
        archived_units
            .iter()
            .all(|path| !path.contains("wants") && !path.contains("unrelated"))
    );
    assert!(first_manifest.files.iter().all(|file| {
        !file.relative_path.contains("release")
            && !file.relative_path.contains("static")
            && !file.relative_path.contains("datadir")
            && !file.relative_path.contains("manifest")
    }));

    let complete_names_before_publication_failure = std::fs::read_dir(&backup_root)
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| parse_backup_id(name).is_some())
        .collect::<BTreeSet<_>>();
    let status_before_publication_failure = tokio::fs::read(&status_path).await.unwrap();
    assert!(
        backup_and_publish_status_with_limit_and_publisher(
            &config,
            &release_manifest,
            &release_identity,
            &backup_root,
            &status_path,
            2,
            &restore_sources,
            robin_highscores::backup::MAX_BACKUP_STATUS_BYTES,
            |_, _| anyhow::bail!("injected definite pre-rename publication failure"),
        )
        .await
        .is_err()
    );
    assert_eq!(
        tokio::fs::read(&status_path).await.unwrap(),
        status_before_publication_failure,
        "a definite pre-rename error must leave the old envelope intact"
    );
    assert_eq!(
        std::fs::read_dir(&backup_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| parse_backup_id(name).is_some())
            .collect::<BTreeSet<_>>(),
        complete_names_before_publication_failure,
        "a definite status error must not accumulate an unreferenced complete backup"
    );

    let status_before_oversize = tokio::fs::read(&status_path).await.unwrap();
    assert!(
        backup_and_publish_status_with_limit(
            &config,
            &release_manifest,
            &release_identity,
            &backup_root,
            &status_path,
            2,
            &restore_sources,
            1,
        )
        .await
        .is_err(),
        "an unpublishable envelope must fail before partial installation"
    );
    assert_eq!(
        tokio::fs::read(&status_path).await.unwrap(),
        status_before_oversize,
        "an oversized candidate must not replace the old readiness envelope"
    );
    assert!(
        std::fs::read_dir(&backup_root)
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().ends_with(".partial")),
        "an oversized candidate must leave no partial backup"
    );

    let second = backup_and_publish_status(
        &config,
        &release_manifest,
        &release_identity,
        &backup_root,
        &status_path,
        2,
        &restore_sources,
    )
    .await
    .unwrap();
    assert!(second.is_dir());
    assert_ne!(first, second);
    assert!(first.exists());
    verify_backup(&second).await.unwrap();
    let two_complete_names = std::fs::read_dir(&backup_root)
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| parse_backup_id(name).is_some())
        .collect::<BTreeSet<_>>();
    assert_eq!(two_complete_names.len(), 2);
    assert!(
        backup_and_publish_status_with_limit_and_publisher(
            &config,
            &release_manifest,
            &release_identity,
            &backup_root,
            &status_path,
            2,
            &restore_sources,
            robin_highscores::backup::MAX_BACKUP_STATUS_BYTES,
            |_, _| anyhow::bail!("injected failed replacement before status publication"),
        )
        .await
        .is_err()
    );
    assert_eq!(
        std::fs::read_dir(&backup_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| parse_backup_id(name).is_some())
            .collect::<BTreeSet<_>>(),
        two_complete_names,
        "a failed replacement must preserve both retained complete generations"
    );

    assert!(
        backup_and_publish_status_with_limit_and_publisher(
            &config,
            &release_manifest,
            &release_identity,
            &backup_root,
            &status_path,
            2,
            &restore_sources,
            robin_highscores::backup::MAX_BACKUP_STATUS_BYTES,
            |path, bytes| match publish_private_atomic(path, bytes)? {
                StatusPublicationOutcome::Published => {
                    Ok(StatusPublicationOutcome::PublishedButIdentityUncertain(
                        anyhow::anyhow!(
                            "injected crash after status publication and before retention"
                        ),
                    ))
                }
                uncertain => Ok(uncertain),
            },
        )
        .await
        .is_err(),
        "publication uncertainty must preserve the truthful newly published generation"
    );
    let crash_status: BackupStatusV4 =
        serde_json::from_slice(&std::fs::read(&status_path).unwrap()).unwrap();
    crash_status.verify(&[0x31; 32]).unwrap();
    let crash_generation = PathBuf::from(&crash_status.backup_directory);
    assert!(crash_generation.is_dir());
    assert_eq!(
        std::fs::read_dir(&backup_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| parse_backup_id(&entry.file_name().to_string_lossy()).is_some())
            .count(),
        3,
        "a crash after status publication may leave exactly retain+1 generations"
    );

    let third = backup_and_publish_status(
        &config,
        &release_manifest,
        &release_identity,
        &backup_root,
        &status_path,
        2,
        &restore_sources,
    )
    .await
    .unwrap();
    assert!(third.is_dir());
    assert!(!first.exists());
    assert!(!second.exists());
    assert!(crash_generation.exists());
    verify_backup(&third).await.unwrap();
    let managed_count = std::fs::read_dir(&backup_root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("backup-v4-")
        })
        .count();
    assert_eq!(managed_count, 2);
    assert!(
        std::fs::read_dir(&status_root)
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                !name.starts_with(".backup-status.json-")
            })
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let outside = directory.path().join("outside-partial-target");
        tokio::fs::create_dir(&outside).await.unwrap();
        write_private_file(&outside.join("preserved"), b"outside")
            .await
            .unwrap();
        let hostile_partial = backup_root.join(format!(".backup-v4-2-{}.partial", "b".repeat(32)));
        symlink(&outside, &hostile_partial).unwrap();
        assert!(
            backup_and_publish_status(
                &config,
                &release_manifest,
                &release_identity,
                &backup_root,
                &status_path,
                2,
                &restore_sources,
            )
            .await
            .is_err()
        );
        assert_eq!(
            tokio::fs::read(outside.join("preserved")).await.unwrap(),
            b"outside"
        );
        tokio::fs::remove_file(hostile_partial).await.unwrap();
    }

    let nested_status = backup_root.join("backup-status.json");
    config.backup_manifest_path = Some(nested_status.clone());
    assert!(
        backup_and_publish_status(
            &config,
            &release_manifest,
            &release_identity,
            &backup_root,
            &nested_status,
            2,
            &restore_sources,
        )
        .await
        .is_err(),
        "backup payload roots must never double as the API-readable status authority"
    );
}
