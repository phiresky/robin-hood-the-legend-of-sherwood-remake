//! Descriptor-pinned publication transaction: private staging, no-replace
//! installation, durability classification, and guarded failed-stage cleanup.
//!
//! Unknown persistence state is deliberately distinct from a known installed
//! output whose parent sync failed. Neither outcome authorizes staging cleanup.
//! Keep all post-rename identity checks and fault-injection boundaries here.

use super::{
    PublicationNodeIdentityV3, ValidatedPublicationV3, open_optional_publication_child_identity_v3,
    open_publication_child_identity_v3, open_publication_child_v3, open_publication_root_v3,
    publication_directory_entries_v3, publication_inventory_matches_after_root_rename_v3,
    publication_node_identity_v3, publication_same_stable_node_v3,
    publication_tree_inventory_v3_from_fd, validate_publication_node_v3,
};
use crate::path_to_manifest;
use anyhow::{Context as _, Result, ensure};
use robin_run_protocol::Digest32;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(super) struct PinnedPublicationStagingV3 {
    pub(super) path: PathBuf,
    pub(super) name: std::ffi::OsString,
    pub(super) root: fs::File,
    pub(super) parent_path: PathBuf,
    pub(super) parent: fs::File,
    pub(super) parent_identity: PublicationNodeIdentityV3,
}

impl PinnedPublicationStagingV3 {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn ensure_live(&self) -> Result<()> {
        let rebound_parent = open_publication_root_v3(&self.parent_path)?;
        ensure!(
            publication_same_stable_node_v3(
                &publication_node_identity_v3(&self.parent.metadata()?),
                &self.parent_identity,
            ) && publication_same_stable_node_v3(
                &publication_node_identity_v3(&rebound_parent.metadata()?),
                &self.parent_identity,
            ),
            "PublicationV3 staging parent was substituted"
        );
        let rebound = open_publication_child_v3(&rebound_parent, Path::new(&self.name))?;
        ensure!(
            publication_same_stable_node_v3(
                &publication_node_identity_v3(&rebound.metadata()?),
                &publication_node_identity_v3(&self.root.metadata()?),
            ),
            "PublicationV3 staging basename was substituted"
        );
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub(super) fn create_pinned_publication_staging_v3(
    output: &Path,
) -> Result<PinnedPublicationStagingV3> {
    use rustix::fs::{Mode, mkdirat};
    use std::os::fd::AsFd as _;
    use std::os::unix::fs::MetadataExt as _;

    let parent_path = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("PublicationV3 output has no parent")?
        .to_path_buf();
    let parent = open_publication_root_v3(&parent_path)?;
    let parent_metadata = parent.metadata()?;
    ensure!(
        parent_metadata.is_dir() && parent_metadata.uid() == rustix::process::geteuid().as_raw(),
        "PublicationV3 output parent is not an owned directory"
    );
    let mut created_name = None;
    for _ in 0..128 {
        let name = std::ffi::OsString::from(format!(
            ".robin-manifestctl-{}-{:016x}.partial",
            std::process::id(),
            fastrand::u64(..)
        ));
        match mkdirat(
            parent.as_fd(),
            Path::new(&name),
            Mode::RUSR | Mode::WUSR | Mode::XUSR,
        ) {
            Ok(()) => {
                created_name = Some(name);
                break;
            }
            Err(rustix::io::Errno::EXIST) => {}
            Err(error) => return Err(error.into()),
        }
    }
    let name = created_name.context("exhausted PublicationV3 staging name attempts")?;
    let root = open_publication_child_v3(&parent, Path::new(&name))?;
    let root_metadata = root.metadata()?;
    ensure!(
        root_metadata.is_dir()
            && root_metadata.uid() == rustix::process::geteuid().as_raw()
            && root_metadata.dev() == parent_metadata.dev(),
        "PublicationV3 staging root is not an owned same-device directory"
    );
    let parent_identity = publication_node_identity_v3(&parent.metadata()?);
    let path = parent_path.join(&name);
    let staging = PinnedPublicationStagingV3 {
        path,
        name,
        root,
        parent_path,
        parent,
        parent_identity,
    };
    staging.ensure_live()?;
    Ok(staging)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn create_pinned_publication_staging_v3(
    _output: &Path,
) -> Result<PinnedPublicationStagingV3> {
    anyhow::bail!("PublicationV3 pinned staging requires Linux openat2")
}

const MAX_FAILED_PUBLICATION_STAGING_ENTRIES: usize = 262_144;
const MAX_FAILED_PUBLICATION_STAGING_DEPTH: usize = 128;

#[derive(Debug)]
pub struct PublicationInstalledButParentSyncFailed {
    pub output: PathBuf,
    pub publication_lock_sha256: Digest32,
    pub source: anyhow::Error,
}

#[derive(Debug)]
pub struct CloudflareMaterializationInstalledButParentSyncFailed {
    pub output: PathBuf,
    pub materialization_sha256: Digest32,
    pub source: anyhow::Error,
}

impl std::fmt::Display for CloudflareMaterializationInstalledButParentSyncFailed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Cloudflare publication materialization {} was atomically installed with receipt {} but parent-directory durability sync failed; the exact immutable output exists and must be treated as installed",
            self.output.display(),
            self.materialization_sha256,
        )
    }
}

