//! Pinned filesystem capabilities and tracked physical mutation primitives.

use anyhow::Context as _;
use robin_highscores::backup::BackupFileV4 as BackupFile;
use robin_highscores::backup::parse_backup_id;
use sha2::Digest as _;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io::Read as _;
use std::io::Write as _;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::path::PathBuf;

#[cfg(target_os = "linux")]
pub(super) fn duplicate_inherited_fd(fd: u32, directory: bool) -> anyhow::Result<std::fs::File> {
    anyhow::ensure!(fd >= 3, "inherited verifier descriptor must be at least 3");
    let path = PathBuf::from(format!("/proc/self/fd/{fd}"));
    let file = std::fs::File::open(&path)?;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        },
        "inherited verifier descriptor has the wrong file type"
    );
    Ok(file)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn duplicate_inherited_fd(_fd: u32, _directory: bool) -> anyhow::Result<std::fs::File> {
    anyhow::bail!("inherited-FD backup verification requires Linux procfs")
}

#[cfg(target_os = "linux")]
fn inherited_fd_target(fd: u32) -> anyhow::Result<PathBuf> {
    let target = std::fs::read_link(format!("/proc/self/fd/{fd}"))?;
    anyhow::ensure!(
        target.is_absolute() && !target.to_string_lossy().ends_with(" (deleted)"),
        "inherited verifier descriptor has no stable absolute name"
    );
    Ok(target)
}

#[cfg(not(target_os = "linux"))]
fn inherited_fd_target(_fd: u32) -> anyhow::Result<PathBuf> {
    anyhow::bail!("inherited-FD backup verification requires Linux procfs")
}

pub(super) fn pinned_file_target(file: &std::fs::File) -> anyhow::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd as _;
        let fd = u32::try_from(file.as_raw_fd())?;
        inherited_fd_target(fd)
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("inherited-FD backup verification requires Linux procfs")
}

pub(super) fn require_pinned_file_name(
    file: &std::fs::File,
    expected: &str,
) -> anyhow::Result<PathBuf> {
    let target = pinned_file_target(file)?;
    anyhow::ensure!(
        target.file_name().and_then(|name| name.to_str()) == Some(expected),
        "inherited verifier descriptor has the wrong canonical filename"
    );
    Ok(target)
}

pub(super) fn duplicate_pinned_file(
    file: &std::fs::File,
    directory: bool,
) -> anyhow::Result<std::fs::File> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd as _;
        duplicate_inherited_fd(u32::try_from(file.as_raw_fd())?, directory)
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("inherited-FD backup verification requires Linux procfs")
}

pub(super) fn revalidate_pinned_root_directory(
    pinned: &std::fs::File,
    expected_path: &Path,
    label: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        expected_path.is_absolute() && pinned_file_target(pinned)? == expected_path,
        "{label} is no longer linked at its canonical path"
    );
    let pinned_metadata = pinned.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(
            pinned_metadata.is_dir()
                && pinned_metadata.uid() == rustix::process::geteuid().as_raw()
                && pinned_metadata.permissions().mode() & 0o777 == 0o700,
            "{label} has the wrong owner or mode"
        );
    }
    let current = open_directory_nofollow(expected_path)?;
    anyhow::ensure!(
        metadata_identity_std(&current.metadata()?) == metadata_identity_std(&pinned_metadata),
        "{label} canonical path no longer names the pinned inode"
    );
    Ok(())
}

pub(super) fn revalidate_operation_lock_path(
    trusted_backup_root: &Path,
    held_lock: &std::fs::File,
) -> anyhow::Result<()> {
    let expected_path = trusted_backup_root.join(".backup-operation.lock");
    anyhow::ensure!(
        pinned_file_target(held_lock)? == expected_path,
        "backup operation lock is no longer linked at its canonical path"
    );
    validate_private_pinned_file(held_lock, 0o600, "backup operation lock")?;
    let root = pin_directory_capability(trusted_backup_root)?;
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt as _;
        let metadata = root.dir_metadata()?;
        anyhow::ensure!(
            metadata_identity(&metadata).owner == rustix::process::geteuid().as_raw()
                && metadata.permissions().mode() & 0o777 == 0o700,
            "backup operation lock parent has the wrong owner or mode"
        );
    }
    let current = open_cap_regular_nofollow(&root, Path::new(".backup-operation.lock"))?;
    validate_private_pinned_file(&current, 0o600, "backup operation lock")?;
    anyhow::ensure!(
        metadata_identity_std(&current.metadata()?)
            == metadata_identity_std(&held_lock.metadata()?),
        "canonical backup operation lock path no longer names the held inode"
    );
    Ok(())
}

