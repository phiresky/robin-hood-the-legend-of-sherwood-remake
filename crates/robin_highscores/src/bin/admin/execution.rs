//! Backup completion owner, kernel fences, heartbeat drain, and publication.

use super::capacity::estimate_backup_space;
use super::cleanup::cleanup_failed_partial_backup;
use super::cleanup::recover_interrupted_complete_cleanups;
use super::cleanup::recover_stale_partial_backups;
use super::cleanup::remove_owned_complete_backup;
use super::cleanup::remove_owned_partial_backup;
use super::cleanup::retain_complete_backups;
use super::filesystem::acquire_backup_operation_lock;
use super::filesystem::cap_entry_exists;
use super::filesystem::copy_open_file;
use super::filesystem::create_backup_directory;
use super::filesystem::duplicate_pinned_file;
use super::filesystem::metadata_identity;
use super::filesystem::open_cap_regular_nofollow;
use super::filesystem::open_directory_nofollow;
use super::filesystem::open_regular_nofollow;
use super::filesystem::pin_directory_capability;
use super::filesystem::read_bounded_pinned_file;
use super::filesystem::read_bounded_regular_nofollow;
use super::filesystem::record_file;
use super::filesystem::revalidate_pinned_regular_path;
use super::filesystem::revalidate_pinned_root_directory;
#[cfg(unix)]
use super::filesystem::set_backup_permissions;
use super::filesystem::set_private_directory;
use super::filesystem::sync_cap_directory;
use super::filesystem::sync_directory;
use super::filesystem::write_private_file;
use super::policy::BACKUP_MANIFEST_SCHEMA_VERSION;
use super::policy::SYSTEMD_UNIT_FILES;
use super::policy::SYSTEMD_USER_ROOT;
use super::sources::copy_pinned_restore_source;
use super::sources::pin_backup_restore_sources;
use super::sources::pin_restore_source;
use super::sources::preserve_release_authority;
use super::sources::validate_backup_restore_source_contract;
use super::verification::verify_backup_authenticated;
use anyhow::Context as _;
use robin_highscores::CampaignStore;
use robin_highscores::Database;
use robin_highscores::ReplayStore;
use robin_highscores::ServerConfig;
use robin_highscores::backup::BackupManifestV4 as BackupManifest;
use robin_highscores::backup::BackupReleaseIdentityV2;
use robin_highscores::backup::BackupRestoreSourceV4 as RestoreSource;
use robin_highscores::backup::BackupStatusV4;
use robin_highscores::backup::BackupVerificationEnvelopeV2;
use robin_highscores::backup::canonical_backup_directories_v4;
use robin_highscores::db_fence::ExclusiveAdmissionGuard;
use robin_highscores::db_fence::ExclusiveQuiescenceGuard;
use robin_highscores::db_fence::RuntimeDatabaseFence;
use robin_run_protocol::canonical_json_bytes;
use sha2::Digest as _;
use sha2::Sha256;
use sqlx::Connection as _;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::sqlite::SqliteJournalMode;
use std::collections::BTreeMap;
use std::io::Read as _;
use std::io::Write as _;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

const BACKUP_LOCK_TTL: Duration = Duration::from_secs(10 * 60);

const BACKUP_WRITER_DRAIN_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Debug)]
enum BackupInstallOutcome {
    Installed,
    InstalledButParentSyncFailed(anyhow::Error),
}

#[derive(Debug)]
enum StatusPublicationOutcome {
    Published,
    PublishedButParentSyncFailed(anyhow::Error),
    PublishedButIdentityUncertain(anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
#[error(
    "authenticated backup readiness envelope was installed at {path} with SHA-256 {envelope_sha256}, but status-parent durability sync failed: {source}"
)]
struct StatusPublicationDurabilityUncertain {
    path: PathBuf,
    envelope_sha256: String,
    #[source]
    source: anyhow::Error,
}

#[derive(Debug, thiserror::Error)]
#[error(
    "authenticated backup readiness envelope may have been installed at {path} with SHA-256 {envelope_sha256}, but its post-publication path identity could not be proven: {source}"
)]
struct StatusPublicationIdentityUncertain {
    path: PathBuf,
    envelope_sha256: String,
    #[source]
    source: anyhow::Error,
}

pub(super) async fn backup_and_publish_status(
    config: &ServerConfig,
    release_manifest_path: &Path,
    release_identity: &BackupReleaseIdentityV2,
    backup_root: &Path,
    status_path: &Path,
    retain_complete: usize,
    restore_sources: &BTreeMap<PathBuf, PathBuf>,
) -> anyhow::Result<PathBuf> {
    backup_and_publish_status_with_limit(
        config,
        release_manifest_path,
        release_identity,
        backup_root,
        status_path,
        retain_complete,
        restore_sources,
        robin_highscores::backup::MAX_BACKUP_STATUS_BYTES,
    )
    .await
}

async fn backup_and_publish_status_with_limit(
    config: &ServerConfig,
    release_manifest_path: &Path,
    release_identity: &BackupReleaseIdentityV2,
    backup_root: &Path,
    status_path: &Path,
    retain_complete: usize,
    restore_sources: &BTreeMap<PathBuf, PathBuf>,
    maximum_status_bytes: usize,
) -> anyhow::Result<PathBuf> {
    backup_and_publish_status_with_limit_and_publisher(
        config,
        release_manifest_path,
        release_identity,
        backup_root,
        status_path,
        retain_complete,
        restore_sources,
        maximum_status_bytes,
        publish_private_atomic,
    )
    .await
}

async fn backup_and_publish_status_with_limit_and_publisher<F>(
    config: &ServerConfig,
    release_manifest_path: &Path,
    release_identity: &BackupReleaseIdentityV2,
    backup_root: &Path,
    status_path: &Path,
    retain_complete: usize,
    restore_sources: &BTreeMap<PathBuf, PathBuf>,
    maximum_status_bytes: usize,
    publish_status: F,
) -> anyhow::Result<PathBuf>
where
    F: FnOnce(&Path, &[u8]) -> anyhow::Result<StatusPublicationOutcome> + Send + 'static,
{
    backup_and_publish_status_with_limit_and_publisher_and_hooks(
        config,
        release_manifest_path,
        release_identity,
        backup_root,
        status_path,
        retain_complete,
        restore_sources,
        maximum_status_bytes,
        publish_status,
        || Ok(()),
        || Ok(()),
    )
    .await
}

async fn backup_and_publish_status_with_limit_and_publisher_and_hooks<F, I, S>(
    config: &ServerConfig,
    release_manifest_path: &Path,
    release_identity: &BackupReleaseIdentityV2,
    backup_root: &Path,
    status_path: &Path,
    retain_complete: usize,
    restore_sources: &BTreeMap<PathBuf, PathBuf>,
    maximum_status_bytes: usize,
    publish_status: F,
    before_install: I,
    before_status_publication: S,
) -> anyhow::Result<PathBuf>
where
    F: FnOnce(&Path, &[u8]) -> anyhow::Result<StatusPublicationOutcome> + Send + 'static,
    I: FnOnce() -> anyhow::Result<()> + Send + 'static,
    S: FnOnce() -> anyhow::Result<()> + Send + 'static,
{
    let config = config.clone();
    let release_manifest_path = release_manifest_path.to_owned();
    let release_identity = release_identity.clone();
    let backup_root = backup_root.to_owned();
    let status_path = status_path.to_owned();
    let restore_sources = restore_sources.clone();
    run_owned_backup(async move {
        backup_and_publish_status_owned(
            &config,
            &release_manifest_path,
            &release_identity,
            &backup_root,
            &status_path,
            retain_complete,
            &restore_sources,
            maximum_status_bytes,
            publish_status,
            before_install,
            before_status_publication,
        )
        .await
    })
    .await
}

