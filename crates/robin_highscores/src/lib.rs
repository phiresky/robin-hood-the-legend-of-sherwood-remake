#![forbid(unsafe_code)]

//! Public high-score API, durable admission queue, and replay storage.
//!
//! The HTTP process never decides that a run is valid. It admits signed replay
//! bytes only after a bounded lexical compact-envelope check; it never
//! base64/zstd/bitcode decodes them. A separate worker process leases jobs,
//! resimulates each replay in a sandboxed verifier against raw game content,
//! and publishes only verifier-derived results. There is no debug or loopback
//! HTTP endpoint capable of promoting a submission.

// Every storage path (openat2 confinement, statx mount identity, procfs,
// systemd readiness) is Linux-specific; nothing builds this crate elsewhere.
#[cfg(not(target_os = "linux"))]
compile_error!("robin_highscores is Linux-only (openat2, statx, procfs, systemd)");

mod authentication;
pub mod config;
pub mod db;
pub mod error;
pub mod identity;
pub mod model;
pub mod physical_work;
pub mod replay_store;
pub mod secure_fs;
pub mod service;
pub mod storage_admission;
mod submission;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod verifier;
pub mod web;
pub mod worker_config;

pub use config::ServerConfig;
pub use db::Database;
pub use replay_store::ReplayStore;

/// Reduce an internal error chain to a stable operational category without
/// formatting private verifier diagnostics, database details, or filesystem
/// paths into logs.
pub fn safe_error_code(error: &anyhow::Error) -> &'static str {
    if let Some(error) = error.downcast_ref::<db::DbError>() {
        error.safe_log_code()
    } else if let Some(error) = error.downcast_ref::<replay_store::StoreError>() {
        error.safe_log_code()
    } else if let Some(error) = error.downcast_ref::<storage_admission::StorageAdmissionError>() {
        error.safe_log_code()
    } else if let Some(error) = error.downcast_ref::<verifier::ProcessError>() {
        error.safe_log_code()
    } else if error.downcast_ref::<std::io::Error>().is_some() {
        "io"
    } else if error.downcast_ref::<std::time::SystemTimeError>().is_some() {
        "system_clock"
    } else {
        "internal"
    }
}

/// Register every verified object found in the digest tree. This closes the
/// promotion-before-transaction crash window: unreferenced rows become normal
/// age-gated GC candidates instead of immortal unindexed files.
pub async fn reconcile_replay_inventory(
    database: &Database,
    store: &ReplayStore,
) -> anyhow::Result<usize> {
    const PAGE_SIZE: usize = 10_000;
    let mut cursor = None;
    let mut reconciled = 0_usize;
    loop {
        let entries = store.inventory_page(cursor, PAGE_SIZE).await?;
        if entries.is_empty() {
            break;
        }
        for entry in &entries {
            drop(store.open_verified(&entry.sha256, entry.bytes).await?);
            match database
                .register_replay_object(&entry.sha256, entry.bytes)
                .await
            {
                Ok(()) | Err(db::DbError::QueueFull) => {}
                Err(error) => return Err(error.into()),
            }
        }
        reconciled = reconciled
            .checked_add(entries.len())
            .ok_or_else(|| anyhow::anyhow!("replay inventory count overflow"))?;
        cursor = entries.last().map(|entry| entry.sha256);
        if entries.len() < PAGE_SIZE {
            break;
        }
    }
    Ok(reconciled)
}

