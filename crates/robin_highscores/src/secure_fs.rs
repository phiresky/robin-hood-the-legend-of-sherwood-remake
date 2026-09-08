//! Descriptor-relative filesystem primitives for private object stores.
//!
//! The ambient path is used only once, while opening the root. Every later
//! lookup is relative to that pinned directory capability. Linux additionally
//! uses `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS)` for file and directory
//! opens; mutation uses cap-std's confined `*at` operations.

use cap_std::fs::{Dir, OpenOptions};
use std::io::ErrorKind;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

/// Shared API/worker state is private to the deployment's dedicated data
/// group. Setgid directories preserve that group on every descendant created
/// by either distinct service principal.
pub(crate) const SHARED_PRIVATE_DIRECTORY_MODE: u32 = 0o2770;
pub(crate) const SHARED_MUTABLE_FILE_MODE: u32 = 0o660;
pub(crate) const SHARED_IMMUTABLE_FILE_MODE: u32 = 0o440;

pub(crate) fn pin_private_root(path: &Path) -> std::io::Result<Dir> {
    #[cfg(target_os = "linux")]
    let file = {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        let fd = openat2(
            rustix::fs::CWD,
            path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(std::io::Error::from)?;
        std::fs::File::from(fd)
    };
    #[cfg(not(target_os = "linux"))]
    let file =
        cap_std::fs::Dir::open_ambient_dir(path, cap_std::ambient_authority())?.into_std_file();
    if !file.metadata()?.is_dir() {
        return Err(std::io::Error::other("storage root is not a directory"));
    }
    #[cfg(unix)]
    if file.metadata()?.permissions().mode() & 0o7777 != SHARED_PRIVATE_DIRECTORY_MODE {
        file.set_permissions(std::fs::Permissions::from_mode(
            SHARED_PRIVATE_DIRECTORY_MODE,
        ))?;
    }
    Ok(Dir::from_std_file(file))
}

pub(crate) fn open_private_dir(parent: &Dir, relative: &Path) -> std::io::Result<Dir> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        use std::os::fd::AsFd as _;
        let fd = openat2(
            parent.as_fd(),
            relative,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(std::io::Error::from)?;
        let file = std::fs::File::from(fd);
        if !file.metadata()?.is_dir() {
            return Err(std::io::Error::other("storage shard is not a directory"));
        }
        Ok(Dir::from_std_file(file))
    }
    #[cfg(not(target_os = "linux"))]
    parent.open_dir(relative)
}

pub(crate) fn ensure_private_dir(parent: &Dir, name: &Path) -> std::io::Result<Dir> {
    match parent.create_dir(name) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let directory = open_private_dir(parent, name)?;
    // Apply the mode through the already-open directory inode.
    #[cfg(unix)]
    {
        let file = directory.try_clone()?.into_std_file();
        if file.metadata()?.permissions().mode() & 0o7777 != SHARED_PRIVATE_DIRECTORY_MODE {
            file.set_permissions(std::fs::Permissions::from_mode(
                SHARED_PRIVATE_DIRECTORY_MODE,
            ))?;
        }
    }
    Ok(directory)
}

pub(crate) fn create_private_file(parent: &Dir, name: &Path) -> std::io::Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let file = parent.open_with(name, &options)?.into_std();
    #[cfg(unix)]
    file.set_permissions(std::fs::Permissions::from_mode(SHARED_MUTABLE_FILE_MODE))?;
    Ok(file)
}

pub(crate) fn open_regular_file(parent: &Dir, name: &Path) -> std::io::Result<std::fs::File> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        use std::os::fd::AsFd as _;
        let fd = openat2(
            parent.as_fd(),
            name,
            OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(std::io::Error::from)?;
        let file = std::fs::File::from(fd);
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other("stored object is not a regular file"));
        }
        Ok(file)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let file = parent.open(name)?.into_std();
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other("stored object is not a regular file"));
        }
        Ok(file)
    }
}

pub(crate) fn open_private_database_file(
    parent: &Dir,
    name: &Path,
    create: bool,
) -> std::io::Result<std::fs::File> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        use std::os::fd::AsFd as _;
        let mut flags = OFlags::RDWR | OFlags::CLOEXEC;
        if create {
            flags |= OFlags::CREATE;
        }
        let mode = if create {
            Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::WGRP
        } else {
            Mode::empty()
        };
        let fd = openat2(
            parent.as_fd(),
            name,
            flags,
            mode,
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(std::io::Error::from)?;
        let file = std::fs::File::from(fd);
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other("database is not a regular file"));
        }
        if file.metadata()?.permissions().mode() & 0o777 != SHARED_MUTABLE_FILE_MODE {
            file.set_permissions(std::fs::Permissions::from_mode(SHARED_MUTABLE_FILE_MODE))?;
        }
        Ok(file)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(create);
        let file = parent.open_with(name, &options)?.into_std();
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other("database is not a regular file"));
        }
        #[cfg(unix)]
        if file.metadata()?.permissions().mode() & 0o777 != SHARED_MUTABLE_FILE_MODE {
            file.set_permissions(std::fs::Permissions::from_mode(SHARED_MUTABLE_FILE_MODE))?;
        }
        Ok(file)
    }
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
