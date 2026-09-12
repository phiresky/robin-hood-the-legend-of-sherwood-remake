//! Safe bridge for descriptors inherited as CLI integers, and shared ownership
//! policy. Do not replace duplication with opening `/proc/self/fd/N`: a fresh
//! open-file-description does not inherit the activation flock.
//!
//! `BorrowedFd::borrow_raw` and `OwnedFd::from_raw_fd` require unsafe, forbidden
//! by this crate. nix 0.29 is retained *only* for this safe raw-descriptor API;
//! already-owned descriptors use rustix. All duplicates acquire CLOEXEC atomically.
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
    #[allow(deprecated)]
    nix_legacy::fcntl::flock(fd, nix_legacy::fcntl::FlockArg::LockExclusiveNonblock)?;
    Ok(())
}

#[derive(Debug, thiserror::Error, Serialize, Deserialize)]
pub(super) enum PrivateFileViolation {
    #[error("{label}: descriptor is not a regular file")]
    Type { label: String },
    #[error("{label}: descriptor owner {actual} differs from effective uid {expected}")]
    Owner {
        label: String,
        actual: u32,
        expected: u32,
    },
    #[error("{label}: descriptor has {actual} links, expected exactly one")]
    Links { label: String, actual: u64 },
}

pub(super) fn ensure_private_regular(mode: u32, owner: u32, links: u64, label: &str) -> Result<()> {
    if !rustix::fs::FileType::from_raw_mode(mode).is_file() {
        return Err(PrivateFileViolation::Type {
            label: label.into(),
        }
        .into());
    }
    let expected = rustix::process::geteuid().as_raw();
    if owner != expected {
        return Err(PrivateFileViolation::Owner {
            label: label.into(),
            actual: owner,
            expected,
        }
        .into());
    }
    if links != 1 {
        return Err(PrivateFileViolation::Links {
            label: label.into(),
            actual: links,
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
}
