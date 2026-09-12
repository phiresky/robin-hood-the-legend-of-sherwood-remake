//! sources responsibilities of the admitted release pipeline.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum VpsSourceEntryKindV1 {
    Directory,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum VpsSourceConsumePhaseV1 {
    Prepared,
    RootUnlinked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VpsSourceConsumeEntryV1 {
    pub(super) path: String,
    pub(super) kind: VpsSourceEntryKindV1,
    pub(super) unix_mode: u32,
    pub(super) sha256: Option<Digest32>,
    pub(super) byte_length: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VpsSourceConsumeJournalV1 {
    pub(super) schema_version: u32,
    pub(super) phase: VpsSourceConsumePhaseV1,
    pub(super) source_commit: String,
    pub(super) plan_sha256: Digest32,
    pub(super) release_manifest_sha256: Digest32,
    pub(super) source_device: u64,
    pub(super) source_inode: u64,
    pub(super) candidate_device: u64,
    pub(super) candidate_inode: u64,
    pub(super) entries: Vec<VpsSourceConsumeEntryV1>,
}

/// Consume the exact uploader-owned source closure after its assembled
/// candidate has been independently authenticated.
///
/// The plan descriptor is read once and binds all derived paths. A durable
/// inventory journal is published before the source root is renamed to its
/// deterministic consuming name, so interruption during deletion can resume
/// only from an exact remaining subset of the original closure.
pub fn consume_vps_sources_v2(
    plan_fd: &Path,
    expected_plan_sha256: &str,
    expected_release_manifest_sha256: &str,
    candidate_root_fd: std::os::fd::RawFd,
    activation_lock_fd: std::os::fd::RawFd,
) -> Result<Digest32> {
    {
        let activation_lock = pin_inherited_vps_activation_lock_v2(activation_lock_fd)?;
        let (plan, plan_bytes, plan_sha256) = load_pinned_vps_plan(plan_fd, expected_plan_sha256)?;
        let release_manifest_sha256 = expected_release_manifest_sha256
            .parse::<Digest32>()
            .context(
                "expected VPS release manifest digest is not canonical lowercase hexadecimal",
            )?;
        ensure!(
            !release_manifest_sha256.is_zero(),
            "expected VPS release manifest digest is zero"
        );
        let candidate_root = pin_inherited_vps_candidate_root_v2(
            &plan.source_commit,
            candidate_root_fd,
            release_manifest_sha256,
        )?;
        consume_vps_sources_in_with(
            &plan,
            &plan_bytes,
            plan_sha256,
            release_manifest_sha256,
            &Path::new(INSTALL_ROOT).join("incoming"),
            &Path::new(INSTALL_ROOT)
                .join("incoming")
                .join(format!(".sources-{}", plan.source_commit)),
            &candidate_root,
            |candidate| {
                let digest = validate_pinned_current_vps_release_root(candidate)?;
                let manifest: VpsReleaseManifestV2 =
                    load_canonical(&candidate.join(RELEASE_MANIFEST_FILE))?;
                ensure!(
                    manifest.canonical_digest()? == digest,
                    "pinned candidate manifest identity changed"
                );
                Ok((digest, manifest.publication_lock_sha256))
            },
            validate_vps_source_publication_v3,
            || activation_lock.ensure_canonical(),
            |_| Ok(()),
            |_| Ok(()),
        )?;
        activation_lock.ensure_canonical()?;
        Ok(release_manifest_sha256)
    }
}

/// Atomically promote the exact candidate directory retained by the outer
/// transaction. No release copy is made: the inherited inode moves from its
/// canonical partial basename to its canonical installed basename.
pub fn promote_inherited_vps_release_v2(
    expected_release_manifest_sha256: &str,
    candidate_root_fd: std::os::fd::RawFd,
    activation_lock_fd: std::os::fd::RawFd,
) -> Result<Digest32> {
    {
        use rustix::fs::{AtFlags, FileType, RenameFlags, renameat_with, statat};
        use std::os::fd::{AsFd as _, AsRawFd as _, OwnedFd};

        let expected = expected_release_manifest_sha256
            .parse::<Digest32>()
            .context("expected VPS release manifest is not canonical lowercase hexadecimal")?;
        ensure!(
            !expected.is_zero(),
            "expected VPS release manifest digest is zero"
        );
        ensure!(
            candidate_root_fd >= 3,
            "candidate-root descriptor must be at least 3"
        );
        let descriptor = PathBuf::from(format!("/proc/self/fd/{candidate_root_fd}"));
        let duplicate = OwnedFd::from(File::open(&descriptor)?);
        let root = PathBuf::from(format!("/proc/self/fd/{}/.", duplicate.as_raw_fd()));
        ensure!(
            validate_pinned_current_vps_release_root(&root)? == expected,
            "retained candidate differs from the out-of-band V2 manifest digest"
        );
        let manifest: VpsReleaseManifestV2 = load_canonical(&root.join(RELEASE_MANIFEST_FILE))?;
        let candidate = pin_inherited_vps_candidate_root_v2(
            &manifest.source_commit,
            candidate_root_fd,
            expected,
        )?;
        let activation_lock = pin_inherited_vps_activation_lock_v2(activation_lock_fd)?;
        activation_lock.ensure_canonical()?;

        if candidate.canonical_path()? == candidate.installed_path {
            candidate.ensure_canonical()?;
            return Ok(expected);
        }
        ensure!(
            candidate.canonical_path()? == candidate.partial_path,
            "retained candidate is neither the partial nor installed release root"
        );
        let release_parent = &candidate.parents.installed_parent_fd;
        let partial_parent_metadata = rustix::fs::fstat(release_parent)?;
        let release_metadata = rustix::fs::fstat(release_parent)?;
        ensure!(
            FileType::from_raw_mode(partial_parent_metadata.st_mode).is_dir()
                && FileType::from_raw_mode(release_metadata.st_mode).is_dir()
                && partial_parent_metadata.st_uid == rustix::process::geteuid().as_raw()
                && release_metadata.st_uid == rustix::process::geteuid().as_raw()
                && partial_parent_metadata.st_mode & 0o777 == 0o750
                && release_metadata.st_mode & 0o777 == 0o750
                && partial_parent_metadata.st_dev == release_metadata.st_dev
                && partial_parent_metadata.st_dev == candidate.device,
            "VPS promotion parents have unsafe identity, owner, mode, or device"
        );
        let partial_name = candidate
            .partial_path
            .file_name()
            .context("partial candidate has no basename")?;
        let installed_name = candidate
            .installed_path
            .file_name()
            .context("installed candidate has no basename")?;
        let named = statat(
            release_parent.as_fd(),
            partial_name,
            AtFlags::SYMLINK_NOFOLLOW,
        )?;
        ensure!(
            named.st_dev == candidate.device && named.st_ino == candidate.inode,
            "partial candidate basename differs from the retained descriptor"
        );
        ensure!(
            statat(
                release_parent.as_fd(),
                installed_name,
                AtFlags::SYMLINK_NOFOLLOW,
            )
            .is_err_and(|error| error == rustix::io::Errno::NOENT),
            "installed VPS release already exists"
        );
        activation_lock.ensure_canonical()?;
        let rename = renameat_with(
            release_parent.as_fd(),
            partial_name,
            release_parent.as_fd(),
            installed_name,
            RenameFlags::NOREPLACE,
        );
        if let Err(error) = rename
            && candidate.canonical_path()? != candidate.installed_path
        {
            return Err(error.into());
        }
        candidate.ensure_canonical()?;
        rustix::fs::fsync(release_parent)?;
        activation_lock.ensure_canonical()?;
        candidate.ensure_canonical()?;
        Ok(expected)
    }
}

pub(super) struct PinnedVpsSourceRoot {
    pub(super) name: std::ffi::OsString,
    pub(super) fd: std::os::fd::OwnedFd,
    pub(super) device: u64,
    pub(super) inode: u64,
}

pub(super) struct PinnedVpsCandidateParentsV2 {
    pub(super) install_root_fd: std::os::fd::OwnedFd,
    pub(super) incoming_parent_fd: std::os::fd::OwnedFd,
    pub(super) installed_parent_fd: std::os::fd::OwnedFd,
    pub(super) install_root_path: PathBuf,
    pub(super) incoming_parent_path: PathBuf,
    pub(super) installed_parent_path: PathBuf,
    pub(super) install_root_device: u64,
    pub(super) install_root_inode: u64,
    pub(super) incoming_parent_device: u64,
    pub(super) incoming_parent_inode: u64,
    pub(super) installed_parent_device: u64,
    pub(super) installed_parent_inode: u64,
}

pub(super) struct PinnedInheritedVpsCandidateRootV2 {
    pub(super) fd: std::os::fd::OwnedFd,
    pub(super) parents: PinnedVpsCandidateParentsV2,
    pub(super) partial_path: PathBuf,
    pub(super) installed_path: PathBuf,
    pub(super) device: u64,
    pub(super) inode: u64,
}

impl PinnedInheritedVpsCandidateRootV2 {
    pub(super) fn canonical_path(&self) -> Result<PathBuf> {
        use rustix::fs::{AtFlags, FileType, statat};
        use std::os::fd::AsFd as _;

        let matches = [
            (&self.parents.installed_parent_fd, &self.partial_path),
            (&self.parents.installed_parent_fd, &self.installed_path),
        ]
        .into_iter()
        .filter(|(parent, path)| {
            path.file_name().is_some_and(|basename| {
                statat(parent.as_fd(), basename, AtFlags::SYMLINK_NOFOLLOW).is_ok_and(|metadata| {
                    FileType::from_raw_mode(metadata.st_mode).is_dir()
                        && metadata.st_dev == self.device
                        && metadata.st_ino == self.inode
                })
            })
        })
        .map(|(_, path)| path.clone())
        .collect::<Vec<_>>();
        ensure!(
            matches.len() == 1,
            "inherited VPS candidate must name exactly one canonical partial or installed root"
        );
        Ok(matches.into_iter().next().expect("length checked"))
    }

    pub(super) fn ensure_canonical(&self) -> Result<()> {
        use rustix::fs::{FileType, Mode, OFlags, ResolveFlags, openat2};
        use std::os::fd::AsFd as _;
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let pinned = rustix::fs::fstat(&self.fd)?;
        ensure!(
            FileType::from_raw_mode(pinned.st_mode).is_dir()
                && pinned.st_dev == self.device
                && pinned.st_ino == self.inode
                && pinned.st_uid == rustix::process::geteuid().as_raw()
                && pinned.st_mode & 0o777 == 0o550,
            "inherited VPS candidate descriptor changed"
        );
        let validate_named_parent = |path: &Path,
                                     retained: &std::os::fd::OwnedFd,
                                     expected_device: u64,
                                     expected_inode: u64|
         -> Result<()> {
            let named = fs::symlink_metadata(path)?;
            let held = rustix::fs::fstat(retained)?;
            ensure!(
                named.is_dir()
                    && !named.file_type().is_symlink()
                    && fs::canonicalize(path)? == path
                    && named.uid() == rustix::process::geteuid().as_raw()
                    && named.permissions().mode() & 0o777 == 0o750
                    && named.dev() == expected_device
                    && named.ino() == expected_inode
                    && held.st_uid == rustix::process::geteuid().as_raw()
                    && held.st_mode & 0o777 == 0o750
                    && held.st_dev == expected_device
                    && held.st_ino == expected_inode,
                "retained VPS candidate parent authority changed"
            );
            Ok(())
        };
        validate_named_parent(
            &self.parents.install_root_path,
            &self.parents.install_root_fd,
            self.parents.install_root_device,
            self.parents.install_root_inode,
        )?;
        let rebound_install_root = openat2(
            rustix::fs::CWD,
            &self.parents.install_root_path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let rebound_install_root = rustix::fs::fstat(&rebound_install_root)?;
        ensure!(
            rebound_install_root.st_dev == self.parents.install_root_device
                && rebound_install_root.st_ino == self.parents.install_root_inode,
            "retained VPS install root pathname changed"
        );
        for (name, path, retained, expected_device, expected_inode) in [
            (
                "incoming",
                &self.parents.incoming_parent_path,
                &self.parents.incoming_parent_fd,
                self.parents.incoming_parent_device,
                self.parents.incoming_parent_inode,
            ),
            (
                "releases",
                &self.parents.installed_parent_path,
                &self.parents.installed_parent_fd,
                self.parents.installed_parent_device,
                self.parents.installed_parent_inode,
            ),
        ] {
            validate_named_parent(path, retained, expected_device, expected_inode)?;
            let rebound = openat2(
                self.parents.install_root_fd.as_fd(),
                name,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
                Mode::empty(),
                ResolveFlags::BENEATH
                    | ResolveFlags::NO_SYMLINKS
                    | ResolveFlags::NO_MAGICLINKS
                    | ResolveFlags::NO_XDEV,
            )?;
            let rebound = rustix::fs::fstat(&rebound)?;
            ensure!(
                rebound.st_dev == expected_device && rebound.st_ino == expected_inode,
                "retained VPS candidate parent pathname changed"
            );
        }
        let canonical_path = self.canonical_path()?;
        let named = fs::symlink_metadata(&canonical_path)?;
        ensure!(
            named.is_dir()
                && !named.file_type().is_symlink()
                && fs::canonicalize(&canonical_path)? == canonical_path
                && named.dev() == self.device
                && named.ino() == self.inode
                && named.uid() == rustix::process::geteuid().as_raw()
                && named.permissions().mode() & 0o777 == 0o550,
            "inherited VPS candidate canonical pathname changed"
        );
        let basename = canonical_path
            .file_name()
            .context("inherited VPS candidate has no basename")?;
        let parent = &self.parents.installed_parent_fd;
        let rebound = openat2(
            parent.as_fd(),
            basename,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::BENEATH
                | ResolveFlags::NO_SYMLINKS
                | ResolveFlags::NO_MAGICLINKS
                | ResolveFlags::NO_XDEV,
        )?;
        let rebound = rustix::fs::fstat(&rebound)?;
        ensure!(
            rebound.st_dev == self.device && rebound.st_ino == self.inode,
            "inherited VPS candidate path does not name the retained descriptor"
        );
        Ok(())
    }
}

pub(super) fn pin_inherited_vps_candidate_root_v2(
    source_commit: &str,
    candidate_root_fd: std::os::fd::RawFd,
    expected_manifest_sha256: Digest32,
) -> Result<PinnedInheritedVpsCandidateRootV2> {
    pin_inherited_vps_candidate_root_at(
        source_commit,
        candidate_root_fd,
        expected_manifest_sha256,
        &Path::new(INSTALL_ROOT).join("incoming"),
        &Path::new(INSTALL_ROOT).join("releases"),
    )
}

pub(super) fn pin_inherited_vps_candidate_root_at(
    source_commit: &str,
    candidate_root_fd: std::os::fd::RawFd,
    expected_manifest_sha256: Digest32,
    incoming_root: &Path,
    releases_root: &Path,
) -> Result<PinnedInheritedVpsCandidateRootV2> {
    pin_inherited_vps_candidate_root_at_with(
        source_commit,
        candidate_root_fd,
        expected_manifest_sha256,
        incoming_root,
        releases_root,
        |root| {
            let digest = validate_pinned_current_vps_release_root(root)?;
            let manifest: VpsReleaseManifestV2 = load_canonical(&root.join(RELEASE_MANIFEST_FILE))?;
            Ok((digest, manifest.source_commit))
        },
    )
}

pub(super) fn pin_vps_candidate_parents_at(
    incoming_root: &Path,
    releases_root: &Path,
) -> Result<PinnedVpsCandidateParentsV2> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::os::fd::AsFd as _;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let install_root = incoming_root
        .parent()
        .context("VPS incoming root has no install parent")?;
    ensure!(
        incoming_root
            .file_name()
            .is_some_and(|name| name == "incoming")
            && releases_root
                .file_name()
                .is_some_and(|name| name == "releases")
            && releases_root.parent() == Some(install_root)
            && normalized_absolute(install_root)
            && normalized_absolute(incoming_root)
            && normalized_absolute(releases_root),
        "VPS candidate parents are not the exact normalized incoming/releases siblings"
    );
    let install_named = fs::symlink_metadata(install_root)?;
    ensure!(
        install_named.is_dir()
            && !install_named.file_type().is_symlink()
            && fs::canonicalize(install_root)? == install_root
            && install_named.uid() == rustix::process::geteuid().as_raw()
            && install_named.permissions().mode() & 0o777 == 0o750,
        "VPS candidate install root has unsafe path, owner, or mode"
    );
    let install_root_fd = openat2(
        rustix::fs::CWD,
        install_root,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let install_pinned = rustix::fs::fstat(&install_root_fd)?;
    ensure!(
        install_pinned.st_dev == install_named.dev()
            && install_pinned.st_ino == install_named.ino(),
        "VPS candidate install root changed while it was pinned"
    );
    let pin_child = |name: &str, path: &Path| -> Result<std::os::fd::OwnedFd> {
        let named = fs::symlink_metadata(path)?;
        ensure!(
            named.is_dir()
                && !named.file_type().is_symlink()
                && fs::canonicalize(path)? == path
                && named.uid() == rustix::process::geteuid().as_raw()
                && named.permissions().mode() & 0o777 == 0o750
                && named.dev() == install_pinned.st_dev,
            "VPS candidate parent has unsafe path, owner, mode, or device"
        );
        let child = openat2(
            install_root_fd.as_fd(),
            name,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::BENEATH
                | ResolveFlags::NO_SYMLINKS
                | ResolveFlags::NO_MAGICLINKS
                | ResolveFlags::NO_XDEV,
        )?;
        let pinned = rustix::fs::fstat(&child)?;
        ensure!(
            pinned.st_dev == named.dev()
                && pinned.st_ino == named.ino()
                && pinned.st_uid == named.uid()
                && pinned.st_mode & 0o777 == 0o750,
            "VPS candidate parent changed while it was pinned"
        );
        Ok(child)
    };
    let incoming_parent_fd = pin_child("incoming", incoming_root)?;
    let installed_parent_fd = pin_child("releases", releases_root)?;
    let incoming_parent = rustix::fs::fstat(&incoming_parent_fd)?;
    let installed_parent = rustix::fs::fstat(&installed_parent_fd)?;
    Ok(PinnedVpsCandidateParentsV2 {
        install_root_fd,
        incoming_parent_fd,
        installed_parent_fd,
        install_root_path: install_root.to_path_buf(),
        incoming_parent_path: incoming_root.to_path_buf(),
        installed_parent_path: releases_root.to_path_buf(),
        install_root_device: install_pinned.st_dev,
        install_root_inode: install_pinned.st_ino,
        incoming_parent_device: incoming_parent.st_dev,
        incoming_parent_inode: incoming_parent.st_ino,
        installed_parent_device: installed_parent.st_dev,
        installed_parent_inode: installed_parent.st_ino,
    })
}

pub(super) fn pin_inherited_vps_candidate_root_at_with<V>(
    source_commit: &str,
    candidate_root_fd: std::os::fd::RawFd,
    expected_manifest_sha256: Digest32,
    incoming_root: &Path,
    releases_root: &Path,
    validate_candidate: V,
) -> Result<PinnedInheritedVpsCandidateRootV2>
where
    V: Fn(&Path) -> Result<(Digest32, String)>,
{
    use rustix::fs::FileType;
    use std::os::fd::{AsRawFd as _, OwnedFd};

    let parents = pin_vps_candidate_parents_at(incoming_root, releases_root)?;

    ensure!(
        candidate_root_fd >= 3,
        "inherited VPS candidate descriptor must be at least 3"
    );
    let inherited = fd_policy::stat(candidate_root_fd)?;
    ensure!(
        FileType::from_raw_mode(inherited.st_mode).is_dir()
            && inherited.st_uid == rustix::process::geteuid().as_raw()
            && inherited.st_mode & 0o777 == 0o550,
        "inherited VPS candidate descriptor has unsafe type, owner, or mode"
    );
    let descriptor_path = PathBuf::from(format!("/proc/self/fd/{candidate_root_fd}"));
    let fd = OwnedFd::from(File::open(&descriptor_path)?);
    let pinned = rustix::fs::fstat(&fd)?;
    ensure!(
        pinned.st_dev == inherited.st_dev && pinned.st_ino == inherited.st_ino,
        "inherited VPS candidate descriptor changed while it was duplicated"
    );
    let partial = releases_root.join(format!("{source_commit}.partial"));
    let installed = releases_root.join(source_commit);
    let candidate = PinnedInheritedVpsCandidateRootV2 {
        fd,
        parents,
        partial_path: partial,
        installed_path: installed,
        device: pinned.st_dev,
        inode: pinned.st_ino,
    };
    candidate.ensure_canonical()?;
    let root = PathBuf::from(format!("/proc/self/fd/{}/.", candidate.fd.as_raw_fd()));
    let (manifest_sha256, manifest_source_commit) = validate_candidate(&root)?;
    ensure!(
        manifest_sha256 == expected_manifest_sha256,
        "inherited VPS candidate differs from the out-of-band V2 manifest digest"
    );
    ensure!(
        manifest_source_commit == source_commit,
        "inherited VPS candidate source commit differs from the plan"
    );
    candidate.ensure_canonical()?;
    Ok(candidate)
}

pub(super) fn consume_vps_sources_in_with<CV, PV, EL, AR, AU>(
    plan: &VpsReleasePlanV2,
    plan_bytes: &[u8],
    plan_sha256: Digest32,
    release_manifest_sha256: Digest32,
    incoming_parent: &Path,
    logical_source: &Path,
    inherited_candidate: &PinnedInheritedVpsCandidateRootV2,
    validate_candidate: CV,
    validate_publication: PV,
    ensure_lock: EL,
    after_rename: AR,
    after_root_unlink: AU,
) -> Result<()>
where
    CV: Fn(&Path) -> Result<(Digest32, Digest32)>,
    PV: Fn(&PinnedVpsSourceRoot, &Path) -> Result<Digest32>,
    EL: Fn() -> Result<()>,
    AR: Fn(&Path) -> Result<()>,
    AU: Fn(&Path) -> Result<()>,
{
    use rustix::fs::{
        AtFlags, Mode, OFlags, RenameFlags, ResolveFlags, openat2, renameat_with, statat, unlinkat,
    };
    use std::os::fd::{AsFd as _, AsRawFd as _};
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let expected_incoming = Path::new(INSTALL_ROOT).join("incoming");
    if incoming_parent == expected_incoming {
        ensure!(
            fs::canonicalize(incoming_parent)? == incoming_parent,
            "VPS incoming parent is not canonical"
        );
        reject_mounts_at_or_below(incoming_parent)?;
    }
    let incoming_metadata = fs::symlink_metadata(incoming_parent)?;
    ensure!(
        incoming_metadata.is_dir()
            && !incoming_metadata.file_type().is_symlink()
            && incoming_metadata.uid() == rustix::process::geteuid().as_raw()
            && incoming_metadata.permissions().mode() & 0o777 == 0o750,
        "VPS incoming parent must be EUID-owned mode 0750"
    );
    let parent_fd = openat2(
        rustix::fs::CWD,
        incoming_parent,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let parent = rustix::fs::fstat(&parent_fd)?;
    ensure!(
        parent.st_dev == incoming_metadata.dev()
            && parent.st_ino == incoming_metadata.ino()
            && parent.st_uid == rustix::process::geteuid().as_raw(),
        "VPS incoming parent changed while it was pinned"
    );

    let source_name = std::ffi::OsString::from(format!(".sources-{}", plan.source_commit));
    let consuming_name =
        std::ffi::OsString::from(format!(".sources-{}.consuming", plan.source_commit));
    let journal_name =
        std::ffi::OsString::from(format!(".sources-{}.consume-v1.json", plan.source_commit));
    let journal_temporary_name = std::ffi::OsString::from(format!(
        ".sources-{}.consume-v1.json.new",
        plan.source_commit
    ));
    let terminal_journal_name = std::ffi::OsString::from(format!(
        ".sources-{}.consume-v1.complete.json",
        plan.source_commit
    ));
    let terminal_journal_temporary_name = std::ffi::OsString::from(format!(
        ".sources-{}.consume-v1.complete.json.new",
        plan.source_commit
    ));

    let candidate_fd = &inherited_candidate.fd;
    let candidate = rustix::fs::fstat(candidate_fd)?;
    ensure!(
        candidate.st_uid == rustix::process::geteuid().as_raw()
            && candidate.st_dev == parent.st_dev
            && candidate.st_mode & 0o777 == 0o550,
        "VPS candidate root identity, device, owner, or mode is unsafe"
    );
    let candidate_path = PathBuf::from(format!("/proc/self/fd/{}/.", candidate_fd.as_raw_fd()));
    let (validated_candidate_manifest, candidate_publication_lock) =
        validate_candidate(&candidate_path)?;
    ensure!(
        validated_candidate_manifest == release_manifest_sha256,
        "pinned VPS candidate differs from the expected release manifest digest"
    );
    ensure!(
        fs::read(candidate_path.join(SOURCE_COMMIT_FILE))?
            == format!("{}\n", plan.source_commit).as_bytes(),
        "pinned VPS candidate source commit differs from the plan"
    );
    let ensure_authorities = || -> Result<()> {
        ensure_lock()?;
        ensure_named_vps_incoming_parent(incoming_parent, &parent)?;
        inherited_candidate.ensure_canonical()
    };
    ensure_authorities()?;

    // Recovery names are transaction evidence. Authenticate the exact
    // candidate authority before moving any of them back into place.
    for name in [
        &journal_name,
        &journal_temporary_name,
        &terminal_journal_name,
        &terminal_journal_temporary_name,
    ] {
        restore_vps_source_document_removing(&parent_fd, name, &ensure_authorities)?;
    }

    reconcile_vps_source_journal_temporary(
        &parent_fd,
        &journal_name,
        &journal_temporary_name,
        &source_name,
        &consuming_name,
        &plan.source_commit,
        VpsSourceConsumePhaseV1::Prepared,
        &ensure_authorities,
    )?;
    reconcile_vps_source_journal_temporary(
        &parent_fd,
        &terminal_journal_name,
        &terminal_journal_temporary_name,
        &source_name,
        &consuming_name,
        &plan.source_commit,
        VpsSourceConsumePhaseV1::RootUnlinked,
        &ensure_authorities,
    )?;
    let source_exists = pinned_entry_exists(&parent_fd, &source_name)?;
    let consuming_exists = pinned_entry_exists(&parent_fd, &consuming_name)?;
    ensure!(
        !(source_exists && consuming_exists),
        "both VPS source and consuming roots exist; preserve ambiguous evidence"
    );

    if pinned_entry_exists(&parent_fd, &terminal_journal_name)? {
        let terminal = load_vps_source_consume_journal(&parent_fd, &terminal_journal_name)?;
        validate_vps_source_consume_journal(
            &terminal,
            plan,
            plan_sha256,
            release_manifest_sha256,
            candidate.st_dev,
            candidate.st_ino,
        )?;
        ensure!(
            terminal.phase == VpsSourceConsumePhaseV1::RootUnlinked
                && !source_exists
                && !consuming_exists,
            "terminal VPS source journal coexists with a source root"
        );
        rustix::fs::fsync(&parent_fd)?;
        ensure_authorities()?;
        ensure_lock()?;
        ensure_named_vps_incoming_parent(incoming_parent, &parent)?;
        if pinned_entry_exists(&parent_fd, &journal_name)? {
            let mut expected_prepared = terminal.clone();
            expected_prepared.phase = VpsSourceConsumePhaseV1::Prepared;
            remove_exact_vps_source_journal(
                &parent_fd,
                &journal_name,
                &expected_prepared,
                &ensure_authorities,
            )?;
        }
        remove_exact_vps_source_journal(
            &parent_fd,
            &terminal_journal_name,
            &terminal,
            &ensure_authorities,
        )?;
        ensure_authorities()?;
        return Ok(());
    }

    let mut journal = if pinned_entry_exists(&parent_fd, &journal_name)? {
        load_vps_source_consume_journal(&parent_fd, &journal_name)?
    } else if source_exists {
        let source = open_vps_source_root(&parent_fd, &parent, source_name.clone(), 0o700)?;
        let journal = build_vps_source_consume_journal(
            plan,
            plan_bytes,
            plan_sha256,
            release_manifest_sha256,
            &source,
            candidate.st_dev,
            candidate.st_ino,
            &validate_publication,
            logical_source,
            candidate_publication_lock,
        )?;
        ensure_authorities()?;
        publish_vps_source_consume_journal(
            &parent_fd,
            &journal_name,
            &journal_temporary_name,
            &journal,
            &ensure_authorities,
        )?;
        journal
    } else if consuming_exists {
        anyhow::bail!("VPS consuming source root exists without its durable inventory journal");
    } else {
        // A completed retry is idempotent after the exact candidate is still
        // authenticated above.
        rustix::fs::fsync(&parent_fd)?;
        ensure_authorities()?;
        return Ok(());
    };
    validate_vps_source_consume_journal(
        &journal,
        plan,
        plan_sha256,
        release_manifest_sha256,
        candidate.st_dev,
        candidate.st_ino,
    )?;
    ensure_authorities()?;
    ensure!(
        journal.phase == VpsSourceConsumePhaseV1::Prepared,
        "the primary VPS source journal is not in prepared phase"
    );
    if !source_exists && !consuming_exists {
        rustix::fs::fsync(&parent_fd)?;
        ensure_authorities()?;
        let terminal = publish_vps_source_terminal_journal(
            &parent_fd,
            &terminal_journal_name,
            &terminal_journal_temporary_name,
            &journal,
            &ensure_authorities,
        )?;
        ensure_authorities()?;
        remove_exact_vps_source_journal(&parent_fd, &journal_name, &journal, &ensure_authorities)?;
        ensure_authorities()?;
        remove_exact_vps_source_journal(
            &parent_fd,
            &terminal_journal_name,
            &terminal,
            &ensure_authorities,
        )?;
        ensure_authorities()?;
        return Ok(());
    }

    let consuming = if consuming_exists {
        let root = open_vps_source_root(&parent_fd, &parent, consuming_name.clone(), 0o700)?;
        ensure!(
            root.device == journal.source_device && root.inode == journal.source_inode,
            "VPS consuming source root differs from its durable journal"
        );
        root
    } else {
        ensure!(
            source_exists,
            "VPS source root disappeared before consumption"
        );
        let source = open_vps_source_root(&parent_fd, &parent, source_name.clone(), 0o700)?;
        ensure!(
            source.device == journal.source_device && source.inode == journal.source_inode,
            "VPS source root differs from its durable journal"
        );
        ensure_authorities()?;
        renameat_with(
            parent_fd.as_fd(),
            &source_name,
            parent_fd.as_fd(),
            &consuming_name,
            RenameFlags::NOREPLACE,
        )
        .or_else(|rename_error| {
            let observed = statat(
                parent_fd.as_fd(),
                &consuming_name,
                AtFlags::SYMLINK_NOFOLLOW,
            );
            if observed.as_ref().is_ok_and(|metadata| {
                metadata.st_dev == source.device && metadata.st_ino == source.inode
            }) && statat(parent_fd.as_fd(), &source_name, AtFlags::SYMLINK_NOFOLLOW)
                .is_err_and(|error| error == rustix::io::Errno::NOENT)
            {
                Ok(())
            } else {
                Err(rename_error)
            }
        })?;
        rustix::fs::fsync(&parent_fd)?;
        after_rename(&incoming_parent.join(&consuming_name))?;
        PinnedVpsSourceRoot {
            name: consuming_name.clone(),
            ..source
        }
    };

    validate_vps_source_inventory_subset(&consuming, &journal.entries)?;
    ensure_authorities()?;
    let mut seen = 1;
    clear_pinned_vps_directory(
        &consuming.fd,
        Path::new(""),
        rustix::process::geteuid().as_raw(),
        consuming.device,
        0,
        &mut seen,
        &journal.entries,
        &ensure_authorities,
    )?;
    let named = statat(
        parent_fd.as_fd(),
        &consuming.name,
        AtFlags::SYMLINK_NOFOLLOW,
    )?;
    ensure!(
        named.st_dev == consuming.device && named.st_ino == consuming.inode,
        "VPS consuming source basename was substituted before root unlink"
    );
    ensure_authorities()?;
    unlinkat(parent_fd.as_fd(), &consuming.name, AtFlags::REMOVEDIR)?;
    ensure!(
        rustix::fs::fstat(&consuming.fd)?.st_nlink == 0,
        "VPS consuming source inode remains linked after root unlink"
    );
    rustix::fs::fsync(&parent_fd)?;
    ensure_authorities()?;
    after_root_unlink(incoming_parent)?;
    ensure_authorities()?;
    let terminal = publish_vps_source_terminal_journal(
        &parent_fd,
        &terminal_journal_name,
        &terminal_journal_temporary_name,
        &journal,
        &ensure_authorities,
    )?;
    ensure_authorities()?;
    remove_exact_vps_source_journal(&parent_fd, &journal_name, &journal, &ensure_authorities)?;
    ensure_authorities()?;
    remove_exact_vps_source_journal(
        &parent_fd,
        &terminal_journal_name,
        &terminal,
        &ensure_authorities,
    )?;
    ensure_authorities()?;
    journal.entries.clear();
    Ok(())
}

pub(super) fn ensure_named_vps_incoming_parent(
    incoming_parent: &Path,
    expected: &rustix::fs::Stat,
) -> Result<()> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let observed = fs::symlink_metadata(incoming_parent)?;
    ensure!(
        observed.is_dir()
            && !observed.file_type().is_symlink()
            && fs::canonicalize(incoming_parent)? == incoming_parent
            && observed.uid() == rustix::process::geteuid().as_raw()
            && observed.permissions().mode() & 0o777 == 0o750
            && observed.dev() == expected.st_dev
            && observed.ino() == expected.st_ino,
        "canonical VPS incoming parent changed during source consumption"
    );
    Ok(())
}

pub(super) fn open_vps_source_root(
    parent_fd: &std::os::fd::OwnedFd,
    parent: &rustix::fs::Stat,
    name: std::ffi::OsString,
    expected_mode: u32,
) -> Result<PinnedVpsSourceRoot> {
    use rustix::fs::{FileType, Mode, OFlags, ResolveFlags, openat2};
    use std::os::fd::AsFd as _;

    let fd = openat2(
        parent_fd.as_fd(),
        &name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let metadata = rustix::fs::fstat(&fd)?;
    ensure!(
        FileType::from_raw_mode(metadata.st_mode).is_dir()
            && metadata.st_uid == rustix::process::geteuid().as_raw()
            && metadata.st_dev == parent.st_dev
            && metadata.st_mode & 0o777 == expected_mode,
        "VPS source root has unsafe identity, device, owner, or mode"
    );
    Ok(PinnedVpsSourceRoot {
        name,
        fd,
        device: metadata.st_dev,
        inode: metadata.st_ino,
    })
}

pub(super) fn open_named_vps_source_root(
    source: &PinnedVpsSourceRoot,
    logical_source: &Path,
) -> Result<std::os::fd::OwnedFd> {
    use rustix::fs::{FileType, Mode, OFlags, ResolveFlags, openat2};

    let rebound = openat2(
        rustix::fs::CWD,
        logical_source,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let metadata = rustix::fs::fstat(&rebound)?;
    ensure!(
        FileType::from_raw_mode(metadata.st_mode).is_dir()
            && metadata.st_uid == rustix::process::geteuid().as_raw()
            && metadata.st_dev == source.device
            && metadata.st_ino == source.inode
            && metadata.st_mode & 0o777 == 0o700,
        "named VPS source root differs from its pinned descriptor"
    );
    Ok(rebound)
}

pub(super) fn open_pinned_vps_source_publication(
    source: &PinnedVpsSourceRoot,
) -> Result<std::os::fd::OwnedFd> {
    use rustix::fs::{FileType, Mode, OFlags, ResolveFlags, openat2};
    use std::os::fd::AsFd as _;

    let publication = openat2(
        source.fd.as_fd(),
        Path::new("publication-v3"),
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let metadata = rustix::fs::fstat(&publication)?;
    ensure!(
        FileType::from_raw_mode(metadata.st_mode).is_dir()
            && metadata.st_uid == rustix::process::geteuid().as_raw()
            && metadata.st_dev == source.device
            && metadata.st_mode & 0o777 == 0o700,
        "VPS source PublicationV3 root has unsafe identity, device, owner, or mode"
    );
    Ok(publication)
}

pub(super) fn same_stable_vps_source_node(
    left: &rustix::fs::Stat,
    right: &rustix::fs::Stat,
) -> bool {
    left.st_dev == right.st_dev
        && left.st_ino == right.st_ino
        && left.st_uid == right.st_uid
        && left.st_gid == right.st_gid
        && left.st_nlink == right.st_nlink
        && left.st_mode == right.st_mode
        && left.st_size == right.st_size
        && left.st_mtime == right.st_mtime
        && left.st_mtime_nsec == right.st_mtime_nsec
        && left.st_ctime == right.st_ctime
        && left.st_ctime_nsec == right.st_ctime_nsec
}

pub(super) fn with_pinned_vps_source_publication<T, F>(
    source: &PinnedVpsSourceRoot,
    logical_source: &Path,
    use_publication: F,
) -> Result<T>
where
    F: FnOnce(&Path, &File) -> Result<T>,
{
    let named_source = open_named_vps_source_root(source, logical_source)?;
    let publication = open_pinned_vps_source_publication(source)?;
    let publication_identity = rustix::fs::fstat(&publication)?;
    let named_publication = {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        use std::os::fd::AsFd as _;
        openat2(
            named_source.as_fd(),
            Path::new("publication-v3"),
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::BENEATH
                | ResolveFlags::NO_SYMLINKS
                | ResolveFlags::NO_MAGICLINKS
                | ResolveFlags::NO_XDEV,
        )?
    };
    ensure!(
        same_stable_vps_source_node(
            &publication_identity,
            &rustix::fs::fstat(&named_publication)?,
        ),
        "VPS source PublicationV3 differs between its retained and named parents"
    );

    let publication_file = File::from(publication);
    let logical_publication = logical_source.join("publication-v3");
    let result = use_publication(&logical_publication, &publication_file)?;

    open_named_vps_source_root(source, logical_source)?;
    let rebound_publication = open_pinned_vps_source_publication(source)?;
    ensure!(
        same_stable_vps_source_node(
            &publication_identity,
            &rustix::fs::fstat(&rebound_publication)?,
        ),
        "pinned VPS source PublicationV3 changed during validation"
    );
    Ok(result)
}

pub(super) fn validate_vps_source_publication_v3(
    source: &PinnedVpsSourceRoot,
    logical_source: &Path,
) -> Result<Digest32> {
    with_pinned_vps_source_publication(
        source,
        logical_source,
        |logical_publication, publication| {
            let validated = crate::publication_v3::validate_pinned_publication_v3(
                logical_publication,
                publication,
            )?;
            validated.ensure_live()?;
            Ok(validated.lock_sha256())
        },
    )
}

pub(super) fn build_vps_source_consume_journal<PV>(
    plan: &VpsReleasePlanV2,
    plan_bytes: &[u8],
    plan_sha256: Digest32,
    release_manifest_sha256: Digest32,
    source: &PinnedVpsSourceRoot,
    candidate_device: u64,
    candidate_inode: u64,
    validate_publication: &PV,
    logical_source: &Path,
    candidate_publication_lock: Digest32,
) -> Result<VpsSourceConsumeJournalV1>
where
    PV: Fn(&PinnedVpsSourceRoot, &Path) -> Result<Digest32>,
{
    ensure!(
        plan.publication_v3 == logical_source.join("publication-v3"),
        "VPS publication source must be exact .sources-COMMIT/publication-v3"
    );
    for binary in &plan.binaries {
        ensure!(
            binary.source == logical_source.join("bin").join(binary.role.output_name()),
            "VPS binary source is outside the exact source closure"
        );
    }
    for config in &plan.configs {
        ensure!(
            config.source
                == logical_source
                    .join("config")
                    .join(config.role.output_name()),
            "VPS config source is outside the exact source closure"
        );
    }
    for host in &plan.host_files {
        ensure!(
            host.source == logical_source.join("host").join(host.role.output_path()),
            "VPS host input is outside the exact source closure"
        );
    }

    let entries = inventory_pinned_vps_source_root(source)?;
    let by_path = entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut expected = BTreeSet::from([".".to_owned()]);
    let mut insert_file = |path: &str| -> Result<()> {
        ensure!(valid_relative_manifest_path(path), "unsafe VPS source path");
        expected.insert(path.to_owned());
        let mut parent = Path::new(path).parent();
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            expected.insert(path_to_manifest(directory)?);
            parent = directory.parent();
        }
        Ok(())
    };
    insert_file("vps-release-plan-v2.json")?;
    for binary in &plan.binaries {
        insert_file(&format!("bin/{}", binary.role.output_name()))?;
    }
    for config in &plan.configs {
        insert_file(&format!("config/{}", config.role.output_name()))?;
    }
    for host in &plan.host_files {
        insert_file(&format!("host/{}", host.role.output_path()))?;
    }

    ensure!(
        validate_publication(source, logical_source)? == candidate_publication_lock,
        "source PublicationV3 lock differs from the candidate-bound publication lock"
    );
    for entry in &entries {
        if entry.path == "publication-v3" || entry.path.starts_with("publication-v3/") {
            expected.insert(entry.path.clone());
        }
    }
    ensure!(
        entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<BTreeSet<_>>()
            == expected,
        "VPS source closure has missing or extra entries"
    );

    validate_source_entry(&by_path, ".", VpsSourceEntryKindV1::Directory, 0o700, None)?;
    for directory in [
        "bin",
        "config",
        "host",
        "host/systemd",
        "host/systemd/user",
        "host/deploy",
        "host/deploy/tests",
    ] {
        validate_source_entry(
            &by_path,
            directory,
            VpsSourceEntryKindV1::Directory,
            0o700,
            None,
        )?;
    }
    validate_source_entry(
        &by_path,
        "vps-release-plan-v2.json",
        VpsSourceEntryKindV1::File,
        0o400,
        Some(&ArtifactRefV1 {
            sha256: plan_sha256,
            byte_length: plan_bytes.len() as u64,
            media_type: "application/json".to_owned(),
        }),
    )?;
    for binary in &plan.binaries {
        validate_source_entry(
            &by_path,
            &format!("bin/{}", binary.role.output_name()),
            VpsSourceEntryKindV1::File,
            0o550,
            Some(&binary.artifact),
        )?;
    }
    for config in &plan.configs {
        validate_source_entry(
            &by_path,
            &format!("config/{}", config.role.output_name()),
            VpsSourceEntryKindV1::File,
            0o440,
            Some(&config.artifact),
        )?;
    }
    for host in &plan.host_files {
        validate_source_entry(
            &by_path,
            &format!("host/{}", host.role.output_path()),
            VpsSourceEntryKindV1::File,
            canonical_file_mode(host.role.output_path()),
            Some(&host.artifact),
        )?;
    }

    Ok(VpsSourceConsumeJournalV1 {
        schema_version: 1,
        phase: VpsSourceConsumePhaseV1::Prepared,
        source_commit: plan.source_commit.clone(),
        plan_sha256,
        release_manifest_sha256,
        source_device: source.device,
        source_inode: source.inode,
        candidate_device,
        candidate_inode,
        entries,
    })
}

pub(super) fn validate_source_entry(
    entries: &BTreeMap<&str, &VpsSourceConsumeEntryV1>,
    path: &str,
    kind: VpsSourceEntryKindV1,
    unix_mode: u32,
    artifact: Option<&ArtifactRefV1>,
) -> Result<()> {
    let entry = entries
        .get(path)
        .with_context(|| format!("VPS source closure is missing {path}"))?;
    ensure!(
        entry.kind == kind && entry.unix_mode == unix_mode,
        "VPS source entry {path} has the wrong kind or mode"
    );
    match artifact {
        Some(artifact) => ensure!(
            entry.sha256 == Some(artifact.sha256)
                && entry.byte_length == Some(artifact.byte_length),
            "VPS source entry {path} differs from its plan artifact"
        ),
        None => ensure!(
            entry.sha256.is_none() && entry.byte_length.is_none(),
            "VPS source directory {path} carries file identity"
        ),
    }
    Ok(())
}

pub(super) fn inventory_pinned_vps_source_root(
    source: &PinnedVpsSourceRoot,
) -> Result<Vec<VpsSourceConsumeEntryV1>> {
    let root = rustix::fs::fstat(&source.fd)?;
    let mut entries = vec![VpsSourceConsumeEntryV1 {
        path: ".".to_owned(),
        kind: VpsSourceEntryKindV1::Directory,
        unix_mode: root.st_mode & 0o777,
        sha256: None,
        byte_length: None,
    }];
    let mut seen = 1;
    inventory_pinned_vps_source_directory(
        &source.fd,
        Path::new(""),
        source.device,
        0,
        &mut seen,
        &mut entries,
    )?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

pub(super) fn inventory_pinned_vps_source_directory(
    directory_fd: &std::os::fd::OwnedFd,
    relative_root: &Path,
    expected_device: u64,
    depth: usize,
    seen: &mut usize,
    entries: &mut Vec<VpsSourceConsumeEntryV1>,
) -> Result<()> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, RawDir, ResolveFlags, openat2, statat};
    use std::ffi::OsString;
    use std::os::fd::AsFd as _;
    use std::os::unix::ffi::OsStringExt as _;

    ensure!(
        depth <= MAX_FAILED_VPS_STAGING_DEPTH,
        "VPS source closure exceeds traversal depth bound"
    );
    let scan_fd = openat2(
        directory_fd.as_fd(),
        ".",
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let mut buffer = Vec::with_capacity(65_536);
    let mut names = Vec::new();
    let mut directory = RawDir::new(&scan_fd, buffer.spare_capacity_mut());
    while let Some(entry) = directory.next() {
        let entry = entry?;
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." {
            names.push(OsString::from_vec(name.to_vec()));
        }
    }
    names.sort();
    for name in names {
        *seen = seen.checked_add(1).context("VPS source entry overflow")?;
        ensure!(
            *seen <= MAX_FAILED_VPS_STAGING_ENTRIES,
            "VPS source closure exceeds entry bound"
        );
        let metadata = statat(directory_fd.as_fd(), &name, AtFlags::SYMLINK_NOFOLLOW)?;
        ensure!(
            metadata.st_uid == rustix::process::geteuid().as_raw()
                && metadata.st_dev == expected_device,
            "VPS source closure contains mixed ownership or devices"
        );
        let relative = relative_root.join(&name);
        let path = path_to_manifest(&relative)?;
        let file_type = FileType::from_raw_mode(metadata.st_mode);
        if file_type.is_dir() {
            let child_fd = openat2(
                directory_fd.as_fd(),
                &name,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
                Mode::empty(),
                ResolveFlags::BENEATH
                    | ResolveFlags::NO_SYMLINKS
                    | ResolveFlags::NO_MAGICLINKS
                    | ResolveFlags::NO_XDEV,
            )?;
            let pinned = rustix::fs::fstat(&child_fd)?;
            ensure!(
                pinned.st_dev == metadata.st_dev
                    && pinned.st_ino == metadata.st_ino
                    && pinned.st_uid == metadata.st_uid
                    && pinned.st_mode == metadata.st_mode,
                "VPS source directory changed while it was pinned"
            );
            entries.push(VpsSourceConsumeEntryV1 {
                path,
                kind: VpsSourceEntryKindV1::Directory,
                unix_mode: metadata.st_mode & 0o777,
                sha256: None,
                byte_length: None,
            });
            inventory_pinned_vps_source_directory(
                &child_fd,
                &relative,
                expected_device,
                depth + 1,
                seen,
                entries,
            )?;
        } else if file_type.is_file() {
            ensure!(
                metadata.st_nlink == 1,
                "VPS source contains a hard-linked file"
            );
            let file_fd = openat2(
                directory_fd.as_fd(),
                &name,
                OFlags::RDONLY | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::BENEATH
                    | ResolveFlags::NO_SYMLINKS
                    | ResolveFlags::NO_MAGICLINKS
                    | ResolveFlags::NO_XDEV,
            )?;
            let pinned = rustix::fs::fstat(&file_fd)?;
            ensure!(
                pinned.st_dev == metadata.st_dev
                    && pinned.st_ino == metadata.st_ino
                    && pinned.st_uid == metadata.st_uid
                    && pinned.st_mode == metadata.st_mode
                    && pinned.st_nlink == metadata.st_nlink
                    && pinned.st_size == metadata.st_size,
                "VPS source file changed while it was pinned"
            );
            let mut pinned_file = File::from(file_fd);
            let sha256 = Digest32::digest_reader(&mut pinned_file)?;
            entries.push(VpsSourceConsumeEntryV1 {
                path,
                kind: VpsSourceEntryKindV1::File,
                unix_mode: metadata.st_mode & 0o777,
                sha256: Some(sha256),
                byte_length: Some(metadata.st_size as u64),
            });
        } else {
            anyhow::bail!("VPS source closure contains a link or special node");
        }
    }
    Ok(())
}

pub(super) fn clear_pinned_vps_directory<F>(
    directory_fd: &std::os::fd::OwnedFd,
    relative_root: &Path,
    expected_uid: u32,
    expected_device: u64,
    depth: usize,
    seen: &mut usize,
    expected_entries: &[VpsSourceConsumeEntryV1],
    ensure_authority: &F,
) -> Result<()>
where
    F: Fn() -> Result<()>,
{
    use rustix::fs::{
        AtFlags, FileType, Mode, OFlags, RawDir, ResolveFlags, fchmod, openat2, statat, unlinkat,
    };
    use std::ffi::OsString;
    use std::io::{Seek as _, SeekFrom};
    use std::os::fd::AsFd as _;
    use std::os::unix::ffi::OsStringExt as _;

    ensure!(
        depth <= MAX_FAILED_VPS_STAGING_DEPTH,
        "VPS source cleanup exceeds traversal depth bound"
    );
    let directory = rustix::fs::fstat(directory_fd)?;
    ensure!(
        FileType::from_raw_mode(directory.st_mode).is_dir()
            && directory.st_uid == expected_uid
            && directory.st_dev == expected_device,
        "pinned VPS source cleanup directory has unsafe identity"
    );

    ensure_authority()?;
    fchmod(directory_fd, Mode::from_raw_mode(0o700))?;
    ensure_authority()?;
    let writable = rustix::fs::fstat(directory_fd)?;
    ensure!(
        writable.st_dev == directory.st_dev
            && writable.st_ino == directory.st_ino
            && writable.st_uid == expected_uid
            && writable.st_mode & 0o777 == 0o700,
        "pinned VPS source cleanup directory changed during permission transition"
    );

    let scan_fd = openat2(
        directory_fd.as_fd(),
        ".",
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let mut buffer = Vec::with_capacity(65_536);
    let mut names = Vec::new();
    let mut entries = RawDir::new(&scan_fd, buffer.spare_capacity_mut());
    while let Some(entry) = entries.next() {
        let entry = entry?;
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." {
            names.push(OsString::from_vec(name.to_vec()));
        }
    }
    names.sort();

    for name in names {
        *seen = seen
            .checked_add(1)
            .context("VPS source cleanup entry count overflow")?;
        ensure!(
            *seen <= MAX_FAILED_VPS_STAGING_ENTRIES,
            "VPS source cleanup exceeds entry bound"
        );
        let named = statat(directory_fd.as_fd(), &name, AtFlags::SYMLINK_NOFOLLOW)?;
        ensure!(
            named.st_uid == expected_uid && named.st_dev == expected_device,
            "VPS source cleanup entry has mixed ownership or devices"
        );
        let relative = relative_root.join(&name);
        let expected_path = path_to_manifest(&relative)?;
        let expected_entry = expected_entries
            .iter()
            .find(|entry| entry.path == expected_path)
            .with_context(|| {
                format!("VPS source cleanup found unjournaled entry {expected_path}")
            })?;
        let kind = FileType::from_raw_mode(named.st_mode);
        if kind.is_dir() {
            ensure!(
                expected_entry.kind == VpsSourceEntryKindV1::Directory
                    && expected_entry.sha256.is_none()
                    && expected_entry.byte_length.is_none(),
                "VPS source cleanup directory differs from its durable journal"
            );
            let child = openat2(
                directory_fd.as_fd(),
                &name,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
                Mode::empty(),
                ResolveFlags::BENEATH
                    | ResolveFlags::NO_SYMLINKS
                    | ResolveFlags::NO_MAGICLINKS
                    | ResolveFlags::NO_XDEV,
            )?;
            let pinned = rustix::fs::fstat(&child)?;
            ensure!(
                pinned.st_dev == named.st_dev
                    && pinned.st_ino == named.st_ino
                    && pinned.st_uid == named.st_uid
                    && pinned.st_mode == named.st_mode,
                "VPS source cleanup directory changed while it was pinned"
            );
            clear_pinned_vps_directory(
                &child,
                &relative,
                expected_uid,
                expected_device,
                depth + 1,
                seen,
                expected_entries,
                ensure_authority,
            )?;
            let rebound = statat(directory_fd.as_fd(), &name, AtFlags::SYMLINK_NOFOLLOW)?;
            ensure!(
                rebound.st_dev == pinned.st_dev
                    && rebound.st_ino == pinned.st_ino
                    && rebound.st_uid == pinned.st_uid
                    && FileType::from_raw_mode(rebound.st_mode).is_dir()
                    && rebound.st_mode & 0o777 == 0o700,
                "VPS source cleanup directory basename was substituted"
            );
            ensure_authority()?;
            unlinkat(directory_fd.as_fd(), &name, AtFlags::REMOVEDIR)?;
            ensure!(
                rustix::fs::fstat(&child)?.st_nlink == 0,
                "VPS source cleanup directory remains linked after unlink"
            );
        } else if kind.is_file() {
            ensure!(named.st_nlink == 1, "VPS source cleanup found a hard link");
            let child = openat2(
                directory_fd.as_fd(),
                &name,
                OFlags::RDONLY | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::BENEATH
                    | ResolveFlags::NO_SYMLINKS
                    | ResolveFlags::NO_MAGICLINKS
                    | ResolveFlags::NO_XDEV,
            )?;
            let pinned = rustix::fs::fstat(&child)?;
            ensure!(
                pinned.st_dev == named.st_dev
                    && pinned.st_ino == named.st_ino
                    && pinned.st_uid == named.st_uid
                    && pinned.st_mode == named.st_mode
                    && pinned.st_nlink == 1
                    && pinned.st_size == named.st_size,
                "VPS source cleanup file changed while it was pinned"
            );
            ensure!(
                expected_entry.kind == VpsSourceEntryKindV1::File
                    && expected_entry.byte_length == Some(pinned.st_size as u64),
                "VPS source cleanup file length differs from its durable journal"
            );
            let mut child = File::from(child);
            let first_digest = Digest32::digest_reader(&mut child)?;
            ensure!(
                expected_entry.sha256 == Some(first_digest),
                "VPS source cleanup file bytes differ from its durable journal"
            );
            let after_first_hash = rustix::fs::fstat(&child)?;
            ensure!(
                after_first_hash.st_dev == pinned.st_dev
                    && after_first_hash.st_ino == pinned.st_ino
                    && after_first_hash.st_uid == pinned.st_uid
                    && after_first_hash.st_mode == pinned.st_mode
                    && after_first_hash.st_nlink == pinned.st_nlink
                    && after_first_hash.st_size == pinned.st_size
                    && after_first_hash.st_mtime == pinned.st_mtime
                    && after_first_hash.st_mtime_nsec == pinned.st_mtime_nsec
                    && after_first_hash.st_ctime == pinned.st_ctime
                    && after_first_hash.st_ctime_nsec == pinned.st_ctime_nsec,
                "VPS source cleanup file changed while it was hashed"
            );
            let rebound = statat(directory_fd.as_fd(), &name, AtFlags::SYMLINK_NOFOLLOW)?;
            ensure!(
                rebound.st_dev == pinned.st_dev
                    && rebound.st_ino == pinned.st_ino
                    && rebound.st_uid == pinned.st_uid
                    && rebound.st_mode == pinned.st_mode
                    && rebound.st_nlink == 1
                    && rebound.st_size == pinned.st_size
                    && rebound.st_mtime == pinned.st_mtime
                    && rebound.st_mtime_nsec == pinned.st_mtime_nsec
                    && rebound.st_ctime == pinned.st_ctime
                    && rebound.st_ctime_nsec == pinned.st_ctime_nsec,
                "VPS source cleanup file basename was substituted"
            );
            ensure_authority()?;
            child.seek(SeekFrom::Start(0))?;
            ensure!(
                Digest32::digest_reader(&mut child)? == first_digest,
                "VPS source cleanup file changed at its unlink boundary"
            );
            let final_pinned = rustix::fs::fstat(&child)?;
            let final_named = statat(directory_fd.as_fd(), &name, AtFlags::SYMLINK_NOFOLLOW)?;
            ensure!(
                final_pinned.st_dev == pinned.st_dev
                    && final_pinned.st_ino == pinned.st_ino
                    && final_pinned.st_uid == pinned.st_uid
                    && final_pinned.st_mode == pinned.st_mode
                    && final_pinned.st_nlink == 1
                    && final_pinned.st_size == pinned.st_size
                    && final_pinned.st_mtime == pinned.st_mtime
                    && final_pinned.st_mtime_nsec == pinned.st_mtime_nsec
                    && final_pinned.st_ctime == pinned.st_ctime
                    && final_pinned.st_ctime_nsec == pinned.st_ctime_nsec
                    && final_named.st_dev == final_pinned.st_dev
                    && final_named.st_ino == final_pinned.st_ino
                    && final_named.st_mode == final_pinned.st_mode
                    && final_named.st_nlink == final_pinned.st_nlink
                    && final_named.st_size == final_pinned.st_size
                    && final_named.st_mtime == final_pinned.st_mtime
                    && final_named.st_mtime_nsec == final_pinned.st_mtime_nsec
                    && final_named.st_ctime == final_pinned.st_ctime
                    && final_named.st_ctime_nsec == final_pinned.st_ctime_nsec,
                "VPS source cleanup file identity changed at its unlink boundary"
            );
            unlinkat(directory_fd.as_fd(), &name, AtFlags::empty())?;
            ensure!(
                rustix::fs::fstat(&child)?.st_nlink == 0,
                "VPS source cleanup file remains linked after unlink"
            );
        } else {
            anyhow::bail!("VPS source cleanup found a symlink or special node");
        }
        rustix::fs::fsync(directory_fd)?;
        ensure_authority()?;
    }
    Ok(())
}

pub(super) fn validate_vps_source_consume_journal(
    journal: &VpsSourceConsumeJournalV1,
    plan: &VpsReleasePlanV2,
    plan_sha256: Digest32,
    release_manifest_sha256: Digest32,
    candidate_device: u64,
    candidate_inode: u64,
) -> Result<()> {
    ensure!(
        journal.schema_version == 1
            && journal.source_commit == plan.source_commit
            && journal.plan_sha256 == plan_sha256
            && journal.release_manifest_sha256 == release_manifest_sha256
            && journal.candidate_device == candidate_device
            && journal.candidate_inode == candidate_inode
            && journal.source_device != 0
            && journal.source_inode != 0
            && !journal.entries.is_empty()
            && journal
                .entries
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path),
        "VPS source consume journal identity or ordering is invalid"
    );
    for entry in &journal.entries {
        ensure!(
            entry.path == "." || valid_relative_manifest_path(&entry.path),
            "VPS source consume journal has an unsafe path"
        );
        match entry.kind {
            VpsSourceEntryKindV1::Directory => ensure!(
                entry.sha256.is_none() && entry.byte_length.is_none(),
                "VPS source consume journal directory has file identity"
            ),
            VpsSourceEntryKindV1::File => ensure!(
                entry.sha256.is_some() && entry.byte_length.is_some(),
                "VPS source consume journal file lacks identity"
            ),
        }
    }
    Ok(())
}

pub(super) fn load_vps_source_consume_journal(
    parent_fd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsString,
) -> Result<VpsSourceConsumeJournalV1> {
    let bytes = read_pinned_vps_source_file(parent_fd, name, MAX_DOCUMENT_BYTES)?;
    let journal: VpsSourceConsumeJournalV1 = strict_json_from_slice(&bytes)?;
    ensure!(
        canonical_json_bytes(&journal)? == bytes,
        "VPS source consume journal is not byte-for-byte canonical JSON"
    );
    Ok(journal)
}

pub(super) struct PinnedVpsSourceDocument {
    pub(super) file: File,
    pub(super) metadata: rustix::fs::Stat,
    pub(super) bytes: Vec<u8>,
}

pub(super) fn read_pinned_vps_source_file(
    parent_fd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsString,
    limit: u64,
) -> Result<Vec<u8>> {
    Ok(pin_vps_source_document(parent_fd, name, limit, &[0o400])?.bytes)
}

pub(super) fn pin_vps_source_document(
    parent_fd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsString,
    limit: u64,
    allowed_modes: &[u32],
) -> Result<PinnedVpsSourceDocument> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::io::Read as _;
    use std::os::fd::AsFd as _;

    let fd = openat2(
        parent_fd.as_fd(),
        name,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let parent = rustix::fs::fstat(parent_fd)?;
    let metadata = rustix::fs::fstat(&fd)?;
    fd_policy::ensure_private_regular(
        metadata.st_mode,
        metadata.st_uid,
        metadata.st_nlink as u64,
        &format!("VPS source consume journal metadata is unsafe"),
    )?;
    ensure!(
        metadata.st_dev == parent.st_dev
            && allowed_modes.contains(&(metadata.st_mode & 0o777))
            && metadata.st_size >= 0
            && metadata.st_size as u64 <= limit,
        "VPS source consume journal metadata is unsafe"
    );
    let mut file = File::from(fd);
    let mut bytes = Vec::with_capacity(metadata.st_size as usize);
    file.read_to_end(&mut bytes)?;
    let observed = rustix::fs::fstat(&file)?;
    ensure!(
        bytes.len() as u64 == metadata.st_size as u64
            && observed.st_dev == metadata.st_dev
            && observed.st_ino == metadata.st_ino
            && observed.st_uid == metadata.st_uid
            && observed.st_nlink == metadata.st_nlink
            && observed.st_mode == metadata.st_mode
            && observed.st_size == metadata.st_size,
        "VPS source consume journal changed while read"
    );
    Ok(PinnedVpsSourceDocument {
        file,
        metadata,
        bytes,
    })
}

pub(super) fn same_vps_source_document(
    left: &PinnedVpsSourceDocument,
    right: &PinnedVpsSourceDocument,
) -> bool {
    left.metadata.st_dev == right.metadata.st_dev
        && left.metadata.st_ino == right.metadata.st_ino
        && left.metadata.st_uid == right.metadata.st_uid
        && left.metadata.st_nlink == right.metadata.st_nlink
        && left.metadata.st_mode == right.metadata.st_mode
        && left.metadata.st_size == right.metadata.st_size
        && left.bytes == right.bytes
}

pub(super) fn remove_pinned_vps_source_document<F>(
    parent_fd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsString,
    expected_bytes: Option<&[u8]>,
    allowed_modes: &[u32],
    ensure_authority: &F,
) -> Result<()>
where
    F: Fn() -> Result<()>,
{
    use rustix::fs::{AtFlags, RenameFlags, renameat_with, unlinkat};
    use std::os::fd::AsFd as _;

    let removing_name = std::ffi::OsString::from(format!("{}.removing", name.to_string_lossy()));
    let pinned = if pinned_entry_exists(parent_fd, &removing_name)? {
        ensure!(
            !pinned_entry_exists(parent_fd, name)?,
            "VPS source document and its removing name both exist"
        );
        pin_vps_source_document(parent_fd, &removing_name, MAX_DOCUMENT_BYTES, allowed_modes)?
    } else {
        let pinned = pin_vps_source_document(parent_fd, name, MAX_DOCUMENT_BYTES, allowed_modes)?;
        if let Some(expected) = expected_bytes {
            ensure!(
                pinned.bytes == expected,
                "VPS source document changed before guarded removal"
            );
        }
        ensure_authority()?;
        renameat_with(
            parent_fd.as_fd(),
            name,
            parent_fd.as_fd(),
            &removing_name,
            RenameFlags::NOREPLACE,
        )
        .or_else(|rename_error| -> Result<()> {
            let observed = pin_vps_source_document(
                parent_fd,
                &removing_name,
                MAX_DOCUMENT_BYTES,
                allowed_modes,
            );
            if observed
                .as_ref()
                .is_ok_and(|observed| same_vps_source_document(&pinned, observed))
                && !pinned_entry_exists(parent_fd, name)?
            {
                Ok(())
            } else {
                Err(rename_error.into())
            }
        })?;
        rustix::fs::fsync(parent_fd)?;
        let observed =
            pin_vps_source_document(parent_fd, &removing_name, MAX_DOCUMENT_BYTES, allowed_modes)?;
        ensure!(
            same_vps_source_document(&pinned, &observed),
            "VPS source document changed during guarded removal"
        );
        pinned
    };
    if let Some(expected) = expected_bytes {
        ensure!(
            pinned.bytes == expected,
            "VPS source document removing name has unexpected bytes"
        );
    }
    let observed =
        pin_vps_source_document(parent_fd, &removing_name, MAX_DOCUMENT_BYTES, allowed_modes)?;
    ensure!(
        same_vps_source_document(&pinned, &observed),
        "VPS source document removing name was substituted"
    );
    ensure_authority()?;
    unlinkat(parent_fd.as_fd(), &removing_name, AtFlags::empty())?;
    ensure!(
        rustix::fs::fstat(&pinned.file)?.st_nlink == 0,
        "VPS source document inode remains linked after guarded removal"
    );
    rustix::fs::fsync(parent_fd)?;
    Ok(())
}

pub(super) fn restore_vps_source_document_removing<F>(
    parent_fd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsString,
    ensure_authority: &F,
) -> Result<()>
where
    F: Fn() -> Result<()>,
{
    use rustix::fs::{RenameFlags, renameat_with};
    use std::os::fd::AsFd as _;

    let removing_name = std::ffi::OsString::from(format!("{}.removing", name.to_string_lossy()));
    if !pinned_entry_exists(parent_fd, &removing_name)? {
        return Ok(());
    }
    ensure!(
        !pinned_entry_exists(parent_fd, name)?,
        "VPS source document and its removing name both exist"
    );
    let pinned = pin_vps_source_document(
        parent_fd,
        &removing_name,
        MAX_DOCUMENT_BYTES,
        &[0o400, 0o600],
    )?;
    ensure_authority()?;
    renameat_with(
        parent_fd.as_fd(),
        &removing_name,
        parent_fd.as_fd(),
        name,
        RenameFlags::NOREPLACE,
    )
    .or_else(|rename_error| -> Result<()> {
        let observed =
            pin_vps_source_document(parent_fd, name, MAX_DOCUMENT_BYTES, &[0o400, 0o600]);
        if observed
            .as_ref()
            .is_ok_and(|observed| same_vps_source_document(&pinned, observed))
            && !pinned_entry_exists(parent_fd, &removing_name)?
        {
            Ok(())
        } else {
            Err(rename_error.into())
        }
    })?;
    rustix::fs::fsync(parent_fd)?;
    let observed = pin_vps_source_document(parent_fd, name, MAX_DOCUMENT_BYTES, &[0o400, 0o600])?;
    ensure!(
        same_vps_source_document(&pinned, &observed),
        "VPS source document changed while recovering guarded removal"
    );
    Ok(())
}

pub(super) fn pinned_entry_exists(
    parent_fd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsString,
) -> Result<bool> {
    use rustix::fs::{AtFlags, statat};
    use std::os::fd::AsFd as _;

    match statat(parent_fd.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => Ok(true),
        Err(rustix::io::Errno::NOENT) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn reconcile_vps_source_journal_temporary<F>(
    parent_fd: &std::os::fd::OwnedFd,
    journal_name: &std::ffi::OsString,
    temporary_name: &std::ffi::OsString,
    source_name: &std::ffi::OsString,
    consuming_name: &std::ffi::OsString,
    expected_commit: &str,
    expected_phase: VpsSourceConsumePhaseV1,
    ensure_authority: &F,
) -> Result<()>
where
    F: Fn() -> Result<()>,
{
    use rustix::fs::{AtFlags, RenameFlags, renameat_with, statat};
    use std::os::fd::AsFd as _;

    let temporary = match statat(parent_fd.as_fd(), temporary_name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) => metadata,
        Err(rustix::io::Errno::NOENT) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    fd_policy::ensure_private_regular(
        temporary.st_mode,
        temporary.st_uid,
        temporary.st_nlink as u64,
        &format!("VPS source consume journal temporary is unsafe"),
    )?;
    let temporary_journal = load_vps_source_consume_journal(parent_fd, temporary_name);
    if let Ok(temporary_journal) = temporary_journal {
        ensure!(
            temporary_journal.source_commit == expected_commit
                && temporary_journal.phase == expected_phase,
            "VPS source consume journal temporary has the wrong commit or phase"
        );
        if pinned_entry_exists(parent_fd, journal_name)? {
            let current = load_vps_source_consume_journal(parent_fd, journal_name)?;
            ensure!(
                current == temporary_journal,
                "VPS source consume journal and temporary disagree"
            );
            remove_exact_vps_source_journal(
                parent_fd,
                temporary_name,
                &temporary_journal,
                ensure_authority,
            )?;
        } else {
            ensure_authority()?;
            renameat_with(
                parent_fd.as_fd(),
                temporary_name,
                parent_fd.as_fd(),
                journal_name,
                RenameFlags::NOREPLACE,
            )
            .or_else(|rename_error| {
                let installed = load_vps_source_consume_journal(parent_fd, journal_name);
                let temporary_absent = !pinned_entry_exists(parent_fd, temporary_name)?;
                if installed
                    .as_ref()
                    .is_ok_and(|journal| *journal == temporary_journal)
                    && temporary_absent
                {
                    Ok::<(), anyhow::Error>(())
                } else {
                    Err(rename_error.into())
                }
            })?;
        }
        rustix::fs::fsync(parent_fd)?;
        return Ok(());
    }
    let source_exists = pinned_entry_exists(parent_fd, source_name)?;
    let consuming_exists = pinned_entry_exists(parent_fd, consuming_name)?;
    let journal_exists = pinned_entry_exists(parent_fd, journal_name)?;
    let safe_invalid_prepared = expected_phase == VpsSourceConsumePhaseV1::Prepared
        && source_exists
        && !consuming_exists
        && !journal_exists;
    let safe_invalid_terminal = expected_phase == VpsSourceConsumePhaseV1::RootUnlinked
        && !source_exists
        && !consuming_exists
        && !journal_exists;
    ensure!(
        safe_invalid_prepared || safe_invalid_terminal,
        "invalid VPS source journal temporary coexists with mutated consumption state"
    );
    remove_pinned_vps_source_document(
        parent_fd,
        temporary_name,
        None,
        &[0o600, 0o400],
        ensure_authority,
    )?;
    Ok(())
}

pub(super) fn publish_vps_source_terminal_journal<F>(
    parent_fd: &std::os::fd::OwnedFd,
    terminal_name: &std::ffi::OsString,
    temporary_name: &std::ffi::OsString,
    prepared: &VpsSourceConsumeJournalV1,
    ensure_authority: &F,
) -> Result<VpsSourceConsumeJournalV1>
where
    F: Fn() -> Result<()>,
{
    use rustix::fs::{Mode, OFlags, RenameFlags, ResolveFlags, fchmod, openat2, renameat_with};
    use std::io::Write as _;
    use std::os::fd::AsFd as _;

    ensure!(
        prepared.phase == VpsSourceConsumePhaseV1::Prepared,
        "VPS source journal is not in prepared phase"
    );
    let mut journal = prepared.clone();
    journal.phase = VpsSourceConsumePhaseV1::RootUnlinked;
    let bytes = canonical_json_bytes(&journal)?;
    ensure!(
        bytes.len() as u64 <= MAX_DOCUMENT_BYTES,
        "terminal VPS source consume journal exceeds the reader bound"
    );
    let temporary_fd = openat2(
        parent_fd.as_fd(),
        temporary_name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let mut temporary = File::from(temporary_fd);
    temporary.write_all(&bytes)?;
    temporary.flush()?;
    fchmod(&temporary, Mode::from_raw_mode(0o400))?;
    temporary.sync_all()?;
    ensure_authority()?;
    renameat_with(
        parent_fd.as_fd(),
        temporary_name,
        parent_fd.as_fd(),
        terminal_name,
        RenameFlags::NOREPLACE,
    )
    .or_else(|rename_error| {
        let installed = load_vps_source_consume_journal(parent_fd, terminal_name);
        let temporary_absent = !pinned_entry_exists(parent_fd, temporary_name)?;
        if installed
            .as_ref()
            .is_ok_and(|observed| *observed == journal)
            && temporary_absent
        {
            Ok::<(), anyhow::Error>(())
        } else {
            Err(rename_error.into())
        }
    })?;
    rustix::fs::fsync(parent_fd)?;
    ensure!(
        load_vps_source_consume_journal(parent_fd, terminal_name)? == journal,
        "terminal VPS source journal changed after publication"
    );
    Ok(journal)
}

pub(super) fn publish_vps_source_consume_journal<F>(
    parent_fd: &std::os::fd::OwnedFd,
    journal_name: &std::ffi::OsString,
    temporary_name: &std::ffi::OsString,
    journal: &VpsSourceConsumeJournalV1,
    ensure_authority: &F,
) -> Result<()>
where
    F: Fn() -> Result<()>,
{
    use rustix::fs::{Mode, OFlags, RenameFlags, ResolveFlags, fchmod, openat2, renameat_with};
    use std::io::Write as _;
    use std::os::fd::AsFd as _;

    let bytes = canonical_json_bytes(journal)?;
    ensure!(
        bytes.len() as u64 <= MAX_DOCUMENT_BYTES,
        "VPS source consume journal exceeds the reader bound"
    );
    let temporary_fd = openat2(
        parent_fd.as_fd(),
        temporary_name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let mut temporary = File::from(temporary_fd);
    temporary.write_all(&bytes)?;
    temporary.flush()?;
    fchmod(&temporary, Mode::from_raw_mode(0o400))?;
    temporary.sync_all()?;
    ensure_authority()?;
    renameat_with(
        parent_fd.as_fd(),
        temporary_name,
        parent_fd.as_fd(),
        journal_name,
        RenameFlags::NOREPLACE,
    )?;
    rustix::fs::fsync(parent_fd)?;
    ensure!(
        load_vps_source_consume_journal(parent_fd, journal_name)? == *journal,
        "published VPS source consume journal changed"
    );
    Ok(())
}

pub(super) fn validate_vps_source_inventory_subset(
    source: &PinnedVpsSourceRoot,
    expected: &[VpsSourceConsumeEntryV1],
) -> Result<()> {
    let expected = expected
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let actual = inventory_pinned_vps_source_root(source)?;
    for entry in actual {
        let original = expected
            .get(entry.path.as_str())
            .with_context(|| format!("VPS consuming source has extra entry {}", entry.path))?;
        let cleanup_mode = entry.kind == VpsSourceEntryKindV1::Directory
            && entry.unix_mode == 0o700
            && original.kind == VpsSourceEntryKindV1::Directory;
        ensure!(
            entry.kind == original.kind
                && (entry.unix_mode == original.unix_mode || cleanup_mode)
                && entry.sha256 == original.sha256
                && entry.byte_length == original.byte_length,
            "VPS consuming source entry {} differs from its durable inventory",
            entry.path
        );
    }
    Ok(())
}

pub(super) fn remove_exact_vps_source_journal<F>(
    parent_fd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsString,
    expected: &VpsSourceConsumeJournalV1,
    ensure_authority: &F,
) -> Result<()>
where
    F: Fn() -> Result<()>,
{
    let bytes = canonical_json_bytes(expected)?;
    ensure!(
        bytes.len() as u64 <= MAX_DOCUMENT_BYTES
            && load_vps_source_consume_journal(parent_fd, name)? == *expected,
        "VPS source consume journal changed before removal"
    );
    remove_pinned_vps_source_document(parent_fd, name, Some(&bytes), &[0o400], ensure_authority)
}