pub(super) fn revalidate_pinned_regular_path(
    pinned: &std::fs::File,
    expected_path: &Path,
    expected_mode: u32,
    expected_parent_mode: Option<u32>,
    label: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        pinned_file_target(pinned)? == expected_path,
        "{label} is no longer linked at its exact admitted path"
    );
    validate_private_pinned_file(pinned, expected_mode, label)?;
    let parent_path = expected_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{label} path has no parent"))?;
    let name = expected_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("{label} path has no filename"))?;
    let parent = pin_directory_capability(parent_path)?;
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt as _;
        let metadata = parent.dir_metadata()?;
        anyhow::ensure!(
            metadata_identity(&metadata).owner == rustix::process::geteuid().as_raw()
                && metadata_identity_std(&pinned.metadata()?).device
                    == metadata_identity(&metadata).device,
            "{label} or its parent has the wrong owner or device"
        );
        if let Some(expected_parent_mode) = expected_parent_mode {
            anyhow::ensure!(
                metadata.permissions().mode() & 0o777 == expected_parent_mode,
                "{label} parent has the wrong mode"
            );
        }
    }
    let current = open_cap_regular_nofollow(&parent, Path::new(name))?;
    validate_private_pinned_file(&current, expected_mode, label)?;
    anyhow::ensure!(
        metadata_identity_std(&current.metadata()?) == metadata_identity_std(&pinned.metadata()?),
        "{label} path no longer names the pinned inode"
    );
    Ok(())
}

pub(super) fn revalidate_pinned_directory_path(
    pinned: &std::fs::File,
    expected_path: &Path,
    trusted_parent_path: &Path,
    label: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        expected_path.parent() == Some(trusted_parent_path)
            && pinned_file_target(pinned)? == expected_path,
        "{label} is no longer linked at its exact admitted path"
    );
    let pinned_metadata = pinned.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(
            pinned_metadata.is_dir()
                && pinned_metadata.uid() == rustix::process::geteuid().as_raw()
                && pinned_metadata.permissions().mode() & 0o777 == 0o700,
            "{label} has the wrong owner or mode"
        );
    }
    let parent = pin_directory_capability(trusted_parent_path)?;
    let parent_metadata = parent.dir_metadata()?;
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt as _;
        anyhow::ensure!(
            metadata_identity(&parent_metadata).owner == rustix::process::geteuid().as_raw()
                && parent_metadata.permissions().mode() & 0o777 == 0o700
                && metadata_identity_std(&pinned_metadata).device
                    == metadata_identity(&parent_metadata).device,
            "{label} or its parent has the wrong owner, mode, or device"
        );
    }
    let name = expected_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("{label} path has no filename"))?;
    let current = open_cap_directory_nofollow(&parent, Path::new(name))?;
    anyhow::ensure!(
        metadata_identity(&current.dir_metadata()?) == metadata_identity_std(&pinned_metadata),
        "{label} path no longer names the pinned inode"
    );
    Ok(())
}

pub(super) fn validate_expected_digest(value: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && value.bytes().any(|byte| byte != b'0'),
        "expected backup manifest digest is not canonical nonzero SHA-256"
    );
    Ok(())
}

pub(super) fn validate_private_pinned_file(
    file: &std::fs::File,
    mode: u32,
    label: &str,
) -> anyhow::Result<()> {
    let metadata = file.metadata()?;
    anyhow::ensure!(metadata.is_file(), "{label} is not a regular file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(
            metadata.uid() == rustix::process::geteuid().as_raw()
                && metadata.permissions().mode() & 0o777 == mode
                && metadata.nlink() == 1,
            "{label} has the wrong owner, mode, or link count"
        );
    }
    Ok(())
}

pub(super) fn read_bounded_pinned_file(
    file: std::fs::File,
    maximum: u64,
) -> anyhow::Result<Vec<u8>> {
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= maximum,
        "pinned document is not a bounded regular file"
    );
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len())?);
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        u64::try_from(bytes.len())? <= maximum,
        "pinned document exceeds its byte limit"
    );
    Ok(bytes)
}

