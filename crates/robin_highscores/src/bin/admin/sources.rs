//! Pinned restore sources and preserved immutable release authority.

use super::filesystem::FileIdentity;
use super::filesystem::cap_entry_exists;
use super::filesystem::copy_open_file;
use super::filesystem::duplicate_pinned_file;
use super::filesystem::metadata_identity;
use super::filesystem::metadata_identity_std;
use super::filesystem::open_cap_directory_nofollow;
use super::filesystem::open_cap_regular_nofollow;
use super::filesystem::open_regular_nofollow;
use super::filesystem::pin_directory_capability;
use super::filesystem::pinned_file_target;
use super::filesystem::read_bounded_pinned_file;
use super::filesystem::read_bounded_regular_nofollow;
use super::filesystem::remove_pinned_regular_via_tombstone;
use super::filesystem::revalidate_pinned_regular_path;
use super::filesystem::revalidate_pinned_root_directory;
use super::filesystem::sync_cap_directory;
use super::filesystem::unlink_pinned_regular;
use super::filesystem::validate_managed_metadata;
use super::filesystem::validate_private_pinned_file;
use super::policy::RELEASE_AUTHORITY_STORE;
use super::policy::SYSTEMD_UNIT_FILES;
use super::policy::SYSTEMD_USER_ROOT;
use robin_highscores::ServerConfig;
use robin_highscores::backup::BackupReleaseIdentityV2;
use robin_highscores::backup::load_backup_release_identity_preserved_file;
use sha2::Digest as _;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io::Write as _;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::path::PathBuf;

pub(super) fn pin_preserved_release_authority_from_root(
    root_guard: &std::fs::File,
    release_identity: &BackupReleaseIdentityV2,
    trusted_backup_root: &Path,
) -> anyhow::Result<(std::fs::File, std::fs::File, PathBuf)> {
    let root = cap_std::fs::Dir::from_std_file(duplicate_pinned_file(root_guard, true)?);
    let root_metadata = root.dir_metadata()?;
    let store = open_cap_directory_nofollow(&root, Path::new(RELEASE_AUTHORITY_STORE))?;
    let store_metadata = store.dir_metadata()?;
    #[cfg(unix)]
    {
        use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            store_metadata.uid() == rustix::process::geteuid().as_raw()
                && store_metadata.dev() == root_metadata.dev()
                && store_metadata.permissions().mode() & 0o777 == 0o700,
            "preserved release-authority store has the wrong owner, device, or mode"
        );
    }
    let store_guard = store.try_clone()?.into_std_file();
    let store_path = trusted_backup_root.join(RELEASE_AUTHORITY_STORE);
    anyhow::ensure!(
        pinned_file_target(&store_guard)? == store_path,
        "preserved release-authority store is outside the canonical backup root"
    );
    let name = release_authority_file_name(release_identity)?;
    let target = store_path.join(&name);
    let authority = open_cap_regular_nofollow(&store, Path::new(&name))?;
    validate_private_pinned_file(&authority, 0o400, "preserved release authority")?;
    anyhow::ensure!(
        metadata_identity_std(&authority.metadata()?).device()
            == metadata_identity(&root_metadata).device()
            && pinned_file_target(&authority)? == target,
        "preserved release authority has the wrong device or canonical path"
    );
    Ok((store_guard, authority, target))
}

pub(super) fn validate_backup_restore_source_contract(
    config: &ServerConfig,
    sources: &BTreeMap<PathBuf, PathBuf>,
) -> anyhow::Result<()> {
    let moderation = config
        .moderation_bearer_token_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("backup requires a moderation bearer secret path"))?;
    let mut expected = BTreeSet::from([
        config.cursor_secret_path.clone(),
        config.competition_run_grant_secret_path.clone(),
        config.run_preflight_grant_secret_path.clone(),
        moderation.clone(),
    ]);
    expected.extend(
        SYSTEMD_UNIT_FILES
            .into_iter()
            .map(|unit| Path::new(SYSTEMD_USER_ROOT).join(unit)),
    );
    let actual = sources.keys().cloned().collect::<BTreeSet<_>>();
    anyhow::ensure!(
        actual == expected,
        "restore source maps must name exactly the four secrets and five installed user units"
    );
    Ok(())
}

