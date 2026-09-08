//! Pinned offline/transaction verification and authenticated inventory checks.

use super::filesystem::BackupTreePaths;
use super::filesystem::acquire_backup_operation_lock;
use super::filesystem::backup_tree_paths_cap;
use super::filesystem::duplicate_inherited_fd;
use super::filesystem::duplicate_pinned_file;
use super::filesystem::metadata_identity;
use super::filesystem::metadata_identity_std;
use super::filesystem::open_cap_directory_nofollow;
use super::filesystem::open_cap_regular_nofollow;
use super::filesystem::pin_directory_capability;
use super::filesystem::pinned_file_target;
use super::filesystem::read_bounded_pinned_file;
use super::filesystem::read_bounded_regular_nofollow;
use super::filesystem::read_cap_regular_bounded;
use super::filesystem::read_cap_regular_bounded_with_mode;
use super::filesystem::read_cap_regular_bounded_with_mode_and_identity;
use super::filesystem::record_cap_file_with_identity;
use super::filesystem::require_pinned_file_name;
use super::filesystem::revalidate_operation_lock_path;
use super::filesystem::revalidate_pinned_directory_path;
use super::filesystem::revalidate_pinned_regular_path;
use super::filesystem::revalidate_pinned_root_directory;
use super::filesystem::validate_expected_digest;
use super::filesystem::validate_private_pinned_file;
use super::policy::DEFAULT_BACKUP_AUTHORITY_KEY;
use super::policy::DEFAULT_BACKUP_ROOT;
use super::policy::DEFAULT_BACKUP_STATUS;
use super::policy::INSTALLED_RELEASE_ROOT;
use super::policy::RELEASE_AUTHORITY_STORE;
use super::sources::load_preserved_release_authority;
use super::sources::pin_preserved_release_authority_from_root;
use robin_highscores::backup::BackupCurrentStatusEvidenceV2;
use robin_highscores::backup::BackupFileV4 as BackupFile;
use robin_highscores::backup::BackupManifestV4 as BackupManifest;
use robin_highscores::backup::BackupReleaseIdentityV2;
use robin_highscores::backup::BackupStatusV4;
use robin_highscores::backup::BackupVerificationEnvelopeV2;
use robin_highscores::backup::BackupVerificationReceiptV2;
use robin_highscores::backup::load_backup_release_identity_oob_file;
use robin_highscores::backup::load_backup_release_identity_preserved_file;
use robin_highscores::backup::parse_backup_id;
use robin_run_protocol::canonical_json_bytes;
use sha2::Digest as _;
use sha2::Sha256;
use sqlx::Connection as _;
use sqlx::Row as _;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VerifiedBackup {
    created_at_unix_ms: u64,
    database_schema_version: i64,
    manifest_sha256: String,
    release_identity: BackupReleaseIdentityV2,
    file_count: u64,
    directory_count: u64,
    total_bytes: u64,
    tree: BackupTreePaths,
}

impl VerifiedBackup {
    pub(super) fn created_at_unix_ms(&self) -> u64 {
        self.created_at_unix_ms
    }
    pub(super) fn database_schema_version(&self) -> i64 {
        self.database_schema_version
    }
    pub(super) fn manifest_sha256(&self) -> &String {
        &self.manifest_sha256
    }
    pub(super) fn release_identity(&self) -> &BackupReleaseIdentityV2 {
        &self.release_identity
    }
    pub(super) fn file_count(&self) -> u64 {
        self.file_count
    }
    pub(super) fn directory_count(&self) -> u64 {
        self.directory_count
    }
    pub(super) fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
    pub(super) fn tree(&self) -> &BackupTreePaths {
        &self.tree
    }
    pub(super) fn into_tree(self) -> BackupTreePaths {
        self.tree
    }
}

pub(super) struct GuardedBackupVerificationReceipt {
    receipt: BackupVerificationReceiptV2,
    operation_lock: std::fs::File,
    trusted_backup_root: PathBuf,
    _status_guard: Option<std::fs::File>,
    _backup_directory_guard: std::fs::File,
}

impl GuardedBackupVerificationReceipt {
    #[cfg(test)]
    pub(super) fn receipt(&self) -> &BackupVerificationReceiptV2 {
        &self.receipt
    }
}

pub(super) struct GuardedVerifierCommand {
    backup: GuardedBackupVerificationReceipt,
    release_guard: std::fs::File,
    release_store_guard: Option<std::fs::File>,
    release_target: PathBuf,
    release_expected_mode: u32,
    expected_release: BackupReleaseIdentityV2,
    historical_release_authority: bool,
    backup_authority_key_guard: std::fs::File,
    expected_backup_authority_key: [u8; 32],
    backup_root_guard: std::fs::File,
}

impl GuardedVerifierCommand {
    pub(super) async fn canonical_receipt_bytes(&self) -> anyhow::Result<Vec<u8>> {
        revalidate_pinned_root_directory(
            &self.backup_root_guard,
            Path::new(DEFAULT_BACKUP_ROOT),
            "backup root",
        )?;
        revalidate_operation_lock_path(
            &self.backup.trusted_backup_root,
            &self.backup.operation_lock,
        )?;
        if let Some(store) = &self.release_store_guard {
            revalidate_pinned_directory_path(
                store,
                &Path::new(DEFAULT_BACKUP_ROOT).join(RELEASE_AUTHORITY_STORE),
                Path::new(DEFAULT_BACKUP_ROOT),
                "preserved release-authority store",
            )?;
        }
        revalidate_pinned_regular_path(
            &self.release_guard,
            &self.release_target,
            self.release_expected_mode,
            Some(if self.historical_release_authority {
                0o700
            } else {
                0o550
            }),
            "out-of-band release manifest",
        )?;
        revalidate_pinned_regular_path(
            &self.backup_authority_key_guard,
            Path::new(DEFAULT_BACKUP_AUTHORITY_KEY),
            0o400,
            Some(0o700),
            "backup authority HMAC key",
        )?;
        anyhow::ensure!(
            read_bounded_pinned_file(
                duplicate_pinned_file(&self.backup_authority_key_guard, false)?,
                32,
            )? == self.expected_backup_authority_key,
            "backup authority HMAC key changed during verification"
        );
        let final_release = if self.historical_release_authority {
            load_backup_release_identity_preserved_file(duplicate_pinned_file(
                &self.release_guard,
                false,
            )?)
            .await?
        } else {
            load_backup_release_identity_oob_file(duplicate_pinned_file(
                &self.release_guard,
                false,
            )?)
            .await?
        };
        anyhow::ensure!(
            final_release == self.expected_release,
            "out-of-band release authority changed during verification"
        );
        revalidate_pinned_regular_path(
            &self.release_guard,
            &self.release_target,
            self.release_expected_mode,
            Some(if self.historical_release_authority {
                0o700
            } else {
                0o550
            }),
            "out-of-band release manifest",
        )?;
        revalidate_pinned_regular_path(
            &self.backup_authority_key_guard,
            Path::new(DEFAULT_BACKUP_AUTHORITY_KEY),
            0o400,
            Some(0o700),
            "backup authority HMAC key",
        )?;
        revalidate_operation_lock_path(
            &self.backup.trusted_backup_root,
            &self.backup.operation_lock,
        )?;
        if let Some(store) = &self.release_store_guard {
            revalidate_pinned_directory_path(
                store,
                &Path::new(DEFAULT_BACKUP_ROOT).join(RELEASE_AUTHORITY_STORE),
                Path::new(DEFAULT_BACKUP_ROOT),
                "preserved release-authority store",
            )?;
        }
        revalidate_pinned_root_directory(
            &self.backup_root_guard,
            Path::new(DEFAULT_BACKUP_ROOT),
            "backup root",
        )?;
        Ok(canonical_json_bytes(&self.backup.receipt)?)
    }
}