/// Own the whole operation, including admission and final cleanup. Dropping the
/// caller's waiter must not release operation.lock or either kernel fence while
/// a backup is still running. This owner is never aborted by its caller.
async fn run_owned_backup<T, F>(operation: F) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: std::future::Future<Output = anyhow::Result<T>> + Send + 'static,
{
    tokio::spawn(operation)
        .await
        .map_err(|error| anyhow::anyhow!(error).context("backup owner task failed"))?
}

async fn backup_and_publish_status_owned<F, I, S>(
    config: &ServerConfig,
    release_manifest_path: &Path,
    release_identity: &BackupReleaseIdentityV2,
    backup_root: &Path,
    status_path: &Path,
    retain_complete: usize,
    restore_sources: &BTreeMap<PathBuf, PathBuf>,
    maximum_status_bytes: usize,
    publish_status: F,
    before_install: I,
    before_status_publication: S,
) -> anyhow::Result<PathBuf>
where
    F: FnOnce(&Path, &[u8]) -> anyhow::Result<StatusPublicationOutcome>,
    I: FnOnce() -> anyhow::Result<()>,
    S: FnOnce() -> anyhow::Result<()>,
{
    anyhow::ensure!(
        maximum_status_bytes > 0
            && maximum_status_bytes <= robin_highscores::backup::MAX_BACKUP_STATUS_BYTES,
        "backup status byte limit is invalid"
    );
    anyhow::ensure!(
        retain_complete >= 1,
        "at least one complete backup is required"
    );
    anyhow::ensure!(backup_root.is_absolute(), "backup root must be absolute");
    anyhow::ensure!(
        config.release_manifest_path.as_deref() == Some(release_manifest_path),
        "backup release manifest path must exactly match release_manifest_path"
    );
    release_identity.validate()?;
    anyhow::ensure!(
        release_identity.database_schema_version == robin_highscores::db::CURRENT_SCHEMA_VERSION,
        "active release database schema differs from the running backup authority"
    );
    validate_backup_restore_source_contract(config, restore_sources)?;
    anyhow::ensure!(
        status_path.is_absolute(),
        "backup status path must be absolute"
    );
    anyhow::ensure!(
        status_path.file_name().and_then(|name| name.to_str()) == Some("backup-status.json"),
        "backup status path must name backup-status.json"
    );
    anyhow::ensure!(
        config.backup_manifest_path.as_deref() == Some(status_path),
        "published status path must exactly match backup_manifest_path"
    );
    let backup_root_metadata = tokio::fs::symlink_metadata(backup_root).await?;
    anyhow::ensure!(
        backup_root_metadata.is_dir() && !backup_root_metadata.file_type().is_symlink(),
        "backup root must be a real directory"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            backup_root_metadata.uid() == rustix::process::geteuid().as_raw()
                && backup_root_metadata.permissions().mode() & 0o777 == 0o700,
            "backup root has the wrong owner or mode"
        );
    }
    let canonical_backup_root = tokio::fs::canonicalize(backup_root).await?;
    anyhow::ensure!(
        canonical_backup_root == backup_root,
        "backup root must not traverse symlinks or aliases"
    );
    let backup_root = canonical_backup_root.as_path();
    let backup_authority_source = pin_restore_source(
        &config.backup_authority_hmac_secret_path,
        0o400,
        Some(32),
        None,
        "backup authority HMAC key",
    )?;
    let backup_authority_key: [u8; 32] = backup_authority_source
        .bytes()
        .try_into()
        .map_err(|_| anyhow::anyhow!("backup authority HMAC key has the wrong length"))?;
    anyhow::ensure!(
        backup_authority_key.iter().any(|byte| *byte != 0),
        "backup authority HMAC key must not be all zero"
    );
    let status_parent = status_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("backup status path has no parent"))?;
    let status_parent_metadata = tokio::fs::symlink_metadata(status_parent).await?;
    anyhow::ensure!(
        status_parent_metadata.is_dir() && !status_parent_metadata.file_type().is_symlink(),
        "backup status parent must be a real directory"
    );
    let canonical_status_parent = tokio::fs::canonicalize(status_parent).await?;
    anyhow::ensure!(
        canonical_status_parent == status_parent,
        "backup status parent must not traverse symlinks or aliases"
    );
    anyhow::ensure!(
        !canonical_status_parent.starts_with(backup_root),
        "backup status path must remain outside the protected backup payload root"
    );
    let status_parent_capability = pin_directory_capability(status_parent)?;
    let status_parent_guard = status_parent_capability.try_clone()?.into_std_file();
    revalidate_pinned_root_directory(&status_parent_guard, status_parent, "backup status parent")?;
    let status_name = Path::new("backup-status.json");
    let existing_status = if cap_entry_exists(&status_parent_capability, status_name)? {
        let status_file = open_cap_regular_nofollow(&status_parent_capability, status_name)?;
        revalidate_pinned_regular_path(
            &status_file,
            status_path,
            0o400,
            Some(0o700),
            "existing backup status",
        )?;
        let status_bytes = read_bounded_pinned_file(
            duplicate_pinned_file(&status_file, false)?,
            u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
        )?;
        revalidate_pinned_regular_path(
            &status_file,
            status_path,
            0o400,
            Some(0o700),
            "existing backup status",
        )?;
        anyhow::ensure!(
            read_bounded_pinned_file(
                duplicate_pinned_file(&status_file, false)?,
                u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
            )? == status_bytes,
            "existing backup status changed during admission"
        );
        let status: BackupStatusV4 = serde_json::from_slice(&status_bytes)?;
        anyhow::ensure!(
            canonical_json_bytes(&status)? == status_bytes,
            "existing backup status is not canonical JSON"
        );
        status.verify(&backup_authority_key)?;
        anyhow::ensure!(
            Path::new(&status.backup_directory) == backup_root.join(&status.backup_id),
            "existing backup status is outside the canonical backup root"
        );
        Some((status, status_file, status_bytes))
    } else {
        None
    };

    let _operation_lock = acquire_backup_operation_lock(backup_root)?;
    backup_authority_source.revalidate("backup authority HMAC key")?;
    revalidate_pinned_root_directory(&status_parent_guard, status_parent, "backup status parent")?;
    if let Some((_, status_file, status_bytes)) = &existing_status {
        revalidate_pinned_regular_path(
            status_file,
            status_path,
            0o400,
            Some(0o700),
            "existing backup status",
        )?;
        anyhow::ensure!(
            read_bounded_pinned_file(
                duplicate_pinned_file(status_file, false)?,
                u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
            )? == *status_bytes,
            "existing backup status changed before cleanup admission"
        );
    } else {
        anyhow::ensure!(
            !cap_entry_exists(&status_parent_capability, status_name)?,
            "backup status appeared while acquiring the operation lock"
        );
    }
    recover_interrupted_complete_cleanups(backup_root, &backup_authority_key)?;
    recover_stale_partial_backups(backup_root)?;
    if let Some((status, status_file, status_bytes)) = &existing_status {
        // Reconcile a crash after status publication but before post-publish
        // retention before allocating another generation. Prune only a crash
        // backlog down to the configured steady-state count; healthy retained
        // redundancy remains intact until the replacement is fully published.
        let maximum_admitted_generations = retain_complete
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("retention generation bound overflows"))?;
        backup_authority_source.revalidate("backup authority HMAC key before retention")?;
        retain_complete_backups(
            backup_root,
            &status.backup_id,
            retain_complete,
            maximum_admitted_generations,
            release_identity,
            &backup_authority_key,
        )
        .await?;
        backup_authority_source.revalidate("backup authority HMAC key after retention")?;
        revalidate_pinned_root_directory(
            &status_parent_guard,
            status_parent,
            "backup status parent",
        )?;
        revalidate_pinned_regular_path(
            status_file,
            status_path,
            0o400,
            Some(0o700),
            "existing backup status",
        )?;
        anyhow::ensure!(
            read_bounded_pinned_file(
                duplicate_pinned_file(status_file, false)?,
                u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
            )? == *status_bytes,
            "existing backup status changed during pre-backup retention"
        );
    }

    let database = Database::connect(config).await?;
    let (backup_lock, exclusive_fence) = acquire_backup_write_authority(&database).await?;
    let result: anyhow::Result<PathBuf> = run_with_backup_lock_heartbeat(
        &database,
        &backup_lock,
        async {
            wait_for_maintenance_writers(&database, &backup_lock).await?;
            exclusive_fence.revalidate()?;
            anyhow::ensure!(
                database.active_maintenance_write_lease_count().await? == 0,
                "maintenance writer lease appeared after the exclusive TTL drain"
            );
            let replay =
                ReplayStore::create(config.replay_directory.clone(), config.max_replay_bytes)
                    .await?;
            let campaign = CampaignStore::create(
                config.campaign_state_directory.clone(),
                config.max_campaign_bytes,
            )
            .await?;

    // Recompute the same typed estimate used by deployment while holding the
    // operation lock and immediately before any partial backup byte is
    // created. Existing retained backups are already charged to live free
    // space; exactly one additional generation is required here.
    estimate_backup_space(
        config,
        release_identity,
        backup_root,
        status_path,
        restore_sources,
    )
    .await?
    .ensure_available()?;
    preserve_release_authority(backup_root, release_manifest_path, release_identity).await?;

    let created_at_unix_ms = u64::try_from(robin_highscores::model::now_epoch_ms()?)?;
    let identifier = format!(
        "backup-v4-{created_at_unix_ms}-{}",
        uuid::Uuid::now_v7().simple()
    );
    let partial = backup_root.join(format!(".{identifier}.partial"));
    let complete = backup_root.join(&identifier);
    create_backup_directory(&partial).await?;
    set_private_directory(&partial).await?;
    let backup_result = backup_locked(
        config,
        &database,
                &replay,
                &campaign,
        &backup_lock,
        &partial,
        created_at_unix_ms,
        release_identity,
        restore_sources,
    )
    .await;
            if let Err(error) = backup_result {
                return Err(cleanup_failed_partial_backup(&database, backup_root, &partial, error).await);
            }
            let manifest_bytes = read_bounded_regular_nofollow(
                &partial.join("backup-manifest.json"),
                u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
            )
            .await?;
            let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
            anyhow::ensure!(
                canonical_json_bytes(&manifest)? == manifest_bytes,
                "new backup manifest is not canonical before envelope publication"
            );
            backup_authority_source
                .revalidate("backup authority HMAC key before envelope publication")?;
            let envelope = BackupVerificationEnvelopeV2::new_authenticated(
                identifier.clone(),
                &manifest,
                &backup_authority_key,
            )?;
            let envelope_path = partial.join("backup-verification-envelope.json");
            write_private_file(&envelope_path, &canonical_json_bytes(&envelope)?).await?;
            #[cfg(unix)]
            set_backup_permissions(
                &envelope_path,
                std::fs::Permissions::from_mode(0o400),
            )
            .await?;
            sync_directory(&partial).await?;
            refresh_backup_lock(&database, &backup_lock).await?;
            let verified = match verify_backup_authenticated(&partial, &backup_authority_key).await {
        Ok(verified) => verified,
        Err(error) => {
            remove_owned_partial_backup(backup_root, &partial)?;
            return Err(error);
        }
    };
    anyhow::ensure!(
        (*verified.release_identity()) == *release_identity,
        "verified backup release identity differs from the active installed release"
    );
    if robin_highscores::secure_fs::available_space(backup_root)? < config.minimum_storage_free_bytes {
        remove_owned_partial_backup(backup_root, &partial)?;
        anyhow::bail!("completed backup would violate the configured storage floor");
    }

    // Build and bound the complete authenticated readiness envelope while the
    // backup is still a removable partial. An installed backup must never be
    // left behind merely because its projection cannot be published.
    let status_bytes_result = async {
        let manifest_bytes = read_bounded_regular_nofollow(
            &partial.join("backup-manifest.json"),
            u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
        )
        .await?;
        anyhow::ensure!(
            hex::encode(Sha256::digest(&manifest_bytes)) == (*verified.manifest_sha256()),
            "verified backup manifest changed before status publication"
        );
        let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
        anyhow::ensure!(
            canonical_json_bytes(&manifest)? == manifest_bytes
                && manifest.created_at_unix_ms == verified.created_at_unix_ms()
                && manifest.database_schema_version == verified.database_schema_version()
                && manifest.release_identity == (*verified.release_identity())
                && u64::try_from(manifest.files.len())? == verified.file_count()
                && manifest.directory_count()? == verified.directory_count()
                && manifest.total_bytes()? == verified.total_bytes(),
            "verified backup manifest changed identity before status publication"
        );
        backup_authority_source
            .revalidate("backup authority HMAC key before status publication")?;
        let status = BackupStatusV4::new_authenticated(
            identifier.clone(),
                        complete.to_string_lossy().into_owned(),
                        manifest,
                        &backup_authority_key,
                    )?;
                    status.verify(&backup_authority_key)?;
        let status_bytes = canonical_json_bytes(&status)?;
        anyhow::ensure!(
            status_bytes.len() <= maximum_status_bytes,
            "authenticated backup readiness envelope exceeds its byte limit"
        );
        Ok::<_, anyhow::Error>(status_bytes)
    }
    .await;
    let status_bytes = match status_bytes_result {
        Ok(bytes) => bytes,
        Err(error) => {
            remove_owned_partial_backup(backup_root, &partial)?;
            return Err(error);
        }
    };
    refresh_backup_lock(&database, &backup_lock).await?;
    if let Err(error) = before_install().and_then(|()| {
        backup_authority_source.revalidate("backup authority HMAC key before backup installation")
    }) {
        remove_owned_partial_backup(backup_root, &partial)?;
        return Err(error.context("backup authority changed before backup installation"));
    }
    match install_verified_partial(&partial, &complete, backup_root) {
        Ok(BackupInstallOutcome::Installed) => {}
        Ok(BackupInstallOutcome::InstalledButParentSyncFailed(error)) => {
            anyhow::bail!(
                "verified backup was installed at {} but backup-root durability sync failed; status was not published: {error:#}",
                complete.display()
            );
        }
        Err(error) => {
            remove_owned_partial_backup(backup_root, &partial)?;
            return Err(error);
        }
    }
    let complete = tokio::fs::canonicalize(&complete).await?;
    anyhow::ensure!(
        complete == backup_root.join(&identifier),
        "completed backup directory is not exactly backup-root/backup-id"
    );

    refresh_backup_lock(&database, &backup_lock).await?;
    if let Err(error) = before_status_publication().and_then(|()| {
        backup_authority_source.revalidate("backup authority HMAC key at status publication")
    }) {
        remove_owned_complete_backup(
            backup_root,
            &complete,
            &identifier,
            verified.tree(),
            &backup_authority_key,
        )?;
        return Err(error.context("backup authority changed before status publication"));
    }
    let publication = match publish_status(status_path, &status_bytes) {
        Ok(outcome) => outcome,
        Err(publication_error) => {
            remove_owned_complete_backup(
                backup_root,
                &complete,
                &identifier,
                verified.tree(),
                &backup_authority_key,
            )
            .with_context(|| {
                format!(
                    "status publication failed before installation ({publication_error:#}); remove exact unreferenced backup {}",
                    complete.display()
                )
            })?;
            return Err(publication_error.context(
                "status publication failed before installation; exact unreferenced backup removed",
            ));
        }
    };
    match publication {
        StatusPublicationOutcome::Published => {}
        StatusPublicationOutcome::PublishedButParentSyncFailed(source) => {
            return Err(StatusPublicationDurabilityUncertain {
                path: status_path.to_owned(),
                envelope_sha256: hex::encode(Sha256::digest(&status_bytes)),
                source,
            }
            .into());
        }
        StatusPublicationOutcome::PublishedButIdentityUncertain(source) => {
            return Err(StatusPublicationIdentityUncertain {
                path: status_path.to_owned(),
                envelope_sha256: hex::encode(Sha256::digest(&status_bytes)),
                source,
            }
            .into());
        }
    }
    // Retention follows publication so the previously published backup is
    // never removed while the old status still names it. If pruning fails,
    // the command fails but the new authenticated status and backup remain a
    // truthful, complete readiness boundary.
    refresh_backup_lock(&database, &backup_lock).await?;
    backup_authority_source.revalidate("backup authority HMAC key before final retention")?;
    retain_complete_backups(
        backup_root,
        &identifier,
                retain_complete,
                retain_complete
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("retention generation bound overflows"))?,
                release_identity,
                &backup_authority_key,
    )
    .await?;
    backup_authority_source.revalidate("backup authority HMAC key at backup completion")?;
    Ok(complete)
        },
    )
    .await;
    let release = release_backup_gate_and_close_pool_under_exclusive_fence(
        &database,
        &backup_lock,
        &exclusive_fence,
    )
    .await;
    match (result, release) {
        (Ok(value), Ok(true)) => Ok(value),
        (Ok(_), Ok(false)) => anyhow::bail!("backup writer gate disappeared before release"),
        (Ok(_), Err(error)) => Err(error),
        (Err(operation), Ok(true)) => Err(operation),
        (Err(operation), Ok(false)) => {
            Err(operation.context("backup failed and its writer gate disappeared before release"))
        }
        (Err(operation), Err(release)) => Err(operation.context(format!(
            "backup failed and releasing its writer gate also failed: {release}"
        ))),
    }
}