pub(super) fn release_authority_file_name(
    release_identity: &BackupReleaseIdentityV2,
) -> anyhow::Result<String> {
    release_identity.validate()?;
    Ok(format!(
        "{}.vps-release-manifest-v2.json",
        release_identity.vps_release_manifest_sha256
    ))
}

pub(super) async fn preserve_release_authority(
    backup_root: &Path,
    release_manifest_path: &Path,
    release_identity: &BackupReleaseIdentityV2,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        backup_root.is_absolute() && std::fs::canonicalize(backup_root)? == backup_root,
        "backup root is not its canonical real path"
    );
    let bytes = read_bounded_regular_nofollow(release_manifest_path, 64 * 1024 * 1024).await?;
    anyhow::ensure!(
        hex::encode(Sha256::digest(&bytes)) == release_identity.vps_release_manifest_sha256,
        "release authority bytes differ from the active release identity"
    );
    let root = pin_directory_capability(backup_root)?;
    let root_guard = root.try_clone()?.into_std_file();
    revalidate_pinned_root_directory(&root_guard, backup_root, "backup root")?;
    let mut directory_builder = cap_std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use cap_std::fs::DirBuilderExt as _;
        directory_builder.mode(0o700);
    }
    match root.create_dir_with(RELEASE_AUTHORITY_STORE, &directory_builder) {
        Ok(()) => {
            let authority = open_cap_directory_nofollow(&root, Path::new(RELEASE_AUTHORITY_STORE))?;
            sync_cap_directory(&authority)?;
            sync_cap_directory(&root)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let authority = open_cap_directory_nofollow(&root, Path::new(RELEASE_AUTHORITY_STORE))?;
    let authority_guard = authority.try_clone()?.into_std_file();
    let authority_path = backup_root.join(RELEASE_AUTHORITY_STORE);
    revalidate_pinned_root_directory(&authority_guard, &authority_path, "release-authority store")?;
    let root_metadata = root.dir_metadata()?;
    let authority_metadata = authority.dir_metadata()?;
    #[cfg(unix)]
    {
        use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            authority_metadata.uid() == rustix::process::geteuid().as_raw()
                && authority_metadata.dev() == root_metadata.dev()
                && authority_metadata.permissions().mode() & 0o777 == 0o700,
            "release-authority store has the wrong owner, device, or mode"
        );
    }
    let name = release_authority_file_name(release_identity)?;
    let partial_name = format!(".{name}.partial");
    let discarded_partial_name = format!("{partial_name}.discard");
    if cap_entry_exists(&authority, Path::new(&discarded_partial_name))? {
        let discarded = open_cap_regular_nofollow(&authority, Path::new(&discarded_partial_name))?;
        validate_private_pinned_file(&discarded, 0o400, "discarded release-authority partial")?;
        let discarded_identity = metadata_identity_std(&discarded.metadata()?);
        let discarded_bytes =
            read_bounded_pinned_file(duplicate_pinned_file(&discarded, false)?, 64 * 1024 * 1024)?;
        anyhow::ensure!(
            metadata_identity(&authority.symlink_metadata(&discarded_partial_name)?)
                == discarded_identity,
            "discarded release-authority partial was substituted before recovery"
        );
        unlink_pinned_regular(
            &authority,
            &discarded_partial_name,
            &discarded,
            discarded_identity,
            0o400,
            "discarded release-authority partial",
        )?;
        drop(discarded_bytes);
    }
    match authority.symlink_metadata(&partial_name) {
        Ok(metadata) => {
            validate_managed_metadata(&metadata, &root_metadata, false)?;
            #[cfg(unix)]
            {
                use cap_std::fs::PermissionsExt as _;
                anyhow::ensure!(
                    metadata.permissions().mode() & 0o777 == 0o400,
                    "release-authority partial has the wrong mode"
                );
            }
            let partial = open_cap_regular_nofollow(&authority, Path::new(&partial_name))?;
            validate_private_pinned_file(&partial, 0o400, "release-authority partial")?;
            let partial_bytes = read_bounded_pinned_file(
                duplicate_pinned_file(&partial, false)?,
                64 * 1024 * 1024,
            )?;
            if partial_bytes != bytes {
                let partial_identity = metadata_identity_std(&partial.metadata()?);
                remove_pinned_regular_via_tombstone(
                    &authority,
                    &partial_name,
                    &discarded_partial_name,
                    &partial,
                    partial_identity,
                    0o400,
                    &partial_bytes,
                    64 * 1024 * 1024,
                    "release-authority partial",
                )?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if !cap_entry_exists(&authority, Path::new(&partial_name))?
        && !cap_entry_exists(&authority, Path::new(&name))?
    {
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt as _;
            options.mode(0o400);
        }
        let mut file = authority.open_with(&partial_name, &options)?.into_std();
        file.write_all(&bytes)?;
        file.sync_all()?;
        validate_private_pinned_file(&file, 0o400, "preserved release authority partial")?;
        sync_cap_directory(&authority)?;
    }
    if !cap_entry_exists(&authority, Path::new(&name))? {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            use std::os::fd::AsFd as _;
            rustix::fs::renameat_with(
                authority.as_fd(),
                Path::new(&partial_name),
                authority.as_fd(),
                Path::new(&name),
                rustix::fs::RenameFlags::NOREPLACE,
            )?;
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        anyhow::bail!("release-authority NOREPLACE publication requires Linux renameat2");
        sync_cap_directory(&authority)?;
    } else if cap_entry_exists(&authority, Path::new(&partial_name))? {
        let partial = open_cap_regular_nofollow(&authority, Path::new(&partial_name))?;
        validate_private_pinned_file(&partial, 0o400, "release-authority partial")?;
        let partial_identity = metadata_identity_std(&partial.metadata()?);
        let partial_bytes =
            read_bounded_pinned_file(duplicate_pinned_file(&partial, false)?, 64 * 1024 * 1024)?;
        anyhow::ensure!(
            partial_bytes == bytes
                && metadata_identity(&authority.symlink_metadata(&partial_name)?)
                    == partial_identity,
            "release-authority partial changed before reconciliation"
        );
        remove_pinned_regular_via_tombstone(
            &authority,
            &partial_name,
            &discarded_partial_name,
            &partial,
            partial_identity,
            0o400,
            &partial_bytes,
            64 * 1024 * 1024,
            "release-authority partial",
        )?;
    }
    let file = open_cap_regular_nofollow(&authority, Path::new(&name))?;
    validate_private_pinned_file(&file, 0o400, "preserved release authority")?;
    #[cfg(unix)]
    {
        anyhow::ensure!(
            metadata_identity_std(&file.metadata()?).device()
                == metadata_identity(&root_metadata).device(),
            "preserved release authority crosses a filesystem boundary"
        );
    }
    let preserved =
        read_bounded_pinned_file(duplicate_pinned_file(&file, false)?, 64 * 1024 * 1024)?;
    anyhow::ensure!(
        preserved == bytes,
        "preserved release authority differs from the exact active manifest bytes"
    );
    let final_identity = metadata_identity_std(&file.metadata()?);
    let loaded =
        load_backup_release_identity_preserved_file(duplicate_pinned_file(&file, false)?).await?;
    anyhow::ensure!(
        loaded == *release_identity,
        "preserved release authority has a different typed identity"
    );
    revalidate_pinned_root_directory(&root_guard, backup_root, "backup root")?;
    revalidate_pinned_root_directory(&authority_guard, &authority_path, "release-authority store")?;
    let current = open_cap_regular_nofollow(&authority, Path::new(&name))?;
    validate_private_pinned_file(&current, 0o400, "preserved release authority")?;
    anyhow::ensure!(
        metadata_identity_std(&current.metadata()?) == final_identity
            && read_bounded_pinned_file(duplicate_pinned_file(&current, false)?, 64 * 1024 * 1024,)?
                == bytes,
        "preserved release authority was substituted before publication completed"
    );
    Ok(())
}

pub(super) async fn load_preserved_release_authority(
    backup_root: &Path,
    release_identity: &BackupReleaseIdentityV2,
) -> anyhow::Result<BackupReleaseIdentityV2> {
    anyhow::ensure!(
        backup_root.is_absolute() && std::fs::canonicalize(backup_root)? == backup_root,
        "backup root is not its canonical real path"
    );
    let root = pin_directory_capability(backup_root)?;
    let root_guard = root.try_clone()?.into_std_file();
    revalidate_pinned_root_directory(&root_guard, backup_root, "backup root")?;
    let root_metadata = root.dir_metadata()?;
    let (store_guard, file, target) =
        pin_preserved_release_authority_from_root(&root_guard, release_identity, backup_root)?;
    let file_identity = metadata_identity_std(&file.metadata()?);
    let authority_bytes =
        read_bounded_pinned_file(duplicate_pinned_file(&file, false)?, 64 * 1024 * 1024)?;
    let loaded =
        load_backup_release_identity_preserved_file(duplicate_pinned_file(&file, false)?).await?;
    anyhow::ensure!(
        loaded == *release_identity,
        "preserved release authority differs from the backup identity"
    );
    revalidate_pinned_root_directory(&root_guard, backup_root, "backup root")?;
    revalidate_pinned_root_directory(
        &store_guard,
        &backup_root.join(RELEASE_AUTHORITY_STORE),
        "release-authority store",
    )?;
    revalidate_pinned_regular_path(
        &file,
        &target,
        0o400,
        Some(0o700),
        "preserved release authority",
    )?;
    anyhow::ensure!(
        file_identity.device() == metadata_identity(&root_metadata).device()
            && metadata_identity_std(&file.metadata()?) == file_identity
            && read_bounded_pinned_file(duplicate_pinned_file(&file, false)?, 64 * 1024 * 1024,)?
                == authority_bytes,
        "preserved release authority changed during verification"
    );
    Ok(loaded)
}

pub(super) fn validate_installed_unit_source(
    path: &Path,
    authority: &robin_highscores::backup::BackupReleaseUnitV2,
) -> anyhow::Result<()> {
    pin_restore_source(
        path,
        authority.unix_mode,
        Some(authority.artifact.byte_length),
        Some(authority.artifact.sha256.to_string().as_str()),
        "installed user unit",
    )?;
    Ok(())
}

pub(super) fn validate_secret_source(path: &Path, exact_length: Option<u64>) -> anyhow::Result<()> {
    pin_restore_source(path, 0o400, exact_length, None, "backup secret source")?;
    Ok(())
}

#[derive(Debug)]
pub(super) struct PinnedRestoreSource {
    path: PathBuf,
    file: std::fs::File,
    identity: FileIdentity,
    expected_mode: u32,
    bytes: Vec<u8>,
    sha256: String,
}

impl PinnedRestoreSource {
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(super) fn revalidate(&self, label: &str) -> anyhow::Result<()> {
        revalidate_pinned_regular_path(&self.file, &self.path, self.expected_mode, None, label)?;
        anyhow::ensure!(
            metadata_identity_std(&self.file.metadata()?) == self.identity
                && read_pinned_source_bytes(&self.file, u64::try_from(self.bytes.len())?)?
                    == self.bytes,
            "{label} changed after it was pinned"
        );
        Ok(())
    }
}

fn read_pinned_source_bytes(file: &std::fs::File, maximum: u64) -> anyhow::Result<Vec<u8>> {
    use std::io::{Seek as _, SeekFrom};

    let mut duplicate = duplicate_pinned_file(file, false)?;
    duplicate.seek(SeekFrom::Start(0))?;
    read_bounded_pinned_file(duplicate, maximum)
}

pub(super) fn pin_restore_source(
    path: &Path,
    expected_mode: u32,
    exact_length: Option<u64>,
    expected_sha256: Option<&str>,
    label: &str,
) -> anyhow::Result<PinnedRestoreSource> {
    anyhow::ensure!(path.is_absolute(), "{label} path must be absolute");
    anyhow::ensure!(
        std::fs::canonicalize(path)? == path,
        "{label} path must not traverse aliases or symlinks"
    );
    let file = open_regular_nofollow(path)?;
    validate_private_pinned_file(&file, expected_mode, label)?;
    let metadata = file.metadata()?;
    if let Some(exact_length) = exact_length {
        anyhow::ensure!(
            metadata.len() == exact_length,
            "{label} has the wrong length"
        );
    } else {
        anyhow::ensure!(
            (1..=64 * 1024).contains(&metadata.len()),
            "{label} is empty or oversized"
        );
    }
    let bytes = read_pinned_source_bytes(&file, metadata.len())?;
    anyhow::ensure!(
        u64::try_from(bytes.len())? == metadata.len(),
        "{label} changed length while it was admitted"
    );
    let sha256 = hex::encode(Sha256::digest(&bytes));
    if let Some(expected_sha256) = expected_sha256 {
        anyhow::ensure!(
            sha256 == expected_sha256,
            "{label} differs from its authenticated release artifact"
        );
    }
    let source = PinnedRestoreSource {
        path: path.to_owned(),
        identity: metadata_identity_std(&metadata),
        expected_mode,
        file,
        bytes,
        sha256,
    };
    source.revalidate(label)?;
    Ok(source)
}

pub(super) fn pin_backup_restore_sources(
    config: &ServerConfig,
    readable_sources: &BTreeMap<PathBuf, PathBuf>,
    release_identity: &BackupReleaseIdentityV2,
) -> anyhow::Result<BTreeMap<PathBuf, PinnedRestoreSource>> {
    validate_backup_restore_source_contract(config, readable_sources)?;
    let mut pinned = BTreeMap::new();
    for (original, exact_length) in [
        (&config.cursor_secret_path, Some(32_u64)),
        (&config.competition_run_grant_secret_path, Some(32_u64)),
        (&config.run_preflight_grant_secret_path, Some(32_u64)),
    ] {
        let readable = readable_sources
            .get(original)
            .map(PathBuf::as_path)
            .unwrap_or(original);
        pinned.insert(
            original.clone(),
            pin_restore_source(readable, 0o400, exact_length, None, "backup secret source")?,
        );
    }
    let moderation = config
        .moderation_bearer_token_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("backup requires the moderation bearer secret"))?;
    let moderation_readable = readable_sources
        .get(moderation)
        .map(PathBuf::as_path)
        .unwrap_or(moderation);
    pinned.insert(
        moderation.clone(),
        pin_restore_source(
            moderation_readable,
            0o400,
            None,
            None,
            "backup moderation secret",
        )?,
    );
    for unit in SYSTEMD_UNIT_FILES {
        let original = Path::new(SYSTEMD_USER_ROOT).join(unit);
        let readable = readable_sources
            .get(&original)
            .cloned()
            .unwrap_or_else(|| original.clone());
        let authority = release_identity
            .installed_user_units
            .iter()
            .find(|authority| authority.release_relative_path == format!("systemd/user/{unit}"))
            .ok_or_else(|| {
                anyhow::anyhow!("active release omits user-unit authority for {unit}")
            })?;
        let expected_sha256 = authority.artifact.sha256.to_string();
        pinned.insert(
            original,
            pin_restore_source(
                &readable,
                authority.unix_mode,
                Some(authority.artifact.byte_length),
                Some(&expected_sha256),
                "installed user unit",
            )?,
        );
    }
    Ok(pinned)
}

pub(super) async fn copy_pinned_restore_source(
    source: &PinnedRestoreSource,
    destination: &Path,
) -> anyhow::Result<()> {
    copy_pinned_restore_source_with_hook(source, destination, || Ok(())).await
}

async fn copy_pinned_restore_source_with_hook<F>(
    source: &PinnedRestoreSource,
    destination: &Path,
    after_copy: F,
) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    use std::io::{Seek as _, SeekFrom};

    source.revalidate("backup restore source before copy")?;
    let mut duplicate = duplicate_pinned_file(&source.file, false)?;
    duplicate.seek(SeekFrom::Start(0))?;
    copy_open_file(tokio::fs::File::from_std(duplicate), destination).await?;
    after_copy()?;
    source.revalidate("backup restore source after copy")?;
    let target =
        read_bounded_regular_nofollow(destination, u64::try_from(source.bytes.len())?).await?;
    anyhow::ensure!(
        target == source.bytes && hex::encode(Sha256::digest(&target)) == source.sha256,
        "copied restore source differs from its pinned admitted bytes"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