#[derive(Clone, Copy)]
pub(super) enum BackupManifestExpectation<'a> {
    Explicit(&'a str),
    AuthenticatedStatus,
}

pub(super) enum BackupVerificationMode<'a> {
    Offline {
        backup_root_fd: u32,
        backup_directory_fd: u32,
        expected_manifest_sha256: &'a str,
    },
    Transaction {
        backup_root_fd: u32,
    },
}

#[cfg(test)]
async fn verify_backup_with_expected(
    directory: &Path,
    expected_manifest_sha256: &str,
    expected_release: &BackupReleaseIdentityV2,
) -> anyhow::Result<VerifiedBackup> {
    expected_release.validate()?;
    anyhow::ensure!(
        expected_manifest_sha256.len() == 64
            && expected_manifest_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && expected_manifest_sha256.bytes().any(|byte| byte != b'0'),
        "expected backup manifest digest is not canonical nonzero SHA-256"
    );
    let verified = verify_backup(directory).await?;
    let manifest_bytes = read_bounded_regular_nofollow(
        &directory.join("backup-manifest.json"),
        u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
    )
    .await?;
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
    manifest.validate_production_restore_paths()?;
    anyhow::ensure!(
        verified.manifest_sha256 == expected_manifest_sha256,
        "backup manifest differs from the out-of-band expected digest"
    );
    anyhow::ensure!(
        &verified.release_identity == expected_release,
        "backup release identity differs from the out-of-band expected identity"
    );
    Ok(verified)
}

pub(super) fn pin_transaction_backup_from_status(
    backup_root_fd: u32,
    status_envelope_fd: u32,
    trusted_backup_root: &Path,
    trusted_status_path: &Path,
) -> anyhow::Result<(std::fs::File, std::fs::File)> {
    let root_guard = duplicate_inherited_fd(backup_root_fd, true)?;
    revalidate_pinned_root_directory(&root_guard, trusted_backup_root, "transaction backup root")?;
    let status_file = duplicate_inherited_fd(status_envelope_fd, false)?;
    anyhow::ensure!(
        require_pinned_file_name(&status_file, "backup-status.json")? == trusted_status_path,
        "transaction status envelope is outside the trusted authority"
    );
    validate_private_pinned_file(&status_file, 0o400, "transaction status envelope")?;
    let status_bytes = read_bounded_pinned_file(
        duplicate_pinned_file(&status_file, false)?,
        u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
    )?;
    let status: BackupStatusV4 = serde_json::from_slice(&status_bytes)?;
    anyhow::ensure!(
        canonical_json_bytes(&status)? == status_bytes,
        "transaction status envelope is not canonical JSON"
    );
    let timestamp = parse_backup_id(&status.backup_id)
        .ok_or_else(|| anyhow::anyhow!("transaction status has an invalid backup ID"))?;
    let exact_child_path = trusted_backup_root.join(&status.backup_id);
    anyhow::ensure!(
        timestamp == status.created_at_unix_ms
            && status.backup_directory == exact_child_path.to_string_lossy(),
        "transaction status does not select the exact canonical backup child"
    );

    let root = cap_std::fs::Dir::from_std_file(duplicate_pinned_file(&root_guard, true)?);
    let child = open_cap_directory_nofollow(&root, Path::new(&status.backup_id))?.into_std_file();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = child.metadata()?;
        anyhow::ensure!(
            metadata.uid() == rustix::process::geteuid().as_raw()
                && metadata.permissions().mode() & 0o777 == 0o700,
            "transaction backup child has the wrong owner or mode"
        );
    }
    anyhow::ensure!(
        pinned_file_target(&child)? == exact_child_path,
        "transaction backup child is not linked at its authenticated path"
    );
    revalidate_pinned_root_directory(&root_guard, trusted_backup_root, "transaction backup root")?;
    Ok((root_guard, child))
}

fn pin_offline_backup_from_root(
    backup_root_fd: u32,
    backup_directory_fd: u32,
    trusted_backup_root: &Path,
) -> anyhow::Result<(std::fs::File, std::fs::File, String)> {
    let root_guard = duplicate_inherited_fd(backup_root_fd, true)?;
    revalidate_pinned_root_directory(&root_guard, trusted_backup_root, "offline backup root")?;
    let directory_guard = duplicate_inherited_fd(backup_directory_fd, true)?;
    let target = pinned_file_target(&directory_guard)?;
    let backup_id = target
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| parse_backup_id(name).is_some())
        .ok_or_else(|| anyhow::anyhow!("offline backup child has a noncanonical identifier"))?
        .to_owned();
    anyhow::ensure!(
        target == trusted_backup_root.join(&backup_id),
        "offline backup child is outside the canonical backup root"
    );
    let root = cap_std::fs::Dir::from_std_file(duplicate_pinned_file(&root_guard, true)?);
    let current = open_cap_directory_nofollow(&root, Path::new(&backup_id))?.into_std_file();
    let root_metadata = root_guard.metadata()?;
    let child_metadata = directory_guard.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            child_metadata.uid() == rustix::process::geteuid().as_raw()
                && child_metadata.dev() == root_metadata.dev()
                && child_metadata.permissions().mode() & 0o777 == 0o700,
            "offline backup child has the wrong owner, device, or mode"
        );
    }
    anyhow::ensure!(
        metadata_identity_std(&current.metadata()?) == metadata_identity_std(&child_metadata),
        "offline backup child descriptor differs from the canonical root entry"
    );
    revalidate_pinned_root_directory(&root_guard, trusted_backup_root, "offline backup root")?;
    Ok((root_guard, directory_guard, backup_id))
}

