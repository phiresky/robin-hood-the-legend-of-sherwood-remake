//! Read-only, kernel-fenced verification of the live SQLite schema.
//!
//! The verifier never opens the live database through SQLite. It first owns
//! both runtime fence locks exclusively, pins the database and any WAL/SHM
//! sidecars, and copies the database plus WAL from those same descriptors into
//! a private directory. SQLite is allowed to recover and inspect only that
//! disposable copy. Descriptor hashes, metadata, and path bindings are checked
//! before and after the query so an uncooperative writer or path replacement
//! fails closed.

use crate::db::CURRENT_SCHEMA_VERSION;
use crate::db_fence::{ExclusiveAdmissionGuard, ExclusiveQuiescenceGuard, RuntimeDatabaseFence};
use anyhow::Context as _;
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection as _, Row as _};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::{FileExt as _, MetadataExt as _, PermissionsExt as _};

const PRODUCTION_DATABASE_PATH: &str =
    "/home/robinhood/.local/share/robin-highscores/database/highscores.sqlite3";
const EXCLUSIVE_FENCE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const EXCLUSIVE_FENCE_POLL_INTERVAL: Duration = Duration::from_millis(5);
const DATABASE_DIRECTORY_MODE: u32 = 0o2770;
const DATABASE_FILE_MODE: u32 = 0o660;
const SNAPSHOT_DIRECTORY_MODE: u32 = 0o700;
const SNAPSHOT_FILE_MODE: u32 = 0o600;
const DATABASE_LEAF: &str = "highscores.sqlite3";
const WAL_LEAF: &str = "highscores.sqlite3-wal";
const SHM_LEAF: &str = "highscores.sqlite3-shm";

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveDatabaseSchemaProbeV2 {
    pub database_schema_version: i64,
    pub schema_version: u32,
    pub source_commit: String,
    pub vps_release_manifest_sha256: String,
}

impl LiveDatabaseSchemaProbeV2 {
    pub fn from_attestation(
        attestation: &crate::runtime_authority::CandidateReleaseAttestationV2,
        verified_database_schema_version: i64,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            verified_database_schema_version == attestation.database_schema_version,
            "verified live schema differs from candidate release authority"
        );
        Ok(Self {
            database_schema_version: verified_database_schema_version,
            schema_version: 2,
            source_commit: attestation.source_commit.clone(),
            vps_release_manifest_sha256: attestation.vps_release_manifest_sha256.clone(),
        })
    }
}

/// Verify the fixed production database without loading mutable runtime
/// configuration. The expected version comes from an independently attested
/// candidate `VpsReleaseManifestV2`.
pub async fn verify_live_database_schema_v2(expected_schema: i64) -> anyhow::Result<i64> {
    let fence =
        RuntimeDatabaseFence::open(Path::new(crate::config::DEFAULT_RUNTIME_FENCE_DIRECTORY))?;
    verify_live_database_schema_with_fence(
        Path::new(PRODUCTION_DATABASE_PATH),
        fence,
        expected_schema,
    )
    .await
}

#[cfg(test)]
async fn verify_live_database_schema_at(
    database_path: &Path,
    fence_directory: &Path,
    expected_schema: i64,
) -> anyhow::Result<i64> {
    let fence = RuntimeDatabaseFence::open_test(fence_directory)?;
    verify_live_database_schema_with_fence(database_path, fence, expected_schema).await
}

