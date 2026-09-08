//! Public high-score API, durable admission queue, and replay storage.
//!
//! The HTTP process never decides that a run is valid. It admits signed replay
//! bytes only after a bounded lexical compact-envelope check; it never
//! base64/zstd/bitcode decodes them. A separate worker process leases jobs,
//! executes the exact-build verifier, and publishes only verifier-derived
//! results. This separation is deliberate: there is no debug or loopback HTTP
//! endpoint capable of promoting a submission.

pub mod backup;
pub mod campaign_store;
pub mod config;
pub mod db;
pub mod db_fence;
pub mod deployment;
pub mod error;
pub mod identity;
pub mod live_schema;
pub mod model;
pub mod physical_work;
pub mod replay_store;
pub mod runtime_authority;
mod secure_fs;
pub mod storage_admission;
mod submission;
pub mod verifier;
pub mod web;

pub use campaign_store::{CampaignInventoryEntry, CampaignStore, CampaignStoreError};
pub use config::ServerConfig;
pub use db::Database;
pub use replay_store::ReplayStore;
use std::time::Duration;

/// Reduce an internal error chain to a stable operational category without
/// formatting private verifier diagnostics, database details, or filesystem
/// paths into logs.
pub fn safe_error_code(error: &anyhow::Error) -> &'static str {
    if let Some(error) = error.downcast_ref::<db::DbError>() {
        error.safe_log_code()
    } else if let Some(error) = error.downcast_ref::<replay_store::StoreError>() {
        error.safe_log_code()
    } else if let Some(error) = error.downcast_ref::<campaign_store::CampaignStoreError>() {
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

/// Register all verified private campaign objects after a crash between file
/// promotion and the database transaction.
pub async fn reconcile_campaign_inventory(
    database: &Database,
    store: &CampaignStore,
) -> anyhow::Result<usize> {
    let entries = store.inventory().await?;
    for entry in &entries {
        database
            .register_campaign_object(&entry.sha256, entry.bytes)
            .await?;
    }
    Ok(entries.len())
}

/// Retain accepted live chains and age-gate rejected/forked/deleted objects.
/// Token-specific quarantine makes every purge resumable without reviving a
/// path that an older collector could later delete.
pub async fn garbage_collect_campaigns(
    database: &Database,
    store: &CampaignStore,
    orphan_retention: Duration,
    batch_size: u32,
) -> anyhow::Result<usize> {
    let now = u64::try_from(model::now_epoch_ms()?)?;
    let cutoff = now.saturating_sub(u64::try_from(orphan_retention.as_millis())?);
    let mut candidates = database.claimed_campaign_purges(batch_size).await?;
    let remaining =
        batch_size.saturating_sub(u32::try_from(candidates.len()).unwrap_or(batch_size));
    if remaining > 0 {
        candidates.extend(
            database
                .claim_campaign_gc_candidates(now, cutoff, remaining)
                .await?,
        );
    }
    let mut completed = 0;
    for candidate in candidates {
        let quarantine = store
            .quarantine_for_purge(&candidate.sha256, &candidate.claim_token)
            .await?;
        store.remove_quarantined(&quarantine).await?;
        if !database
            .finish_campaign_purge(&candidate.sha256, &candidate.claim_token, now)
            .await?
        {
            anyhow::bail!("campaign purge claim was lost before finalization");
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
        let mut config = ServerConfig::default();
        config.database_path = directory.path().join("highscores.sqlite3");
        config.replay_directory = directory.path().join("replays");
        config.orphan_replay_retention_hours = 1;
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
            .execute(database.pool())
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
                .fetch_one(database.pool())
                .await
                .unwrap();
        assert_eq!(state, "purged");
    }
}