pub(super) fn acquire_backup_operation_lock(backup_root: &Path) -> anyhow::Result<std::fs::File> {
    let root = pin_directory_capability(backup_root)?;
    let name = Path::new(".backup-operation.lock");
    #[cfg(target_os = "linux")]
    let (file, created) = {
        use std::os::fd::AsFd as _;
        let create = robin_highscores::secure_fs::open_no_symlinks_at(
            root.as_fd(),
            name,
            rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            rustix::fs::ResolveFlags::BENEATH | rustix::fs::ResolveFlags::NO_XDEV,
        );
        match create {
            Ok(descriptor) => (std::fs::File::from(descriptor), true),
            Err(rustix::io::Errno::EXIST) => {
                let descriptor = robin_highscores::secure_fs::open_no_symlinks_at(
                    root.as_fd(),
                    name,
                    rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                    rustix::fs::ResolveFlags::BENEATH | rustix::fs::ResolveFlags::NO_XDEV,
                )?;
                (std::fs::File::from(descriptor), false)
            }
            Err(error) => return Err(error.into()),
        }
    };
    #[cfg(not(target_os = "linux"))]
    let (file, created) = {
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        (root.open_with(name, &options)?.into_std(), false)
    };
    #[cfg(unix)]
    if created {
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.sync_all()?;
        sync_cap_directory(&root)?;
    }
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file(),
        "backup operation lock is not a regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let path_metadata = root.symlink_metadata(name)?;
        anyhow::ensure!(
            metadata.nlink() == 1
                && metadata.permissions().mode() & 0o777 == 0o600
                && metadata.uid() == rustix::process::geteuid().as_raw()
                && metadata_identity_std(&metadata) == metadata_identity(&path_metadata),
            "backup operation lock has the wrong owner, mode, link count, or path identity"
        );
    }
    fs2::FileExt::try_lock_exclusive(&file)
        .map_err(|error| anyhow::anyhow!("another backup operation is active: {error}"))?;
    Ok(file)
}

pub(super) fn cap_entry_exists(root: &cap_std::fs::Dir, relative: &Path) -> anyhow::Result<bool> {
    match root.symlink_metadata(relative) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn remove_pinned_regular_via_tombstone(
    parent: &cap_std::fs::Dir,
    source_name: &str,
    tombstone_name: &str,
    pinned: &std::fs::File,
    expected_identity: FileIdentity,
    expected_mode: u32,
    expected_bytes: &[u8],
    maximum_bytes: u64,
    label: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !cap_entry_exists(parent, Path::new(tombstone_name))?,
        "{label} deletion tombstone already exists"
    );
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::fd::AsFd as _;
        rustix::fs::renameat_with(
            parent.as_fd(),
            Path::new(source_name),
            parent.as_fd(),
            Path::new(tombstone_name),
            rustix::fs::RenameFlags::NOREPLACE,
        )?;
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    anyhow::bail!("pinned cleanup deletion requires Linux renameat2");
    sync_cap_directory(parent)?;
    let moved = open_cap_regular_nofollow(parent, Path::new(tombstone_name))?;
    validate_private_pinned_file(pinned, expected_mode, label)?;
    validate_private_pinned_file(&moved, expected_mode, label)?;
    anyhow::ensure!(
        metadata_identity_std(&moved.metadata()?) == expected_identity
            && metadata_identity_std(&pinned.metadata()?) == expected_identity
            && read_bounded_pinned_file(duplicate_pinned_file(&moved, false)?, maximum_bytes)?
                == expected_bytes,
        "{label} was substituted while moving to its deletion tombstone; the replacement was preserved"
    );
    unlink_pinned_regular(
        parent,
        tombstone_name,
        &moved,
        expected_identity,
        expected_mode,
        label,
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(
            pinned.metadata()?.nlink() == 0,
            "{label} original pinned inode remained linked after tombstone unlink"
        );
    }
    Ok(())
}

pub(super) fn unlink_pinned_regular(
    parent: &cap_std::fs::Dir,
    name: &str,
    pinned: &std::fs::File,
    expected_identity: FileIdentity,
    expected_mode: u32,
    label: &str,
) -> anyhow::Result<()> {
    unlink_pinned_regular_with_hook(
        parent,
        name,
        pinned,
        expected_identity,
        expected_mode,
        label,
        || Ok(()),
    )
}

