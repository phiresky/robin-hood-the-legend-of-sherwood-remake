//! Safe bridge for descriptors inherited as CLI integers, and shared ownership
//! policy. Do not replace duplication with opening `/proc/self/fd/N`: a fresh
//! open-file-description does not inherit the activation flock.
//!
//! `BorrowedFd::borrow_raw` and `OwnedFd::from_raw_fd` require unsafe, forbidden
//! by this crate. nix 0.29 is retained *only* for this safe raw-descriptor API;
//! already-owned descriptors use rustix. All duplicates acquire CLOEXEC atomically.
//!
//! TODO: nix 0.31 (the workspace version) no longer offers raw-descriptor
//! `fcntl`/`fstat`/`dup` (they take `AsFd`), so moving to it would require an
//! unsafe `BorrowedFd::borrow_raw`, which the crate-level
//! `#![forbid(unsafe_code)]` rules out. `pidfd_getfd(pidfd_open(getpid()), N)`
//! could produce a safe `OwnedFd` sharing the open-file-description for
//! dup/stat/flock, but cannot clear close-on-exec on the inherited descriptor
//! number itself (needed before exec'ing the activation script). Revisit once
//! the unsafe policy or the activation hand-off changes.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::os::fd::RawFd;

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub(super) struct InheritedFd(pub(super) RawFd);

impl<'de> Deserialize<'de> for InheritedFd {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "an inherited descriptor must be acquired from a live process, not deserialized",
        ))
    }
}

impl InheritedFd {
    pub(super) fn duplicate(fd: RawFd) -> Result<Self> {
        Ok(Self(nix_legacy::fcntl::fcntl(
            fd,
            nix_legacy::fcntl::FcntlArg::F_DUPFD_CLOEXEC(3),
        )?))
    }
}

impl Drop for InheritedFd {
    fn drop(&mut self) {
        // close must not retry after EINTR: the descriptor may already have
        // been released and reused by another thread.
        let _ = nix_legacy::unistd::close(self.0);
    }
}

pub(super) type RawStat = nix_legacy::sys::stat::FileStat;
pub(super) fn stat(fd: RawFd) -> Result<RawStat> {
    Ok(nix_legacy::sys::stat::fstat(fd)?)
}

pub(super) fn clear_close_on_exec(fd: RawFd) -> Result<()> {
    let bits = nix_legacy::fcntl::fcntl(fd, nix_legacy::fcntl::FcntlArg::F_GETFD)?;
    let mut flags = nix_legacy::fcntl::FdFlag::from_bits_truncate(bits);
    flags.remove(nix_legacy::fcntl::FdFlag::FD_CLOEXEC);
    nix_legacy::fcntl::fcntl(fd, nix_legacy::fcntl::FcntlArg::F_SETFD(flags))?;
    Ok(())
}