/// Resume durable token-specific purges, then claim and process another batch.
pub async fn garbage_collect_replays(
    database: &Database,
    store: &ReplayStore,
    config: &ServerConfig,
    batch_size: u32,
) -> anyhow::Result<usize> {
    let now = u64::try_from(model::now_epoch_ms()?)?;
    let orphan_age_ms = config
        .orphan_replay_retention_hours
        .checked_mul(60 * 60 * 1_000)
        .ok_or_else(|| anyhow::anyhow!("orphan retention overflow"))?;
    let orphan_before = now.saturating_sub(orphan_age_ms);
    let rejected_age_ms = config
        .rejected_replay_retention_hours
        .checked_mul(60 * 60 * 1_000)
        .ok_or_else(|| anyhow::anyhow!("rejected retention overflow"))?;
    let rejected_before = now.saturating_sub(rejected_age_ms);
    let expired_rejected = database
        .expire_rejected_submissions(rejected_before, now)
        .await?;
    if expired_rejected > 0 {
        tracing::info!(expired_rejected, "expired retained rejected submissions");
    }
    let mut candidates = database.claimed_replay_purges(batch_size).await?;
    let remaining =
        batch_size.saturating_sub(u32::try_from(candidates.len()).unwrap_or(batch_size));
    if remaining > 0 {
        candidates.extend(
            database
                .claim_replay_gc_candidates(now, orphan_before, remaining)
                .await?,
        );
    }
    let mut completed = 0;
    for candidate in candidates {
        let quarantine = store
            .quarantine_for_purge(
                &candidate.sha256,
                candidate.byte_length,
                &candidate.claim_token,
            )
            .await?;
        store.remove_quarantined(&quarantine).await?;
        if !database
            .finish_replay_purge(&candidate.sha256, &candidate.claim_token, now)
            .await?
        {
            anyhow::bail!("replay purge claim was lost before finalization");
        }
        completed += 1;
    }
    Ok(completed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures_util::stream;
    use sha2::{Digest as _, Sha256};

    #[test]
    fn safe_error_codes_never_format_private_error_text() {
        let sentinel = "/srv/private/full-game/secret-retail-path";
        let generic = anyhow::anyhow!(sentinel);
        assert_eq!(safe_error_code(&generic), "internal");

        let database = anyhow::Error::new(db::DbError::ResultInvariant(sentinel.to_owned()));
        assert_eq!(safe_error_code(&database), "result_invariant");

        let verifier =
            anyhow::Error::new(verifier::ProcessError::InvalidResult(sentinel.to_owned()));
        assert_eq!(safe_error_code(&verifier), "verifier_invalid_result");
    }

    #[tokio::test]
    async fn reconciled_failed_submission_artifact_becomes_collectible_orphan() {
        let directory = tempfile::tempdir().unwrap();
        let config = ServerConfig {
            database_path: directory.path().join("highscores.sqlite3"),
            replay_directory: directory.path().join("replays"),
            orphan_replay_retention_hours: 1,
            ..Default::default()
        };
        let database = Database::migrate(&config).await.unwrap();
        let store = ReplayStore::create(config.replay_directory.clone(), 1024)
            .await
            .unwrap();
        let bytes = Bytes::from_static(b"promoted before rejected DB transaction");
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        store
            .store_stream(
                stream::iter([Ok::<_, std::convert::Infallible>(bytes.clone())]),
                digest,
                bytes.len() as u64,
            )
            .await
            .unwrap();

        assert_eq!(
            reconcile_replay_inventory(&database, &store).await.unwrap(),
            1
        );
        sqlx::query("UPDATE replay_objects SET created_at_ms = 0 WHERE sha256 = ?")
            .bind(digest.as_slice())
            .execute(database.fixture_pool())
            .await
            .unwrap();
        assert_eq!(
            garbage_collect_replays(&database, &store, &config, 10)
                .await
                .unwrap(),
            1
        );
        assert!(!store.path_for_digest(&digest).exists());
        let state: String =
            sqlx::query_scalar("SELECT purge_state FROM replay_objects WHERE sha256 = ?")
                .bind(digest.as_slice())
                .fetch_one(database.fixture_pool())
                .await
                .unwrap();
        assert_eq!(state, "purged");
    }

    /// `ops/backup.sh` snapshots the DB first and copies the object tree
    /// afterwards, so a restore can pair a DB with a slightly different tree:
    /// an orphan purged in between is missing, and an object stored in between
    /// has no row. Startup reconcile + GC must accept both.
    #[tokio::test]
    async fn restored_snapshot_tolerates_object_trees_out_of_sync() {
        let directory = tempfile::tempdir().unwrap();
        let live = ServerConfig {
            database_path: directory.path().join("live/highscores.sqlite3"),
            replay_directory: directory.path().join("live/replays"),
            orphan_replay_retention_hours: 1,
            ..Default::default()
        };
        let database = Database::migrate(&live).await.unwrap();
        let store = ReplayStore::create(live.replay_directory.clone(), 1024)
            .await
            .unwrap();
        let store_bytes = |store: ReplayStore, bytes: &'static [u8]| async move {
            let bytes = Bytes::from_static(bytes);
            let digest: [u8; 32] = Sha256::digest(&bytes).into();
            store
                .store_stream(
                    stream::iter([Ok::<_, std::convert::Infallible>(bytes.clone())]),
                    digest,
                    bytes.len() as u64,
                )
                .await
                .unwrap();
            digest
        };
        let purged_later = store_bytes(store.clone(), b"orphan purged after the snapshot").await;
        reconcile_replay_inventory(&database, &store).await.unwrap();
        sqlx::query("UPDATE replay_objects SET created_at_ms = 0")
            .execute(database.fixture_pool())
            .await
            .unwrap();
        let snapshot = directory.path().join("snapshot.sqlite3");
        db::snapshot_database(&live.database_path, &snapshot, 1_000)
            .await
            .unwrap();
        database.close().await;

        let restored = ServerConfig {
            database_path: directory.path().join("restored/highscores.sqlite3"),
            replay_directory: directory.path().join("restored/replays"),
            orphan_replay_retention_hours: 1,
            ..Default::default()
        };
        std::fs::create_dir_all(restored.database_path.parent().unwrap()).unwrap();
        std::fs::copy(&snapshot, &restored.database_path).unwrap();
        let restored_database = Database::connect(&restored).await.unwrap();
        let restored_store = ReplayStore::create(restored.replay_directory.clone(), 1024)
            .await
            .unwrap();
        let written_later = store_bytes(restored_store.clone(), b"stored after the snapshot").await;
        assert!(!restored_store.path_for_digest(&purged_later).exists());

        assert_eq!(
            reconcile_replay_inventory(&restored_database, &restored_store)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            garbage_collect_replays(&restored_database, &restored_store, &restored, 10)
                .await
                .unwrap(),
            1
        );
        let state_of = |digest: [u8; 32]| {
            let database = restored_database.clone();
            async move {
                sqlx::query_scalar::<_, String>(
                    "SELECT purge_state FROM replay_objects WHERE sha256 = ?",
                )
                .bind(digest.as_slice())
                .fetch_one(database.fixture_pool())
                .await
                .unwrap()
            }
        };
        assert_eq!(state_of(purged_later).await, "purged");
        assert_eq!(state_of(written_later).await, "live");
        assert!(restored_store.path_for_digest(&written_later).exists());
        restored_database.close().await;
    }
}