pub(super) fn unlink_pinned_regular_with_hook<F>(
    parent: &cap_std::fs::Dir,
    name: &str,
    pinned: &std::fs::File,
    expected_identity: FileIdentity,
    expected_mode: u32,
    label: &str,
    before_unlink: F,
) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    validate_private_pinned_file(pinned, expected_mode, label)?;
    anyhow::ensure!(
        metadata_identity_std(&pinned.metadata()?) == expected_identity,
        "{label} pinned authority changed before removal"
    );
    before_unlink()?;
    let current = open_cap_regular_nofollow(parent, Path::new(name))?;
    validate_private_pinned_file(pinned, expected_mode, label)?;
    validate_private_pinned_file(&current, expected_mode, label)?;
    anyhow::ensure!(
        metadata_identity_std(&pinned.metadata()?) == expected_identity
            && metadata_identity_std(&current.metadata()?) == expected_identity,
        "{label} was substituted at its unlink boundary; the replacement was preserved"
    );
    parent.remove_file(name)?;
    sync_cap_directory(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(
            pinned.metadata()?.nlink() == 0 && current.metadata()?.nlink() == 0,
            "{label} pinned inode remained linked after unlink"
        );
    }
    anyhow::ensure!(
        !cap_entry_exists(parent, Path::new(name))?,
        "{label} pathname reappeared after unlink"
    );
    Ok(())
}

pub(super) fn cleanup_tombstone_relative(relative: &str) -> anyhow::Result<String> {
    let path = Path::new(relative);
    anyhow::ensure!(
        !relative.is_empty()
            && !path.is_absolute()
            && path
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
        "cleanup tombstone source path is unsafe"
    );
    let tombstone = format!(
        ".cleanup-unlink-v1-{}",
        hex::encode(Sha256::digest(relative.as_bytes()))
    );
    Ok(path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(
            || PathBuf::from(&tombstone),
            |parent| parent.join(&tombstone),
        )
        .to_string_lossy()
        .replace('\\', "/"))
}

pub(super) fn expected_backup_cleanup_file_mode(relative: &str) -> anyhow::Result<u32> {
    Ok(
        if relative == "backup-verification-envelope.json"
            || relative == cleanup_tombstone_relative("backup-verification-envelope.json")?
        {
            0o400
        } else {
            0o600
        },
    )
}

pub(super) fn valid_partial_backup_name(name: &str) -> bool {
    let Some(complete) = name
        .strip_prefix('.')
        .and_then(|name| name.strip_suffix(".partial"))
    else {
        return false;
    };
    parse_backup_id(complete).is_some()
}

pub(super) fn validate_managed_directory_tree(
    root: &cap_std::fs::Dir,
    backup_root_metadata: &cap_std::fs::Metadata,
) -> anyhow::Result<()> {
    validate_managed_metadata(&root.dir_metadata()?, backup_root_metadata, true)?;
    let mut pending = vec![(root.try_clone()?, 0_usize)];
    let mut validated_directories = Vec::new();
    let mut visited = 0_usize;
    while let Some((directory, depth)) = pending.pop() {
        validated_directories.push(directory.try_clone()?);
        anyhow::ensure!(depth <= 32, "managed partial backup is too deep");
        for entry in directory.entries()? {
            let entry = entry?;
            visited = visited
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("partial topology count overflows"))?;
            anyhow::ensure!(
                visited <= 1_000_000,
                "managed partial backup is too large to recover safely"
            );
            let name = entry.file_name();
            let metadata = directory.symlink_metadata(&name)?;
            anyhow::ensure!(
                !metadata.file_type().is_symlink() && (metadata.is_dir() || metadata.is_file()),
                "managed partial backup contains a link or special node"
            );
            validate_managed_metadata(&metadata, backup_root_metadata, metadata.is_dir())
                .with_context(|| {
                    format!("validate managed backup node {}", name.to_string_lossy())
                })?;
            if metadata.is_dir() {
                let child = open_cap_directory_nofollow(&directory, Path::new(&name))?;
                anyhow::ensure!(
                    metadata_identity(&child.dir_metadata()?) == metadata_identity(&metadata),
                    "managed partial backup directory was substituted while opening"
                );
                pending.push((child, depth + 1));
            } else {
                let child = open_cap_regular_nofollow(&directory, Path::new(&name))?;
                anyhow::ensure!(
                    metadata_identity_std(&child.metadata()?) == metadata_identity(&metadata),
                    "managed partial backup file was substituted while opening"
                );
            }
        }
    }
    #[cfg(unix)]
    for directory in validated_directories {
        directory
            .into_std_file()
            .set_permissions(std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FileIdentity {
    device: u64,
    inode: u64,
    owner: u32,
}

impl FileIdentity {
    /// Compare recorded journal data without letting consumers fabricate an
    /// observed filesystem identity. Journal authentication remains the caller's job.
    pub(super) fn matches_parts(self, device: u64, inode: u64, owner: u32) -> bool {
        self.device == device && self.inode == inode && self.owner == owner
    }

    pub(super) fn device(self) -> u64 {
        self.device
    }
    pub(super) fn inode(self) -> u64 {
        self.inode
    }
    pub(super) fn owner(self) -> u32 {
        self.owner
    }
}

pub(super) fn metadata_identity(metadata: &cap_std::fs::Metadata) -> FileIdentity {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt as _;
        FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
            owner: metadata.uid(),
        }
    }
    #[cfg(not(unix))]
    FileIdentity {
        device: 0,
        inode: 0,
        owner: 0,
    }
}