async fn wait_for_maintenance_writers(
    database: &Database,
    backup_lock: &str,
) -> anyhow::Result<()> {
    wait_for_maintenance_writers_with_timing(
        database,
        backup_lock,
        BACKUP_WRITER_DRAIN_TIMEOUT,
        Duration::from_millis(100),
    )
    .await
}

async fn wait_for_maintenance_writers_with_timing(
    database: &Database,
    backup_lock: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !timeout.is_zero() && !poll_interval.is_zero(),
        "maintenance-writer drain timing must be positive"
    );
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if database.active_maintenance_write_lease_count().await? == 0 {
            return Ok(());
        }
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "timed out draining state-mutating requests after closing backup admission"
        );
        anyhow::ensure!(
            database
                .refresh_backup_lock(backup_lock, BACKUP_LOCK_TTL)
                .await?,
            "backup writer gate expired while draining admitted requests"
        );
        tokio::time::sleep(poll_interval).await;
    }
}

/// Close a database that will not reach this command's EX phase. The shared
/// guard covers optional gate release, SQLx return/rollback, and awaited pool
/// close. No post-connect error path may rely on `SqlitePool` Drop.
async fn close_pre_exclusive_database_pool(
    database: &Database,
    backup_lock: Option<&str>,
) -> anyhow::Result<Option<bool>> {
    let shared = database.runtime_fence().acquire_one_off_shared().await?;
    let release = match backup_lock {
        Some(token) => Some(database.release_backup_lock(token).await),
        None => None,
    };
    let reconciliation = match (&release, backup_lock) {
        (Some(Err(_)), Some(token)) => Some(database.backup_lock_token_present(token).await),
        _ => None,
    };
    let idle_before_close = database.wait_for_idle().await;
    database.close_pool_under_fence().await;
    let shared_validation = shared.revalidate();
    let runtime_validation = database.runtime_fence().revalidate();
    drop(shared);

    let mut cleanup_error: Option<anyhow::Error> = None;
    for (label, result) in [
        ("draining the pre-exclusive SQLite pool", idle_before_close),
        (
            "revalidating the retained shared database fence",
            shared_validation,
        ),
        (
            "revalidating the runtime database fence",
            runtime_validation,
        ),
    ] {
        if let Err(error) = result {
            cleanup_error = Some(match cleanup_error {
                Some(previous) => previous.context(format!("{label} also failed: {error:#}")),
                None => error.context(label),
            });
        }
    }
    if let Some(error) = cleanup_error {
        return Err(match release {
            Some(Err(release)) => anyhow::Error::from(release)
                .context(format!("pre-exclusive pool cleanup also failed: {error:#}")),
            _ => error,
        });
    }
    match (release, reconciliation) {
        (None, None) => Ok(None),
        (Some(Ok(released)), None) => Ok(Some(released)),
        (Some(Err(release)), Some(Ok(false))) => Err(anyhow::Error::from(release).context(
            "pre-exclusive gate release reported an error, but exact-token reconciliation proves it absent",
        )),
        (Some(Err(release)), Some(Ok(true))) => Err(anyhow::Error::from(release).context(
            "pre-exclusive gate release reported an error and exact-token reconciliation proves it remains",
        )),
        (Some(Err(release)), Some(Err(reconciliation))) => Err(anyhow::Error::from(release)
            .context(format!(
                "pre-exclusive gate release and exact-token reconciliation both failed: {reconciliation}"
            ))),
        _ => anyhow::bail!("pre-exclusive cleanup produced an impossible release state"),
    }
}