impl std::error::Error for CloudflareMaterializationInstalledButParentSyncFailed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

#[derive(Debug)]
pub struct CloudflareMaterializationPersistenceStateUncertain {
    pub last_staging_path: PathBuf,
    pub candidate_device: u64,
    pub candidate_inode: u64,
    pub last_parent_path: PathBuf,
    pub parent_device: u64,
    pub parent_inode: u64,
    pub intended_output: PathBuf,
}

impl std::fmt::Display for CloudflareMaterializationPersistenceStateUncertain {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Cloudflare publication materialization persistence is uncertain; preserve candidate dev={} ino={} last staging name {}, pinned parent dev={} ino={} last path {}, and intended output {} for operator reconciliation",
            self.candidate_device,
            self.candidate_inode,
            self.last_staging_path.display(),
            self.parent_device,
            self.parent_inode,
            self.last_parent_path.display(),
            self.intended_output.display(),
        )
    }
}

impl std::error::Error for CloudflareMaterializationPersistenceStateUncertain {}

#[derive(Debug)]
pub(super) struct PublicationPersistenceStateUncertain {
    pub(super) staging_path: PathBuf,
    pub(super) candidate_device: u64,
    pub(super) candidate_inode: u64,
    pub(super) parent_path: PathBuf,
    pub(super) parent_device: u64,
    pub(super) parent_inode: u64,
    pub(super) intended_output: PathBuf,
}

impl std::fmt::Display for PublicationPersistenceStateUncertain {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "PublicationV3 persistence state is uncertain; preserve candidate dev={} ino={} last staging name {}, pinned parent dev={} ino={} last path {}, and intended output {} for operator reconciliation",
            self.candidate_device,
            self.candidate_inode,
            self.staging_path.display(),
            self.parent_device,
            self.parent_inode,
            self.parent_path.display(),
            self.intended_output.display(),
        )
    }
}

impl std::error::Error for PublicationPersistenceStateUncertain {}

fn publication_persistence_state_uncertain(
    staging: &PinnedPublicationStagingV3,
    candidate: &PublicationNodeIdentityV3,
    output: &Path,
) -> anyhow::Error {
    PublicationPersistenceStateUncertain {
        staging_path: staging.path.clone(),
        candidate_device: candidate.device,
        candidate_inode: candidate.inode,
        parent_path: staging.parent_path.clone(),
        parent_device: staging.parent_identity.device,
        parent_inode: staging.parent_identity.inode,
        intended_output: output.to_path_buf(),
    }
    .into()
}

impl std::fmt::Display for PublicationInstalledButParentSyncFailed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "publication {} was atomically installed with lock {} but parent-directory durability sync failed; the immutable final exists and must be validated and treated as published",
            self.output.display(),
            self.publication_lock_sha256,
        )
    }
}

impl std::error::Error for PublicationInstalledButParentSyncFailed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

pub(super) fn installed_publication_durability_error(
    output: &Path,
    publication_lock_sha256: Digest32,
    source: anyhow::Error,
) -> anyhow::Error {
    PublicationInstalledButParentSyncFailed {
        output: output.to_path_buf(),
        publication_lock_sha256,
        source,
    }
    .into()
}

#[derive(Debug)]
pub(super) enum PublicationPersistenceOutcome {
    Published,
    PublishedButParentSyncFailed(anyhow::Error),
}

pub(super) fn persist_publication_staging(
    staging: &PinnedPublicationStagingV3,
    candidate: &ValidatedPublicationV3,
    output: &Path,
) -> Result<PublicationPersistenceOutcome> {
    persist_publication_staging_with(
        staging,
        candidate,
        output,
        || {},
        |parent, source, destination| {
            use std::os::fd::AsFd as _;
            rustix::fs::renameat_with(
                parent.as_fd(),
                source,
                parent.as_fd(),
                destination,
                rustix::fs::RenameFlags::NOREPLACE,
            )?;
            Ok(())
        },
        |parent| {
            parent.sync_all()?;
            Ok(())
        },
    )
}