pub(super) fn metadata_identity_std(metadata: &std::fs::Metadata) -> FileIdentity {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
            owner: metadata.uid(),
        }
    }
    #[cfg(not(unix))]
    FileIdentity {
        device: 0,
        inode: 0,
        owner: 0,
    }
}

pub(super) fn validate_managed_metadata(
    metadata: &cap_std::fs::Metadata,
    backup_root_metadata: &cap_std::fs::Metadata,
    directory: bool,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        metadata_identity(metadata).owner == metadata_identity(backup_root_metadata).owner,
        "managed backup node has a foreign owner"
    );
    #[cfg(unix)]
    {
        use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            metadata.dev() == backup_root_metadata.dev()
                && metadata.permissions().mode() & 0o077 == 0
                && (directory || metadata.nlink() == 1),
            "managed backup node changed filesystem, privacy mode, or link count"
        );
    }
    Ok(())
}

pub(super) fn pin_directory_capability(path: &Path) -> anyhow::Result<cap_std::fs::Dir> {
    Ok(cap_std::fs::Dir::from_std_file(open_directory_nofollow(
        path,
    )?))
}

pub(super) fn open_cap_directory_nofollow(
    parent: &cap_std::fs::Dir,
    name: &Path,
) -> anyhow::Result<cap_std::fs::Dir> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsFd as _;
        let descriptor = robin_highscores::secure_fs::open_no_symlinks_at(
            parent.as_fd(),
            name,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::DIRECTORY,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::BENEATH | rustix::fs::ResolveFlags::NO_XDEV,
        )?;
        Ok(cap_std::fs::Dir::from_std_file(std::fs::File::from(
            descriptor,
        )))
    }
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::ensure!(
            !parent.symlink_metadata(name)?.file_type().is_symlink(),
            "managed directory is a symlink"
        );
        Ok(parent.open_dir(name)?)
    }
}

pub(super) fn open_cap_regular_nofollow(
    parent: &cap_std::fs::Dir,
    name: &Path,
) -> anyhow::Result<std::fs::File> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsFd as _;
        let descriptor = robin_highscores::secure_fs::open_no_symlinks_at(
            parent.as_fd(),
            name,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::BENEATH | rustix::fs::ResolveFlags::NO_XDEV,
        )?;
        let file = std::fs::File::from(descriptor);
        anyhow::ensure!(
            file.metadata()?.is_file(),
            "managed node is not a regular file"
        );
        Ok(file)
    }
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::ensure!(
            !parent.symlink_metadata(name)?.file_type().is_symlink(),
            "managed file is a symlink"
        );
        let file = parent.open(name)?.into_std();
        anyhow::ensure!(
            file.metadata()?.is_file(),
            "managed node is not a regular file"
        );
        Ok(file)
    }
}

pub(super) fn sync_cap_directory(directory: &cap_std::fs::Dir) -> anyhow::Result<()> {
    directory.try_clone()?.into_std_file().sync_all()?;
    Ok(())
}