fn authenticated_release_identity_from_backup(
    directory: &std::fs::File,
    backup_id: &str,
    backup_authority_key: &[u8; 32],
) -> anyhow::Result<(BackupReleaseIdentityV2, String)> {
    let root = cap_std::fs::Dir::from_std_file(duplicate_pinned_file(directory, true)?);
    let manifest_bytes = read_cap_regular_bounded(
        &root,
        Path::new("backup-manifest.json"),
        u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
    )?;
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
    anyhow::ensure!(
        canonical_json_bytes(&manifest)? == manifest_bytes,
        "offline backup manifest is not canonical JSON"
    );
    manifest.validate()?;
    anyhow::ensure!(
        parse_backup_id(backup_id) == Some(manifest.created_at_unix_ms),
        "offline backup ID differs from its manifest timestamp"
    );
    let envelope_bytes = read_cap_regular_bounded_with_mode(
        &root,
        Path::new("backup-verification-envelope.json"),
        u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
        0o400,
    )?;
    let envelope: BackupVerificationEnvelopeV2 = serde_json::from_slice(&envelope_bytes)?;
    anyhow::ensure!(
        canonical_json_bytes(&envelope)? == envelope_bytes,
        "offline backup verification envelope is not canonical JSON"
    );
    envelope.verify_manifest(backup_authority_key, backup_id, &manifest)?;
    Ok((
        manifest.release_identity,
        hex::encode(Sha256::digest(&manifest_bytes)),
    ))
}

pub(super) async fn run_pinned_verifier_command(
    mode: BackupVerificationMode<'_>,
    status_envelope_fd: Option<u32>,
    backup_authority_key_fd: u32,
    expected_release_manifest_fd: Option<u32>,
    expected_source_commit: &str,
    expected_vps_release_manifest_sha256: &str,
    expected_publication_lock_sha256: &str,
) -> anyhow::Result<GuardedVerifierCommand> {
    let backup_authority_key_guard = duplicate_inherited_fd(backup_authority_key_fd, false)?;
    let backup_authority_key_target =
        require_pinned_file_name(&backup_authority_key_guard, "backup-authority-hmac.key")?;
    anyhow::ensure!(
        backup_authority_key_target == Path::new(DEFAULT_BACKUP_AUTHORITY_KEY),
        "pinned backup-authority key is outside the trusted production authority"
    );
    validate_private_pinned_file(
        &backup_authority_key_guard,
        0o400,
        "backup authority HMAC key",
    )?;
    let backup_authority_key: [u8; 32] = read_bounded_pinned_file(
        duplicate_pinned_file(&backup_authority_key_guard, false)?,
        32,
    )?
    .try_into()
    .map_err(|_| anyhow::anyhow!("backup authority HMAC key has the wrong length"))?;
    anyhow::ensure!(
        backup_authority_key.iter().any(|byte| *byte != 0),
        "backup authority HMAC key is all zero"
    );
    let (
        backup,
        pinned_release,
        release_store_guard,
        release_target,
        release_expected_mode,
        expected_release,
        historical_release_authority,
        backup_root_guard,
    ) = match mode {
        BackupVerificationMode::Offline {
            backup_root_fd,
            backup_directory_fd,
            expected_manifest_sha256,
        } => {
            anyhow::ensure!(
                status_envelope_fd.is_none() && expected_release_manifest_fd.is_none(),
                "offline verification derives authority from the authenticated backup envelope"
            );
            let (root_guard, directory_guard, backup_id) = pin_offline_backup_from_root(
                backup_root_fd,
                backup_directory_fd,
                Path::new(DEFAULT_BACKUP_ROOT),
            )?;
            let (authenticated_release, authenticated_manifest_sha256) =
                authenticated_release_identity_from_backup(
                    &directory_guard,
                    &backup_id,
                    &backup_authority_key,
                )?;
            anyhow::ensure!(
                authenticated_manifest_sha256 == expected_manifest_sha256,
                "authenticated backup manifest differs from the explicit expected digest"
            );
            anyhow::ensure!(
                authenticated_release.source_commit == expected_source_commit
                    && authenticated_release.vps_release_manifest_sha256
                        == expected_vps_release_manifest_sha256
                    && authenticated_release.publication_lock_sha256
                        == expected_publication_lock_sha256,
                "authenticated backup release differs from the explicit expected identity"
            );
            let (store_guard, release_guard, release_target) =
                pin_preserved_release_authority_from_root(
                    &root_guard,
                    &authenticated_release,
                    Path::new(DEFAULT_BACKUP_ROOT),
                )?;
            let expected_release = load_backup_release_identity_preserved_file(
                duplicate_pinned_file(&release_guard, false)?,
            )
            .await?;
            anyhow::ensure!(
                expected_release == authenticated_release,
                "preserved release authority differs from the authenticated backup envelope"
            );
            let backup = verify_backup_pinned_offline_with_expected(
                backup_directory_fd,
                expected_manifest_sha256,
                &expected_release,
                &backup_authority_key,
                Path::new(DEFAULT_BACKUP_ROOT),
                true,
            )
            .await?;
            (
                backup,
                release_guard,
                Some(store_guard),
                release_target,
                0o400,
                expected_release,
                true,
                root_guard,
            )
        }
        BackupVerificationMode::Transaction { backup_root_fd } => {
            let release_fd = expected_release_manifest_fd.ok_or_else(|| {
                anyhow::anyhow!("transaction verification requires a pinned release manifest")
            })?;
            let pinned_release = duplicate_inherited_fd(release_fd, false)?;
            let release_target =
                require_pinned_file_name(&pinned_release, "vps-release-manifest-v2.json")?;
            anyhow::ensure!(
                release_target
                    == Path::new(INSTALLED_RELEASE_ROOT)
                        .join(expected_source_commit)
                        .join("vps-release-manifest-v2.json"),
                "transaction release manifest is outside the exact installed source release"
            );
            validate_private_pinned_file(&pinned_release, 0o440, "out-of-band release authority")?;
            let expected_release = load_backup_release_identity_oob_file(duplicate_pinned_file(
                &pinned_release,
                false,
            )?)
            .await?;
            anyhow::ensure!(
                expected_release.source_commit == expected_source_commit
                    && expected_release.vps_release_manifest_sha256
                        == expected_vps_release_manifest_sha256
                    && expected_release.publication_lock_sha256 == expected_publication_lock_sha256,
                "out-of-band release manifest differs from the explicit expected release identity"
            );
            let status_envelope_fd = status_envelope_fd.ok_or_else(|| {
                anyhow::anyhow!("transaction verification requires a pinned status envelope")
            })?;
            let (backup, root_guard) = verify_transaction_backup_from_root_pinned(
                backup_root_fd,
                status_envelope_fd,
                &expected_release,
                &backup_authority_key,
                Path::new(DEFAULT_BACKUP_ROOT),
                Path::new(DEFAULT_BACKUP_STATUS),
                true,
            )
            .await?;
            (
                backup,
                pinned_release,
                None,
                release_target,
                0o440,
                expected_release,
                false,
                root_guard,
            )
        }
    };
    Ok(GuardedVerifierCommand {
        backup,
        release_guard: pinned_release,
        release_store_guard,
        release_target,
        release_expected_mode,
        expected_release,
        historical_release_authority,
        backup_authority_key_guard,
        expected_backup_authority_key: backup_authority_key,
        backup_root_guard,
    })
}