pub(super) fn persist_publication_staging_with<B, R, S>(
    staging: &PinnedPublicationStagingV3,
    candidate: &ValidatedPublicationV3,
    output: &Path,
    before_rename: B,
    rename_stage: R,
    sync_parent: S,
) -> Result<PublicationPersistenceOutcome>
where
    B: FnOnce(),
    R: FnOnce(&fs::File, &Path, &Path) -> Result<()>,
    S: FnOnce(&fs::File) -> Result<()>,
{
    staging.ensure_live()?;
    candidate.ensure_live()?;
    ensure!(
        publication_same_stable_node_v3(
            &publication_node_identity_v3(&staging.root.metadata()?),
            &publication_node_identity_v3(&candidate.root.metadata()?),
        ),
        "validated PublicationV3 candidate is not the pinned staging root"
    );
    candidate.sync_exact_tree()?;
    let output_parent_path = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("PublicationV3 output has no parent")?;
    let output_name = output
        .file_name()
        .context("PublicationV3 output has no basename")?;
    ensure!(
        output_parent_path == staging.parent_path
            && open_optional_publication_child_identity_v3(
                &staging.parent,
                Path::new(output_name)
            )?
            .is_none(),
        "PublicationV3 final output already exists or changed parent"
    );
    before_rename();
    staging.ensure_live()?;
    candidate.ensure_live()?;
    let root_identity = publication_node_identity_v3(&candidate.root.metadata()?);

    let rename_result = rename_stage(
        &staging.parent,
        Path::new(&staging.name),
        Path::new(output_name),
    );
    let source_after = match open_optional_publication_child_identity_v3(
        &staging.parent,
        Path::new(&staging.name),
    ) {
        Ok(source) => source,
        Err(_) => {
            return Err(publication_persistence_state_uncertain(
                staging,
                &root_identity,
                output,
            ));
        }
    };
    let output_after = match open_optional_publication_child_identity_v3(
        &staging.parent,
        Path::new(output_name),
    ) {
        Ok(output) => output,
        Err(_) => {
            return Err(publication_persistence_state_uncertain(
                staging,
                &root_identity,
                output,
            ));
        }
    };
    let source_is_candidate = source_after
        .as_ref()
        .map(|source| {
            Ok::<bool, anyhow::Error>(publication_same_stable_node_v3(
                &publication_node_identity_v3(&source.metadata()?),
                &root_identity,
            ))
        })
        .transpose()?
        .unwrap_or(false);
    let output_is_candidate = output_after
        .as_ref()
        .map(|installed| {
            Ok::<bool, anyhow::Error>(publication_same_stable_node_v3(
                &publication_node_identity_v3(&installed.metadata()?),
                &root_identity,
            ))
        })
        .transpose()?
        .unwrap_or(false);
    let mut installed_uncertainty = None;
    match (rename_result, source_is_candidate, output_is_candidate) {
        (Ok(()), false, true) => {}
        (Err(error), true, false) => return Err(anyhow::anyhow!("{error:#}")),
        (Err(error), false, true) => {
            installed_uncertainty =
                Some(error.context("rename reported failure after PublicationV3 became installed"));
        }
        _ => {
            return Err(publication_persistence_state_uncertain(
                staging,
                &root_identity,
                output,
            ));
        }
    }

    match publication_tree_inventory_v3_from_fd(output, &candidate.root) {
        Ok(installed_inventory)
            if publication_inventory_matches_after_root_rename_v3(
                &candidate.inventory,
                &installed_inventory,
            ) => {}
        Ok(_) | Err(_) => {
            return Err(publication_persistence_state_uncertain(
                staging,
                &root_identity,
                output,
            ));
        }
    }
    if let Err(error) = sync_parent(&staging.parent) {
        installed_uncertainty = Some(match installed_uncertainty {
            Some(rename_error) => rename_error.context(format!(
                "installed PublicationV3 parent sync also failed: {error:#}"
            )),
            None => error.context("sync installed PublicationV3 parent"),
        });
    }
    if open_publication_root_v3(&staging.parent_path)
        .and_then(|rebound_parent| {
            ensure!(
                publication_same_stable_node_v3(
                    &publication_node_identity_v3(&rebound_parent.metadata()?),
                    &publication_node_identity_v3(&staging.parent.metadata()?),
                ),
                "PublicationV3 output parent changed after persistence"
            );
            let rebound_output =
                open_publication_child_v3(&rebound_parent, Path::new(output_name))?;
            ensure!(
                publication_same_stable_node_v3(
                    &publication_node_identity_v3(&rebound_output.metadata()?),
                    &root_identity,
                ),
                "PublicationV3 output basename changed after persistence"
            );
            Ok(())
        })
        .is_err()
    {
        return Err(publication_persistence_state_uncertain(
            staging,
            &root_identity,
            output,
        ));
    }
    match publication_tree_inventory_v3_from_fd(output, &candidate.root) {
        Ok(terminal_inventory)
            if publication_inventory_matches_after_root_rename_v3(
                &candidate.inventory,
                &terminal_inventory,
            ) => {}
        Ok(_) | Err(_) => {
            return Err(publication_persistence_state_uncertain(
                staging,
                &root_identity,
                output,
            ));
        }
    }
    Ok(match installed_uncertainty {
        None => PublicationPersistenceOutcome::Published,
        Some(error) => PublicationPersistenceOutcome::PublishedButParentSyncFailed(error),
    })
}

