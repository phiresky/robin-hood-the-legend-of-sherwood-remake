//! Transaction-bound backup HMAC key activation and exact-descriptor recovery.

use super::policy::DEFAULT_BACKUP_AUTHORITY_KEY;
use super::policy::VPS_ACTIVATION_ROOT;
use anyhow::Context as _;
use robin_run_protocol::canonical_json_bytes;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;
use sha2::Sha256;
use std::io::Read as _;
use std::io::Write as _;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::path::PathBuf;

const BACKUP_AUTHORITY_INTENT_SCHEMA_VERSION: u32 = 1;

const BACKUP_AUTHORITY_INTENT_NAME: &str = ".backup-authority-hmac-key.intent-v1.json";

const BACKUP_AUTHORITY_INTENT_TEMP_NAME: &str = ".backup-authority-hmac-key.intent-v1.json.new";

const MAX_BACKUP_AUTHORITY_INTENT_BYTES: u64 = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupAuthorityKeyIntentV1 {
    schema_version: u32,
    source_commit: String,
    activation_lock_device: u64,
    activation_lock_inode: u64,
    secret_parent_device: u64,
    secret_parent_inode: u64,
    key_device: u64,
    key_inode: u64,
    key_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackupAuthorityKeyBoundary {
    AnonymousKeySynced,
    IntentTemporaryCreated,
    IntentTemporaryWritten,
    IntentTemporarySynced,
    IntentPublished,
    KeyLinked,
    KeyDirectorySynced,
    KeyValidated,
    RecoveryTemporaryRemoved,
    RecoveryIntentRemoved,
    OuterAuthorityValidated,
    CompletionIntentRemoved,
    CompletionDirectorySynced,
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct OwnedRawFd(i32);

#[cfg(target_os = "linux")]
impl Drop for OwnedRawFd {
    fn drop(&mut self) {
        let _ = nix_legacy::unistd::close(self.0);
    }
}

#[cfg(target_os = "linux")]
struct PinnedActivationLock {
    inherited: OwnedRawFd,
    opt_root: PathBuf,
    opt_device: u64,
    opt_inode: u64,
    lock_device: u64,
    lock_inode: u64,
}

#[cfg(target_os = "linux")]
impl PinnedActivationLock {
    fn ensure_canonical(&self) -> anyhow::Result<()> {
        use rustix::fs::{FlockOperation, Mode, OFlags, ResolveFlags, flock, openat2};
        use rustix::io::Errno;
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let root_metadata = std::fs::symlink_metadata(&self.opt_root)?;
        anyhow::ensure!(
            root_metadata.is_dir()
                && !root_metadata.file_type().is_symlink()
                && std::fs::canonicalize(&self.opt_root)? == self.opt_root
                && root_metadata.uid() == rustix::process::geteuid().as_raw()
                && root_metadata.permissions().mode() & 0o777 == 0o750
                && root_metadata.dev() == self.opt_device
                && root_metadata.ino() == self.opt_inode,
            "activation opt root changed while the fifth-key transaction was active"
        );
        let root = openat2(
            rustix::fs::CWD,
            &self.opt_root,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let canonical = openat2(
            &root,
            "activation.lock",
            OFlags::RDWR | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let canonical_metadata = rustix::fs::fstat(&canonical)?;
        let inherited_metadata = nix_legacy::sys::stat::fstat(self.inherited.0)?;
        anyhow::ensure!(
            rustix::fs::FileType::from_raw_mode(canonical_metadata.st_mode).is_file()
                && canonical_metadata.st_uid == rustix::process::geteuid().as_raw()
                && canonical_metadata.st_nlink == 1
                && canonical_metadata.st_mode & 0o777 == 0o600
                && canonical_metadata.st_size == 0
                && canonical_metadata.st_dev == self.opt_device
                && canonical_metadata.st_dev == self.lock_device
                && canonical_metadata.st_ino == self.lock_inode
                && inherited_metadata.st_dev == self.lock_device
                && inherited_metadata.st_ino == self.lock_inode
                && inherited_metadata.st_uid == canonical_metadata.st_uid
                && inherited_metadata.st_nlink == 1
                && inherited_metadata.st_mode & 0o777 == 0o600
                && inherited_metadata.st_size == 0,
            "inherited activation lock is no longer the canonical owner-only lock inode"
        );
        match flock(&canonical, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => {
                flock(&canonical, FlockOperation::Unlock)?;
                anyhow::bail!("inherited activation lock no longer holds exclusion");
            }
            Err(Errno::WOULDBLOCK) => {}
            Err(error) => return Err(error.into()),
        }
        #[allow(deprecated)]
        nix_legacy::fcntl::flock(
            self.inherited.0,
            nix_legacy::fcntl::FlockArg::LockExclusiveNonblock,
        )
        .context("inherited activation lock is not the open file description holding exclusion")?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn pin_activation_lock_at(
    opt_root: &Path,
    activation_lock_fd: u32,
) -> anyhow::Result<PinnedActivationLock> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    anyhow::ensure!(
        activation_lock_fd >= 3,
        "inherited activation lock descriptor must be at least 3"
    );
    let raw = i32::try_from(activation_lock_fd)?;
    let inherited = OwnedRawFd(nix_legacy::unistd::dup(raw)?);
    nix_legacy::fcntl::fcntl(
        inherited.0,
        nix_legacy::fcntl::FcntlArg::F_SETFD(nix_legacy::fcntl::FdFlag::FD_CLOEXEC),
    )?;
    let root_metadata = std::fs::symlink_metadata(opt_root)?;
    anyhow::ensure!(
        root_metadata.is_dir()
            && !root_metadata.file_type().is_symlink()
            && std::fs::canonicalize(opt_root)? == opt_root
            && root_metadata.uid() == rustix::process::geteuid().as_raw()
            && root_metadata.permissions().mode() & 0o777 == 0o750,
        "activation opt root must be canonical, EUID-owned, and mode 0750"
    );
    let root = openat2(
        rustix::fs::CWD,
        opt_root,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let pinned_root = rustix::fs::fstat(&root)?;
    anyhow::ensure!(
        pinned_root.st_dev == root_metadata.dev() && pinned_root.st_ino == root_metadata.ino(),
        "activation opt root changed while it was pinned"
    );
    let canonical = openat2(
        &root,
        "activation.lock",
        OFlags::RDWR | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let canonical_metadata = rustix::fs::fstat(&canonical)?;
    let inherited_metadata = nix_legacy::sys::stat::fstat(inherited.0)?;
    anyhow::ensure!(
        rustix::fs::FileType::from_raw_mode(inherited_metadata.st_mode).is_file()
            && inherited_metadata.st_uid == rustix::process::geteuid().as_raw()
            && inherited_metadata.st_nlink == 1
            && inherited_metadata.st_mode & 0o777 == 0o600
            && inherited_metadata.st_size == 0
            && inherited_metadata.st_dev == pinned_root.st_dev
            && inherited_metadata.st_dev == canonical_metadata.st_dev
            && inherited_metadata.st_ino == canonical_metadata.st_ino,
        "inherited activation lock is not the canonical owner-only lock inode"
    );
    let pinned = PinnedActivationLock {
        inherited,
        opt_root: opt_root.to_path_buf(),
        opt_device: pinned_root.st_dev,
        opt_inode: pinned_root.st_ino,
        lock_device: inherited_metadata.st_dev,
        lock_inode: inherited_metadata.st_ino,
    };
    pinned.ensure_canonical()?;
    Ok(pinned)
}

#[cfg(target_os = "linux")]
struct PinnedSecretParent {
    fd: std::os::fd::OwnedFd,
    path: PathBuf,
    device: u64,
    inode: u64,
    uid: u32,
}

#[cfg(target_os = "linux")]
impl PinnedSecretParent {
    fn ensure_canonical(&self) -> anyhow::Result<()> {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let metadata = std::fs::symlink_metadata(&self.path)?;
        anyhow::ensure!(
            metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && std::fs::canonicalize(&self.path)? == self.path
                && metadata.uid() == self.uid
                && metadata.permissions().mode() & 0o777 == 0o700
                && metadata.dev() == self.device
                && metadata.ino() == self.inode,
            "backup-authority secret parent changed during initialization"
        );
        let reopened = openat2(
            rustix::fs::CWD,
            &self.path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let reopened = rustix::fs::fstat(&reopened)?;
        anyhow::ensure!(
            reopened.st_dev == self.device && reopened.st_ino == self.inode,
            "backup-authority secret parent path was replaced"
        );
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn pin_secret_parent(key_path: &Path, expected_uid: u32) -> anyhow::Result<PinnedSecretParent> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    anyhow::ensure!(
        key_path.is_absolute(),
        "backup-authority key path is not absolute"
    );
    let parent_path = key_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("backup-authority key has no parent"))?;
    anyhow::ensure!(
        key_path.file_name().and_then(|name| name.to_str()) == Some("backup-authority-hmac.key"),
        "backup-authority key has a non-canonical filename"
    );
    let metadata = std::fs::symlink_metadata(parent_path)?;
    anyhow::ensure!(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && std::fs::canonicalize(parent_path)? == parent_path
            && metadata.uid() == expected_uid
            && metadata.permissions().mode() & 0o777 == 0o700,
        "backup-authority secret parent must be canonical, expected-user-owned, and mode 0700"
    );
    let fd = openat2(
        rustix::fs::CWD,
        parent_path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let pinned = rustix::fs::fstat(&fd)?;
    anyhow::ensure!(
        pinned.st_dev == metadata.dev() && pinned.st_ino == metadata.ino(),
        "backup-authority secret parent changed while it was pinned"
    );
    Ok(PinnedSecretParent {
        fd,
        path: parent_path.to_path_buf(),
        device: pinned.st_dev,
        inode: pinned.st_ino,
        uid: expected_uid,
    })
}

#[cfg(target_os = "linux")]
fn named_entry_exists(parent: &PinnedSecretParent, name: &str) -> anyhow::Result<bool> {
    use rustix::fs::{AtFlags, statat};
    match statat(&parent.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => Ok(true),
        Err(rustix::io::Errno::NOENT) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct PinnedKeyIdentity {
    device: u64,
    inode: u64,
    sha256: String,
}

#[cfg(target_os = "linux")]
fn load_published_backup_authority_key(
    parent: &PinnedSecretParent,
) -> anyhow::Result<PinnedKeyIdentity> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags, openat2, statat};
    use std::os::fd::AsFd as _;

    let name = "backup-authority-hmac.key";
    let named = statat(parent.fd.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
    anyhow::ensure!(
        FileType::from_raw_mode(named.st_mode).is_file()
            && named.st_uid == parent.uid
            && named.st_dev == parent.device
            && named.st_nlink == 1
            && named.st_size == 32
            && named.st_mode & 0o777 == 0o400,
        "backup-authority key has unsafe type, owner, device, links, size, or mode"
    );
    let fd = openat2(
        parent.fd.as_fd(),
        name,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let mut file = std::fs::File::from(fd);
    let opened = file.metadata()?;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    anyhow::ensure!(
        opened.is_file()
            && opened.dev() == named.st_dev
            && opened.ino() == named.st_ino
            && opened.uid() == parent.uid
            && opened.nlink() == 1
            && opened.len() == 32
            && opened.permissions().mode() & 0o777 == 0o400,
        "backup-authority key changed while it was pinned"
    );
    let mut key = [0_u8; 32];
    file.read_exact(&mut key)?;
    let mut trailing = [0_u8; 1];
    anyhow::ensure!(
        file.read(&mut trailing)? == 0 && key != [0; 32],
        "backup-authority key is not an exact nonzero 32-byte key"
    );
    let observed = statat(parent.fd.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
    anyhow::ensure!(
        observed.st_dev == named.st_dev
            && observed.st_ino == named.st_ino
            && observed.st_uid == named.st_uid
            && observed.st_nlink == 1
            && observed.st_size == 32
            && observed.st_mode & 0o777 == 0o400,
        "backup-authority key path changed while it was read"
    );
    Ok(PinnedKeyIdentity {
        device: named.st_dev,
        inode: named.st_ino,
        sha256: hex::encode(Sha256::digest(key)),
    })
}

#[cfg(target_os = "linux")]
struct PinnedIntent {
    document: BackupAuthorityKeyIntentV1,
    bytes: Vec<u8>,
    device: u64,
    inode: u64,
}

#[cfg(target_os = "linux")]
fn load_backup_authority_intent(
    parent: &PinnedSecretParent,
    name: &str,
) -> anyhow::Result<PinnedIntent> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags, openat2, statat};
    use std::os::fd::AsFd as _;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let named = statat(parent.fd.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
    anyhow::ensure!(
        FileType::from_raw_mode(named.st_mode).is_file()
            && named.st_uid == parent.uid
            && named.st_dev == parent.device
            && named.st_nlink == 1
            && (1..=MAX_BACKUP_AUTHORITY_INTENT_BYTES as i64).contains(&named.st_size)
            && named.st_mode & 0o777 == 0o400,
        "backup-authority intent has unsafe type, owner, device, links, size, or mode"
    );
    let fd = openat2(
        parent.fd.as_fd(),
        name,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let file = std::fs::File::from(fd);
    let opened = file.metadata()?;
    anyhow::ensure!(
        opened.dev() == named.st_dev
            && opened.ino() == named.st_ino
            && opened.uid() == parent.uid
            && opened.nlink() == 1
            && opened.permissions().mode() & 0o777 == 0o400
            && opened.len() == u64::try_from(named.st_size)?,
        "backup-authority intent changed while it was pinned"
    );
    let mut bytes = Vec::with_capacity(usize::try_from(opened.len())?);
    file.take(MAX_BACKUP_AUTHORITY_INTENT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        u64::try_from(bytes.len())? == opened.len(),
        "backup-authority intent changed length while it was read"
    );
    let document: BackupAuthorityKeyIntentV1 = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        canonical_json_bytes(&document)? == bytes,
        "backup-authority intent is not exact canonical JSON"
    );
    let observed = statat(parent.fd.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
    anyhow::ensure!(
        observed.st_dev == named.st_dev
            && observed.st_ino == named.st_ino
            && observed.st_uid == named.st_uid
            && observed.st_nlink == 1
            && observed.st_size == named.st_size
            && observed.st_mode & 0o777 == 0o400,
        "backup-authority intent path changed while it was read"
    );
    Ok(PinnedIntent {
        document,
        bytes,
        device: named.st_dev,
        inode: named.st_ino,
    })
}

#[cfg(target_os = "linux")]
fn validate_backup_authority_intent(
    intent: &BackupAuthorityKeyIntentV1,
    source_commit: &str,
    parent: &PinnedSecretParent,
    activation_lock: &PinnedActivationLock,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        intent.schema_version == BACKUP_AUTHORITY_INTENT_SCHEMA_VERSION
            && intent.source_commit == source_commit
            && intent.activation_lock_device == activation_lock.lock_device
            && intent.activation_lock_inode == activation_lock.lock_inode
            && intent.secret_parent_device == parent.device
            && intent.secret_parent_inode == parent.inode
            && intent.key_device == parent.device
            && intent.key_inode != 0
            && intent.key_sha256.len() == 64
            && intent
                .key_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && intent.key_sha256.bytes().any(|byte| byte != b'0'),
        "backup-authority intent is stale or does not bind this exact transaction"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_key_against_intent(
    key: &PinnedKeyIdentity,
    intent: &BackupAuthorityKeyIntentV1,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        key.device == intent.key_device
            && key.inode == intent.key_inode
            && key.sha256 == intent.key_sha256,
        "published backup-authority key differs from its durable intent"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn remove_exact_named_entry(
    parent: &PinnedSecretParent,
    name: &str,
    expected_device: u64,
    expected_inode: u64,
) -> anyhow::Result<()> {
    use rustix::fs::{AtFlags, Mode, OFlags, ResolveFlags, openat2, statat, unlinkat};
    use std::os::fd::AsFd as _;

    let pinned = openat2(
        parent.fd.as_fd(),
        name,
        OFlags::PATH | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let pinned_metadata = rustix::fs::fstat(&pinned)?;
    let named = statat(parent.fd.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
    anyhow::ensure!(
        pinned_metadata.st_dev == expected_device
            && pinned_metadata.st_ino == expected_inode
            && named.st_dev == expected_device
            && named.st_ino == expected_inode,
        "backup-authority recovery entry changed before removal"
    );
    unlinkat(parent.fd.as_fd(), name, AtFlags::empty())?;
    anyhow::ensure!(
        rustix::fs::fstat(&pinned)?.st_nlink == 0,
        "backup-authority recovery inode remained linked after removal"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn reconcile_intent_temporary<H>(
    parent: &PinnedSecretParent,
    intent_exists: bool,
    key_exists: bool,
    hook: &mut H,
) -> anyhow::Result<()>
where
    H: FnMut(BackupAuthorityKeyBoundary) -> anyhow::Result<()>,
{
    use rustix::fs::{AtFlags, FileType, statat};
    use std::os::fd::AsFd as _;

    if !named_entry_exists(parent, BACKUP_AUTHORITY_INTENT_TEMP_NAME)? {
        return Ok(());
    }
    anyhow::ensure!(
        !intent_exists && !key_exists,
        "backup-authority intent temporary coexists with published authority evidence"
    );
    let temporary = statat(
        parent.fd.as_fd(),
        BACKUP_AUTHORITY_INTENT_TEMP_NAME,
        AtFlags::SYMLINK_NOFOLLOW,
    )?;
    anyhow::ensure!(
        FileType::from_raw_mode(temporary.st_mode).is_file()
            && temporary.st_uid == parent.uid
            && temporary.st_dev == parent.device
            && temporary.st_nlink == 1
            && temporary.st_size >= 0
            && temporary.st_size <= MAX_BACKUP_AUTHORITY_INTENT_BYTES as i64
            && matches!(temporary.st_mode & 0o777, 0o000 | 0o200 | 0o400 | 0o600),
        "backup-authority intent temporary has unsafe type, owner, device, links, size, or mode"
    );
    remove_exact_named_entry(
        parent,
        BACKUP_AUTHORITY_INTENT_TEMP_NAME,
        temporary.st_dev,
        temporary.st_ino,
    )?;
    rustix::fs::fsync(&parent.fd)?;
    hook(BackupAuthorityKeyBoundary::RecoveryTemporaryRemoved)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn valid_backup_authority_source_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(target_os = "linux")]
fn publish_anonymous_backup_authority_key_with<F>(
    anonymous: &std::fs::File,
    parent: &PinnedSecretParent,
    target: &str,
    proc_fd_root: &Path,
    direct_link: F,
) -> anyhow::Result<()>
where
    F: FnOnce() -> rustix::io::Result<()>,
{
    use rustix::fs::{AtFlags, Mode, OFlags, ResolveFlags, linkat, openat2};
    use std::os::fd::{AsFd as _, AsRawFd as _};

    let retained = rustix::fs::fstat(anonymous)?;
    match direct_link() {
        Ok(()) => {}
        Err(error) if error == rustix::io::Errno::NOENT => {
            let proc_path = proc_fd_root.join(anonymous.as_raw_fd().to_string());
            linkat(
                rustix::fs::CWD,
                &proc_path,
                parent.fd.as_fd(),
                target,
                AtFlags::SYMLINK_FOLLOW,
            )
            .with_context(|| {
                format!(
                    "publish anonymous backup-authority key through {}",
                    proc_path.display()
                )
            })?;
        }
        Err(error) => return Err(error).context("publish anonymous backup-authority key"),
    }

    let published = openat2(
        parent.fd.as_fd(),
        target,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .context("pin published backup-authority key")?;
    let observed = rustix::fs::fstat(&published)?;
    if observed.st_dev != retained.st_dev
        || observed.st_ino != retained.st_ino
        || observed.st_uid != retained.st_uid
        || observed.st_nlink != 1
        || observed.st_mode & 0o777 != retained.st_mode & 0o777
        || observed.st_size != retained.st_size
    {
        drop(published);
        remove_exact_named_entry(parent, target, observed.st_dev, observed.st_ino)?;
        rustix::fs::fsync(&parent.fd)?;
        anyhow::bail!("published backup-authority key is not the retained anonymous inode");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn initialize_backup_authority_key_v2_at<H>(
    opt_root: &Path,
    key_path: &Path,
    expected_uid: u32,
    source_commit: &str,
    activation_lock_fd: u32,
    mut hook: H,
) -> anyhow::Result<()>
where
    H: FnMut(BackupAuthorityKeyBoundary) -> anyhow::Result<()>,
{
    use rustix::fs::{
        AtFlags, Mode, OFlags, RenameFlags, ResolveFlags, fchmod, linkat, openat, openat2,
        renameat_with,
    };
    use std::os::fd::AsFd as _;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    anyhow::ensure!(
        valid_backup_authority_source_commit(source_commit),
        "backup-authority source commit is not exact lowercase 40-character hexadecimal"
    );
    let activation_lock = pin_activation_lock_at(opt_root, activation_lock_fd)
        .context("pin activation lock for backup-authority initialization")?;
    let parent =
        pin_secret_parent(key_path, expected_uid).context("pin backup-authority secret parent")?;
    activation_lock
        .ensure_canonical()
        .context("revalidate activation lock before backup-authority initialization")?;
    parent
        .ensure_canonical()
        .context("revalidate secret parent before backup-authority initialization")?;

    loop {
        let intent_exists = named_entry_exists(&parent, BACKUP_AUTHORITY_INTENT_NAME)?;
        let key_exists = named_entry_exists(&parent, "backup-authority-hmac.key")?;
        reconcile_intent_temporary(&parent, intent_exists, key_exists, &mut hook)?;

        if intent_exists {
            let intent = load_backup_authority_intent(&parent, BACKUP_AUTHORITY_INTENT_NAME)?;
            validate_backup_authority_intent(
                &intent.document,
                source_commit,
                &parent,
                &activation_lock,
            )?;
            if key_exists {
                let key = load_published_backup_authority_key(&parent)?;
                validate_key_against_intent(&key, &intent.document)?;
                activation_lock.ensure_canonical()?;
                parent.ensure_canonical()?;
                return Ok(());
            }
            activation_lock.ensure_canonical()?;
            remove_exact_named_entry(
                &parent,
                BACKUP_AUTHORITY_INTENT_NAME,
                intent.device,
                intent.inode,
            )?;
            rustix::fs::fsync(&parent.fd)?;
            hook(BackupAuthorityKeyBoundary::RecoveryIntentRemoved)?;
            continue;
        }
        anyhow::ensure!(
            !key_exists,
            "backup-authority key exists without its transaction intent"
        );
        break;
    }

    activation_lock.ensure_canonical()?;
    let anonymous = openat(
        parent.fd.as_fd(),
        ".",
        OFlags::RDWR | OFlags::CLOEXEC | OFlags::TMPFILE,
        Mode::from_raw_mode(0o400),
    )
    .context("create anonymous backup-authority key")?;
    fchmod(&anonymous, Mode::from_raw_mode(0o400))
        .context("chmod anonymous backup-authority key")?;
    let mut anonymous = std::fs::File::from(anonymous);
    let key = loop {
        let candidate: [u8; 32] = rand::random();
        if candidate != [0; 32] {
            break candidate;
        }
    };
    anonymous.write_all(&key)?;
    anonymous
        .sync_all()
        .context("sync anonymous backup-authority key")?;
    let key_metadata = anonymous.metadata()?;
    anyhow::ensure!(
        key_metadata.is_file()
            && key_metadata.dev() == parent.device
            && key_metadata.uid() == parent.uid
            && key_metadata.nlink() == 0
            && key_metadata.len() == 32
            && key_metadata.permissions().mode() & 0o777 == 0o400,
        "anonymous backup-authority key has unsafe identity, owner, links, size, or mode"
    );
    hook(BackupAuthorityKeyBoundary::AnonymousKeySynced)?;
    let intent = BackupAuthorityKeyIntentV1 {
        schema_version: BACKUP_AUTHORITY_INTENT_SCHEMA_VERSION,
        source_commit: source_commit.to_owned(),
        activation_lock_device: activation_lock.lock_device,
        activation_lock_inode: activation_lock.lock_inode,
        secret_parent_device: parent.device,
        secret_parent_inode: parent.inode,
        key_device: key_metadata.dev(),
        key_inode: key_metadata.ino(),
        key_sha256: hex::encode(Sha256::digest(key)),
    };
    validate_backup_authority_intent(&intent, source_commit, &parent, &activation_lock)?;
    let intent_bytes = canonical_json_bytes(&intent)?;
    anyhow::ensure!(
        u64::try_from(intent_bytes.len())? <= MAX_BACKUP_AUTHORITY_INTENT_BYTES,
        "backup-authority intent exceeds its byte limit"
    );
    let temporary = openat2(
        parent.fd.as_fd(),
        BACKUP_AUTHORITY_INTENT_TEMP_NAME,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o400),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .context("create backup-authority intent temporary")?;
    fchmod(&temporary, Mode::from_raw_mode(0o400))
        .context("chmod backup-authority intent temporary")?;
    let mut temporary = std::fs::File::from(temporary);
    hook(BackupAuthorityKeyBoundary::IntentTemporaryCreated)?;
    temporary.write_all(&intent_bytes)?;
    hook(BackupAuthorityKeyBoundary::IntentTemporaryWritten)?;
    temporary
        .sync_all()
        .context("sync backup-authority intent temporary")?;
    rustix::fs::fsync(&parent.fd)
        .context("sync secret parent before backup-authority intent publication")?;
    hook(BackupAuthorityKeyBoundary::IntentTemporarySynced)?;
    activation_lock.ensure_canonical()?;
    renameat_with(
        parent.fd.as_fd(),
        BACKUP_AUTHORITY_INTENT_TEMP_NAME,
        parent.fd.as_fd(),
        BACKUP_AUTHORITY_INTENT_NAME,
        RenameFlags::NOREPLACE,
    )
    .context("publish backup-authority intent")?;
    rustix::fs::fsync(&parent.fd)
        .context("sync secret parent after backup-authority intent publication")?;
    let published_intent = load_backup_authority_intent(&parent, BACKUP_AUTHORITY_INTENT_NAME)?;
    anyhow::ensure!(
        published_intent.document == intent && published_intent.bytes == intent_bytes,
        "published backup-authority intent differs from the retained transaction intent"
    );
    hook(BackupAuthorityKeyBoundary::IntentPublished)?;
    activation_lock.ensure_canonical()?;
    publish_anonymous_backup_authority_key_with(
        &anonymous,
        &parent,
        "backup-authority-hmac.key",
        Path::new("/proc/self/fd"),
        || {
            linkat(
                anonymous.as_fd(),
                "",
                parent.fd.as_fd(),
                "backup-authority-hmac.key",
                AtFlags::EMPTY_PATH,
            )
        },
    )?;
    hook(BackupAuthorityKeyBoundary::KeyLinked)?;
    rustix::fs::fsync(&parent.fd)
        .context("sync secret parent after backup-authority key publication")?;
    hook(BackupAuthorityKeyBoundary::KeyDirectorySynced)?;
    let published_key = load_published_backup_authority_key(&parent)?;
    validate_key_against_intent(&published_key, &intent)?;
    anyhow::ensure!(
        published_key.device == key_metadata.dev() && published_key.inode == key_metadata.ino(),
        "published backup-authority key is not the retained anonymous inode"
    );
    activation_lock.ensure_canonical()?;
    parent.ensure_canonical()?;
    hook(BackupAuthorityKeyBoundary::KeyValidated)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn complete_backup_authority_key_v2_at<V, H>(
    opt_root: &Path,
    key_path: &Path,
    expected_uid: u32,
    source_commit: &str,
    activation_lock_fd: u32,
    validate_outer_authority: V,
    mut hook: H,
) -> anyhow::Result<()>
where
    V: FnOnce() -> anyhow::Result<()>,
    H: FnMut(BackupAuthorityKeyBoundary) -> anyhow::Result<()>,
{
    anyhow::ensure!(
        valid_backup_authority_source_commit(source_commit),
        "backup-authority source commit is not exact lowercase 40-character hexadecimal"
    );
    let activation_lock = pin_activation_lock_at(opt_root, activation_lock_fd)?;
    let parent = pin_secret_parent(key_path, expected_uid)?;
    activation_lock.ensure_canonical()?;
    parent.ensure_canonical()?;
    anyhow::ensure!(
        !named_entry_exists(&parent, BACKUP_AUTHORITY_INTENT_TEMP_NAME)?,
        "backup-authority completion found an unresolved intent temporary"
    );
    let key = load_published_backup_authority_key(&parent)?;
    let intent = if named_entry_exists(&parent, BACKUP_AUTHORITY_INTENT_NAME)? {
        let intent = load_backup_authority_intent(&parent, BACKUP_AUTHORITY_INTENT_NAME)?;
        validate_backup_authority_intent(
            &intent.document,
            source_commit,
            &parent,
            &activation_lock,
        )?;
        validate_key_against_intent(&key, &intent.document)?;
        Some(intent)
    } else {
        None
    };
    validate_outer_authority()?;
    hook(BackupAuthorityKeyBoundary::OuterAuthorityValidated)?;
    activation_lock.ensure_canonical()?;
    parent.ensure_canonical()?;
    anyhow::ensure!(
        load_published_backup_authority_key(&parent)? == key,
        "backup-authority key changed during outer authority validation"
    );
    if let Some(intent) = intent {
        let reloaded = load_backup_authority_intent(&parent, BACKUP_AUTHORITY_INTENT_NAME)?;
        anyhow::ensure!(
            reloaded.device == intent.device
                && reloaded.inode == intent.inode
                && reloaded.bytes == intent.bytes,
            "backup-authority intent changed during outer authority validation"
        );
        remove_exact_named_entry(
            &parent,
            BACKUP_AUTHORITY_INTENT_NAME,
            intent.device,
            intent.inode,
        )?;
        hook(BackupAuthorityKeyBoundary::CompletionIntentRemoved)?;
        rustix::fs::fsync(&parent.fd)?;
        hook(BackupAuthorityKeyBoundary::CompletionDirectorySynced)?;
        anyhow::ensure!(
            !named_entry_exists(&parent, BACKUP_AUTHORITY_INTENT_NAME)?,
            "backup-authority intent remains after completion"
        );
    } else {
        // An already completed invocation has only the durable key. It is
        // accepted here, never by initialization, because the caller just
        // re-proved the entire production runtime authority as present.
        rustix::fs::fsync(&parent.fd)?;
    }
    activation_lock.ensure_canonical()?;
    parent.ensure_canonical()?;
    anyhow::ensure!(
        load_published_backup_authority_key(&parent)? == key,
        "backup-authority key changed during completion"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
pub(super) fn initialize_backup_authority_key_v2(
    source_commit: &str,
    activation_lock_fd: u32,
) -> anyhow::Result<()> {
    initialize_backup_authority_key_v2_at(
        Path::new(VPS_ACTIVATION_ROOT),
        Path::new(DEFAULT_BACKUP_AUTHORITY_KEY),
        rustix::process::geteuid().as_raw(),
        source_commit,
        activation_lock_fd,
        |_| Ok(()),
    )
}

#[cfg(not(target_os = "linux"))]
pub(super) fn initialize_backup_authority_key_v2(
    _source_commit: &str,
    _activation_lock_fd: u32,
) -> anyhow::Result<()> {
    anyhow::bail!("transactional backup-authority initialization requires Linux")
}

#[cfg(target_os = "linux")]
pub(super) fn complete_backup_authority_key_v2<V>(
    source_commit: &str,
    activation_lock_fd: u32,
    validate_outer_authority: V,
) -> anyhow::Result<()>
where
    V: FnOnce() -> anyhow::Result<()>,
{
    complete_backup_authority_key_v2_at(
        Path::new(VPS_ACTIVATION_ROOT),
        Path::new(DEFAULT_BACKUP_AUTHORITY_KEY),
        rustix::process::geteuid().as_raw(),
        source_commit,
        activation_lock_fd,
        validate_outer_authority,
        |_| Ok(()),
    )
}

#[cfg(not(target_os = "linux"))]
pub(super) fn complete_backup_authority_key_v2<V>(
    _source_commit: &str,
    _activation_lock_fd: u32,
    _validate_outer_authority: V,
) -> anyhow::Result<()>
where
    V: FnOnce() -> anyhow::Result<()>,
{
    anyhow::bail!("transactional backup-authority completion requires Linux")
}

#[cfg(test)]
mod tests;