#[cfg(test)]
pub(super) async fn verify_backup_pinned_with_expected(
    backup_directory_fd: u32,
    status_envelope_fd: u32,
    expected_manifest_sha256: &str,
    expected_release: &BackupReleaseIdentityV2,
    trusted_backup_root: &Path,
    trusted_status_path: &Path,
    require_production_restore_paths: bool,
) -> anyhow::Result<GuardedBackupVerificationReceipt> {
    verify_backup_pinned_with_expected_and_hook(
        backup_directory_fd,
        status_envelope_fd,
        BackupManifestExpectation::Explicit(expected_manifest_sha256),
        expected_release,
        trusted_backup_root,
        trusted_status_path,
        require_production_restore_paths,
        || Ok(()),
    )
    .await
}

pub(super) async fn verify_backup_pinned_offline_with_expected(
    backup_directory_fd: u32,
    expected_manifest_sha256: &str,
    expected_release: &BackupReleaseIdentityV2,
    backup_authority_key: &[u8; 32],
    trusted_backup_root: &Path,
    require_production_restore_paths: bool,
) -> anyhow::Result<GuardedBackupVerificationReceipt> {
    verify_backup_pinned_core(
        backup_directory_fd,
        None,
        BackupManifestExpectation::Explicit(expected_manifest_sha256),
        expected_release,
        backup_authority_key,
        trusted_backup_root,
        None,
        require_production_restore_paths,
        || Ok(()),
    )
    .await
}

async fn verify_transaction_backup_pinned(
    backup_directory_fd: u32,
    status_envelope_fd: u32,
    expected_release: &BackupReleaseIdentityV2,
    backup_authority_key: &[u8; 32],
    trusted_backup_root: &Path,
    trusted_status_path: &Path,
    require_production_restore_paths: bool,
) -> anyhow::Result<GuardedBackupVerificationReceipt> {
    verify_backup_pinned_core(
        backup_directory_fd,
        Some(status_envelope_fd),
        BackupManifestExpectation::AuthenticatedStatus,
        expected_release,
        backup_authority_key,
        trusted_backup_root,
        Some(trusted_status_path),
        require_production_restore_paths,
        || Ok(()),
    )
    .await
}

pub(super) async fn verify_transaction_backup_from_root_pinned(
    backup_root_fd: u32,
    status_envelope_fd: u32,
    expected_release: &BackupReleaseIdentityV2,
    backup_authority_key: &[u8; 32],
    trusted_backup_root: &Path,
    trusted_status_path: &Path,
    require_production_restore_paths: bool,
) -> anyhow::Result<(GuardedBackupVerificationReceipt, std::fs::File)> {
    let (root_guard, backup_directory) = pin_transaction_backup_from_status(
        backup_root_fd,
        status_envelope_fd,
        trusted_backup_root,
        trusted_status_path,
    )?;
    #[cfg(target_os = "linux")]
    let backup_directory_fd = {
        use std::os::fd::AsRawFd as _;
        u32::try_from(backup_directory.as_raw_fd())?
    };
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("transaction backup verification requires Linux procfs");
    let backup = verify_transaction_backup_pinned(
        backup_directory_fd,
        status_envelope_fd,
        expected_release,
        backup_authority_key,
        trusted_backup_root,
        trusted_status_path,
        require_production_restore_paths,
    )
    .await?;
    Ok((backup, root_guard))
}

#[cfg(test)]
pub(super) async fn verify_backup_pinned_with_expected_and_hook<F>(
    backup_directory_fd: u32,
    status_envelope_fd: u32,
    manifest_expectation: BackupManifestExpectation<'_>,
    expected_release: &BackupReleaseIdentityV2,
    trusted_backup_root: &Path,
    trusted_status_path: &Path,
    require_production_restore_paths: bool,
    before_final_authority_check: F,
) -> anyhow::Result<GuardedBackupVerificationReceipt>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    verify_backup_pinned_core(
        backup_directory_fd,
        Some(status_envelope_fd),
        manifest_expectation,
        expected_release,
        &[0x31; 32],
        trusted_backup_root,
        Some(trusted_status_path),
        require_production_restore_paths,
        before_final_authority_check,
    )
    .await
}