#[cfg(target_os = "linux")]
pub(super) fn discard_failed_publication_staging(
    staging: PinnedPublicationStagingV3,
) -> Result<()> {
    discard_failed_publication_staging_with(&staging, |_| {})
}

#[cfg(target_os = "linux")]
pub(super) fn discard_failed_publication_staging_with<F>(
    staging: &PinnedPublicationStagingV3,
    mut before_operation: F,
) -> Result<()>
where
    F: FnMut(usize),
{
    use rustix::fs::{AtFlags, FileType, Mode, fchmod, statat, unlinkat};
    use std::os::fd::AsFd as _;

    #[derive(Debug)]
    struct CleanupDirectoryV3 {
        path: String,
        descriptor: fs::File,
        parent_index: Option<usize>,
        name: Option<std::ffi::OsString>,
        identity: PublicationNodeIdentityV3,
    }

    #[derive(Debug)]
    struct CleanupLeafV3 {
        parent_index: usize,
        name: std::ffi::OsString,
        identity: PublicationNodeIdentityV3,
        symlink: bool,
    }

    fn symlink_identity(stat: &rustix::fs::Stat) -> PublicationNodeIdentityV3 {
        PublicationNodeIdentityV3 {
            device: stat.st_dev,
            inode: stat.st_ino,
            owner: stat.st_uid,
            group: stat.st_gid,
            links: stat.st_nlink,
            mode: stat.st_mode,
            length: stat.st_size as u64,
            modified_seconds: stat.st_mtime,
            modified_nanoseconds: stat.st_mtime_nsec as i64,
            changed_seconds: stat.st_ctime,
            changed_nanoseconds: stat.st_ctime_nsec as i64,
        }
    }

    staging.ensure_live()?;
    let expected_uid = rustix::process::geteuid().as_raw();
    let expected_device = publication_node_identity_v3(&staging.root.metadata()?).device;
    let mut directories = vec![CleanupDirectoryV3 {
        path: ".".into(),
        descriptor: staging.root.try_clone()?,
        parent_index: None,
        name: None,
        identity: publication_node_identity_v3(&staging.root.metadata()?),
    }];
    let mut leaves = Vec::new();
    let mut cursor = 0;
    let mut seen = 1_usize;
    while cursor < directories.len() {
        let depth = Path::new(&directories[cursor].path).components().count();
        ensure!(
            depth <= MAX_FAILED_PUBLICATION_STAGING_DEPTH,
            "failed PublicationV3 staging exceeds cleanup depth bound"
        );
        fchmod(
            directories[cursor].descriptor.as_fd(),
            Mode::RUSR | Mode::WUSR | Mode::XUSR,
        )?;
        directories[cursor].identity =
            publication_node_identity_v3(&directories[cursor].descriptor.metadata()?);
        let entries = publication_directory_entries_v3(&directories[cursor].descriptor)?;
        for (name, observed_inode, observed_type) in entries {
            seen = seen
                .checked_add(1)
                .context("failed PublicationV3 staging cleanup entry overflow")?;
            ensure!(
                seen <= MAX_FAILED_PUBLICATION_STAGING_ENTRIES,
                "failed PublicationV3 staging exceeds cleanup entry bound"
            );
            let child_path = if directories[cursor].path == "." {
                path_to_manifest(Path::new(&name))?
            } else {
                format!(
                    "{}/{}",
                    directories[cursor].path,
                    path_to_manifest(Path::new(&name))?
                )
            };
            match observed_type {
                FileType::Directory => {
                    let child = open_publication_child_v3(
                        &directories[cursor].descriptor,
                        Path::new(&name),
                    )?;
                    let identity = publication_node_identity_v3(&child.metadata()?);
                    validate_publication_node_v3(
                        &identity,
                        expected_uid,
                        expected_device,
                        &child_path,
                    )?;
                    ensure!(
                        observed_inode == 0 || observed_inode == identity.inode,
                        "failed PublicationV3 staging directory changed during cleanup scan"
                    );
                    directories.push(CleanupDirectoryV3 {
                        path: child_path,
                        descriptor: child,
                        parent_index: Some(cursor),
                        name: Some(name),
                        identity,
                    });
                }
                FileType::RegularFile => {
                    let child = open_publication_child_identity_v3(
                        &directories[cursor].descriptor,
                        Path::new(&name),
                    )?;
                    let identity = publication_node_identity_v3(&child.metadata()?);
                    validate_publication_node_v3(
                        &identity,
                        expected_uid,
                        expected_device,
                        &child_path,
                    )?;
                    ensure!(
                        identity.links == 1
                            && (observed_inode == 0 || observed_inode == identity.inode),
                        "failed PublicationV3 staging contains a hard-linked or substituted file"
                    );
                    leaves.push(CleanupLeafV3 {
                        parent_index: cursor,
                        name,
                        identity,
                        symlink: false,
                    });
                }
                FileType::Symlink => {
                    let stat = statat(
                        directories[cursor].descriptor.as_fd(),
                        Path::new(&name),
                        AtFlags::SYMLINK_NOFOLLOW,
                    )?;
                    let identity = symlink_identity(&stat);
                    validate_publication_node_v3(
                        &identity,
                        expected_uid,
                        expected_device,
                        &child_path,
                    )?;
                    ensure!(
                        observed_inode == 0 || observed_inode == identity.inode,
                        "failed PublicationV3 staging symlink changed during cleanup scan"
                    );
                    leaves.push(CleanupLeafV3 {
                        parent_index: cursor,
                        name,
                        identity,
                        symlink: true,
                    });
                }
                _ => anyhow::bail!(
                    "failed PublicationV3 staging contains a special node at {child_path}; preserve pinned stage"
                ),
            }
        }
        cursor += 1;
    }

    let mut operation = 0_usize;
    for leaf in leaves.into_iter().rev() {
        before_operation(operation);
        operation += 1;
        staging.ensure_live()?;
        let parent = &directories[leaf.parent_index].descriptor;
        let rebound = if leaf.symlink {
            symlink_identity(&statat(
                parent.as_fd(),
                Path::new(&leaf.name),
                AtFlags::SYMLINK_NOFOLLOW,
            )?)
        } else {
            publication_node_identity_v3(
                &open_publication_child_identity_v3(parent, Path::new(&leaf.name))?.metadata()?,
            )
        };
        ensure!(
            rebound == leaf.identity,
            "failed PublicationV3 staging leaf was substituted; preserve pinned stage"
        );
        unlinkat(parent.as_fd(), Path::new(&leaf.name), AtFlags::empty())?;
    }
    for index in (1..directories.len()).rev() {
        before_operation(operation);
        operation += 1;
        staging.ensure_live()?;
        let directory = &directories[index];
        let parent = &directories[directory
            .parent_index
            .context("cleanup directory has no parent")?];
        let name = directory
            .name
            .as_ref()
            .context("cleanup directory has no name")?;
        let rebound = open_publication_child_v3(&parent.descriptor, Path::new(name))?;
        let rebound_identity = publication_node_identity_v3(&rebound.metadata()?);
        ensure!(
            publication_same_stable_node_v3(&rebound_identity, &directory.identity)
                && publication_directory_entries_v3(&rebound)?.is_empty(),
            "failed PublicationV3 staging directory was substituted; preserve pinned stage"
        );
        unlinkat(
            parent.descriptor.as_fd(),
            Path::new(name),
            AtFlags::REMOVEDIR,
        )?;
    }
    before_operation(operation);
    staging.ensure_live()?;
    ensure!(
        publication_directory_entries_v3(&staging.root)?.is_empty(),
        "failed PublicationV3 staging root changed before final cleanup"
    );
    unlinkat(
        staging.parent.as_fd(),
        Path::new(&staging.name),
        AtFlags::REMOVEDIR,
    )?;
    ensure!(
        open_optional_publication_child_identity_v3(&staging.parent, Path::new(&staging.name),)?
            .is_none(),
        "failed PublicationV3 staging basename remains after cleanup"
    );
    staging.parent.sync_all()?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(super) fn discard_failed_publication_staging(
    _staging: PinnedPublicationStagingV3,
) -> Result<()> {
    anyhow::bail!("PublicationV3 guarded cleanup requires Linux dirfds")
}
