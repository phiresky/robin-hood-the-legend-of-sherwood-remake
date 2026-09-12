//! staging responsibilities of the admitted release pipeline.
use super::*;

pub(super) const MAX_FAILED_VPS_STAGING_ENTRIES: usize = 262_144;
pub(super) const MAX_FAILED_VPS_STAGING_DEPTH: usize = 128;

#[derive(Debug)]
pub struct VpsReleaseInstalledButParentSyncFailed {
    pub output: PathBuf,
    pub release_manifest_sha256: Digest32,
    pub source_commit: String,
    pub source: anyhow::Error,
}

impl std::fmt::Display for VpsReleaseInstalledButParentSyncFailed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "VPS release {} for source {} was atomically installed with manifest {} but parent-directory durability sync failed; validate the immutable final and treat it as installed",
            self.output.display(),
            self.source_commit,
            self.release_manifest_sha256,
        )
    }
}

impl std::error::Error for VpsReleaseInstalledButParentSyncFailed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

pub(super) fn vps_installed_durability_error(
    output: &Path,
    release_manifest_sha256: Digest32,
    source_commit: &str,
    source: anyhow::Error,
) -> anyhow::Error {
    VpsReleaseInstalledButParentSyncFailed {
        output: output.to_path_buf(),
        release_manifest_sha256,
        source_commit: source_commit.to_owned(),
        source,
    }
    .into()
}

#[derive(Debug)]
pub(super) enum VpsPersistenceOutcome {
    Installed,
    InstalledButParentSyncFailed(anyhow::Error),
}

pub(super) fn persist_vps_staging(
    staging: &tempfile::TempDir,
    output: &Path,
) -> Result<VpsPersistenceOutcome> {
    persist_vps_staging_with(staging, output, |parent| {
        File::open(parent)?.sync_all()?;
        Ok(())
    })
}

pub(super) fn persist_vps_staging_with<F>(
    staging: &tempfile::TempDir,
    output: &Path,
    sync_parent: F,
) -> Result<VpsPersistenceOutcome>
where
    F: FnOnce(&Path) -> Result<()>,
{
    crate::sync_directory_tree(staging.path())?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        staging.path(),
        rustix::fs::CWD,
        output,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .with_context(|| {
        format!(
            "atomically install {} as {}",
            staging.path().display(),
            output.display()
        )
    })?;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = (staging, output);
        anyhow::bail!("VPS release installation requires atomic rename NOREPLACE support");
    }
    let parent = output.parent().context("VPS output has no parent")?;
    Ok(match sync_parent(parent) {
        Ok(()) => VpsPersistenceOutcome::Installed,
        Err(error) => VpsPersistenceOutcome::InstalledButParentSyncFailed(error),
    })
}

pub(super) fn discard_failed_vps_staging(staging: tempfile::TempDir) -> Result<()> {
    let staging_path = staging.path().to_path_buf();
    if let Err(cleanup_guard_error) = make_failed_vps_staging_removable(&staging_path) {
        let preserved = staging.keep();
        return Err(cleanup_guard_error.context(format!(
            "unsafe failed VPS staging was preserved at {}",
            preserved.display()
        )));
    }
    staging
        .close()
        .with_context(|| format!("remove failed VPS staging {}", staging_path.display()))?;
    ensure!(
        fs::symlink_metadata(&staging_path)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "failed VPS staging still exists after cleanup: {}",
        staging_path.display()
    );
    Ok(())
}

pub(super) fn make_failed_vps_staging_removable(root: &Path) -> Result<()> {
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let metadata = fs::symlink_metadata(root)
            .with_context(|| format!("inspect failed VPS staging {}", root.display()))?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "failed VPS staging root is not a real directory"
        );
        let expected_uid = metadata.uid();
        let expected_device = metadata.dev();
        ensure!(
            expected_uid == rustix::process::geteuid().as_raw(),
            "failed VPS staging root is not owned by the current user"
        );
        reject_mounts_at_or_below(root)?;
        let mut seen = 1_usize;
        let mut pending = vec![(root.to_path_buf(), 0_usize)];
        let mut directories = Vec::new();
        while let Some((directory, depth)) = pending.pop() {
            ensure!(
                depth <= MAX_FAILED_VPS_STAGING_DEPTH,
                "failed VPS staging exceeds cleanup depth bound"
            );
            let metadata = fs::symlink_metadata(&directory)?;
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "failed VPS staging directory was substituted"
            );
            ensure!(
                metadata.uid() == expected_uid && metadata.dev() == expected_device,
                "failed VPS staging contains mixed ownership or devices"
            );
            directories.push(directory.clone());
            let mut entries = fs::read_dir(&directory)?.collect::<std::io::Result<Vec<_>>>()?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                seen = seen
                    .checked_add(1)
                    .context("failed VPS staging entry count overflow")?;
                ensure!(
                    seen <= MAX_FAILED_VPS_STAGING_ENTRIES,
                    "failed VPS staging exceeds cleanup entry bound"
                );
                let metadata = fs::symlink_metadata(entry.path())?;
                ensure!(
                    metadata.uid() == expected_uid && metadata.dev() == expected_device,
                    "failed VPS staging contains mixed ownership or devices"
                );
                if metadata.file_type().is_symlink() {
                    continue;
                }
                if metadata.is_file() {
                    ensure!(
                        metadata.nlink() == 1,
                        "failed VPS staging contains a hard-linked file"
                    );
                    continue;
                }
                ensure!(
                    metadata.is_dir(),
                    "failed VPS staging contains a special node"
                );
                pending.push((entry.path(), depth + 1));
            }
        }
        directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        for directory in directories {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        }
    }

    Ok(())
}