pub(super) fn valid_complete_backup_name(name: &str) -> bool {
    parse_backup_id(name).is_some()
}

pub(super) fn read_cap_regular_bounded(
    root: &cap_std::fs::Dir,
    relative: &Path,
    maximum: u64,
) -> anyhow::Result<Vec<u8>> {
    read_cap_regular_bounded_with_mode(root, relative, maximum, 0o600)
}

pub(super) fn read_cap_regular_bounded_with_mode(
    root: &cap_std::fs::Dir,
    relative: &Path,
    maximum: u64,
    expected_mode: u32,
) -> anyhow::Result<Vec<u8>> {
    Ok(read_cap_regular_bounded_with_mode_and_identity(root, relative, maximum, expected_mode)?.0)
}

pub(super) fn read_cap_regular_bounded_with_mode_and_identity(
    root: &cap_std::fs::Dir,
    relative: &Path,
    maximum: u64,
    expected_mode: u32,
) -> anyhow::Result<(Vec<u8>, FileIdentity)> {
    let file = open_cap_regular_nofollow(root, relative)?;
    validate_private_pinned_file(&file, expected_mode, "pinned backup file")?;
    let identity = metadata_identity_std(&file.metadata()?);
    let bytes = read_bounded_pinned_file(duplicate_pinned_file(&file, false)?, maximum)?;
    anyhow::ensure!(
        metadata_identity_std(&file.metadata()?) == identity,
        "pinned backup file changed identity while reading"
    );
    Ok((bytes, identity))
}