async fn verify_backup_pinned_core<F>(
    backup_directory_fd: u32,
    status_envelope_fd: Option<u32>,
    manifest_expectation: BackupManifestExpectation<'_>,
    expected_release: &BackupReleaseIdentityV2,
    backup_authority_key: &[u8; 32],
    trusted_backup_root: &Path,
    trusted_status_path: Option<&Path>,
    require_production_restore_paths: bool,
    before_final_authority_check: F,
) -> anyhow::Result<GuardedBackupVerificationReceipt>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    if let BackupManifestExpectation::Explicit(expected_digest) = manifest_expectation {
        validate_expected_digest(expected_digest)?;
    }
    expected_release.validate()?;
    anyhow::ensure!(
        trusted_status_path.is_none_or(Path::is_absolute) && trusted_backup_root.is_absolute(),
        "pinned verifier authority paths must be absolute"
    );
    let operation_lock = acquire_backup_operation_lock(trusted_backup_root)?;
    let status_authority = match (status_envelope_fd, trusted_status_path) {
        (Some(fd), Some(path)) => {
            let file = duplicate_inherited_fd(fd, false)?;
            let target = require_pinned_file_name(&file, "backup-status.json")?;
            anyhow::ensure!(
                target == path,
                "pinned status envelope is outside the trusted production authority"
            );
            validate_private_pinned_file(&file, 0o400, "backup status envelope")?;
            let bytes = read_bounded_pinned_file(
                duplicate_pinned_file(&file, false)?,
                u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
            )?;
            let status: BackupStatusV4 = serde_json::from_slice(&bytes)?;
            anyhow::ensure!(
                canonical_json_bytes(&status)? == bytes,
                "pinned backup status envelope is not canonical JSON"
            );
            status.verify(backup_authority_key)?;
            Some((file, bytes, status, path))
        }
        (None, None) => None,
        _ => anyhow::bail!("status descriptor and trusted path must be supplied together"),
    };

    let directory_guard = duplicate_inherited_fd(backup_directory_fd, true)?;
    let directory_target = pinned_file_target(&directory_guard)?;
    let root_metadata = directory_guard.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(
            root_metadata.uid() == rustix::process::geteuid().as_raw()
                && root_metadata.permissions().mode() & 0o777 == 0o700,
            "pinned backup directory has the wrong owner or mode"
        );
    }
    let backup_id = directory_target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("pinned backup directory has no UTF-8 identifier"))?;
    anyhow::ensure!(
        parse_backup_id(backup_id).is_some()
            && directory_target.parent() == Some(trusted_backup_root)
            && status_authority.as_ref().is_none_or(|(_, _, status, _)| {
                directory_target == Path::new(&status.backup_directory)
                    && status.backup_id == backup_id
            }),
        "pinned backup directory name differs from the authenticated backup ID"
    );
    let root = cap_std::fs::Dir::from_std_file(duplicate_pinned_file(&directory_guard, true)?);
    let require_current_schema = matches!(
        manifest_expectation,
        BackupManifestExpectation::AuthenticatedStatus
    );
    let verified = verify_backup_capability(
        &root,
        backup_id,
        backup_authority_key,
        require_current_schema,
    )
    .await?;
    anyhow::ensure!(
        &verified.release_identity == expected_release,
        "pinned backup differs from the out-of-band release identity"
    );
    if let BackupManifestExpectation::Explicit(expected_digest) = manifest_expectation {
        anyhow::ensure!(
            verified.manifest_sha256 == expected_digest,
            "pinned backup differs from the out-of-band manifest digest"
        );
    }
    let manifest_bytes = read_cap_regular_bounded(
        &root,
        Path::new("backup-manifest.json"),
        u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
    )?;
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
    if require_production_restore_paths {
        manifest.validate_production_restore_paths()?;
    }
    if let Some((_, _, status, _)) = &status_authority {
        anyhow::ensure!(
            status.backup_manifest_sha256 == verified.manifest_sha256
                && status.release_identity == verified.release_identity
                && status.database_schema_version == verified.database_schema_version
                && status.created_at_unix_ms == verified.created_at_unix_ms
                && status.file_count == verified.file_count
                && status.directory_count == verified.directory_count
                && status.total_bytes == verified.total_bytes,
            "authenticated status envelope differs from the pinned verified backup"
        );
    }
    let envelope_bytes = read_cap_regular_bounded_with_mode(
        &root,
        Path::new("backup-verification-envelope.json"),
        u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
        0o400,
    )?;
    let receipt_verified = verified.clone();
    let receipt = BackupVerificationReceiptV2 {
        schema_version: 2,
        verification_envelope_sha256: hex::encode(Sha256::digest(&envelope_bytes)),
        verification_envelope_byte_length: u64::try_from(envelope_bytes.len())?,
        current_status: status_authority.as_ref().map(|(_, bytes, _, _)| {
            BackupCurrentStatusEvidenceV2 {
                sha256: hex::encode(Sha256::digest(bytes)),
                byte_length: u64::try_from(bytes.len()).expect("bounded status length fits u64"),
            }
        }),
        backup_id: backup_id.to_owned(),
        backup_directory: directory_target.to_string_lossy().into_owned(),
        backup_manifest_sha256: verified.manifest_sha256,
        release_identity: verified.release_identity,
        database_schema_version: verified.database_schema_version,
        file_count: verified.file_count,
        directory_count: verified.directory_count,
        total_bytes: verified.total_bytes,
    };
    receipt.validate()?;
    before_final_authority_check()?;
    revalidate_operation_lock_path(trusted_backup_root, &operation_lock)?;
    if let Some((status_file, status_bytes, _, trusted_status_path)) = &status_authority {
        revalidate_pinned_regular_path(
            status_file,
            trusted_status_path,
            0o400,
            Some(0o700),
            "backup status envelope",
        )?;
        anyhow::ensure!(
            read_bounded_pinned_file(
                duplicate_pinned_file(status_file, false)?,
                u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
            )? == *status_bytes,
            "pinned backup status bytes changed during verification"
        );
        revalidate_pinned_regular_path(
            status_file,
            trusted_status_path,
            0o400,
            Some(0o700),
            "backup status envelope",
        )?;
    }
    revalidate_pinned_directory_path(
        &directory_guard,
        Path::new(&receipt.backup_directory),
        trusted_backup_root,
        "backup payload directory",
    )?;
    let final_verified = verify_backup_capability(
        &root,
        backup_id,
        backup_authority_key,
        require_current_schema,
    )
    .await?;
    anyhow::ensure!(
        final_verified == receipt_verified,
        "pinned backup verification result changed at the receipt boundary"
    );
    revalidate_pinned_directory_path(
        &directory_guard,
        Path::new(&receipt.backup_directory),
        trusted_backup_root,
        "backup payload directory",
    )?;
    revalidate_operation_lock_path(trusted_backup_root, &operation_lock)?;
    Ok(GuardedBackupVerificationReceipt {
        receipt,
        operation_lock,
        trusted_backup_root: trusted_backup_root.to_owned(),
        _status_guard: status_authority.map(|(file, _, _, _)| file),
        _backup_directory_guard: directory_guard,
    })
}

