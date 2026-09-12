//! Owns the canonical activation lock open-file-description.
//!
//! The descriptor and inode fields are private to this module. Callers may
//! inherit the held authority, but cannot construct or retarget it. The two
//! constructors preserve distinct checks for fresh acquisition and inheritance
//! of the exact open-file-description already holding exclusion.

use super::{INSTALL_ROOT, InheritedFd, fd_policy};
use anyhow::{Context as _, Result, ensure};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
enum PinnedVpsActivationLockDescriptorV2 {
    Owned(std::os::fd::OwnedFd),
    InheritedDuplicate(InheritedFd),
}

impl PinnedVpsActivationLockDescriptorV2 {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        use std::os::fd::AsRawFd as _;

        match self {
            Self::Owned(fd) => fd.as_raw_fd(),
            Self::InheritedDuplicate(fd) => fd.0,
        }
    }
}

#[derive(Debug)]
pub struct PinnedVpsActivationLockV2 {
    lock_fd: PinnedVpsActivationLockDescriptorV2,
    opt_fd: std::os::fd::OwnedFd,
    opt_root: PathBuf,
    opt_device: u64,
    opt_inode: u64,
    lock_device: u64,
    lock_inode: u64,
}

impl PinnedVpsActivationLockV2 {
    pub fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.lock_fd.as_raw_fd()
    }

    pub fn clear_close_on_exec(&self) -> Result<()> {
        fd_policy::clear_close_on_exec(self.lock_fd.as_raw_fd())?;
        Ok(())
    }

    pub fn ensure_canonical(&self) -> Result<()> {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        use std::os::fd::AsFd as _;
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let path_metadata = fs::symlink_metadata(&self.opt_root)?;
        ensure!(
            path_metadata.is_dir()
                && !path_metadata.file_type().is_symlink()
                && fs::canonicalize(&self.opt_root)? == self.opt_root
                && path_metadata.uid() == rustix::process::geteuid().as_raw()
                && path_metadata.permissions().mode() & 0o777 == 0o750
                && path_metadata.dev() == self.opt_device
                && path_metadata.ino() == self.opt_inode,
            "canonical activation opt root changed while its lock was held"
        );
        let current_opt = openat2(
            rustix::fs::CWD,
            &self.opt_root,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let current_opt_metadata = rustix::fs::fstat(&current_opt)?;
        let pinned_opt_metadata = rustix::fs::fstat(&self.opt_fd)?;
        ensure!(
            current_opt_metadata.st_dev == self.opt_device
                && current_opt_metadata.st_ino == self.opt_inode
                && pinned_opt_metadata.st_dev == self.opt_device
                && pinned_opt_metadata.st_ino == self.opt_inode,
            "canonical activation opt root differs from its held descriptor"
        );
        let current_lock = openat2(
            current_opt.as_fd(),
            "activation.lock",
            OFlags::RDWR | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let current_lock_metadata = rustix::fs::fstat(&current_lock)?;
        let held_lock_metadata = fd_policy::stat(self.lock_fd.as_raw_fd())?;
        fd_policy::ensure_private_regular(
            current_lock_metadata.st_mode,
            current_lock_metadata.st_uid,
            current_lock_metadata.st_nlink as u64,
            &format!("canonical activation lock changed while its descriptor was held"),
        )?;
        ensure!(
            current_lock_metadata.st_mode & 0o777 == 0o600
                && current_lock_metadata.st_dev == self.lock_device
                && current_lock_metadata.st_ino == self.lock_inode
                && held_lock_metadata.st_dev == self.lock_device
                && held_lock_metadata.st_ino == self.lock_inode
                && held_lock_metadata.st_nlink == 1
                && held_lock_metadata.st_mode & 0o777 == 0o600,
            "canonical activation lock changed while its descriptor was held"
        );
        Ok(())
    }
}

pub fn acquire_vps_activation_lock_v2() -> Result<PinnedVpsActivationLockV2> {
    acquire_vps_activation_lock_at(Path::new(INSTALL_ROOT))
}

pub fn pin_inherited_vps_activation_lock_v2(
    activation_lock_fd: std::os::fd::RawFd,
) -> Result<PinnedVpsActivationLockV2> {
    pin_inherited_vps_activation_lock_at(Path::new(INSTALL_ROOT), activation_lock_fd)
}

pub(super) fn pin_inherited_vps_activation_lock_at(
    opt_root: &Path,
    activation_lock_fd: std::os::fd::RawFd,
) -> Result<PinnedVpsActivationLockV2> {
    use rustix::fs::{FlockOperation, Mode, OFlags, ResolveFlags, flock, openat2};
    use rustix::io::Errno;
    use std::os::fd::AsFd as _;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    ensure!(
        activation_lock_fd >= 3,
        "inherited activation lock descriptor must be at least 3"
    );
    let lock_fd = InheritedFd::duplicate(activation_lock_fd)?;
    // InheritedFd::duplicate sets CLOEXEC atomically.
    let opt_metadata = fs::symlink_metadata(opt_root)?;
    ensure!(
        opt_metadata.is_dir()
            && !opt_metadata.file_type().is_symlink()
            && fs::canonicalize(opt_root)? == opt_root
            && opt_metadata.uid() == rustix::process::geteuid().as_raw()
            && opt_metadata.permissions().mode() & 0o777 == 0o750,
        "activation opt root must be canonical, EUID-owned, and mode 0750"
    );
    let opt_fd = openat2(
        rustix::fs::CWD,
        opt_root,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let pinned_opt = rustix::fs::fstat(&opt_fd)?;
    ensure!(
        pinned_opt.st_dev == opt_metadata.dev() && pinned_opt.st_ino == opt_metadata.ino(),
        "activation opt root changed while inherited lock was pinned"
    );
    let inherited_metadata = fd_policy::stat(lock_fd.0)?;
    let canonical_lock = openat2(
        opt_fd.as_fd(),
        "activation.lock",
        OFlags::RDWR | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let canonical_metadata = rustix::fs::fstat(&canonical_lock)?;
    fd_policy::ensure_private_regular(
        inherited_metadata.st_mode,
        inherited_metadata.st_uid,
        inherited_metadata.st_nlink as u64,
        &format!("inherited activation lock is not the canonical owner-only lock inode"),
    )?;
    ensure!(
        inherited_metadata.st_mode & 0o777 == 0o600
            && inherited_metadata.st_dev == pinned_opt.st_dev
            && inherited_metadata.st_dev == canonical_metadata.st_dev
            && inherited_metadata.st_ino == canonical_metadata.st_ino,
        "inherited activation lock is not the canonical owner-only lock inode"
    );
    match flock(&canonical_lock, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => {
            flock(&canonical_lock, FlockOperation::Unlock)?;
            anyhow::bail!(
                "inherited activation lock descriptor does not already own the exclusive lock"
            );
        }
        Err(Errno::WOULDBLOCK) => {}
        Err(error) => return Err(error.into()),
    }
    fd_policy::assert_inherited_exclusive_lock(lock_fd.0)
        .context("inherited activation lock is not the open file description holding exclusion")?;
    let pinned = PinnedVpsActivationLockV2 {
        lock_fd: PinnedVpsActivationLockDescriptorV2::InheritedDuplicate(lock_fd),
        opt_fd,
        opt_root: opt_root.to_path_buf(),
        opt_device: pinned_opt.st_dev,
        opt_inode: pinned_opt.st_ino,
        lock_device: inherited_metadata.st_dev,
        lock_inode: inherited_metadata.st_ino,
    };
    pinned.ensure_canonical()?;
    Ok(pinned)
}

pub(super) fn acquire_vps_activation_lock_at(opt_root: &Path) -> Result<PinnedVpsActivationLockV2> {
    use rustix::fs::{FlockOperation, Mode, OFlags, ResolveFlags, flock, openat2};
    use rustix::io::Errno;
    use std::os::fd::{AsFd as _, AsRawFd as _};
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let opt_metadata = fs::symlink_metadata(opt_root)?;
    ensure!(
        opt_metadata.is_dir()
            && !opt_metadata.file_type().is_symlink()
            && fs::canonicalize(opt_root)? == opt_root
            && opt_metadata.uid() == rustix::process::geteuid().as_raw()
            && opt_metadata.permissions().mode() & 0o777 == 0o750,
        "activation opt root must be canonical, EUID-owned, and mode 0750"
    );
    let opt_fd = openat2(
        rustix::fs::CWD,
        opt_root,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let pinned_opt = rustix::fs::fstat(&opt_fd)?;
    ensure!(
        pinned_opt.st_dev == opt_metadata.dev() && pinned_opt.st_ino == opt_metadata.ino(),
        "activation opt root changed while being pinned"
    );
    let lock_name = Path::new("activation.lock");
    let (lock_fd, created) = match openat2(
        opt_fd.as_fd(),
        lock_name,
        OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    ) {
        Ok(fd) => (fd, true),
        Err(Errno::EXIST) => (
            openat2(
                opt_fd.as_fd(),
                lock_name,
                OFlags::RDWR | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
            )?,
            false,
        ),
        Err(error) => return Err(error.into()),
    };
    let lock_metadata = rustix::fs::fstat(&lock_fd)?;
    let lock_std_metadata = fs::metadata(format!("/proc/self/fd/{}", lock_fd.as_raw_fd()))?;
    ensure!(
        lock_std_metadata.is_file()
            && lock_metadata.st_uid == rustix::process::geteuid().as_raw()
            && lock_std_metadata.nlink() == 1
            && lock_std_metadata.permissions().mode() & 0o777 == 0o600
            && lock_metadata.st_dev == pinned_opt.st_dev,
        "activation lock has unsafe owner, type, links, mode, or device"
    );
    let observed_lock = openat2(
        opt_fd.as_fd(),
        lock_name,
        OFlags::RDWR | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let observed = rustix::fs::fstat(&observed_lock)?;
    ensure!(
        observed.st_dev == lock_metadata.st_dev && observed.st_ino == lock_metadata.st_ino,
        "activation lock path changed after pinning"
    );
    if created {
        rustix::fs::fsync(&opt_fd)?;
    }
    flock(&lock_fd, FlockOperation::NonBlockingLockExclusive)
        .context("another VPS activation holds the shared lock")?;
    let final_observed = rustix::fs::fstat(&openat2(
        opt_fd.as_fd(),
        lock_name,
        OFlags::RDWR | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?)?;
    ensure!(
        final_observed.st_dev == lock_metadata.st_dev
            && final_observed.st_ino == lock_metadata.st_ino,
        "activation lock path changed after locking"
    );
    Ok(PinnedVpsActivationLockV2 {
        lock_fd: PinnedVpsActivationLockDescriptorV2::Owned(lock_fd),
        opt_fd,
        opt_root: opt_root.to_path_buf(),
        opt_device: pinned_opt.st_dev,
        opt_inode: pinned_opt.st_ino,
        lock_device: lock_metadata.st_dev,
        lock_inode: lock_metadata.st_ino,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    #[test]
    fn inherited_authority_keeps_exclusion_after_original_owner_drops() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o750))?;
        let held = acquire_vps_activation_lock_at(root.path())?;
        let inherited = pin_inherited_vps_activation_lock_at(root.path(), held.as_raw_fd())?;
        assert!(acquire_vps_activation_lock_at(root.path()).is_err());
        drop(held);
        inherited.ensure_canonical()?;
        assert!(acquire_vps_activation_lock_at(root.path()).is_err());
        drop(inherited);
        acquire_vps_activation_lock_at(root.path())?.ensure_canonical()?;
        Ok(())
    }
}