pub(super) fn record_cap_file_with_identity(
    root: &cap_std::fs::Dir,
    relative: &Path,
) -> anyhow::Result<(BackupFile, FileIdentity)> {
    let mut file = open_cap_regular_nofollow(root, relative)?;
    validate_private_pinned_file(&file, 0o600, "pinned backup file")?;
    let metadata = file.metadata()?;
    let mut digest = Sha256::new();
    let mut byte_length = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        byte_length = byte_length
            .checked_add(u64::try_from(read)?)
            .ok_or_else(|| anyhow::anyhow!("pinned backup file length overflows"))?;
        digest.update(&buffer[..read]);
    }
    anyhow::ensure!(
        byte_length == metadata.len() && file.metadata()?.len() == metadata.len(),
        "pinned backup file changed length while hashing"
    );
    let file_identity = metadata_identity_std(&metadata);
    anyhow::ensure!(
        metadata_identity_std(&file.metadata()?) == file_identity,
        "pinned backup file changed identity while hashing"
    );
    Ok((
        BackupFile {
            relative_path: relative
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("pinned backup path is not UTF-8"))?
                .replace('\\', "/"),
            byte_length,
            sha256: hex::encode(digest.finalize()),
        },
        file_identity,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BackupTreePaths {
    files: BTreeSet<String>,
    directories: BTreeSet<String>,
    file_identities: BTreeMap<String, FileIdentity>,
    directory_identities: BTreeMap<String, FileIdentity>,
    root_identity: FileIdentity,
}

impl BackupTreePaths {
    pub(super) fn files(&self) -> &BTreeSet<String> {
        &self.files
    }
    pub(super) fn directories(&self) -> &BTreeSet<String> {
        &self.directories
    }
    pub(super) fn file_identities(&self) -> &BTreeMap<String, FileIdentity> {
        &self.file_identities
    }
    pub(super) fn directory_identities(&self) -> &BTreeMap<String, FileIdentity> {
        &self.directory_identities
    }
    pub(super) fn root_identity(&self) -> FileIdentity {
        self.root_identity
    }
}

pub(super) fn backup_tree_paths_cap(root: &cap_std::fs::Dir) -> anyhow::Result<BackupTreePaths> {
    let root_metadata = root.dir_metadata()?;
    let root_identity = metadata_identity(&root_metadata);
    let mut pending = vec![(root.try_clone()?, PathBuf::new(), 0_usize)];
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let mut file_identities = BTreeMap::new();
    let mut directory_identities = BTreeMap::new();
    let mut visited = 0_usize;
    while let Some((directory, prefix, depth)) = pending.pop() {
        anyhow::ensure!(
            depth <= robin_highscores::backup::MAX_BACKUP_TREE_DEPTH,
            "pinned backup tree is too deep"
        );
        for entry in directory.entries()? {
            let entry = entry?;
            visited = visited
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("pinned backup topology overflows"))?;
            anyhow::ensure!(
                visited <= robin_highscores::backup::MAX_BACKUP_MANIFEST_FILES + 2,
                "pinned backup topology exceeds its entry limit"
            );
            let name = entry.file_name();
            let name_text = name
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("pinned backup entry is not UTF-8"))?;
            anyhow::ensure!(
                name_text.len() <= robin_highscores::backup::MAX_BACKUP_COMPONENT_BYTES,
                "pinned backup component is too long"
            );
            let relative = prefix.join(name_text);
            anyhow::ensure!(
                relative.as_os_str().as_encoded_bytes().len()
                    <= robin_highscores::backup::MAX_BACKUP_RELATIVE_PATH_BYTES,
                "pinned backup path is too long"
            );
            let metadata = directory.symlink_metadata(&name)?;
            anyhow::ensure!(
                !metadata.file_type().is_symlink() && (metadata.is_dir() || metadata.is_file()),
                "pinned backup tree contains a link or special node"
            );
            validate_managed_metadata(&metadata, &root_metadata, metadata.is_dir())?;
            if metadata.is_dir() {
                #[cfg(unix)]
                {
                    use cap_std::fs::PermissionsExt as _;
                    anyhow::ensure!(
                        metadata.permissions().mode() & 0o777 == 0o700,
                        "pinned backup directory has mode {:04o}, not 0700: {}",
                        metadata.permissions().mode() & 0o777,
                        relative.display(),
                    );
                }
                let child = open_cap_directory_nofollow(&directory, Path::new(&name))?;
                anyhow::ensure!(
                    metadata_identity(&child.dir_metadata()?) == metadata_identity(&metadata),
                    "pinned backup directory was substituted while opening"
                );
                let relative_text = relative.to_string_lossy().replace('\\', "/");
                directories.insert(relative_text.clone());
                directory_identities.insert(relative_text, metadata_identity(&metadata));
                pending.push((child, relative, depth + 1));
            } else {
                let child = open_cap_regular_nofollow(&directory, Path::new(&name))?;
                anyhow::ensure!(
                    metadata_identity_std(&child.metadata()?) == metadata_identity(&metadata),
                    "pinned backup file was substituted while opening"
                );
                let relative = relative
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("pinned backup path is not UTF-8"))?
                    .replace('\\', "/");
                #[cfg(unix)]
                {
                    use cap_std::fs::PermissionsExt as _;
                    let expected_mode = expected_backup_cleanup_file_mode(&relative)?;
                    anyhow::ensure!(
                        metadata.permissions().mode() & 0o777 == expected_mode,
                        "pinned backup file has a noncanonical mode"
                    );
                }
                anyhow::ensure!(
                    files.insert(relative.clone()),
                    "pinned backup repeats a path"
                );
                file_identities.insert(relative, metadata_identity(&metadata));
            }
        }
    }
    Ok(BackupTreePaths {
        files,
        directories,
        file_identities,
        directory_identities,
        root_identity,
    })
}

pub(super) async fn record_file(root: &Path, path: &Path) -> anyhow::Result<BackupFile> {
    use tokio::io::AsyncReadExt as _;

    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut file = options.open(path).await?;
    let metadata = file.metadata().await?;
    anyhow::ensure!(metadata.is_file(), "backup source is not a regular file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(metadata.nlink() == 1, "backup source is multiply linked");
    }
    let mut digest = Sha256::new();
    let mut byte_length = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        byte_length = byte_length
            .checked_add(u64::try_from(read)?)
            .ok_or_else(|| anyhow::anyhow!("backup file length overflows"))?;
        digest.update(&buffer[..read]);
    }
    anyhow::ensure!(
        byte_length == metadata.len() && file.metadata().await?.len() == metadata.len(),
        "backup file changed length while hashing"
    );
    Ok(BackupFile {
        relative_path: path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/"),
        byte_length,
        sha256: hex::encode(digest.finalize()),
    })
}