async fn acquire_backup_write_authority(
    database: &Database,
) -> anyhow::Result<(String, ExclusiveBackupDatabaseFence)> {
    // Publish the durable gate under the ordinary shared database fence, then
    // release that shared generation before attempting exclusive admission.
    // New processes may briefly pass the turnstile, but must observe this gate
    // and leave without starting their real database operation.
    let mut gate_operation = match database.begin_fenced_operation().await {
        Ok(operation) => operation,
        Err(error) => {
            let original = anyhow::Error::from(error);
            return match close_pre_exclusive_database_pool(database, None).await {
                Ok(_) => Err(original),
                Err(cleanup) => Err(original.context(format!(
                    "closing the pre-exclusive database pool also failed: {cleanup:#}"
                ))),
            };
        }
    };
    let backup_lock_result = database
        .acquire_backup_lock("robin-highscores-admin", BACKUP_LOCK_TTL)
        .await;
    let gate_finish = database.finish_fenced_operation(&mut gate_operation).await;
    let backup_lock = match (backup_lock_result, gate_finish) {
        (Ok(token), Ok(())) => token,
        (Ok(token), Err(error)) => {
            let original = anyhow::Error::from(error);
            return match close_pre_exclusive_database_pool(database, Some(&token)).await {
                Ok(Some(true)) => Err(original),
                Ok(Some(false)) => Err(original.context(
                    "gate publication succeeded but its exact token disappeared during pre-exclusive cleanup",
                )),
                Ok(None) => Err(original.context(
                    "pre-exclusive cleanup omitted the published backup-gate token",
                )),
                Err(cleanup) => Err(original.context(format!(
                    "releasing the gate and closing the pre-exclusive pool also failed: {cleanup:#}"
                ))),
            };
        }
        (Err(error), Ok(())) => {
            let original = anyhow::Error::from(error);
            return match close_pre_exclusive_database_pool(database, None).await {
                Ok(_) => Err(original),
                Err(cleanup) => Err(original.context(format!(
                    "closing the pre-exclusive database pool also failed: {cleanup:#}"
                ))),
            };
        }
        (Err(operation), Err(finish)) => {
            let original = anyhow::Error::from(operation)
                .context(format!("database fence drain also failed: {finish}"));
            return match close_pre_exclusive_database_pool(database, None).await {
                Ok(_) => Err(original),
                Err(cleanup) => Err(original.context(format!(
                    "closing the pre-exclusive database pool also failed: {cleanup:#}"
                ))),
            };
        }
    };
    let exclusive_fence = match acquire_exclusive_backup_database_fence(database, &backup_lock)
        .await
    {
        Ok(fence) => fence,
        Err(error) => {
            return match close_pre_exclusive_database_pool(
                    database,
                    Some(&backup_lock),
                )
                .await
                {
                    Ok(Some(true)) => Err(error),
                    Ok(Some(false)) => Err(error
                        .context("exclusive database fence failed and backup gate disappeared")),
                    Ok(None) => Err(error.context(
                        "exclusive database fence failed and cleanup omitted its backup gate",
                    )),
                    Err(cleanup) => Err(error.context(format!(
                        "exclusive database fence failed and pre-exclusive cleanup also failed: {cleanup:#}"
                    ))),
                };
        }
    };
    Ok((backup_lock, exclusive_fence))
}