pub(super) fn assert_inherited_exclusive_lock(fd: RawFd) -> Result<()> {
    // This call intentionally operates on the inherited open-file-description.
    // `nix::fcntl::Flock` would need an owned `File`/`OwnedFd` (see module docs).
    #[allow(deprecated)]
    nix_legacy::fcntl::flock(fd, nix_legacy::fcntl::FlockArg::LockExclusiveNonblock)?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub(super) enum PrivateFileViolation {
    #[error("{label}: descriptor is not a regular file")]
    Type { label: String },
    #[error("{label}: descriptor is not a directory")]
    DirectoryType { label: String },
    #[error("{label}: descriptor owner {actual} differs from effective uid {expected}")]
    Owner {
        label: String,
        actual: u32,
        expected: u32,
    },
    #[error("{label}: descriptor has {actual} links, expected exactly one")]
    Links { label: String, actual: u64 },
    #[error("{label}: descriptor mode {actual:04o} is not one of the allowed modes {}", octal_modes(.allowed))]
    Mode {
        label: String,
        actual: u32,
        allowed: Vec<u32>,
    },
    #[error("{label}: descriptor size {actual} is outside the allowed range {minimum}..={maximum}")]
    Size {
        label: String,
        actual: i64,
        minimum: u64,
        maximum: u64,
    },
    #[error("{label}: descriptor device {actual} differs from expected device {expected}")]
    Device {
        label: String,
        actual: u64,
        expected: u64,
    },
}

fn octal_modes(modes: &[u32]) -> String {
    let modes: Vec<String> = modes.iter().map(|mode| format!("{mode:04o}")).collect();
    format!("[{}]", modes.join(", "))
}

fn ensure_owner(owner: u32, label: &str) -> Result<()> {
    let expected = rustix::process::geteuid().as_raw();
    if owner != expected {
        return Err(PrivateFileViolation::Owner {
            label: label.into(),
            actual: owner,
            expected,
        }
        .into());
    }
    Ok(())
}

/// A private regular file: regular type, owned by the effective uid, exactly
/// one link. Mode, size and device rules are checked separately by the caller
/// with [`ensure_mode`], [`ensure_size`] and [`ensure_device`].
pub(super) fn ensure_private_regular(mode: u32, owner: u32, links: u64, label: &str) -> Result<()> {
    if !rustix::fs::FileType::from_raw_mode(mode).is_file() {
        return Err(PrivateFileViolation::Type {
            label: label.into(),
        }
        .into());
    }
    ensure_owner(owner, label)?;
    ensure_single_link(links, label)
}

/// A private directory: directory type, owned by the effective uid, and
/// permission bits exactly `expected_mode`.
pub(super) fn ensure_private_directory(
    mode: u32,
    owner: u32,
    expected_mode: u32,
    label: &str,
) -> Result<()> {
    if !rustix::fs::FileType::from_raw_mode(mode).is_dir() {
        return Err(PrivateFileViolation::DirectoryType {
            label: label.into(),
        }
        .into());
    }
    ensure_owner(owner, label)?;
    ensure_mode(mode, &[expected_mode], label)
}

/// The entry has exactly one hard link.
pub(super) fn ensure_single_link(links: u64, label: &str) -> Result<()> {
    if links != 1 {
        return Err(PrivateFileViolation::Links {
            label: label.into(),
            actual: links,
        }
        .into());
    }
    Ok(())
}

/// The permission bits (`mode & 0o777`) are one of `allowed`.
pub(super) fn ensure_mode(mode: u32, allowed: &[u32], label: &str) -> Result<()> {
    let actual = mode & 0o777;
    if !allowed.contains(&actual) {
        return Err(PrivateFileViolation::Mode {
            label: label.into(),
            actual,
            allowed: allowed.to_vec(),
        }
        .into());
    }
    Ok(())
}

/// The size lies within `minimum..=maximum` (a negative size always fails).
pub(super) fn ensure_size(size: i64, minimum: u64, maximum: u64, label: &str) -> Result<()> {
    let within = u64::try_from(size).is_ok_and(|size| size >= minimum && size <= maximum);
    if !within {
        return Err(PrivateFileViolation::Size {
            label: label.into(),
            actual: size,
            minimum,
            maximum,
        }
        .into());
    }
    Ok(())
}

/// The entry lives on the `expected` device.
pub(super) fn ensure_device(actual: u64, expected: u64, label: &str) -> Result<()> {
    if actual != expected {
        return Err(PrivateFileViolation::Device {
            label: label.into(),
            actual,
            expected,
        }
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_policy_reports_the_failing_clause() {
        let owner = rustix::process::geteuid().as_raw();
        assert!(ensure_private_regular(0o100400, owner, 1, "fixture").is_ok());
        for (mode, uid, links, expected) in [
            (0o040400, owner, 1, "regular"),
            (0o100400, owner.wrapping_add(1), 1, "owner"),
            (0o100400, owner, 2, "links"),
        ] {
            let error = ensure_private_regular(mode, uid, links, "fixture").unwrap_err();
            assert!(error.downcast_ref::<PrivateFileViolation>().is_some());
            assert!(error.to_string().contains(expected));
        }
    }

    #[test]
    fn directory_mode_size_and_device_policy_report_the_failing_clause() {
        let owner = rustix::process::geteuid().as_raw();
        assert!(ensure_private_directory(0o040550, owner, 0o550, "fixture").is_ok());
        for (mode, uid, expected) in [
            (0o100550, owner, "directory"),
            (0o040550, owner.wrapping_add(1), "owner"),
            (0o040750, owner, "mode 0750"),
        ] {
            let error = ensure_private_directory(mode, uid, 0o550, "fixture").unwrap_err();
            assert!(error.downcast_ref::<PrivateFileViolation>().is_some());
            assert!(error.to_string().contains(expected), "{error}");
        }
        assert!(ensure_mode(0o100600, &[0o400, 0o600], "fixture").is_ok());
        assert!(ensure_size(0, 0, 0, "fixture").is_ok());
        assert!(ensure_size(-1, 0, 10, "fixture").is_err());
        assert!(ensure_size(0, 1, 10, "fixture").is_err());
        assert!(ensure_size(11, 1, 10, "fixture").is_err());
        assert!(
            ensure_device(1, 2, "fixture")
                .unwrap_err()
                .to_string()
                .contains("device")
        );
    }
}
