//! Backup readiness authority verification, shared by admission and operator status.

use super::AppState;
use crate::backup::{BackupReleaseIdentityV2, BackupStatusV4, load_backup_release_identity};
use crate::error::ApiError;
#[cfg(target_os = "linux")]
use tokio::io::AsyncReadExt as _;

pub(super) async fn ensure_backup_ready(state: &AppState) -> Result<(), ApiError> {
    if let (Some(age), Some(maximum_hours)) = (
        backup_age_ms(state).await?,
        state.config.maximum_backup_age_hours,
    ) {
        let maximum = maximum_hours
            .checked_mul(60 * 60 * 1_000)
            .ok_or(ApiError::Internal)?;
        if age > maximum {
            tracing::error!(age, maximum, "most recent verified backup is stale");
            return Err(ApiError::Unavailable);
        }
    }
    Ok(())
}

pub(super) async fn backup_age_ms(state: &AppState) -> Result<Option<u64>, ApiError> {
    if state.config.backup_manifest_path.is_none() {
        return Ok(None);
    }
    let release_manifest_path = state
        .config
        .release_manifest_path
        .as_deref()
        .ok_or(ApiError::Unavailable)?;
    let active_release = load_backup_release_identity(release_manifest_path)
        .await
        .map_err(|error| {
            tracing::error!(
                %error,
                path = %release_manifest_path.display(),
                "installed release identity is unavailable"
            );
            ApiError::Unavailable
        })?;
    backup_age_ms_with_active_release(state, &active_release).await
}

pub(super) async fn backup_age_ms_with_active_release(
    state: &AppState,
    active_release: &BackupReleaseIdentityV2,
) -> Result<Option<u64>, ApiError> {
    let path = state
        .config
        .backup_manifest_path
        .as_deref()
        .ok_or(ApiError::Unavailable)?;
    let bytes = read_bounded_nofollow(
        path,
        u64::try_from(crate::backup::MAX_BACKUP_STATUS_BYTES).map_err(|_| ApiError::Internal)?,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, path = %path.display(), "backup status manifest is unavailable");
        ApiError::Unavailable
    })?;
    let document: BackupStatusV4 =
        serde_json::from_slice(&bytes).map_err(|_| ApiError::Unavailable)?;
    if robin_run_protocol::canonical_json_bytes(&document).map_err(|_| ApiError::Unavailable)?
        != bytes
    {
        return Err(ApiError::Unavailable);
    }
    document
        .verify(&state.backup_authority_hmac_key)
        .map_err(|_| {
            tracing::error!(
                error_code = "backup_status_authentication_failed",
                "verified backup status is invalid or unauthenticated"
            );
            ApiError::Unavailable
        })?;
    if document.database_schema_version != crate::db::CURRENT_SCHEMA_VERSION {
        tracing::error!(
            status_database_schema = document.database_schema_version,
            current_database_schema = crate::db::CURRENT_SCHEMA_VERSION,
            "verified backup belongs to a different database schema"
        );
        return Err(ApiError::Unavailable);
    }
    let status_parent = path.parent().ok_or(ApiError::Unavailable)?;
    let state_root = status_parent.parent().ok_or(ApiError::Unavailable)?;
    let expected_backup_directory = state_root.join("backups").join(&document.backup_id);
    if std::path::Path::new(&document.backup_directory) != expected_backup_directory {
        tracing::error!(
            backup_id = %document.backup_id,
            "verified backup status directory is not exactly backup-root/backup-id"
        );
        return Err(ApiError::Unavailable);
    }
    if &document.release_identity != active_release {
        tracing::error!(
            status_source_commit = %document.release_identity.source_commit,
            active_source_commit = %active_release.source_commit,
            "verified backup belongs to a different installed release"
        );
        return Err(ApiError::Unavailable);
    }
    let now = u64::try_from(crate::model::now_epoch_ms().map_err(|_| ApiError::Internal)?)
        .map_err(|_| ApiError::Internal)?;
    if document.created_at_unix_ms > now {
        tracing::error!(
            created_at = document.created_at_unix_ms,
            now,
            "verified backup status is future-dated"
        );
        return Err(ApiError::Unavailable);
    }
    Ok(Some(now - document.created_at_unix_ms))
}