struct ExclusiveBackupDatabaseFence {
    runtime: RuntimeDatabaseFence,
    admission: ExclusiveAdmissionGuard,
    quiescence: ExclusiveQuiescenceGuard,
}

impl ExclusiveBackupDatabaseFence {
    fn revalidate(&self) -> anyhow::Result<()> {
        self.runtime
            .validate_exclusive_pair(&self.admission, &self.quiescence)
    }
}

async fn acquire_exclusive_backup_database_fence(
    database: &Database,
    backup_lock: &str,
) -> anyhow::Result<ExclusiveBackupDatabaseFence> {
    let runtime = database.runtime_fence().clone();
    let deadline = tokio::time::Instant::now() + BACKUP_WRITER_DRAIN_TIMEOUT;
    let admission = loop {
        if let Some(guard) = runtime.try_lock_exclusive_admission()? {
            break guard;
        }
        // Never block the gate heartbeat behind flock. A brief shared pass is
        // sufficient to refresh it while all existing operations remain
        // covered by the data fence.
        if let Some(shared) = runtime.try_acquire_one_off_shared()? {
            refresh_backup_lock(database, backup_lock).await?;
            database.wait_for_idle().await?;
            shared.revalidate()?;
            drop(shared);
        }
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "timed out acquiring the exclusive database-admission turnstile"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };

    let quiescence = loop {
        admission.revalidate()?;
        if let Some(guard) = runtime.try_lock_exclusive_quiescence()? {
            break guard;
        }
        // Exclusive admission prevents new joins. Taking the data lock shared
        // while older readers drain keeps each heartbeat itself fenced.
        let shared = runtime
            .acquire_shared_quiescence_while_admission_exclusive(&admission)
            .await?;
        refresh_backup_lock(database, backup_lock).await?;
        database.wait_for_idle().await?;
        shared.revalidate()?;
        drop(shared);
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "timed out draining existing database operations"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    runtime.validate_exclusive_pair(&admission, &quiescence)?;
    refresh_backup_lock(database, backup_lock).await?;
    database.wait_for_idle().await?;
    let pair = ExclusiveBackupDatabaseFence {
        runtime,
        admission,
        quiescence,
    };
    pair.revalidate()?;
    Ok(pair)
}

async fn release_backup_gate_and_close_pool_under_exclusive_fence(
    database: &Database,
    backup_lock: &str,
    fence: &ExclusiveBackupDatabaseFence,
) -> anyhow::Result<bool> {
    close_pool_under_exclusive_fence_after_release(
        database,
        backup_lock,
        fence,
        database.release_backup_lock(backup_lock),
    )
    .await
}

async fn close_pool_under_exclusive_fence_after_release<F>(
    database: &Database,
    backup_lock: &str,
    fence: &ExclusiveBackupDatabaseFence,
    release: F,
) -> anyhow::Result<bool>
where
    F: std::future::Future<Output = Result<bool, robin_highscores::db::DbError>>,
{
    let initial_fence_validation = fence.revalidate();
    let release = if initial_fence_validation.is_ok() {
        Some(release.await)
    } else {
        None
    };
    // DELETE errors can be outcome-uncertain. Reconcile the exact token while
    // the EX pair is still retained and the pool is still usable, but never
    // let a reconciliation error skip the unconditional pool drain below.
    let reconciliation = if release.as_ref().is_some_and(Result::is_err) {
        Some(database.backup_lock_token_present(backup_lock).await)
    } else {
        None
    };
    let idle_before_close = database.wait_for_idle().await;
    database.close_pool_under_fence().await;
    let final_fence_validation = fence.revalidate();

    let mut cleanup_error: Option<anyhow::Error> = None;
    for (label, result) in [
        (
            "validating the retained exclusive database fence before gate release",
            initial_fence_validation,
        ),
        ("draining the backup SQLite pool", idle_before_close),
        (
            "revalidating the retained exclusive database fence",
            final_fence_validation,
        ),
    ] {
        if let Err(error) = result {
            cleanup_error = Some(match cleanup_error {
                Some(previous) => previous.context(format!("{label} also failed: {error:#}")),
                None => error.context(label),
            });
        }
    }
    if let Some(error) = cleanup_error {
        return match release {
            Some(Err(release)) => Err(anyhow::Error::from(release)
                .context(format!("exclusive cleanup also failed: {error:#}"))),
            _ => Err(error),
        };
    }
    match (release, reconciliation) {
        (Some(Ok(released)), None) => Ok(released),
        (Some(Err(release)), Some(Ok(false))) => Err(anyhow::Error::from(release)
            .context("backup gate deletion reported an error, but the exact token is absent after reconciliation; pool was closed under EX")),
        (Some(Err(release)), Some(Ok(true))) => Err(anyhow::Error::from(release)
            .context("backup gate deletion reported an error and the exact token remains present; pool was closed under EX")),
        (Some(Err(release)), Some(Err(reconciliation))) => Err(anyhow::Error::from(release).context(
            format!(
                "backup gate deletion was outcome-uncertain and exact-token reconciliation also failed: {reconciliation}"
            ),
        )),
        _ => anyhow::bail!("backup gate cleanup produced an impossible successful state"),
    }
}

fn publish_private_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<StatusPublicationOutcome> {
    publish_private_atomic_with(path, bytes, sync_cap_directory)
}

fn publish_private_atomic_with<F>(
    path: &Path,
    bytes: &[u8],
    sync_parent: F,
) -> anyhow::Result<StatusPublicationOutcome>
where
    F: FnOnce(&cap_std::fs::Dir) -> anyhow::Result<()>,
{
    publish_private_atomic_with_hooks(path, bytes, sync_parent, || Ok(()))
}

fn publish_private_atomic_with_hooks<F, G>(
    path: &Path,
    bytes: &[u8],
    sync_parent: F,
    before_rename: G,
) -> anyhow::Result<StatusPublicationOutcome>
where
    F: FnOnce(&cap_std::fs::Dir) -> anyhow::Result<()>,
    G: FnOnce() -> anyhow::Result<()>,
{
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("backup status path has no parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("private publication path has no filename"))?;
    let directory = pin_directory_capability(parent)?;
    let parent_metadata = directory.dir_metadata()?;
    #[cfg(unix)]
    {
        use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            parent_metadata.uid() == rustix::process::geteuid().as_raw()
                && parent_metadata.permissions().mode() & 0o777 == 0o700,
            "backup status parent has the wrong owner or mode"
        );
    }
    let parent_identity = metadata_identity(&parent_metadata);
    let temporary = format!(
        ".{}-{}.tmp",
        name.to_string_lossy(),
        uuid::Uuid::now_v7().simple()
    );
    let mut options = cap_std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = directory.open_with(&temporary, &options)?.into_std();
    let pre_rename = (|| -> anyhow::Result<()> {
        #[cfg(unix)]
        file.set_permissions(std::fs::Permissions::from_mode(0o400))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        let metadata = file.metadata()?;
        anyhow::ensure!(metadata.is_file(), "backup status temporary is not regular");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            anyhow::ensure!(
                metadata.nlink() == 1 && metadata.permissions().mode() & 0o777 == 0o400,
                "backup status temporary has the wrong mode or link count"
            );
        }
        drop(file);
        before_rename()?;
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            use std::os::fd::AsFd as _;
            rustix::fs::renameat_with(
                directory.as_fd(),
                Path::new(&temporary),
                directory.as_fd(),
                Path::new(name),
                rustix::fs::RenameFlags::empty(),
            )?;
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        directory.rename(&temporary, &directory, Path::new(name))?;
        Ok(())
    })();
    if let Err(error) = pre_rename {
        match directory.symlink_metadata(&temporary) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                directory.remove_file(&temporary)?;
                sync_cap_directory(&directory)?;
            }
            Ok(_) => anyhow::bail!("backup status temporary path changed type"),
            Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => {}
            Err(cleanup) => return Err(cleanup.into()),
        }
        return Err(error);
    }
    if let Err(error) = sync_parent(&directory) {
        return Ok(StatusPublicationOutcome::PublishedButParentSyncFailed(
            error,
        ));
    }
    let reopened = match pin_directory_capability(parent) {
        Ok(reopened) => reopened,
        Err(error) => {
            return Ok(StatusPublicationOutcome::PublishedButIdentityUncertain(
                error,
            ));
        }
    };
    let final_check = (|| -> anyhow::Result<()> {
        let reopened_metadata = reopened.dir_metadata()?;
        anyhow::ensure!(
            metadata_identity(&reopened_metadata) == parent_identity,
            "backup status parent was replaced during publication"
        );
        #[cfg(unix)]
        {
            use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
            anyhow::ensure!(
                reopened_metadata.uid() == rustix::process::geteuid().as_raw()
                    && reopened_metadata.permissions().mode() & 0o777 == 0o700,
                "backup status parent owner or mode changed during publication"
            );
        }
        let mut published = open_cap_regular_nofollow(&reopened, Path::new(name))?;
        let metadata = published.metadata()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            anyhow::ensure!(
                metadata.nlink() == 1
                    && metadata.permissions().mode() & 0o777 == 0o400
                    && metadata.uid() == rustix::process::geteuid().as_raw(),
                "published backup status has the wrong owner, mode, or link count"
            );
        }
        let mut digest = Sha256::new();
        let mut length = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = published.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            length = length
                .checked_add(u64::try_from(read)?)
                .ok_or_else(|| anyhow::anyhow!("published status length overflows"))?;
            digest.update(&buffer[..read]);
        }
        anyhow::ensure!(
            length == u64::try_from(bytes.len())?
                && hex::encode(digest.finalize()) == hex::encode(Sha256::digest(bytes)),
            "published backup status differs from the authenticated envelope"
        );
        Ok(())
    })();
    Ok(match final_check {
        Ok(()) => StatusPublicationOutcome::Published,
        Err(error) => StatusPublicationOutcome::PublishedButIdentityUncertain(error),
    })
}

