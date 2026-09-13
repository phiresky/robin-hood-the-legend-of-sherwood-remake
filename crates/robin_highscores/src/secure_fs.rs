//! Descriptor-relative filesystem primitives for private object stores.
//!
//! The ambient path is used only once, while opening the root. Every later
//! lookup is relative to that pinned directory capability. File and directory
//! opens use `openat2(RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS)` (plus
//! `RESOLVE_BENEATH` for relative lookups); mutation uses cap-std's confined
//! `*at` operations. The crate is Linux-only (see the `compile_error!` in
//! `lib.rs`), so no portability fallbacks exist here.

use cap_std::fs::{Dir, OpenOptions};
use rustix::fs::{CWD, Mode, OFlags, ResolveFlags};
use rustix::path::Arg;
use std::io::ErrorKind;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

/// Reject symlinks without replacing the caller's descriptor identity checks.
///
/// Access, mode and additional confinement remain explicit: relative callers
/// specify BENEATH; mount-bound callers additionally specify NO_XDEV. Prefer
/// the narrower helpers below; this stays public for irregular combinations.
pub fn open_no_symlinks_at(
    directory: impl AsFd,
    path: impl Arg,
    flags: OFlags,
    mode: Mode,
    additional_resolution: ResolveFlags,
) -> rustix::io::Result<OwnedFd> {
    rustix::fs::openat2(
        directory,
        path,
        flags,
        mode,
        additional_resolution | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
}

/// Ambient (CWD-relative or absolute) open with `flags` and no extra resolution.
pub fn open_ambient_no_symlinks(path: impl Arg, flags: OFlags) -> rustix::io::Result<OwnedFd> {
    open_no_symlinks_at(CWD, path, flags, Mode::empty(), ResolveFlags::empty())
}

/// Ambient `O_RDONLY | O_CLOEXEC | O_DIRECTORY` open.
pub fn open_dir_no_symlinks(path: impl Arg) -> rustix::io::Result<OwnedFd> {
    open_ambient_no_symlinks(path, OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY)
}

/// Ambient `O_RDONLY | O_CLOEXEC` open. Performs no file-type check.
pub fn open_file_no_symlinks(path: impl Arg) -> rustix::io::Result<OwnedFd> {
    open_ambient_no_symlinks(path, OFlags::RDONLY | OFlags::CLOEXEC)
}

/// `RESOLVE_BENEATH` open relative to `directory` with `flags` and no mode.
pub fn open_beneath_no_symlinks(
    directory: impl AsFd,
    path: impl Arg,
    flags: OFlags,
) -> rustix::io::Result<OwnedFd> {
    open_no_symlinks_at(directory, path, flags, Mode::empty(), ResolveFlags::BENEATH)
}

/// `RESOLVE_BENEATH | RESOLVE_NO_XDEV` open relative to `directory` with no mode.
pub fn open_beneath_same_mount_no_symlinks(
    directory: impl AsFd,
    path: impl Arg,
    flags: OFlags,
) -> rustix::io::Result<OwnedFd> {
    open_no_symlinks_at(
        directory,
        path,
        flags,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_XDEV,
    )
}

/// `RESOLVE_BENEATH` open relative to `directory` with an explicit creation mode.
pub fn create_beneath_no_symlinks(
    directory: impl AsFd,
    path: impl Arg,
    flags: OFlags,
    mode: Mode,
) -> rustix::io::Result<OwnedFd> {
    open_no_symlinks_at(directory, path, flags, mode, ResolveFlags::BENEATH)
}

/// Open a regular ambient file without following any symlink component.
pub fn open_regular_no_symlinks(path: &Path) -> std::io::Result<std::fs::File> {
    let file = std::fs::File::from(open_file_no_symlinks(path)?);
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("path is not a regular file"));
    }
    Ok(file)
}

/// Bound both the initial metadata and a file that grows while being read.
pub fn read_bounded_regular_file(file: std::fs::File, limit: u64) -> anyhow::Result<Vec<u8>> {
    use std::io::Read as _;
    let metadata = file.metadata()?;
    anyhow::ensure!(metadata.is_file(), "input is not a regular file");
    anyhow::ensure!(
        metadata.len() <= limit,
        "input exceeds the {limit}-byte safety limit"
    );
    let read_limit = limit
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("input limit overflows"))?;
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len())?);
    file.take(read_limit).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        u64::try_from(bytes.len())? <= limit,
        "input grew beyond its safety limit"
    );
    Ok(bytes)
}

pub fn read_bounded_no_symlinks(path: &Path, limit: u64) -> anyhow::Result<Vec<u8>> {
    read_bounded_regular_file(open_regular_no_symlinks(path)?, limit)
}