async fn verify_live_database_schema_with_fence(
    database_path: &Path,
    fence: RuntimeDatabaseFence,
    expected_schema: i64,
) -> anyhow::Result<i64> {
    anyhow::ensure!(
        expected_schema == CURRENT_SCHEMA_VERSION,
        "candidate database schema {expected_schema} differs from compiled schema {CURRENT_SCHEMA_VERSION}"
    );
    let (admission, quiescence) = acquire_exclusive_pair(&fence).await?;
    fence.validate_exclusive_pair(&admission, &quiescence)?;

    let database_path = database_path.to_owned();
    let fenced = fence.clone();
    let (admission, quiescence, snapshot) = tokio::task::spawn_blocking(move || {
        fenced.validate_exclusive_pair(&admission, &quiescence)?;
        let snapshot = LiveDatabaseSnapshot::create(&database_path)?;
        fenced.validate_exclusive_pair(&admission, &quiescence)?;
        Ok::<_, anyhow::Error>((admission, quiescence, snapshot))
    })
    .await
    .map_err(|error| anyhow::anyhow!(error).context("live-schema snapshot task failed"))??;

    let schema = inspect_snapshot_schema(snapshot.database_path()?, expected_schema).await?;

    let fenced = fence.clone();
    tokio::task::spawn_blocking(move || {
        fenced.validate_exclusive_pair(&admission, &quiescence)?;
        snapshot.prove_live_unchanged()?;
        fenced.validate_exclusive_pair(&admission, &quiescence)?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .map_err(|error| anyhow::anyhow!(error).context("live-schema final proof task failed"))??;
    Ok(schema)
}

async fn acquire_exclusive_pair(
    fence: &RuntimeDatabaseFence,
) -> anyhow::Result<(ExclusiveAdmissionGuard, ExclusiveQuiescenceGuard)> {
    let deadline = tokio::time::Instant::now() + EXCLUSIVE_FENCE_TIMEOUT;
    let admission = loop {
        if let Some(guard) = fence.try_lock_exclusive_admission()? {
            break guard;
        }
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "timed out acquiring exclusive database-admission fence"
        );
        tokio::time::sleep(EXCLUSIVE_FENCE_POLL_INTERVAL).await;
    };
    let quiescence = loop {
        if let Some(guard) = fence.try_lock_exclusive_quiescence()? {
            break guard;
        }
        admission.revalidate()?;
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "timed out draining the exclusive database-quiescence fence"
        );
        tokio::time::sleep(EXCLUSIVE_FENCE_POLL_INTERVAL).await;
    };
    fence.validate_exclusive_pair(&admission, &quiescence)?;
    Ok((admission, quiescence))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UnixMetadata {
    device: u64,
    inode: u64,
    mode: u32,
    links: u64,
    owner: u32,
    group: u32,
    special_device: u64,
    bytes: u64,
    block_size: u64,
    blocks: u64,
    accessed_seconds: i64,
    accessed_nanoseconds: i64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DatabaseParentIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    links: u64,
    owner: u32,
    group: u32,
    special_device: u64,
    bytes: u64,
    block_size: u64,
    blocks: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[cfg(unix)]
fn unix_metadata(metadata: &std::fs::Metadata) -> UnixMetadata {
    UnixMetadata {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        links: metadata.nlink(),
        owner: metadata.uid(),
        group: metadata.gid(),
        special_device: metadata.rdev(),
        bytes: metadata.size(),
        block_size: metadata.blksize(),
        blocks: metadata.blocks(),
        accessed_seconds: metadata.atime(),
        accessed_nanoseconds: metadata.atime_nsec(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    }
}

fn database_parent_identity(metadata: UnixMetadata) -> DatabaseParentIdentity {
    DatabaseParentIdentity {
        device: metadata.device,
        inode: metadata.inode,
        mode: metadata.mode,
        links: metadata.links,
        owner: metadata.owner,
        group: metadata.group,
        special_device: metadata.special_device,
        bytes: metadata.bytes,
        block_size: metadata.block_size,
        blocks: metadata.blocks,
        modified_seconds: metadata.modified_seconds,
        modified_nanoseconds: metadata.modified_nanoseconds,
        changed_seconds: metadata.changed_seconds,
        changed_nanoseconds: metadata.changed_nanoseconds,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileState {
    metadata: UnixMetadata,
    sha256: [u8; 32],
}

struct PinnedLiveFile {
    leaf: &'static str,
    file: std::fs::File,
    initial: FileState,
}

struct LiveDatabaseSnapshot {
    parent_path: PathBuf,
    parent: Dir,
    parent_identity: DatabaseParentIdentity,
    database: PinnedLiveFile,
    wal: Option<PinnedLiveFile>,
    shm: Option<PinnedLiveFile>,
    temporary_directory: tempfile::TempDir,
    _temporary_parent: Dir,
    _copied_database: std::fs::File,
    _copied_wal: Option<std::fs::File>,
}

impl LiveDatabaseSnapshot {
    fn create(database_path: &Path) -> anyhow::Result<Self> {
        Self::create_with_hook(database_path, || Ok(()))
    }

    fn create_with_hook<F>(database_path: &Path, after_copy: F) -> anyhow::Result<Self>
    where
        F: FnOnce() -> anyhow::Result<()>,
    {
        #[cfg(not(target_os = "linux"))]
        anyhow::bail!("live database schema verification requires Linux openat2 and flock");

        #[cfg(target_os = "linux")]
        {
            anyhow::ensure!(
                database_path.is_absolute(),
                "database path must be absolute"
            );
            anyhow::ensure!(
                database_path.file_name() == Some(std::ffi::OsStr::new(DATABASE_LEAF)),
                "live database has a noncanonical filename"
            );
            let parent_path = database_path
                .parent()
                .context("live database path has no parent")?
                .to_owned();
            anyhow::ensure!(
                std::fs::canonicalize(&parent_path)? == parent_path,
                "live database parent is not its canonical real path"
            );
            let parent_file = open_directory_nofollow(&parent_path)?;
            validate_database_parent(&parent_file.metadata()?)?;
            let parent_metadata = unix_metadata(&parent_file.metadata()?);
            let parent_identity = database_parent_identity(parent_metadata);
            let parent = Dir::from_std_file(parent_file);

            let database = pin_required_file(
                &parent,
                DATABASE_LEAF,
                parent_metadata.device,
                parent_metadata.group,
            )?;
            let wal = pin_optional_file(
                &parent,
                WAL_LEAF,
                parent_metadata.device,
                parent_metadata.group,
            )?;
            let shm = pin_optional_file(
                &parent,
                SHM_LEAF,
                parent_metadata.device,
                parent_metadata.group,
            )?;
            anyhow::ensure!(
                shm.is_none() || wal.is_some(),
                "live database has an orphan SHM sidecar without its WAL"
            );

            revalidate_database_inventory(&parent, wal.is_some(), shm.is_some())?;
            revalidate_parent_path(&parent_path, parent_identity)?;
            revalidate_live_file(&parent, &database)?;
            revalidate_optional_file(&parent, WAL_LEAF, wal.as_ref())?;
            revalidate_optional_file(&parent, SHM_LEAF, shm.as_ref())?;

            let temporary_directory = tempfile::Builder::new()
                .prefix("robin-live-schema-v2-")
                .tempdir()?;
            std::fs::set_permissions(
                temporary_directory.path(),
                std::fs::Permissions::from_mode(SNAPSHOT_DIRECTORY_MODE),
            )?;
            let temporary_parent_file = open_directory_nofollow(temporary_directory.path())?;
            validate_snapshot_parent(&temporary_parent_file.metadata()?)?;
            let temporary_parent = Dir::from_std_file(temporary_parent_file);
            let copied_database = copy_pinned_file(
                &database,
                &temporary_parent,
                DATABASE_LEAF,
                SNAPSHOT_FILE_MODE,
            )?;
            let copied_wal = match &wal {
                Some(wal) => Some(copy_pinned_file(
                    wal,
                    &temporary_parent,
                    WAL_LEAF,
                    SNAPSHOT_FILE_MODE,
                )?),
                None => None,
            };
            after_copy()?;

            // A second complete same-descriptor capture closes both in-place
            // mutation and short-read races at the copy boundary.
            prove_file_state(&database)?;
            if let Some(wal) = &wal {
                prove_file_state(wal)?;
            }
            if let Some(shm) = &shm {
                prove_file_state(shm)?;
            }
            revalidate_live_file(&parent, &database)?;
            revalidate_optional_file(&parent, WAL_LEAF, wal.as_ref())?;
            revalidate_optional_file(&parent, SHM_LEAF, shm.as_ref())?;
            revalidate_database_inventory(&parent, wal.is_some(), shm.is_some())?;
            revalidate_parent_path(&parent_path, parent_identity)?;

            Ok(Self {
                parent_path,
                parent,
                parent_identity,
                database,
                wal,
                shm,
                temporary_directory,
                _temporary_parent: temporary_parent,
                _copied_database: copied_database,
                _copied_wal: copied_wal,
            })
        }
    }

    fn database_path(&self) -> anyhow::Result<PathBuf> {
        let retained_parent = self._temporary_parent.try_clone()?.into_std_file();
        let current_parent = open_directory_nofollow(self.temporary_directory.path())?;
        anyhow::ensure!(
            unix_metadata(&retained_parent.metadata()?)
                == unix_metadata(&current_parent.metadata()?),
            "schema snapshot directory path was rebound or changed"
        );
        validate_snapshot_parent(&current_parent.metadata()?)?;
        revalidate_copied_file(
            &self._temporary_parent,
            DATABASE_LEAF,
            &self._copied_database,
        )?;
        match &self._copied_wal {
            Some(wal) => revalidate_copied_file(&self._temporary_parent, WAL_LEAF, wal)?,
            None => ensure_leaf_absent(&self._temporary_parent, WAL_LEAF)?,
        }
        ensure_leaf_absent(&self._temporary_parent, SHM_LEAF)?;
        Ok(self.temporary_directory.path().join(DATABASE_LEAF))
    }

    fn prove_live_unchanged(&self) -> anyhow::Result<()> {
        prove_file_state(&self.database)?;
        if let Some(wal) = &self.wal {
            prove_file_state(wal)?;
        }
        if let Some(shm) = &self.shm {
            prove_file_state(shm)?;
        }
        revalidate_live_file(&self.parent, &self.database)?;
        revalidate_optional_file(&self.parent, WAL_LEAF, self.wal.as_ref())?;
        revalidate_optional_file(&self.parent, SHM_LEAF, self.shm.as_ref())?;
        revalidate_database_inventory(&self.parent, self.wal.is_some(), self.shm.is_some())?;
        revalidate_parent_path(&self.parent_path, self.parent_identity)?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn open_directory_nofollow(path: &Path) -> anyhow::Result<std::fs::File> {
    use rustix::fs::{Mode, OFlags};
    let descriptor = crate::secure_fs::open_no_symlinks_at(
        rustix::fs::CWD,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY | OFlags::NOATIME,
        Mode::empty(),
        rustix::fs::ResolveFlags::empty(),
    )
    .map_err(std::io::Error::from)?;
    Ok(std::fs::File::from(descriptor))
}

fn validate_database_parent(metadata: &std::fs::Metadata) -> anyhow::Result<()> {
    anyhow::ensure!(
        metadata.is_dir()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.permissions().mode() & 0o7777 == DATABASE_DIRECTORY_MODE,
        "live database parent has the wrong type, owner, or mode"
    );
    Ok(())
}

fn validate_snapshot_parent(metadata: &std::fs::Metadata) -> anyhow::Result<()> {
    anyhow::ensure!(
        metadata.is_dir()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.permissions().mode() & 0o7777 == SNAPSHOT_DIRECTORY_MODE,
        "schema snapshot directory has the wrong type, owner, or mode"
    );
    Ok(())
}

fn validate_live_file(
    metadata: &std::fs::Metadata,
    parent_device: u64,
    parent_group: u32,
    leaf: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        metadata.is_file()
            && metadata.dev() == parent_device
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.gid() == parent_group
            && metadata.nlink() == 1
            && metadata.permissions().mode() & 0o7777 == DATABASE_FILE_MODE,
        "live database node {leaf} has the wrong type, device, owner, group, link count, or mode"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn open_live_leaf(parent: &Dir, leaf: &str) -> anyhow::Result<std::fs::File> {
    use rustix::fs::{Mode, OFlags};
    use std::os::fd::AsFd as _;
    let descriptor = crate::secure_fs::open_no_symlinks_at(
        parent.as_fd(),
        Path::new(leaf),
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOATIME | OFlags::NONBLOCK,
        Mode::empty(),
        rustix::fs::ResolveFlags::BENEATH | rustix::fs::ResolveFlags::NO_XDEV,
    )
    .map_err(std::io::Error::from)?;
    Ok(std::fs::File::from(descriptor))
}

fn revalidate_database_inventory(
    parent: &Dir,
    wal_present: bool,
    shm_present: bool,
) -> anyhow::Result<()> {
    let mut actual = BTreeSet::new();
    for entry in parent.entries()? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("live database directory contains a non-UTF-8 entry"))?;
        anyhow::ensure!(
            actual.insert(name),
            "live database directory repeats an entry"
        );
    }
    let mut expected = BTreeSet::from([DATABASE_LEAF.to_owned()]);
    if wal_present {
        expected.insert(WAL_LEAF.to_owned());
    }
    if shm_present {
        expected.insert(SHM_LEAF.to_owned());
    }
    anyhow::ensure!(
        actual == expected,
        "live database directory contains an unauthorized journal, sidecar, or extra node"
    );
    Ok(())
}

fn pin_required_file(
    parent: &Dir,
    leaf: &'static str,
    parent_device: u64,
    parent_group: u32,
) -> anyhow::Result<PinnedLiveFile> {
    let file = open_live_leaf(parent, leaf)?;
    validate_live_file(&file.metadata()?, parent_device, parent_group, leaf)?;
    let initial = capture_file_state(&file)?;
    Ok(PinnedLiveFile {
        leaf,
        file,
        initial,
    })
}

fn pin_optional_file(
    parent: &Dir,
    leaf: &'static str,
    parent_device: u64,
    parent_group: u32,
) -> anyhow::Result<Option<PinnedLiveFile>> {
    match open_live_leaf(parent, leaf) {
        Ok(file) => {
            validate_live_file(&file.metadata()?, parent_device, parent_group, leaf)?;
            let initial = capture_file_state(&file)?;
            Ok(Some(PinnedLiveFile {
                leaf,
                file,
                initial,
            }))
        }
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn capture_file_state(file: &std::fs::File) -> anyhow::Result<FileState> {
    let before = unix_metadata(&file.metadata()?);
    let sha256 = hash_pinned_file(file, before.bytes)?;
    let after = unix_metadata(&file.metadata()?);
    anyhow::ensure!(
        before == after,
        "live database metadata changed while hashing"
    );
    Ok(FileState {
        metadata: after,
        sha256,
    })
}

fn hash_pinned_file(file: &std::fs::File, expected_bytes: u64) -> anyhow::Result<[u8; 32]> {
    let mut digest = Sha256::new();
    let mut offset = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    while offset < expected_bytes {
        let remaining = expected_bytes - offset;
        let requested = usize::try_from(remaining.min(buffer.len() as u64))?;
        let read = file.read_at(&mut buffer[..requested], offset)?;
        anyhow::ensure!(read != 0, "live database file shortened while hashing");
        digest.update(&buffer[..read]);
        offset = offset
            .checked_add(u64::try_from(read)?)
            .context("live database hash offset overflow")?;
    }
    let mut beyond = [0_u8; 1];
    anyhow::ensure!(
        file.read_at(&mut beyond, expected_bytes)? == 0,
        "live database file grew while hashing"
    );
    Ok(digest.finalize().into())
}

fn prove_file_state(file: &PinnedLiveFile) -> anyhow::Result<()> {
    let actual = capture_file_state(&file.file)?;
    anyhow::ensure!(
        actual == file.initial,
        "live database node {} changed during schema verification",
        file.leaf
    );
    Ok(())
}

fn copy_pinned_file(
    source: &PinnedLiveFile,
    destination_parent: &Dir,
    destination_leaf: &str,
    mode: u32,
) -> anyhow::Result<std::fs::File> {
    use std::io::Write as _;
    let mut options = cap_std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut destination = destination_parent
        .open_with(Path::new(destination_leaf), &options)?
        .into_std();
    destination.set_permissions(std::fs::Permissions::from_mode(mode))?;

    let mut digest = Sha256::new();
    let mut offset = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    while offset < source.initial.metadata.bytes {
        let remaining = source.initial.metadata.bytes - offset;
        let requested = usize::try_from(remaining.min(buffer.len() as u64))?;
        let read = source.file.read_at(&mut buffer[..requested], offset)?;
        anyhow::ensure!(read != 0, "live database file shortened while copying");
        destination.write_all(&buffer[..read])?;
        digest.update(&buffer[..read]);
        offset = offset
            .checked_add(u64::try_from(read)?)
            .context("live database copy offset overflow")?;
    }
    destination.sync_all()?;
    anyhow::ensure!(
        <[u8; 32]>::from(digest.finalize()) == source.initial.sha256,
        "live database bytes changed between hash and copy"
    );
    let metadata = destination.metadata()?;
    anyhow::ensure!(
        metadata.is_file()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.nlink() == 1
            && metadata.permissions().mode() & 0o7777 == mode
            && metadata.len() == source.initial.metadata.bytes,
        "schema snapshot copy has the wrong type, owner, link count, mode, or length"
    );
    Ok(destination)
}

fn revalidate_parent_path(
    parent_path: &Path,
    expected: DatabaseParentIdentity,
) -> anyhow::Result<()> {
    let current = open_directory_nofollow(parent_path)?;
    validate_database_parent(&current.metadata()?)?;
    anyhow::ensure!(
        database_parent_identity(unix_metadata(&current.metadata()?)) == expected,
        "live database parent path was rebound or changed"
    );
    Ok(())
}

fn revalidate_live_file(parent: &Dir, expected: &PinnedLiveFile) -> anyhow::Result<()> {
    let current = open_live_leaf(parent, expected.leaf)?;
    let current_state = capture_file_state(&current)?;
    anyhow::ensure!(
        current_state == expected.initial,
        "live database path {} was rebound or changed",
        expected.leaf
    );
    Ok(())
}

fn revalidate_optional_file(
    parent: &Dir,
    leaf: &'static str,
    expected: Option<&PinnedLiveFile>,
) -> anyhow::Result<()> {
    match expected {
        Some(expected) => revalidate_live_file(parent, expected),
        None => ensure_leaf_absent(parent, leaf),
    }
}

fn ensure_leaf_absent(parent: &Dir, leaf: &str) -> anyhow::Result<()> {
    match parent.symlink_metadata(Path::new(leaf)) {
        Ok(_) => anyhow::bail!("unexpected database sidecar appeared at {leaf}"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn revalidate_copied_file(
    parent: &Dir,
    leaf: &str,
    expected: &std::fs::File,
) -> anyhow::Result<()> {
    let current = parent.open(Path::new(leaf))?.into_std();
    let expected_metadata = expected.metadata()?;
    let current_metadata = current.metadata()?;
    anyhow::ensure!(
        current_metadata.is_file()
            && current_metadata.dev() == expected_metadata.dev()
            && current_metadata.ino() == expected_metadata.ino()
            && current_metadata.uid() == rustix::process::geteuid().as_raw()
            && current_metadata.nlink() == 1
            && current_metadata.permissions().mode() & 0o7777 == SNAPSHOT_FILE_MODE,
        "schema snapshot path {leaf} was rebound or has invalid metadata"
    );
    Ok(())
}

async fn inspect_snapshot_schema(
    database_path: PathBuf,
    expected_schema: i64,
) -> anyhow::Result<i64> {
    let options = SqliteConnectOptions::new()
        .filename(&database_path)
        .create_if_missing(false)
        .foreign_keys(true);
    let mut connection = sqlx::SqliteConnection::connect_with(&options).await?;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut connection)
        .await?;
    anyhow::ensure!(
        integrity == "ok",
        "live database snapshot failed integrity_check"
    );
    let foreign_key_failures: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check")
            .fetch_one(&mut connection)
            .await?;
    anyhow::ensure!(
        foreign_key_failures == 0,
        "live database snapshot has foreign-key violations"
    );
    let actual_schema_inventory = schema_inventory(&mut connection).await?;
    let mut expected_connection = sqlx::SqliteConnection::connect("sqlite::memory:").await?;
    MIGRATOR
        .run_direct(None, &mut expected_connection, false)
        .await?;
    let expected_schema_inventory = schema_inventory(&mut expected_connection).await?;
    expected_connection.close().await?;
    anyhow::ensure!(
        actual_schema_inventory == expected_schema_inventory,
        "live database snapshot does not contain the exact compiled schema object inventory"
    );
    let rows =
        sqlx::query("SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&mut connection)
            .await?;
    let current = rows.last().map(|row| row.get::<i64, _>("version"));
    let checksums_match = rows
        .iter()
        .zip(MIGRATOR.migrations.iter())
        .all(|(row, migration)| {
            row.get::<i64, _>("version") == migration.version
                && row.get::<Vec<u8>, _>("checksum").as_slice() == migration.checksum.as_ref()
        });
    anyhow::ensure!(
        rows.len() == MIGRATOR.migrations.len()
            && current == Some(expected_schema)
            && rows.iter().all(|row| row.get::<bool, _>("success"))
            && checksums_match,
        "live database snapshot does not contain the exact expected migration chain"
    );
    connection.close().await?;
    Ok(expected_schema)
}

#[derive(Debug, PartialEq, Eq)]
struct SchemaObject {
    object_type: String,
    name: String,
    table_name: String,
    sql: Option<String>,
}

async fn schema_inventory(
    connection: &mut sqlx::SqliteConnection,
) -> anyhow::Result<Vec<SchemaObject>> {
    let rows = sqlx::query(
        "SELECT type, name, tbl_name, sql FROM sqlite_schema \
         ORDER BY type, name, tbl_name, COALESCE(sql, '')",
    )
    .fetch_all(connection)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| SchemaObject {
            object_type: row.get("type"),
            name: row.get("name"),
            table_name: row.get("tbl_name"),
            sql: row.get("sql"),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db_fence::provision_test_runtime_fence;
    use sqlx::migrate::Migrator;
    use sqlx::sqlite::{SqliteConnection, SqliteJournalMode};
    use std::borrow::Cow;
    use std::os::unix::fs::FileTypeExt as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    #[test]
    fn database_parent_identity_ignores_only_access_time() {
        let original = UnixMetadata {
            device: 1,
            inode: 2,
            mode: rustix::fs::FileType::Directory.as_raw_mode() | DATABASE_DIRECTORY_MODE,
            links: 3,
            owner: 4,
            group: 5,
            special_device: 0,
            bytes: 4096,
            block_size: 4096,
            blocks: 8,
            accessed_seconds: 10,
            accessed_nanoseconds: 11,
            modified_seconds: 12,
            modified_nanoseconds: 13,
            changed_seconds: 14,
            changed_nanoseconds: 15,
        };
        let mut accessed = original;
        accessed.accessed_seconds += 1;
        accessed.accessed_nanoseconds += 1;
        assert_eq!(
            database_parent_identity(original),
            database_parent_identity(accessed)
        );

        let mut modified = original;
        modified.modified_nanoseconds += 1;
        assert_ne!(
            database_parent_identity(original),
            database_parent_identity(modified)
        );
        let mut changed = original;
        changed.changed_nanoseconds += 1;
        assert_ne!(
            database_parent_identity(original),
            database_parent_identity(changed)
        );
    }

    fn private_database_root(root: &Path) -> (PathBuf, PathBuf) {
        let database_root = root.join("database");
        std::fs::create_dir(&database_root).unwrap();
        std::fs::set_permissions(
            &database_root,
            std::fs::Permissions::from_mode(DATABASE_DIRECTORY_MODE),
        )
        .unwrap();
        let fence_root = root.join("runtime-fence");
        provision_test_runtime_fence(&fence_root).unwrap();
        (database_root.join(DATABASE_LEAF), fence_root)
    }

    #[test]
    fn database_directory_descriptor_is_opened_without_atime_updates() {
        let root = tempfile::tempdir().unwrap();
        let (database, _fence) = private_database_root(root.path());
        let parent = open_directory_nofollow(database.parent().unwrap()).unwrap();
        let flags = rustix::fs::fcntl_getfl(&parent).unwrap();
        assert!(flags.contains(rustix::fs::OFlags::NOATIME));
    }

    #[test]
    fn schema_probe_receipt_has_exact_canonical_v2_wire() {
        let attestation = crate::runtime_authority::CandidateReleaseAttestationV2 {
            source_commit: "1".repeat(40),
            database_schema_version: CURRENT_SCHEMA_VERSION,
            vps_release_manifest_sha256: "2".repeat(64),
        };
        let receipt =
            LiveDatabaseSchemaProbeV2::from_attestation(&attestation, CURRENT_SCHEMA_VERSION)
                .unwrap();
        assert_eq!(
            robin_run_protocol::canonical_json_bytes(&receipt).unwrap(),
            format!(
                "{{\"database_schema_version\":{CURRENT_SCHEMA_VERSION},\"schema_version\":2,\"source_commit\":\"{}\",\"vps_release_manifest_sha256\":\"{}\"}}",
                "1".repeat(40),
                "2".repeat(64)
            )
            .into_bytes()
        );
        assert!(
            LiveDatabaseSchemaProbeV2::from_attestation(
                &attestation,
                CURRENT_SCHEMA_VERSION.saturating_add(1),
            )
            .is_err()
        );
    }

    async fn migrate_database(path: &Path) {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
        MIGRATOR
            .run_direct(None, &mut connection, false)
            .await
            .unwrap();
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
        for leaf in [WAL_LEAF, SHM_LEAF] {
            let candidate = path.parent().unwrap().join(leaf);
            if candidate.exists() {
                std::fs::remove_file(candidate).unwrap();
            }
        }
        for leaf in [DATABASE_LEAF, WAL_LEAF, SHM_LEAF] {
            let candidate = path.parent().unwrap().join(leaf);
            if candidate.exists() {
                std::fs::set_permissions(
                    candidate,
                    std::fs::Permissions::from_mode(DATABASE_FILE_MODE),
                )
                .unwrap();
            }
        }
    }

    async fn migrate_database_with_current_schema_only_in_wal(path: &Path) -> SqliteConnection {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        let mut first_connection = SqliteConnection::connect_with(&options).await.unwrap();
        let first = Migrator {
            migrations: Cow::Owned(vec![MIGRATOR.migrations[0].clone()]),
            ..Migrator::DEFAULT
        };
        first
            .run_direct(None, &mut first_connection, false)
            .await
            .unwrap();
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&mut first_connection)
            .await
            .unwrap();
        first_connection.close().await.unwrap();

        let mut wal_connection = SqliteConnection::connect_with(&options).await.unwrap();
        sqlx::query("PRAGMA wal_autocheckpoint=0")
            .execute(&mut wal_connection)
            .await
            .unwrap();
        MIGRATOR
            .run_direct(None, &mut wal_connection, false)
            .await
            .unwrap();
        for leaf in [DATABASE_LEAF, WAL_LEAF, SHM_LEAF] {
            let candidate = path.parent().unwrap().join(leaf);
            if candidate.exists() {
                std::fs::set_permissions(
                    candidate,
                    std::fs::Permissions::from_mode(DATABASE_FILE_MODE),
                )
                .unwrap();
            }
        }
        assert!(path.parent().unwrap().join(WAL_LEAF).exists());
        wal_connection
    }

    fn file_states(path: &Path) -> Vec<(PathBuf, Option<FileState>)> {
        [DATABASE_LEAF, WAL_LEAF, SHM_LEAF]
            .into_iter()
            .map(|leaf| {
                let path = path.parent().unwrap().join(leaf);
                let state = if path.exists() {
                    let file = std::fs::OpenOptions::new()
                        .read(true)
                        .custom_flags(rustix::fs::OFlags::NOATIME.bits() as i32)
                        .open(&path)
                        .unwrap();
                    Some(capture_file_state(&file).unwrap())
                } else {
                    None
                };
                (path, state)
            })
            .collect()
    }

    #[tokio::test]
    async fn verifies_current_schema_without_changing_live_database() {
        let root = tempfile::tempdir().unwrap();
        let (database, fence) = private_database_root(root.path());
        migrate_database(&database).await;
        let before = file_states(&database);
        assert_eq!(
            verify_live_database_schema_at(&database, &fence, CURRENT_SCHEMA_VERSION)
                .await
                .unwrap(),
            CURRENT_SCHEMA_VERSION
        );
        assert_eq!(file_states(&database), before);
    }

    #[tokio::test]
    async fn retains_exclusive_fences_until_schema_inspection_finishes() {
        let root = tempfile::tempdir().unwrap();
        let (database, fence_directory) = private_database_root(root.path());
        migrate_database(&database).await;
        let fence = RuntimeDatabaseFence::open_test(&fence_directory).unwrap();
        let existing_operation = fence.acquire_one_off_shared().await.unwrap();
        let database_for_task = database.clone();
        let fence_for_task = fence_directory.clone();
        let verifier = tokio::spawn(async move {
            verify_live_database_schema_at(
                &database_for_task,
                &fence_for_task,
                CURRENT_SCHEMA_VERSION,
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !verifier.is_finished(),
            "live-schema verifier passed a held shared quiescence fence"
        );
        drop(existing_operation);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), verifier)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            CURRENT_SCHEMA_VERSION
        );
    }

    #[tokio::test]
    async fn rejects_candidate_schema_other_than_compiled_schema() {
        let root = tempfile::tempdir().unwrap();
        let (database, fence) = private_database_root(root.path());
        migrate_database(&database).await;
        let error = verify_live_database_schema_at(
            &database,
            &fence,
            CURRENT_SCHEMA_VERSION.saturating_add(1),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("differs from compiled schema"));
    }

    #[tokio::test]
    async fn rejects_schema_objects_not_authorized_by_compiled_migrations() {
        let root = tempfile::tempdir().unwrap();
        let (database, fence) = private_database_root(root.path());
        migrate_database(&database).await;
        let options = SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(false)
            .journal_mode(SqliteJournalMode::Wal);
        let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
        sqlx::query("CREATE TABLE unauthorized_score_override (score INTEGER NOT NULL)")
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
        for leaf in [WAL_LEAF, SHM_LEAF] {
            let candidate = database.parent().unwrap().join(leaf);
            if candidate.exists() {
                std::fs::remove_file(candidate).unwrap();
            }
        }
        std::fs::set_permissions(
            &database,
            std::fs::Permissions::from_mode(DATABASE_FILE_MODE),
        )
        .unwrap();
        let error = verify_live_database_schema_at(&database, &fence, CURRENT_SCHEMA_VERSION)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("schema object inventory"));
    }

    #[tokio::test]
    async fn consumes_schema_changes_present_only_in_wal() {
        let root = tempfile::tempdir().unwrap();
        let (database, fence) = private_database_root(root.path());
        let wal_connection = migrate_database_with_current_schema_only_in_wal(&database).await;

        let main_only = root.path().join("main-file-without-wal.sqlite3");
        std::fs::copy(&database, &main_only).unwrap();
        let main_only_options = SqliteConnectOptions::new()
            .filename(&main_only)
            .read_only(true)
            .immutable(true);
        let mut main_only_connection = SqliteConnection::connect_with(&main_only_options)
            .await
            .unwrap();
        let main_only_schema: i64 =
            sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1")
                .fetch_one(&mut main_only_connection)
                .await
                .unwrap();
        assert_eq!(main_only_schema, MIGRATOR.migrations[0].version);
        main_only_connection.close().await.unwrap();

        let before = file_states(&database);
        assert_eq!(
            verify_live_database_schema_at(&database, &fence, CURRENT_SCHEMA_VERSION)
                .await
                .unwrap(),
            CURRENT_SCHEMA_VERSION
        );
        assert_eq!(file_states(&database), before);
        wal_connection.close().await.unwrap();
    }

    #[tokio::test]
    async fn refuses_symlinked_database_leaf() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let (database, fence) = private_database_root(root.path());
        let outside = root.path().join("outside.sqlite3");
        std::fs::write(&outside, b"not sqlite").unwrap();
        std::fs::set_permissions(
            &outside,
            std::fs::Permissions::from_mode(DATABASE_FILE_MODE),
        )
        .unwrap();
        symlink(&outside, &database).unwrap();
        assert!(
            verify_live_database_schema_at(&database, &fence, CURRENT_SCHEMA_VERSION)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn refuses_hardlinked_live_database() {
        let root = tempfile::tempdir().unwrap();
        let (database, fence) = private_database_root(root.path());
        migrate_database(&database).await;
        std::fs::hard_link(
            &database,
            database.parent().unwrap().join("database-hardlink"),
        )
        .unwrap();
        let error = verify_live_database_schema_at(&database, &fence, CURRENT_SCHEMA_VERSION)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("link count"));
    }

    #[tokio::test]
    async fn refuses_special_mode_bits_on_live_and_copied_database_files() {
        let live_root = tempfile::tempdir().unwrap();
        let (live_database, live_fence) = private_database_root(live_root.path());
        migrate_database(&live_database).await;
        std::fs::set_permissions(
            &live_database,
            std::fs::Permissions::from_mode(DATABASE_FILE_MODE | 0o4000),
        )
        .unwrap();
        let error =
            verify_live_database_schema_at(&live_database, &live_fence, CURRENT_SCHEMA_VERSION)
                .await
                .unwrap_err();
        assert!(error.to_string().contains("mode"));

        let copied_root = tempfile::tempdir().unwrap();
        let (copied_database, _copied_fence) = private_database_root(copied_root.path());
        migrate_database(&copied_database).await;
        let snapshot = LiveDatabaseSnapshot::create(&copied_database).unwrap();
        snapshot
            ._copied_database
            .set_permissions(std::fs::Permissions::from_mode(SNAPSHOT_FILE_MODE | 0o4000))
            .unwrap();
        let error = snapshot.database_path().unwrap_err();
        assert!(error.to_string().contains("invalid metadata"));
    }

    async fn assert_special_leaf_rejects_without_blocking(database: PathBuf, special: PathBuf) {
        let task_database = database.clone();
        let mut task =
            tokio::task::spawn_blocking(move || LiveDatabaseSnapshot::create(&task_database));
        match tokio::time::timeout(Duration::from_secs(1), &mut task).await {
            Ok(result) => {
                let error = result.unwrap().err().unwrap();
                assert!(error.to_string().contains("wrong type"));
            }
            Err(_) => {
                let unblocking = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
                    .open(&special)
                    .unwrap();
                let result = tokio::time::timeout(Duration::from_secs(1), task)
                    .await
                    .expect("special database leaf remained blocked after watchdog unblocked it")
                    .unwrap();
                drop(unblocking);
                assert!(result.is_err());
                panic!("live-schema verifier blocked while opening a special database leaf");
            }
        }
    }

    #[tokio::test]
    async fn rejects_database_wal_and_shm_fifos_without_blocking() {
        for leaf in [DATABASE_LEAF, WAL_LEAF, SHM_LEAF] {
            let root = tempfile::tempdir().unwrap();
            let (database, _fence) = private_database_root(root.path());
            if leaf != DATABASE_LEAF {
                migrate_database(&database).await;
            }
            let special = database.parent().unwrap().join(leaf);
            rustix::fs::mkfifoat(
                rustix::fs::CWD,
                &special,
                rustix::fs::Mode::RUSR
                    | rustix::fs::Mode::WUSR
                    | rustix::fs::Mode::RGRP
                    | rustix::fs::Mode::WGRP,
            )
            .unwrap();
            assert!(special.metadata().unwrap().file_type().is_fifo());
            assert_special_leaf_rejects_without_blocking(database, special).await;
        }
    }

    #[tokio::test]
    async fn rejects_live_rollback_journal_authority() {
        let root = tempfile::tempdir().unwrap();
        let (database, fence) = private_database_root(root.path());
        migrate_database(&database).await;
        let options = SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(false)
            .journal_mode(SqliteJournalMode::Delete);
        let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE uncommitted_schema (value INTEGER NOT NULL)")
            .execute(&mut connection)
            .await
            .unwrap();
        let journal = database
            .parent()
            .unwrap()
            .join(format!("{DATABASE_LEAF}-journal"));
        assert!(journal.metadata().unwrap().len() > 0);
        let error = verify_live_database_schema_at(&database, &fence, CURRENT_SCHEMA_VERSION)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unauthorized journal"));
        sqlx::query("ROLLBACK")
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
    }

    #[tokio::test]
    async fn detects_in_place_live_database_mutation_after_copy() {
        use std::io::Write as _;

        let root = tempfile::tempdir().unwrap();
        let (database, _fence) = private_database_root(root.path());
        migrate_database(&database).await;
        let error = LiveDatabaseSnapshot::create_with_hook(&database, || {
            let mut file = std::fs::OpenOptions::new().append(true).open(&database)?;
            file.write_all(b"uncooperative mutation")?;
            file.sync_all()?;
            Ok(())
        })
        .err()
        .unwrap();
        let message = error.to_string();
        assert!(message.contains("grew") || message.contains("changed"));
    }

    #[tokio::test]
    async fn detects_database_path_replacement_after_copy() {
        let root = tempfile::tempdir().unwrap();
        let (database, _fence) = private_database_root(root.path());
        migrate_database(&database).await;
        let displaced = root.path().join("displaced.sqlite3");
        let error = LiveDatabaseSnapshot::create_with_hook(&database, || {
            std::fs::rename(&database, &displaced)?;
            std::fs::copy(&displaced, &database)?;
            std::fs::set_permissions(
                &database,
                std::fs::Permissions::from_mode(DATABASE_FILE_MODE),
            )?;
            Ok(())
        })
        .err()
        .unwrap();
        assert!(error.to_string().contains("changed"));
    }

    #[tokio::test]
    async fn detects_database_parent_path_replacement_after_copy() {
        let root = tempfile::tempdir().unwrap();
        let (database, _fence) = private_database_root(root.path());
        migrate_database(&database).await;
        let database_parent = database.parent().unwrap().to_owned();
        let displaced_parent = root.path().join("displaced-database");
        let error = LiveDatabaseSnapshot::create_with_hook(&database, || {
            std::fs::rename(&database_parent, &displaced_parent)?;
            std::fs::create_dir(&database_parent)?;
            std::fs::set_permissions(
                &database_parent,
                std::fs::Permissions::from_mode(DATABASE_DIRECTORY_MODE),
            )?;
            Ok(())
        })
        .err()
        .unwrap();
        assert!(
            error
                .to_string()
                .contains("parent path was rebound or changed")
        );
    }

    #[tokio::test]
    async fn detects_wal_path_replacement_after_copy() {
        let root = tempfile::tempdir().unwrap();
        let (database, _fence) = private_database_root(root.path());
        let wal_connection = migrate_database_with_current_schema_only_in_wal(&database).await;
        let wal = database.parent().unwrap().join(WAL_LEAF);
        let displaced = root.path().join("displaced-wal");
        let error = LiveDatabaseSnapshot::create_with_hook(&database, || {
            std::fs::rename(&wal, &displaced)?;
            std::fs::copy(&displaced, &wal)?;
            std::fs::set_permissions(&wal, std::fs::Permissions::from_mode(DATABASE_FILE_MODE))?;
            Ok(())
        })
        .err()
        .unwrap();
        assert!(error.to_string().contains("changed"));
        drop(wal_connection);
    }

    #[tokio::test]
    async fn detects_wal_in_place_mutation_after_copy() {
        use std::io::Write as _;

        let root = tempfile::tempdir().unwrap();
        let (database, _fence) = private_database_root(root.path());
        let wal_connection = migrate_database_with_current_schema_only_in_wal(&database).await;
        let wal = database.parent().unwrap().join(WAL_LEAF);
        let error = LiveDatabaseSnapshot::create_with_hook(&database, || {
            let mut file = std::fs::OpenOptions::new().append(true).open(&wal)?;
            file.write_all(b"uncooperative wal mutation")?;
            file.sync_all()?;
            Ok(())
        })
        .err()
        .unwrap();
        let message = error.to_string();
        assert!(message.contains("grew") || message.contains("changed"));
        drop(wal_connection);
    }

    #[tokio::test]
    async fn detects_shm_path_replacement_after_copy() {
        let root = tempfile::tempdir().unwrap();
        let (database, _fence) = private_database_root(root.path());
        let wal_connection = migrate_database_with_current_schema_only_in_wal(&database).await;
        let shm = database.parent().unwrap().join(SHM_LEAF);
        let displaced = root.path().join("displaced-shm");
        let error = LiveDatabaseSnapshot::create_with_hook(&database, || {
            std::fs::rename(&shm, &displaced)?;
            std::fs::copy(&displaced, &shm)?;
            std::fs::set_permissions(&shm, std::fs::Permissions::from_mode(DATABASE_FILE_MODE))?;
            Ok(())
        })
        .err()
        .unwrap();
        assert!(error.to_string().contains("changed"));
        drop(wal_connection);
    }

    #[tokio::test]
    async fn detects_sidecar_appearing_after_absence_was_pinned() {
        let root = tempfile::tempdir().unwrap();
        let (database, _fence) = private_database_root(root.path());
        migrate_database(&database).await;
        let wal = database.parent().unwrap().join(WAL_LEAF);
        assert!(!wal.exists());
        let error = LiveDatabaseSnapshot::create_with_hook(&database, || {
            std::fs::write(&wal, b"unexpected wal")?;
            std::fs::set_permissions(&wal, std::fs::Permissions::from_mode(DATABASE_FILE_MODE))?;
            Ok(())
        })
        .err()
        .unwrap();
        assert!(error.to_string().contains("unexpected database sidecar"));
    }

    #[tokio::test]
    async fn refuses_orphan_shm_sidecar() {
        let root = tempfile::tempdir().unwrap();
        let (database, fence) = private_database_root(root.path());
        migrate_database(&database).await;
        let shm = database.parent().unwrap().join(SHM_LEAF);
        std::fs::write(&shm, b"stale shm").unwrap();
        std::fs::set_permissions(&shm, std::fs::Permissions::from_mode(DATABASE_FILE_MODE))
            .unwrap();
        let error = verify_live_database_schema_at(&database, &fence, CURRENT_SCHEMA_VERSION)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("orphan SHM"));
    }
}