#[cfg(test)]
pub(super) async fn backup(
    config: &ServerConfig,
    campaign_root: &Path,
    max_campaign_bytes: u64,
    destination: &Path,
    created_at_unix_ms: u64,
    release_identity: &BackupReleaseIdentityV2,
    restore_sources: &BTreeMap<PathBuf, PathBuf>,
) -> anyhow::Result<()> {
    let config = config.clone();
    let campaign_root = campaign_root.to_owned();
    let destination = destination.to_owned();
    let release_identity = release_identity.clone();
    let restore_sources = restore_sources.clone();
    run_owned_backup(async move {
        backup_test_owned(
            &config,
            &campaign_root,
            max_campaign_bytes,
            &destination,
            created_at_unix_ms,
            &release_identity,
            &restore_sources,
        )
        .await
    })
    .await
}

#[cfg(test)]
async fn backup_test_owned(
    config: &ServerConfig,
    campaign_root: &Path,
    max_campaign_bytes: u64,
    destination: &Path,
    created_at_unix_ms: u64,
    release_identity: &BackupReleaseIdentityV2,
    restore_sources: &BTreeMap<PathBuf, PathBuf>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        destination.is_absolute(),
        "backup destination must be absolute"
    );
    anyhow::ensure!(
        !tokio::fs::try_exists(destination).await?,
        "backup destination already exists"
    );
    let _operation_lock = acquire_backup_operation_lock(
        destination
            .parent()
            .ok_or_else(|| anyhow::anyhow!("test backup destination has no parent"))?,
    )?;
    let database = Database::connect(config).await?;
    let (lock, exclusive_fence) = acquire_backup_write_authority(&database).await?;
    let result = run_with_backup_lock_heartbeat(&database, &lock, async {
        wait_for_maintenance_writers(&database, &lock).await?;
        create_backup_directory(destination).await?;
        set_private_directory(destination).await?;
        let replay =
            ReplayStore::create(config.replay_directory.clone(), config.max_replay_bytes).await?;
        anyhow::ensure!(
            max_campaign_bytes > 0,
            "max_campaign_bytes must be positive"
        );
        let campaign = CampaignStore::create(campaign_root.to_owned(), max_campaign_bytes).await?;
        database.health_check().await?;
        replay.readiness_check().await?;
        campaign.readiness_check().await?;
        backup_locked(
            config,
            &database,
            &replay,
            &campaign,
            &lock,
            destination,
            created_at_unix_ms,
            release_identity,
            restore_sources,
        )
        .await?;
        let manifest_bytes = read_bounded_regular_nofollow(
            &destination.join("backup-manifest.json"),
            u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
        )
        .await?;
        let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
        let backup_id = destination
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("test backup destination has no UTF-8 identifier"))?;
        let envelope = BackupVerificationEnvelopeV2::new_authenticated(
            backup_id.to_owned(),
            &manifest,
            &[0x31; 32],
        )?;
        let envelope_path = destination.join("backup-verification-envelope.json");
        write_private_file(&envelope_path, &canonical_json_bytes(&envelope)?).await?;
        #[cfg(unix)]
        set_backup_permissions(&envelope_path, std::fs::Permissions::from_mode(0o400)).await?;
        sync_directory(destination).await
    })
    .await;
    let release = release_backup_gate_and_close_pool_under_exclusive_fence(
        &database,
        &lock,
        &exclusive_fence,
    )
    .await;
    match (result, release) {
        (Ok(value), Ok(true)) => Ok(value),
        (Ok(_), Ok(false)) => anyhow::bail!("backup lock was lost before release"),
        (Ok(_), Err(error)) => Err(error),
        (Err(operation), Ok(true)) => Err(operation),
        (Err(operation), Ok(false)) => {
            Err(operation.context("backup failed and its writer gate disappeared"))
        }
        (Err(operation), Err(release)) => Err(operation.context(format!(
            "backup failed and releasing its writer gate also failed: {release}"
        ))),
    }
}