/// Lock operations retain the same open-file-description and descriptor lifetime.
pub mod file_lock {
    use std::fs::File;
    use std::io;

    macro_rules! operation {
        ($name:ident, $operation:ident) => {
            pub fn $name(file: &File) -> io::Result<()> {
                rustix::fs::flock(file, rustix::fs::FlockOperation::$operation)
                    .map_err(io::Error::from)
            }
        };
    }
    operation!(lock_shared, LockShared);
    operation!(try_lock_shared, NonBlockingLockShared);
    operation!(try_lock_exclusive, NonBlockingLockExclusive);
    operation!(unlock, Unlock);
}

pub fn available_space(path: &Path) -> std::io::Result<u64> {
    let filesystem = rustix::fs::statvfs(path)?;
    filesystem
        .f_frsize
        .checked_mul(filesystem.f_bavail)
        .ok_or_else(|| std::io::Error::other("available filesystem space overflows"))
}

/// Shared API/worker state is private to the deployment's dedicated data
/// group. Setgid directories preserve that group on every descendant created
/// by either distinct service principal.
pub(crate) const SHARED_PRIVATE_DIRECTORY_MODE: u32 = 0o2770;
pub(crate) const SHARED_MUTABLE_FILE_MODE: u32 = 0o660;
pub(crate) const SHARED_IMMUTABLE_FILE_MODE: u32 = 0o440;

pub(crate) fn pin_private_root(path: &Path) -> std::io::Result<Dir> {
    let file = std::fs::File::from(open_dir_no_symlinks(path).map_err(std::io::Error::from)?);
    if !file.metadata()?.is_dir() {
        return Err(std::io::Error::other("storage root is not a directory"));
    }
    if file.metadata()?.permissions().mode() & 0o7777 != SHARED_PRIVATE_DIRECTORY_MODE {
        file.set_permissions(std::fs::Permissions::from_mode(
            SHARED_PRIVATE_DIRECTORY_MODE,
        ))?;
    }
    Ok(Dir::from_std_file(file))
}

pub(crate) fn open_private_dir(parent: &Dir, relative: &Path) -> std::io::Result<Dir> {
    let fd = open_beneath_no_symlinks(
        parent,
        relative,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
    )
    .map_err(std::io::Error::from)?;
    let file = std::fs::File::from(fd);
    if !file.metadata()?.is_dir() {
        return Err(std::io::Error::other("storage shard is not a directory"));
    }
    Ok(Dir::from_std_file(file))
}

pub(crate) fn ensure_private_dir(parent: &Dir, name: &Path) -> std::io::Result<Dir> {
    match parent.create_dir(name) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let directory = open_private_dir(parent, name)?;
    // Apply the mode through the already-open directory inode.
    let file = directory.try_clone()?.into_std_file();
    if file.metadata()?.permissions().mode() & 0o7777 != SHARED_PRIVATE_DIRECTORY_MODE {
        file.set_permissions(std::fs::Permissions::from_mode(
            SHARED_PRIVATE_DIRECTORY_MODE,
        ))?;
    }
    Ok(directory)
}

pub(crate) fn create_private_file(parent: &Dir, name: &Path) -> std::io::Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let file = parent.open_with(name, &options)?.into_std();
    file.set_permissions(std::fs::Permissions::from_mode(SHARED_MUTABLE_FILE_MODE))?;
    Ok(file)
}

pub(crate) fn open_regular_file(parent: &Dir, name: &Path) -> std::io::Result<std::fs::File> {
    let fd = open_beneath_no_symlinks(parent, name, OFlags::RDONLY | OFlags::CLOEXEC)
        .map_err(std::io::Error::from)?;
    let file = std::fs::File::from(fd);
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("stored object is not a regular file"));
    }
    Ok(file)
}

pub(crate) fn open_private_database_file(
    parent: &Dir,
    name: &Path,
    create: bool,
) -> std::io::Result<std::fs::File> {
    let mut flags = OFlags::RDWR | OFlags::CLOEXEC;
    if create {
        flags |= OFlags::CREATE;
    }
    let mode = if create {
        Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::WGRP
    } else {
        Mode::empty()
    };
    let fd = create_beneath_no_symlinks(parent, name, flags, mode).map_err(std::io::Error::from)?;
    let file = std::fs::File::from(fd);
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("database is not a regular file"));
    }
    if file.metadata()?.permissions().mode() & 0o777 != SHARED_MUTABLE_FILE_MODE {
        file.set_permissions(std::fs::Permissions::from_mode(SHARED_MUTABLE_FILE_MODE))?;
    }
    Ok(file)
}

pub(crate) fn sync_private_dir(directory: &Dir) -> std::io::Result<()> {
    directory.try_clone()?.into_std_file().sync_all()
}