pub(super) async fn read_bounded_regular_nofollow(
    path: &Path,
    maximum: u64,
) -> anyhow::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt as _;

    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let file = options.open(path).await?;
    let metadata = file.metadata().await?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= maximum,
        "bounded backup document is not a bounded regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(
            metadata.nlink() == 1,
            "bounded backup document is multiply linked"
        );
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len())?);
    file.take(maximum + 1).read_to_end(&mut bytes).await?;
    anyhow::ensure!(
        u64::try_from(bytes.len())? <= maximum,
        "bounded backup document exceeds its byte limit"
    );
    Ok(bytes)
}

pub(super) async fn copy_open_file(
    source: tokio::fs::File,
    destination: &Path,
) -> anyhow::Result<()> {
    let source = source.into_std().await;
    let destination = destination.to_owned();
    robin_highscores::physical_work::spawn_blocking(move || {
        copy_open_file_sync(source, &destination)
    })
    .await
    .context("backup copy task failed")?
}

pub(super) fn copy_open_file_sync(
    mut source: std::fs::File,
    destination: &Path,
) -> anyhow::Result<()> {
    if let Some(parent) = destination.parent() {
        let mut missing = Vec::new();
        let mut cursor = parent;
        while !cursor.try_exists()? {
            missing.push(cursor.to_owned());
            cursor = cursor
                .parent()
                .ok_or_else(|| anyhow::anyhow!("backup destination has no existing ancestor"))?;
        }
        std::fs::create_dir_all(parent)?;
        for directory in missing.into_iter().rev() {
            #[cfg(unix)]
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
            #[cfg(not(unix))]
            let _ = directory;
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut target = options.open(destination)?;
    std::io::copy(&mut source, &mut target)?;
    target.sync_all()?;
    Ok(())
}

pub(super) async fn write_private_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let path = path.to_owned();
    let bytes = bytes.to_vec();
    robin_highscores::physical_work::spawn_blocking(move || {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(path)?;
        file.write_all(&bytes)?;
        file.sync_all()
    })
    .await
    .context("backup write task failed")??;
    Ok(())
}

pub(super) async fn create_backup_directory(path: &Path) -> anyhow::Result<()> {
    let path = path.to_owned();
    robin_highscores::physical_work::spawn_blocking(move || std::fs::create_dir(path))
        .await
        .context("backup directory creation task failed")??;
    Ok(())
}

#[cfg(unix)]
pub(super) async fn set_backup_permissions(
    path: &Path,
    permissions: std::fs::Permissions,
) -> anyhow::Result<()> {
    let path = path.to_owned();
    robin_highscores::physical_work::spawn_blocking(move || {
        std::fs::set_permissions(path, permissions)
    })
    .await
    .context("backup permission task failed")??;
    Ok(())
}

pub(super) async fn set_private_directory(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        set_backup_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
    }
    Ok(())
}

pub(super) async fn sync_directory(path: &Path) -> anyhow::Result<()> {
    let path = path.to_owned();
    robin_highscores::physical_work::spawn_blocking(move || std::fs::File::open(path)?.sync_all())
        .await
        .context("backup directory sync task failed")??;
    Ok(())
}

pub(super) fn open_directory_nofollow(path: &Path) -> anyhow::Result<std::fs::File> {
    #[cfg(target_os = "linux")]
    {
        let descriptor = robin_highscores::secure_fs::open_no_symlinks_at(
            rustix::fs::CWD,
            path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::DIRECTORY,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::empty(),
        )?;
        let file = std::fs::File::from(descriptor);
        anyhow::ensure!(file.metadata()?.is_dir(), "path is not a directory");
        Ok(file)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
        let file = options.open(path)?;
        anyhow::ensure!(file.metadata()?.is_dir(), "path is not a directory");
        Ok(file)
    }
}

pub(super) fn open_regular_nofollow(path: &Path) -> anyhow::Result<std::fs::File> {
    #[cfg(target_os = "linux")]
    {
        let descriptor = robin_highscores::secure_fs::open_no_symlinks_at(
            rustix::fs::CWD,
            path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::empty(),
        )?;
        let file = std::fs::File::from(descriptor);
        anyhow::ensure!(file.metadata()?.is_file(), "path is not a regular file");
        Ok(file)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        let file = options.open(path)?;
        let metadata = file.metadata()?;
        anyhow::ensure!(metadata.is_file(), "path is not a regular file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            anyhow::ensure!(
                metadata.nlink() == 1,
                "verified backup file is multiply linked"
            );
        }
        Ok(file)
    }
}

#[cfg(test)]
mod tests;