async fn backup_locked(
    config: &ServerConfig,
    database: &Database,
    replay: &ReplayStore,
    campaign: &CampaignStore,
    backup_lock: &str,
    destination: &Path,
    created_at_unix_ms: u64,
    release_identity: &BackupReleaseIdentityV2,
    readable_sources: &BTreeMap<PathBuf, PathBuf>,
) -> anyhow::Result<()> {
    refresh_backup_lock(database, backup_lock).await?;
    release_identity.validate()?;
    // Pin the entire restore-source closure before the first payload byte is
    // copied. Every later copy uses these exact descriptors, then rebinds the
    // source path and bytes; validation is never followed by a pathname reopen.
    let pinned_sources = pin_backup_restore_sources(config, readable_sources, release_identity)?;
    let database_path = destination.join("highscores.sqlite3");
    database.online_backup_to(&database_path).await?;
    scrub_transient_backup_state(&database_path, created_at_unix_ms).await?;
    #[cfg(unix)]
    set_backup_permissions(&database_path, std::fs::Permissions::from_mode(0o600)).await?;
    let mut files = vec![record_file(destination, &database_path).await?];
    let mut restore_sources = vec![RestoreSource {
        original_absolute_path: config.database_path.to_string_lossy().into_owned(),
        archive_relative_path: "highscores.sqlite3".to_owned(),
    }];
    let replay_destination = destination.join("replays");
    create_backup_directory(&replay_destination).await?;
    set_private_directory(&replay_destination).await?;
    let mut cursor = None;
    loop {
        let page = replay.inventory_page(cursor, 10_000).await?;
        if page.is_empty() {
            break;
        }
        refresh_backup_lock(database, backup_lock).await?;
        for entry in &page {
            let source = replay.open_verified(&entry.sha256, entry.bytes).await?;
            let relative = replay
                .path_for_digest(&entry.sha256)
                .strip_prefix(&config.replay_directory)?
                .to_owned();
            let target = replay_destination.join(relative);
            copy_open_file(source, &target).await?;
            files.push(record_file(destination, &target).await?);
        }
        cursor = page.last().map(|entry| entry.sha256);
        if page.len() < 10_000 {
            break;
        }
    }
    let campaign_destination = destination.join("campaigns");
    create_backup_directory(&campaign_destination).await?;
    set_private_directory(&campaign_destination).await?;
    for (index, entry) in campaign.inventory().await?.into_iter().enumerate() {
        if index % 1_000 == 0 {
            refresh_backup_lock(database, backup_lock).await?;
        }
        let relative = PathBuf::from(&hex::encode(entry.sha256)[..2])
            .join(format!("{}.campaign", hex::encode(entry.sha256)));
        let target = campaign_destination.join(relative);
        let source = campaign.open_verified(&entry.sha256).await?;
        copy_open_file(source, &target).await?;
        files.push(record_file(destination, &target).await?);
    }
    refresh_backup_lock(database, backup_lock).await?;
    restore_sources.push(RestoreSource {
        original_absolute_path: config.replay_directory.to_string_lossy().into_owned(),
        archive_relative_path: "replays".to_owned(),
    });
    restore_sources.push(RestoreSource {
        original_absolute_path: config
            .campaign_state_directory
            .to_string_lossy()
            .into_owned(),
        archive_relative_path: "campaigns".to_owned(),
    });

    let cursor_target = destination.join("restore/state/cursor-hmac.key");
    copy_pinned_restore_source(
        pinned_sources
            .get(&config.cursor_secret_path)
            .ok_or_else(|| anyhow::anyhow!("pinned cursor secret is missing"))?,
        &cursor_target,
    )
    .await?;
    files.push(record_file(destination, &cursor_target).await?);
    restore_sources.push(RestoreSource {
        original_absolute_path: config.cursor_secret_path.to_string_lossy().into_owned(),
        archive_relative_path: "restore/state/cursor-hmac.key".to_owned(),
    });
    let grant_target = destination.join("restore/state/competition-run-grant.key");
    copy_pinned_restore_source(
        pinned_sources
            .get(&config.competition_run_grant_secret_path)
            .ok_or_else(|| anyhow::anyhow!("pinned competition-grant secret is missing"))?,
        &grant_target,
    )
    .await?;
    files.push(record_file(destination, &grant_target).await?);
    restore_sources.push(RestoreSource {
        original_absolute_path: config
            .competition_run_grant_secret_path
            .to_string_lossy()
            .into_owned(),
        archive_relative_path: "restore/state/competition-run-grant.key".to_owned(),
    });
    let preflight_target = destination.join("restore/state/run-preflight-grant.key");
    copy_pinned_restore_source(
        pinned_sources
            .get(&config.run_preflight_grant_secret_path)
            .ok_or_else(|| anyhow::anyhow!("pinned run-preflight secret is missing"))?,
        &preflight_target,
    )
    .await?;
    files.push(record_file(destination, &preflight_target).await?);
    restore_sources.push(RestoreSource {
        original_absolute_path: config
            .run_preflight_grant_secret_path
            .to_string_lossy()
            .into_owned(),
        archive_relative_path: "restore/state/run-preflight-grant.key".to_owned(),
    });

    let moderation_original = config
        .moderation_bearer_token_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("backup requires the moderation bearer secret"))?;
    let moderation_target = destination.join("restore/state/moderation-bearer.token");
    copy_pinned_restore_source(
        pinned_sources
            .get(moderation_original)
            .ok_or_else(|| anyhow::anyhow!("pinned moderation secret is missing"))?,
        &moderation_target,
    )
    .await?;
    files.push(record_file(destination, &moderation_target).await?);
    restore_sources.push(RestoreSource {
        original_absolute_path: moderation_original.to_string_lossy().into_owned(),
        archive_relative_path: "restore/state/moderation-bearer.token".to_owned(),
    });

    for unit in SYSTEMD_UNIT_FILES {
        let original = Path::new(SYSTEMD_USER_ROOT).join(unit);
        let target = destination.join("restore/systemd/user").join(unit);
        copy_pinned_restore_source(
            pinned_sources
                .get(&original)
                .ok_or_else(|| anyhow::anyhow!("pinned installed unit is missing: {unit}"))?,
            &target,
        )
        .await?;
        files.push(record_file(destination, &target).await?);
        restore_sources.push(RestoreSource {
            original_absolute_path: original.to_string_lossy().into_owned(),
            archive_relative_path: format!("restore/systemd/user/{unit}"),
        });
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    anyhow::ensure!(
        files
            .windows(2)
            .all(|pair| pair[0].relative_path != pair[1].relative_path),
        "backup contains duplicate archive paths"
    );
    for source in pinned_sources.values() {
        source.revalidate("backup restore source at manifest boundary")?;
    }
    restore_sources
        .sort_by(|left, right| left.archive_relative_path.cmp(&right.archive_relative_path));
    let manifest = BackupManifest {
        schema_version: BACKUP_MANIFEST_SCHEMA_VERSION,
        created_at_unix_ms,
        database_schema_version: robin_highscores::db::CURRENT_SCHEMA_VERSION,
        release_identity: release_identity.clone(),
        root_unix_mode: 0o700,
        restore_sources,
        directories: canonical_backup_directories_v4(&files)?,
        files,
    };
    let manifest_path = destination.join("backup-manifest.json");
    manifest.validate()?;
    let bytes = canonical_json_bytes(&manifest)?;
    anyhow::ensure!(
        bytes.len() <= robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES,
        "canonical backup manifest exceeds the readiness envelope limit"
    );
    write_private_file(&manifest_path, &bytes).await?;
    sync_directory(destination).await?;
    Ok(())
}

async fn scrub_transient_backup_state(
    database_path: &Path,
    snapshot_at_unix_ms: u64,
) -> anyhow::Result<()> {
    scrub_transient_backup_state_with_hook(database_path, snapshot_at_unix_ms, || Ok(())).await
}