/// Exercise write/fsync/unlink through the pinned capability. Cleanup runs
/// even when syncing the probe fails; domain stores keep their own identities.
pub(crate) async fn probe_writable_root(root: std::sync::Arc<Dir>) -> std::io::Result<()> {
    crate::physical_work::spawn_blocking(move || {
        let name = format!(".ready-{}.tmp", uuid::Uuid::now_v7());
        let file = create_private_file(&root, Path::new(&name))?;
        let synced = file.sync_all();
        drop(file);
        let cleaned = root
            .remove_file(&name)
            .and_then(|()| sync_private_dir(&root));
        synced.and(cleaned)
    })
    .await
    .map_err(std::io::Error::other)?
}

/// Publish a fully synced temporary object without replacing an existing one.
/// An existing object must still be opened and checked by the domain store.
pub(crate) async fn link_immutable_object(
    directory: std::sync::Arc<Dir>,
    temporary: String,
    destination: String,
) -> std::io::Result<bool> {
    crate::physical_work::spawn_blocking(move || {
        match directory.hard_link(&temporary, &directory, &destination) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error),
        }
    })
    .await
    .map_err(std::io::Error::other)?
}

pub(crate) async fn remove_temporary_and_sync(
    directory: std::sync::Arc<Dir>,
    name: String,
) -> std::io::Result<()> {
    crate::physical_work::spawn_blocking(move || {
        directory.remove_file(&name)?;
        sync_private_dir(&directory)
    })
    .await
    .map_err(std::io::Error::other)?
}

#[cfg(test)]
mod object_tests {
    use super::*;

    #[test]
    fn bounded_authority_rejects_links_directories_and_oversized_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("authority");
        std::fs::write(&path, b"exact").unwrap();
        assert_eq!(read_bounded_no_symlinks(&path, 5).unwrap(), b"exact");
        assert!(read_bounded_no_symlinks(&path, 4).is_err());
        assert!(read_bounded_no_symlinks(root.path(), 5).is_err());
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(read_bounded_no_symlinks(&alias, 5).is_err());
        let parent_alias = root.path().join("parent-alias");
        std::os::unix::fs::symlink(root.path(), &parent_alias).unwrap();
        assert!(read_bounded_no_symlinks(&parent_alias.join("authority"), 5).is_err());
    }

    #[test]
    fn relative_authority_retains_beneath_and_pinned_directory_semantics() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("pinned");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("authority"), b"original").unwrap();
        let pinned = std::fs::File::open(&directory).unwrap();
        std::fs::rename(&directory, root.path().join("moved")).unwrap();
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("authority"), b"replacement").unwrap();
        let open = |path: &str| {
            open_no_symlinks_at(
                &pinned,
                path,
                OFlags::RDONLY | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::BENEATH | ResolveFlags::NO_XDEV,
            )
        };
        let file = std::fs::File::from(open("authority").unwrap());
        assert_eq!(read_bounded_regular_file(file, 8).unwrap(), b"original");
        assert!(open("../pinned/authority").is_err());
    }
    use std::{io::Write as _, sync::Arc};

    #[tokio::test]
    async fn publication_never_replaces_existing_object_and_cleans_temporary() {
        let temp = tempfile::tempdir().unwrap();
        let root = Arc::new(pin_private_root(temp.path()).unwrap());
        for (name, bytes) in [
            ("first", b"first".as_slice()),
            ("second", b"second".as_slice()),
        ] {
            let mut file = create_private_file(&root, Path::new(name)).unwrap();
            file.write_all(bytes).unwrap();
            file.sync_all().unwrap();
        }
        assert!(
            link_immutable_object(Arc::clone(&root), "first".into(), "object".into())
                .await
                .unwrap()
        );
        assert!(
            !link_immutable_object(Arc::clone(&root), "second".into(), "object".into())
                .await
                .unwrap()
        );
        assert_eq!(root.read("object").unwrap(), b"first");
        remove_temporary_and_sync(Arc::clone(&root), "second".into())
            .await
            .unwrap();
        assert!(root.metadata("second").is_err());
        assert_eq!(root.read("object").unwrap(), b"first");
    }

    #[tokio::test]
    async fn readiness_probe_uses_pinned_directory_and_leaves_no_files() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("store");
        std::fs::create_dir(&original).unwrap();
        let root = Arc::new(pin_private_root(&original).unwrap());
        std::fs::rename(&original, temp.path().join("moved")).unwrap();
        std::fs::create_dir(&original).unwrap();
        probe_writable_root(Arc::clone(&root)).await.unwrap();
        assert_eq!(root.entries().unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(original).unwrap().count(), 0);
    }
}