pub(super) async fn verify_backup_authenticated(
    directory: &Path,
    backup_authority_key: &[u8; 32],
) -> anyhow::Result<VerifiedBackup> {
    verify_backup_with_schema_policy(directory, true, backup_authority_key).await
}

#[cfg(test)]
pub(super) async fn verify_backup(directory: &Path) -> anyhow::Result<VerifiedBackup> {
    verify_backup_authenticated(directory, &[0x31; 32]).await
}

pub(super) async fn verify_backup_with_schema_policy(
    directory: &Path,
    require_current_schema: bool,
    backup_authority_key: &[u8; 32],
) -> anyhow::Result<VerifiedBackup> {
    let directory_name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("backup directory has no UTF-8 identifier"))?;
    let backup_id = if parse_backup_id(directory_name).is_some() {
        directory_name
    } else {
        directory_name
            .strip_prefix('.')
            .and_then(|name| name.strip_suffix(".partial"))
            .filter(|name| parse_backup_id(name).is_some())
            .ok_or_else(|| anyhow::anyhow!("backup directory has a noncanonical identifier"))?
    };
    anyhow::ensure!(
        parse_backup_id(backup_id).is_some(),
        "backup directory has a noncanonical identifier"
    );
    let parent_path = directory
        .parent()
        .ok_or_else(|| anyhow::anyhow!("backup directory has no parent"))?;
    anyhow::ensure!(
        std::fs::canonicalize(parent_path)? == parent_path,
        "backup parent is not its canonical real path"
    );
    let parent = pin_directory_capability(parent_path)?;
    let parent_guard = parent.try_clone()?.into_std_file();
    revalidate_pinned_root_directory(&parent_guard, parent_path, "backup parent")?;
    let parent_metadata = parent.dir_metadata()?;
    #[cfg(unix)]
    {
        use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            parent_metadata.uid() == rustix::process::geteuid().as_raw()
                && parent_metadata.permissions().mode() & 0o777 == 0o700,
            "backup parent has the wrong owner or mode"
        );
    }
    let name = directory
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("backup directory has no filename"))?;
    let pinned = open_cap_directory_nofollow(&parent, Path::new(name))?;
    let pinned_metadata = pinned.dir_metadata()?;
    #[cfg(unix)]
    {
        use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            pinned_metadata.uid() == rustix::process::geteuid().as_raw()
                && pinned_metadata.dev() == parent_metadata.dev()
                && pinned_metadata.permissions().mode() & 0o777 == 0o700,
            "backup directory has the wrong owner, device, or mode"
        );
    }
    let admitted_identity = metadata_identity(&pinned_metadata);
    let verified = verify_backup_capability(
        &pinned,
        backup_id,
        backup_authority_key,
        require_current_schema,
    )
    .await?;
    revalidate_pinned_root_directory(&parent_guard, parent_path, "backup parent")?;
    let current = open_cap_directory_nofollow(&parent, Path::new(name))?;
    anyhow::ensure!(
        metadata_identity(&current.dir_metadata()?) == admitted_identity,
        "backup directory was substituted during verification"
    );
    Ok(verified)
}

async fn verify_backup_capability(
    root: &cap_std::fs::Dir,
    backup_id: &str,
    backup_authority_key: &[u8; 32],
    require_current_schema: bool,
) -> anyhow::Result<VerifiedBackup> {
    verify_backup_capability_with_compiled_schema(
        root,
        backup_id,
        backup_authority_key,
        robin_highscores::db::CURRENT_SCHEMA_VERSION,
        require_current_schema,
    )
    .await
}