async fn scrub_transient_backup_state_with_hook<F>(
    database_path: &Path,
    snapshot_at_unix_ms: u64,
    after_updates: F,
) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    use futures_util::FutureExt as _;

    let snapshot_at_unix_ms = i64::try_from(snapshot_at_unix_ms)?;
    let options = SqliteConnectOptions::new()
        .filename(database_path)
        .create_if_missing(false)
        .journal_mode(SqliteJournalMode::Delete)
        .foreign_keys(true);
    let mut connection = sqlx::SqliteConnection::connect_with(&options).await?;
    // This destination connection has its own SQLx worker, not the live pool.
    // Rollback and close must finish before partial cleanup or EX release, even
    // when a query fails or the body unwinds. The detached backup owner prevents
    // caller cancellation from dropping this closing future.
    let scrub = std::panic::AssertUnwindSafe(async {
        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&mut connection)
            .await?;
        anyhow::ensure!(
            journal_mode.eq_ignore_ascii_case("delete"),
            "backup snapshot did not enter DELETE journal mode before transient-state scrub"
        );
        let mut transaction = connection.begin().await?;
        sqlx::query(
            "UPDATE submissions \
         SET status = 'retry_pending', lease_owner = NULL, lease_expires_at_ms = NULL, \
             next_attempt_at_ms = MIN(next_attempt_at_ms, ?), \
             updated_at_ms = MAX(updated_at_ms, ?) \
         WHERE status = 'verifying'",
        )
        .bind(snapshot_at_unix_ms)
        .bind(snapshot_at_unix_ms)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE submission_upload_reservations \
         SET state = 'abandoned', lease_token = NULL, lease_expires_at_ms = NULL, \
             abandoned_at_ms = MAX(updated_at_ms, reserved_at_ms, ?), \
             updated_at_ms = MAX(updated_at_ms, reserved_at_ms, ?) \
         WHERE state IN ('reserved', 'uploaded')",
        )
        .bind(snapshot_at_unix_ms)
        .bind(snapshot_at_unix_ms)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM maintenance_write_leases")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM maintenance_locks")
            .execute(&mut *transaction)
            .await?;
        after_updates()?;
        transaction.commit().await?;
        Ok::<_, anyhow::Error>(())
    })
    .catch_unwind()
    .await
    .unwrap_or_else(|_| Err(anyhow::anyhow!("backup transient-state scrub panicked")));
    let close = connection.close().await;
    match (scrub, close) {
        (Ok(()), Ok(())) => {}
        (Ok(()), Err(error)) => return Err(error.into()),
        (Err(error), Ok(())) => return Err(error),
        (Err(error), Err(close)) => {
            return Err(error.context(format!(
                "closing the backup destination SQLite worker also failed: {close}"
            )));
        }
    }
    for suffix in ["-wal", "-shm"] {
        anyhow::ensure!(
            !tokio::fs::try_exists(PathBuf::from(format!(
                "{}{}",
                database_path.display(),
                suffix
            )))
            .await?,
            "transient-state scrub left an unmanifested SQLite sidecar"
        );
    }
    std::fs::File::open(database_path)?.sync_all()?;
    Ok(())
}

async fn refresh_backup_lock(database: &Database, token: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        database.refresh_backup_lock(token, BACKUP_LOCK_TTL).await?,
        "backup lock expired or was lost"
    );
    Ok(())
}

async fn run_with_backup_lock_heartbeat<T, F>(
    database: &Database,
    token: &str,
    operation: F,
) -> anyhow::Result<T>
where
    F: std::future::Future<Output = anyhow::Result<T>>,
{
    run_with_backup_lock_heartbeat_using(operation, BACKUP_LOCK_TTL / 3, || {
        refresh_backup_lock(database, token)
    })
    .await
}

async fn run_with_backup_lock_heartbeat_using<T, F, R, RF>(
    operation: F,
    refresh_every: Duration,
    mut refresh: R,
) -> anyhow::Result<T>
where
    F: std::future::Future<Output = anyhow::Result<T>>,
    R: FnMut() -> RF,
    RF: std::future::Future<Output = anyhow::Result<()>>,
{
    use futures_util::FutureExt as _;

    let operation = robin_highscores::physical_work::drain(operation);
    tokio::pin!(operation);
    loop {
        tokio::select! {
            result = &mut operation => return result,
            () = tokio::time::sleep(refresh_every) => {
                let refresh_result = std::panic::AssertUnwindSafe(async { refresh().await })
                    .catch_unwind().await;
                let original = match refresh_result {
                    Ok(Ok(())) => continue,
                    Ok(Err(error)) => error,
                    Err(_) => anyhow::anyhow!("backup lock refresh panicked"),
                };
                // Keep the physical owner and outer EX/operation.lock alive.
                // Continuation still checks the exact token immediately before
                // authenticated installation and again before status publication.
                return Err(match (&mut operation).await {
                    Ok(_) => original,
                    Err(error) => original.context(format!(
                        "backup operation also failed while draining: {error:#}"
                    )),
                });
            }
        }
    }
}

fn install_verified_partial(
    partial: &Path,
    complete: &Path,
    backup_root: &Path,
) -> anyhow::Result<BackupInstallOutcome> {
    install_verified_partial_with(partial, complete, || {
        let directory = open_directory_nofollow(backup_root)?;
        directory.sync_all()?;
        Ok(())
    })
}

fn install_verified_partial_with<F>(
    partial: &Path,
    complete: &Path,
    sync_parent: F,
) -> anyhow::Result<BackupInstallOutcome>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    sync_private_tree_bottom_up(partial)?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        partial,
        rustix::fs::CWD,
        complete,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|error| {
        anyhow::anyhow!("atomically install verified backup without replacement: {error}")
    })?;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        anyhow::ensure!(
            !complete.exists(),
            "completed backup destination already exists"
        );
        std::fs::rename(partial, complete)?;
    }
    Ok(match sync_parent() {
        Ok(()) => BackupInstallOutcome::Installed,
        Err(error) => BackupInstallOutcome::InstalledButParentSyncFailed(error),
    })
}

fn sync_private_tree_bottom_up(root: &Path) -> anyhow::Result<()> {
    let root_metadata = std::fs::symlink_metadata(root)?;
    anyhow::ensure!(
        root_metadata.is_dir() && !root_metadata.file_type().is_symlink(),
        "verified backup staging root must be a real directory"
    );
    let mut pending = vec![(root.to_owned(), false, 0_usize)];
    let mut visited = 0_usize;
    while let Some((path, children_visited, depth)) = pending.pop() {
        anyhow::ensure!(
            depth <= 32,
            "verified backup tree is too deep to sync safely"
        );
        if children_visited {
            open_directory_nofollow(&path)?.sync_all()?;
            continue;
        }
        visited = visited
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("verified backup sync topology count overflows"))?;
        anyhow::ensure!(
            visited <= 1_000_000,
            "verified backup tree is too large to sync safely"
        );
        let metadata = std::fs::symlink_metadata(&path)?;
        anyhow::ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "verified backup directory changed type before durability sync"
        );
        pending.push((path.clone(), true, depth));
        for entry in std::fs::read_dir(&path)? {
            let entry = entry?;
            let child = entry.path();
            let metadata = std::fs::symlink_metadata(&child)?;
            anyhow::ensure!(
                !metadata.file_type().is_symlink() && (metadata.is_dir() || metadata.is_file()),
                "verified backup tree contains a link or special node during durability sync"
            );
            if metadata.is_dir() {
                pending.push((child, false, depth + 1));
            } else {
                open_regular_nofollow(&child)?.sync_all()?;
            }
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "test-support"))]
mod tests;