pub(super) async fn read_bounded_nofollow(
    path: &std::path::Path,
    maximum: u64,
) -> anyhow::Result<Vec<u8>> {
    #[cfg(target_os = "linux")]
    {
        let parent = pin_readiness_status_parent(path)?;
        return read_bounded_from_pinned_status_parent(path, &parent, maximum, || Ok(())).await;
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, maximum);
        anyhow::bail!(
            "private readiness authority requires Linux openat2 path-resolution guarantees"
        )
    }
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReadinessObjectIdentity {
    device: u64,
    inode: u64,
}

#[cfg(target_os = "linux")]
pub(super) struct PinnedReadinessStatusParent {
    directory: std::fs::File,
    configured_path: std::path::PathBuf,
    identity: ReadinessObjectIdentity,
}

#[cfg(target_os = "linux")]
fn readiness_object_identity(metadata: &std::fs::Metadata) -> ReadinessObjectIdentity {
    use std::os::unix::fs::MetadataExt as _;

    ReadinessObjectIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

#[cfg(target_os = "linux")]
fn validate_readiness_status_parent_metadata(metadata: &std::fs::Metadata) -> anyhow::Result<()> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    anyhow::ensure!(
        metadata.is_dir()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.permissions().mode() & 0o7777 == 0o700,
        "private readiness status parent must be an exact euid-owned mode-0700 directory"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
pub(super) fn pin_readiness_status_parent(
    configured_path: &std::path::Path,
) -> anyhow::Result<PinnedReadinessStatusParent> {
    let configured_parent = configured_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("private readiness document has no configured parent"))?;
    anyhow::ensure!(
        configured_path.is_absolute()
            && configured_path
                == configured_parent.join(std::path::Path::new("backup-status.json")),
        "private readiness document must be the exact configured backup-status.json path"
    );
    let configured_anchor = configured_parent.parent().ok_or_else(|| {
        anyhow::anyhow!("private readiness status parent has no configured anchor")
    })?;
    let parent_name = configured_parent.file_name().ok_or_else(|| {
        anyhow::anyhow!("private readiness status parent has no exact child name")
    })?;
    // The configured state root may itself be a dedicated mount. Pin it with
    // symlink/magic-link rejection, then prohibit any mount crossing beneath
    // that authority while resolving its exact status-directory child.
    let anchor_descriptor = rustix::fs::openat2(
        rustix::fs::CWD,
        configured_anchor,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::DIRECTORY,
        rustix::fs::Mode::empty(),
        rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
    )?;
    let anchor = std::fs::File::from(anchor_descriptor);
    use std::os::fd::AsFd as _;
    let descriptor = rustix::fs::openat2(
        anchor.as_fd(),
        std::path::Path::new(parent_name),
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::DIRECTORY,
        rustix::fs::Mode::empty(),
        rustix::fs::ResolveFlags::BENEATH
            | rustix::fs::ResolveFlags::NO_SYMLINKS
            | rustix::fs::ResolveFlags::NO_MAGICLINKS
            | rustix::fs::ResolveFlags::NO_XDEV,
    )?;
    let directory = std::fs::File::from(descriptor);
    let metadata = directory.metadata()?;
    validate_readiness_status_parent_metadata(&metadata)?;
    let identity = readiness_object_identity(&metadata);
    let pinned = PinnedReadinessStatusParent {
        directory,
        configured_path: configured_parent.to_owned(),
        identity,
    };
    revalidate_readiness_status_parent(configured_path, &pinned)?;
    Ok(pinned)
}

#[cfg(target_os = "linux")]
fn revalidate_readiness_status_parent(
    configured_path: &std::path::Path,
    pinned: &PinnedReadinessStatusParent,
) -> anyhow::Result<()> {
    let configured_parent = configured_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("private readiness document has no configured parent"))?;
    anyhow::ensure!(
        configured_path.is_absolute()
            && configured_parent == pinned.configured_path
            && configured_path
                == configured_parent.join(std::path::Path::new("backup-status.json")),
        "private readiness parent capability is not bound to the exact configured path"
    );
    let descriptor_metadata = pinned.directory.metadata()?;
    validate_readiness_status_parent_metadata(&descriptor_metadata)?;
    anyhow::ensure!(
        readiness_object_identity(&descriptor_metadata) == pinned.identity,
        "private readiness parent capability changed identity"
    );
    let path_metadata = std::fs::symlink_metadata(configured_parent)?;
    validate_readiness_status_parent_metadata(&path_metadata)?;
    anyhow::ensure!(
        readiness_object_identity(&path_metadata) == pinned.identity
            && std::fs::canonicalize(configured_parent)? == configured_parent,
        "private readiness parent path was replaced, redirected, or is non-canonical"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_readiness_status_leaf_metadata(
    metadata: &std::fs::Metadata,
    parent_device: u64,
    maximum: u64,
) -> anyhow::Result<()> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    anyhow::ensure!(
        metadata.is_file()
            && metadata.len() <= maximum
            && metadata.dev() == parent_device
            && metadata.nlink() == 1
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.permissions().mode() & 0o7777 == 0o400,
        "private readiness document must be a bounded same-device euid-owned mode-0400 regular file with one link"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn open_readiness_status_leaf(
    parent: &PinnedReadinessStatusParent,
    maximum: u64,
) -> anyhow::Result<(std::fs::File, ReadinessObjectIdentity)> {
    use std::os::fd::AsFd as _;

    let descriptor = rustix::fs::openat2(
        parent.directory.as_fd(),
        std::path::Path::new("backup-status.json"),
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
        rustix::fs::ResolveFlags::BENEATH
            | rustix::fs::ResolveFlags::NO_SYMLINKS
            | rustix::fs::ResolveFlags::NO_MAGICLINKS
            | rustix::fs::ResolveFlags::NO_XDEV,
    )?;
    let file = std::fs::File::from(descriptor);
    let metadata = file.metadata()?;
    validate_readiness_status_leaf_metadata(&metadata, parent.identity.device, maximum)?;
    Ok((file, readiness_object_identity(&metadata)))
}

#[cfg(target_os = "linux")]
async fn read_bounded_readiness_file(file: std::fs::File, maximum: u64) -> anyhow::Result<Vec<u8>> {
    let read_limit = maximum
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("private readiness byte limit overflows"))?;
    let mut bytes = Vec::with_capacity(usize::try_from(maximum.min(64 * 1024))?);
    tokio::fs::File::from_std(file)
        .take(read_limit)
        .read_to_end(&mut bytes)
        .await?;
    anyhow::ensure!(
        bytes.len() <= usize::try_from(maximum)?,
        "private readiness document exceeds its byte limit"
    );
    Ok(bytes)
}

#[cfg(target_os = "linux")]
pub(super) async fn read_bounded_from_pinned_status_parent<F>(
    configured_path: &std::path::Path,
    parent: &PinnedReadinessStatusParent,
    maximum: u64,
    before_final_revalidation: F,
) -> anyhow::Result<Vec<u8>>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    revalidate_readiness_status_parent(configured_path, parent)?;
    let (file, leaf_identity) = open_readiness_status_leaf(parent, maximum)?;
    let bytes = read_bounded_readiness_file(file, maximum).await?;
    let json: serde_json::Value = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        robin_run_protocol::canonical_json_bytes(&json)? == bytes,
        "private readiness document is not canonical JSON"
    );

    before_final_revalidation()?;

    revalidate_readiness_status_parent(configured_path, parent)?;
    let (final_file, final_leaf_identity) = open_readiness_status_leaf(parent, maximum)?;
    anyhow::ensure!(
        final_leaf_identity == leaf_identity,
        "private readiness document changed inode during verification"
    );
    let final_bytes = read_bounded_readiness_file(final_file, maximum).await?;
    anyhow::ensure!(
        final_bytes == bytes,
        "private readiness document changed bytes during verification"
    );
    let path_metadata = std::fs::symlink_metadata(configured_path)?;
    validate_readiness_status_leaf_metadata(&path_metadata, parent.identity.device, maximum)?;
    anyhow::ensure!(
        readiness_object_identity(&path_metadata) == leaf_identity
            && std::fs::canonicalize(configured_path)? == configured_path,
        "private readiness document path was replaced, redirected, or is non-canonical"
    );
    revalidate_readiness_status_parent(configured_path, parent)?;
    Ok(bytes)
}