async fn verify_backup_capability_with_compiled_schema(
    root: &cap_std::fs::Dir,
    backup_id: &str,
    backup_authority_key: &[u8; 32],
    compiled_schema_version: i64,
    require_current_schema: bool,
) -> anyhow::Result<VerifiedBackup> {
    anyhow::ensure!(
        compiled_schema_version >= 2,
        "compiled backup schema policy predates canonical V2 release authority"
    );
    let root_metadata = root.dir_metadata()?;
    #[cfg(unix)]
    {
        use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            root_metadata.uid() == rustix::process::geteuid().as_raw()
                && root_metadata.permissions().mode() & 0o777 == 0o700,
            "pinned backup root has the wrong owner or mode"
        );
    }
    let (manifest_bytes, manifest_identity) = read_cap_regular_bounded_with_mode_and_identity(
        root,
        Path::new("backup-manifest.json"),
        u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
        0o600,
    )?;
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
    anyhow::ensure!(
        canonical_json_bytes(&manifest)? == manifest_bytes,
        "pinned backup manifest is not canonical JSON"
    );
    manifest.validate()?;
    anyhow::ensure!(
        parse_backup_id(backup_id) == Some(manifest.created_at_unix_ms),
        "pinned backup directory identifier differs from its manifest timestamp"
    );
    let now = u64::try_from(robin_highscores::model::now_epoch_ms()?)?;
    anyhow::ensure!(
        manifest.created_at_unix_ms <= now,
        "pinned backup manifest is future-dated"
    );
    let (envelope_bytes, envelope_identity) = read_cap_regular_bounded_with_mode_and_identity(
        root,
        Path::new("backup-verification-envelope.json"),
        u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
        0o400,
    )?;
    let envelope: BackupVerificationEnvelopeV2 = serde_json::from_slice(&envelope_bytes)?;
    anyhow::ensure!(
        canonical_json_bytes(&envelope)? == envelope_bytes,
        "pinned backup verification envelope is not canonical JSON"
    );
    envelope.verify_manifest(backup_authority_key, backup_id, &manifest)?;
    anyhow::ensure!(
        manifest.database_schema_version >= 2
            && manifest.database_schema_version <= compiled_schema_version
            && (!require_current_schema
                || manifest.database_schema_version == compiled_schema_version),
        "pinned backup schema is outside this verifier's authenticated policy"
    );

    let mut expected_paths = BTreeSet::from([
        "backup-manifest.json".to_owned(),
        "backup-verification-envelope.json".to_owned(),
    ]);
    let mut hashed_file_identities = BTreeMap::from([
        ("backup-manifest.json".to_owned(), manifest_identity),
        (
            "backup-verification-envelope.json".to_owned(),
            envelope_identity,
        ),
    ]);
    for expected in &manifest.files {
        let relative = Path::new(&expected.relative_path);
        anyhow::ensure!(
            expected_paths.insert(expected.relative_path.clone()),
            "pinned backup repeats a file path"
        );
        let (actual, identity) = record_cap_file_with_identity(root, relative)?;
        anyhow::ensure!(
            &actual == expected,
            "pinned backup object differs: {}",
            expected.relative_path
        );
        hashed_file_identities.insert(expected.relative_path.clone(), identity);
    }
    let actual_paths = backup_tree_paths_cap(root)?;
    anyhow::ensure!(
        (*actual_paths.files()) == expected_paths,
        "pinned backup contains missing or unexpected files"
    );
    anyhow::ensure!(
        (*actual_paths.directories())
            == manifest
                .directories
                .iter()
                .map(|directory| directory.relative_path.clone())
                .collect(),
        "pinned backup contains missing or unexpected directories"
    );
    anyhow::ensure!(
        (*actual_paths.file_identities()) == hashed_file_identities,
        "pinned backup file inode closure changed between hashing and traversal"
    );
    let initial_tree = actual_paths;

    let database_file = open_cap_regular_nofollow(root, Path::new("highscores.sqlite3"))?;
    validate_private_pinned_file(&database_file, 0o600, "pinned backup database")?;
    anyhow::ensure!(
        (*initial_tree.file_identities()).get("highscores.sqlite3")
            == Some(&metadata_identity_std(&database_file.metadata()?)),
        "pinned backup database inode differs from the hashed manifest object"
    );
    #[cfg(unix)]
    let database_url = {
        use std::os::fd::AsRawFd as _;
        format!(
            "sqlite:///proc/self/fd/{}?mode=ro&immutable=true",
            database_file.as_raw_fd()
        )
    };
    #[cfg(not(unix))]
    anyhow::bail!("pinned backup database verification requires a procfd-capable Unix host");
    #[cfg(unix)]
    let mut connection = sqlx::SqliteConnection::connect(&database_url).await?;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut connection)
        .await?;
    anyhow::ensure!(integrity == "ok", "pinned SQLite integrity check failed");
    let schema: i64 =
        sqlx::query("SELECT MAX(version) AS version FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&mut connection)
            .await?
            .try_get("version")?;
    anyhow::ensure!(
        schema == manifest.database_schema_version,
        "pinned backup database schema differs from its manifest"
    );
    if schema == 2 {
        // VpsReleaseManifestV2 begins at database schema 2. Keep these exact
        // relational/transient-state checks as the historical schema-2
        // verifier even after the running binary advances to a later schema.
        verify_database_object_inventory(&mut connection, &manifest.files).await?;
    } else if schema == robin_highscores::db::CURRENT_SCHEMA_VERSION {
        verify_database_object_inventory(&mut connection, &manifest.files).await?;
    } else {
        anyhow::bail!("pinned backup schema has no compiled semantic inventory verifier: {schema}");
    }
    let foreign_key_failures: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check")
            .fetch_one(&mut connection)
            .await?;
    anyhow::ensure!(
        foreign_key_failures == 0,
        "pinned backup database has foreign-key violations"
    );
    connection.close().await?;
    anyhow::ensure!(
        (*initial_tree.file_identities()).get("highscores.sqlite3")
            == Some(&metadata_identity_std(&database_file.metadata()?)),
        "pinned backup database inode changed during SQLite verification"
    );

    for expected in &manifest.files {
        let (actual, identity) =
            record_cap_file_with_identity(root, Path::new(&expected.relative_path))?;
        anyhow::ensure!(
            &actual == expected
                && (*initial_tree.file_identities()).get(&expected.relative_path)
                    == Some(&identity),
            "pinned backup object changed before receipt: {}",
            expected.relative_path
        );
    }
    let (final_manifest_bytes, final_manifest_identity) =
        read_cap_regular_bounded_with_mode_and_identity(
            root,
            Path::new("backup-manifest.json"),
            u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
            0o600,
        )?;
    let (final_envelope_bytes, final_envelope_identity) =
        read_cap_regular_bounded_with_mode_and_identity(
            root,
            Path::new("backup-verification-envelope.json"),
            u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
            0o400,
        )?;
    anyhow::ensure!(
        final_manifest_bytes == manifest_bytes
            && final_envelope_bytes == envelope_bytes
            && final_manifest_identity == manifest_identity
            && final_envelope_identity == envelope_identity,
        "pinned backup authority documents changed before receipt"
    );
    let final_tree = backup_tree_paths_cap(root)?;
    anyhow::ensure!(
        (*final_tree.files()) == (*initial_tree.files())
            && (*final_tree.directories()) == (*initial_tree.directories())
            && (*final_tree.file_identities()) == (*initial_tree.file_identities())
            && (*final_tree.directory_identities()) == (*initial_tree.directory_identities())
            && final_tree.root_identity() == initial_tree.root_identity(),
        "pinned backup topology changed before receipt"
    );
    let total_bytes = manifest.total_bytes()?;
    let directory_count = manifest.directory_count()?;
    Ok(VerifiedBackup {
        created_at_unix_ms: manifest.created_at_unix_ms,
        database_schema_version: manifest.database_schema_version,
        manifest_sha256: hex::encode(Sha256::digest(&manifest_bytes)),
        release_identity: manifest.release_identity,
        file_count: u64::try_from(manifest.files.len())?,
        directory_count,
        total_bytes,
        tree: final_tree,
    })
}

#[cfg(test)]
async fn verify_historical_backup_chain_with_compiled_schema(
    backup_root: &Path,
    backup_directory: &Path,
    backup_authority_key: &[u8; 32],
    compiled_schema_version: i64,
) -> anyhow::Result<VerifiedBackup> {
    anyhow::ensure!(
        backup_directory.parent() == Some(backup_root),
        "historical test backup is outside its authority root"
    );
    let root = pin_directory_capability(backup_root)?;
    let root_metadata = root.dir_metadata()?;
    let backup_id = backup_directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("historical backup has no canonical ID"))?;
    let child = open_cap_directory_nofollow(&root, Path::new(backup_id))?;
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt as _;
        anyhow::ensure!(
            child.dir_metadata()?.dev() == root_metadata.dev(),
            "historical backup crosses the authority-root device"
        );
    }
    let verified = verify_backup_capability_with_compiled_schema(
        &child,
        backup_id,
        backup_authority_key,
        compiled_schema_version,
        false,
    )
    .await?;
    let preserved =
        load_preserved_release_authority(backup_root, &verified.release_identity).await?;
    anyhow::ensure!(
        preserved == verified.release_identity,
        "historical backup differs from its independent preserved release authority"
    );
    Ok(verified)
}

async fn verify_database_object_inventory(
    connection: &mut sqlx::SqliteConnection,
    files: &[BackupFile],
) -> anyhow::Result<()> {
    let transient_rows: i64 = sqlx::query_scalar(
        "SELECT \
           (SELECT COUNT(*) FROM maintenance_locks) + \
           (SELECT COUNT(*) FROM maintenance_write_leases) + \
           (SELECT COUNT(*) FROM submissions \
              WHERE status = 'verifying' OR lease_owner IS NOT NULL OR lease_expires_at_ms IS NOT NULL) + \
           (SELECT COUNT(*) FROM submission_upload_reservations \
              WHERE state IN ('reserved', 'uploaded') \
                 OR lease_token IS NOT NULL OR lease_expires_at_ms IS NOT NULL)",
    )
    .fetch_one(&mut *connection)
    .await?;
    anyhow::ensure!(
        transient_rows == 0,
        "backup database contains transient maintenance, worker, or upload leases"
    );
    let replay_files = indexed_object_files(files, "replays", ".rhrec", 2)?;
    let campaign_files = indexed_object_files(files, "campaigns", ".campaign", 1)?;
    verify_object_rows(connection, "replay_objects", &replay_files).await?;
    verify_object_rows(connection, "campaign_objects", &campaign_files).await?;

    let invalid_replay_references: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM submissions submission \
         JOIN replay_objects object ON object.sha256 = submission.replay_sha256 \
         WHERE submission.tombstoned_at_ms IS NULL AND object.purge_state != 'live'",
    )
    .fetch_one(&mut *connection)
    .await?;
    anyhow::ensure!(
        invalid_replay_references == 0,
        "backup database has a non-live referenced replay"
    );
    let invalid_campaign_references: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM campaign_object_submission_references reference \
         JOIN submissions submission ON submission.id = reference.submission_id \
         JOIN campaign_objects object ON object.sha256 = reference.sha256 \
         WHERE submission.tombstoned_at_ms IS NULL AND object.purge_state != 'live'",
    )
    .fetch_one(&mut *connection)
    .await?;
    anyhow::ensure!(
        invalid_campaign_references == 0,
        "backup database has a non-live referenced campaign object"
    );
    Ok(())
}

fn indexed_object_files<'a>(
    files: &'a [BackupFile],
    root: &str,
    suffix: &str,
    shard_count: usize,
) -> anyhow::Result<BTreeMap<[u8; 32], &'a BackupFile>> {
    let mut objects = BTreeMap::new();
    let prefix = format!("{root}/");
    for file in files
        .iter()
        .filter(|file| file.relative_path.starts_with(&prefix))
    {
        let components = Path::new(&file.relative_path)
            .components()
            .map(|component| match component {
                std::path::Component::Normal(value) => value
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("object path is not UTF-8")),
                _ => Err(anyhow::anyhow!("object path is unsafe")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        anyhow::ensure!(
            components.len() == shard_count + 2 && components[0] == root,
            "backup object path has the wrong shape"
        );
        let file_name = components
            .last()
            .expect("validated backup object path has a filename");
        let digest_hex = file_name
            .strip_suffix(suffix)
            .ok_or_else(|| anyhow::anyhow!("backup object has the wrong suffix"))?;
        anyhow::ensure!(
            digest_hex.len() == 64
                && digest_hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "backup object name is not a canonical digest"
        );
        for shard in 0..shard_count {
            anyhow::ensure!(
                components[shard + 1] == &digest_hex[shard * 2..shard * 2 + 2],
                "backup object is in the wrong digest shard"
            );
        }
        anyhow::ensure!(
            file.sha256 == digest_hex,
            "backup content digest does not match its content address"
        );
        let digest: [u8; 32] = hex::decode(digest_hex)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("backup object digest has the wrong decoded length"))?;
        anyhow::ensure!(
            objects.insert(digest, file).is_none(),
            "duplicate backup object digest"
        );
    }
    Ok(objects)
}

async fn verify_object_rows(
    connection: &mut sqlx::SqliteConnection,
    table: &str,
    objects: &BTreeMap<[u8; 32], &BackupFile>,
) -> anyhow::Result<()> {
    let query = match table {
        "replay_objects" => {
            "SELECT sha256, byte_length, purge_state FROM replay_objects ORDER BY sha256"
        }
        "campaign_objects" => {
            "SELECT sha256, byte_length, purge_state FROM campaign_objects ORDER BY sha256"
        }
        _ => anyhow::bail!("unsupported object inventory table"),
    };
    for row in sqlx::query(query).fetch_all(&mut *connection).await? {
        let digest_bytes: Vec<u8> = row.try_get("sha256")?;
        let digest: [u8; 32] = digest_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("database object digest has the wrong length"))?;
        let byte_length = u64::try_from(row.try_get::<i64, _>("byte_length")?)?;
        let purge_state: String = row.try_get("purge_state")?;
        match purge_state.as_str() {
            "live" => {
                let file = objects.get(&digest).ok_or_else(|| {
                    anyhow::anyhow!("live database object is missing from backup inventory")
                })?;
                anyhow::ensure!(
                    file.byte_length == byte_length,
                    "database and backup object byte lengths differ"
                );
            }
            "purging" => anyhow::bail!(
                "backup database contains a purging object despite the GC exclusion lock"
            ),
            "purged" => {
                // A post-snapshot upload may legitimately recreate these immutable
                // bytes. It is an unreferenced physical extra in this snapshot and
                // will be reconciled on restore; it is never treated as a live row.
            }
            _ => anyhow::bail!("database object has an unknown purge state"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
