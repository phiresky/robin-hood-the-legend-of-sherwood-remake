//! Kernel-backed admission and quiescence fencing for every SQLite operation.
//!
//! The two lock files live in a pre-provisioned, service-read-only directory.
//! Every normal database operation passes the admission turnstile and joins a
//! process-wide shared quiescence lock. Backup and read-only live-schema probes
//! take both locks exclusively, in that order. The separate turnstile prevents
//! Linux `flock`'s non-fair shared admission from starving an exclusive waiter.

use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const DB_ADMISSION_LOCK: &str = "db-admission.lock";
pub const DB_QUIESCENCE_LOCK: &str = "db-quiescence.lock";
pub const RUNTIME_FENCE_DIRECTORY_MODE: u32 = 0o500;
pub const RUNTIME_FENCE_FILE_MODE: u32 = 0o400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    owner: u32,
}

#[cfg(unix)]
fn identity(metadata: &std::fs::Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt as _;
    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        owner: metadata.uid(),
    }
}

struct RuntimeDatabaseFenceInner {
    directory_path: PathBuf,
    directory: cap_std::fs::Dir,
    directory_identity: FileIdentity,
    directory_mode: u32,
    admission_identity: FileIdentity,
    quiescence_identity: FileIdentity,
}

#[derive(Clone)]
pub struct RuntimeDatabaseFence {
    inner: Arc<RuntimeDatabaseFenceInner>,
}

pub struct SharedQuiescenceGuard {
    fence: RuntimeDatabaseFence,
    file: std::fs::File,
}

pub struct ExclusiveAdmissionGuard {
    fence: RuntimeDatabaseFence,
    file: std::fs::File,
}

pub struct ExclusiveQuiescenceGuard {
    fence: RuntimeDatabaseFence,
    file: std::fs::File,
}

impl RuntimeDatabaseFence {
    pub fn open(directory_path: &Path) -> anyhow::Result<Self> {
        Self::open_inner(directory_path, false)
    }

    pub(crate) fn open_test(directory_path: &Path) -> anyhow::Result<Self> {
        Self::open_inner(directory_path, true)
    }

    fn open_inner(directory_path: &Path, test_fixture: bool) -> anyhow::Result<Self> {
        anyhow::ensure!(
            directory_path.is_absolute(),
            "runtime fence directory must be absolute"
        );
        anyhow::ensure!(
            std::fs::canonicalize(directory_path)? == directory_path,
            "runtime fence directory must be its canonical real path"
        );
        #[cfg(target_os = "linux")]
        let directory_file = {
            use rustix::fs::{Mode, OFlags};
            let descriptor = crate::secure_fs::open_no_symlinks_at(
                rustix::fs::CWD,
                directory_path,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
                Mode::empty(),
                rustix::fs::ResolveFlags::empty(),
            )?;
            std::fs::File::from(descriptor)
        };
        #[cfg(not(target_os = "linux"))]
        anyhow::bail!("the production database fence requires Linux openat2 and flock");

        let directory_metadata = directory_file.metadata()?;
        let directory_mode = if test_fixture {
            0o700
        } else {
            RUNTIME_FENCE_DIRECTORY_MODE
        };
        Self::validate_directory_metadata(&directory_metadata, directory_mode)?;
        let directory_identity = identity(&directory_metadata);
        let directory = cap_std::fs::Dir::from_std_file(directory_file);
        let admission = Self::open_leaf_from(&directory, DB_ADMISSION_LOCK)?;
        let quiescence = Self::open_leaf_from(&directory, DB_QUIESCENCE_LOCK)?;
        let admission_identity = Self::validate_leaf_metadata(
            &admission.metadata()?,
            directory_identity.device,
            DB_ADMISSION_LOCK,
        )?;
        let quiescence_identity = Self::validate_leaf_metadata(
            &quiescence.metadata()?,
            directory_identity.device,
            DB_QUIESCENCE_LOCK,
        )?;
        let fence = Self {
            inner: Arc::new(RuntimeDatabaseFenceInner {
                directory_path: directory_path.to_owned(),
                directory,
                directory_identity,
                directory_mode,
                admission_identity,
                quiescence_identity,
            }),
        };
        fence.revalidate()?;
        Ok(fence)
    }

    #[cfg(unix)]
    fn validate_directory_metadata(
        metadata: &std::fs::Metadata,
        expected_mode: u32,
    ) -> anyhow::Result<()> {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            metadata.is_dir()
                && metadata.uid() == rustix::process::geteuid().as_raw()
                && metadata.permissions().mode() & 0o7777 == expected_mode,
            "runtime fence directory has the wrong type, owner, or mode"
        );
        Ok(())
    }

    #[cfg(unix)]
    fn validate_leaf_metadata(
        metadata: &std::fs::Metadata,
        expected_device: u64,
        label: &str,
    ) -> anyhow::Result<FileIdentity> {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            metadata.is_file()
                && metadata.uid() == rustix::process::geteuid().as_raw()
                && metadata.dev() == expected_device
                && metadata.nlink() == 1
                && metadata.permissions().mode() & 0o7777 == RUNTIME_FENCE_FILE_MODE,
            "{label} has the wrong type, owner, device, mode, or link count"
        );
        Ok(identity(metadata))
    }

    #[cfg(target_os = "linux")]
    fn open_leaf_from(directory: &cap_std::fs::Dir, name: &str) -> anyhow::Result<std::fs::File> {
        use rustix::fs::{Mode, OFlags};
        use std::os::fd::AsFd as _;
        let descriptor = crate::secure_fs::open_no_symlinks_at(
            directory.as_fd(),
            Path::new(name),
            OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
            rustix::fs::ResolveFlags::BENEATH | rustix::fs::ResolveFlags::NO_XDEV,
        )?;
        Ok(std::fs::File::from(descriptor))
    }

    fn open_leaf(&self, name: &str, expected: FileIdentity) -> anyhow::Result<std::fs::File> {
        let file = Self::open_leaf_from(&self.inner.directory, name)?;
        let actual = Self::validate_leaf_metadata(
            &file.metadata()?,
            self.inner.directory_identity.device,
            name,
        )?;
        anyhow::ensure!(actual == expected, "{name} inode was replaced");
        Ok(file)
    }

    pub fn revalidate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            std::fs::canonicalize(&self.inner.directory_path)? == self.inner.directory_path,
            "runtime fence directory path was rebound"
        );
        let current_directory = Self::open_directory_file(&self.inner.directory_path)?;
        let metadata = current_directory.metadata()?;
        Self::validate_directory_metadata(&metadata, self.inner.directory_mode)?;
        anyhow::ensure!(
            identity(&metadata) == self.inner.directory_identity,
            "runtime fence directory inode was replaced"
        );
        drop(self.open_leaf(DB_ADMISSION_LOCK, self.inner.admission_identity)?);
        drop(self.open_leaf(DB_QUIESCENCE_LOCK, self.inner.quiescence_identity)?);
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn open_directory_file(path: &Path) -> anyhow::Result<std::fs::File> {
        use rustix::fs::{Mode, OFlags};
        let descriptor = crate::secure_fs::open_no_symlinks_at(
            rustix::fs::CWD,
            path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            rustix::fs::ResolveFlags::empty(),
        )?;
        Ok(std::fs::File::from(descriptor))
    }

    fn lock_shared_admission_blocking(&self) -> anyhow::Result<std::fs::File> {
        self.revalidate()?;
        let file = self.open_leaf(DB_ADMISSION_LOCK, self.inner.admission_identity)?;
        crate::secure_fs::file_lock::lock_shared(&file)?;
        self.revalidate()?;
        Ok(file)
    }

    fn lock_shared_quiescence_blocking(&self) -> anyhow::Result<SharedQuiescenceGuard> {
        self.revalidate()?;
        let file = self.open_leaf(DB_QUIESCENCE_LOCK, self.inner.quiescence_identity)?;
        crate::secure_fs::file_lock::lock_shared(&file)?;
        self.revalidate()?;
        Ok(SharedQuiescenceGuard {
            fence: self.clone(),
            file,
        })
    }

    pub async fn acquire_one_off_shared(&self) -> anyhow::Result<SharedQuiescenceGuard> {
        let fence = self.clone();
        tokio::task::spawn_blocking(move || {
            let admission = fence.lock_shared_admission_blocking()?;
            let quiescence = fence.lock_shared_quiescence_blocking()?;
            crate::secure_fs::file_lock::unlock(&admission)?;
            fence.revalidate()?;
            Ok(quiescence)
        })
        .await
        .map_err(|error| anyhow::anyhow!(error).context("database fence task failed"))?
    }

    /// Nonblocking one-off database admission used by a backup while it keeps
    /// its durable gate alive and repeatedly attempts the exclusive turnstile.
    pub fn try_acquire_one_off_shared(&self) -> anyhow::Result<Option<SharedQuiescenceGuard>> {
        self.revalidate()?;
        let admission = self.open_leaf(DB_ADMISSION_LOCK, self.inner.admission_identity)?;
        match crate::secure_fs::file_lock::try_lock_shared(&admission) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        let file = self.open_leaf(DB_QUIESCENCE_LOCK, self.inner.quiescence_identity)?;
        let acquired = match crate::secure_fs::file_lock::try_lock_shared(&file) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
            Err(error) => return Err(error.into()),
        };
        crate::secure_fs::file_lock::unlock(&admission)?;
        self.revalidate()?;
        if !acquired {
            return Ok(None);
        }
        let guard = SharedQuiescenceGuard {
            fence: self.clone(),
            file,
        };
        guard.revalidate()?;
        Ok(Some(guard))
    }

    pub fn try_lock_exclusive_admission(&self) -> anyhow::Result<Option<ExclusiveAdmissionGuard>> {
        self.revalidate()?;
        let file = self.open_leaf(DB_ADMISSION_LOCK, self.inner.admission_identity)?;
        match crate::secure_fs::file_lock::try_lock_exclusive(&file) {
            Ok(()) => {
                self.revalidate()?;
                Ok(Some(ExclusiveAdmissionGuard {
                    fence: self.clone(),
                    file,
                }))
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn try_lock_exclusive_quiescence(
        &self,
    ) -> anyhow::Result<Option<ExclusiveQuiescenceGuard>> {
        self.revalidate()?;
        let file = self.open_leaf(DB_QUIESCENCE_LOCK, self.inner.quiescence_identity)?;
        match crate::secure_fs::file_lock::try_lock_exclusive(&file) {
            Ok(()) => {
                self.revalidate()?;
                Ok(Some(ExclusiveQuiescenceGuard {
                    fence: self.clone(),
                    file,
                }))
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Prove that both exclusive guards were acquired from this exact pinned
    /// runtime fence. Callers must retain both guards for the entire SQLite
    /// snapshot/probe operation.
    pub fn validate_exclusive_pair(
        &self,
        admission: &ExclusiveAdmissionGuard,
        quiescence: &ExclusiveQuiescenceGuard,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            Arc::ptr_eq(&self.inner, &admission.fence.inner)
                && Arc::ptr_eq(&self.inner, &quiescence.fence.inner),
            "exclusive database-fence guards belong to different runtime fences"
        );
        admission.revalidate()?;
        quiescence.revalidate()?;
        self.revalidate()
    }

    /// Temporarily take the data lock shared while an exact exclusive
    /// admission guard blocks new entrants. Backup uses this only to refresh
    /// its durable database gate while waiting for earlier readers to drain.
    pub async fn acquire_shared_quiescence_while_admission_exclusive(
        &self,
        admission: &ExclusiveAdmissionGuard,
    ) -> anyhow::Result<SharedQuiescenceGuard> {
        anyhow::ensure!(
            Arc::ptr_eq(&self.inner, &admission.fence.inner),
            "exclusive admission guard belongs to a different runtime fence"
        );
        admission.revalidate()?;
        let fence = self.clone();
        let guard = tokio::task::spawn_blocking(move || fence.lock_shared_quiescence_blocking())
            .await
            .map_err(|error| anyhow::anyhow!(error).context("database fence task failed"))??;
        admission.revalidate()?;
        guard.revalidate()?;
        Ok(guard)
    }

    pub fn directory_path(&self) -> &Path {
        &self.inner.directory_path
    }
}

impl SharedQuiescenceGuard {
    pub fn revalidate(&self) -> anyhow::Result<()> {
        self.fence.revalidate()?;
        anyhow::ensure!(
            identity(&self.file.metadata()?) == self.fence.inner.quiescence_identity,
            "held shared database-quiescence lock inode changed"
        );
        Ok(())
    }
}

impl ExclusiveAdmissionGuard {
    pub fn revalidate(&self) -> anyhow::Result<()> {
        self.fence.revalidate()?;
        anyhow::ensure!(
            identity(&self.file.metadata()?) == self.fence.inner.admission_identity,
            "held exclusive database-admission lock inode changed"
        );
        Ok(())
    }
}

impl ExclusiveQuiescenceGuard {
    pub fn revalidate(&self) -> anyhow::Result<()> {
        self.fence.revalidate()?;
        anyhow::ensure!(
            identity(&self.file.metadata()?) == self.fence.inner.quiescence_identity,
            "held exclusive database-quiescence lock inode changed"
        );
        Ok(())
    }
}

struct ManagerState {
    active: u64,
    generation: u64,
    quiescing: bool,
    shared: Option<SharedQuiescenceGuard>,
}

struct ProcessDatabaseFenceManagerInner {
    runtime: RuntimeDatabaseFence,
    state: Mutex<ManagerState>,
}

#[derive(Clone)]
pub struct ProcessDatabaseFenceManager {
    inner: Arc<ProcessDatabaseFenceManagerInner>,
}

pub struct DatabaseFenceOperation {
    manager: ProcessDatabaseFenceManager,
    generation: u64,
    finished: bool,
}

impl ProcessDatabaseFenceManager {
    pub fn new(runtime: RuntimeDatabaseFence) -> Self {
        Self {
            inner: Arc::new(ProcessDatabaseFenceManagerInner {
                runtime,
                state: Mutex::new(ManagerState {
                    active: 0,
                    generation: 0,
                    quiescing: false,
                    shared: None,
                }),
            }),
        }
    }

    pub async fn begin(&self) -> anyhow::Result<DatabaseFenceOperation> {
        {
            let state = self.inner.state.lock().expect("database fence state");
            anyhow::ensure!(!state.quiescing, "database access is quiescing for backup");
        }
        let manager = self.clone();
        let generation = tokio::task::spawn_blocking(move || {
            let admission = manager.inner.runtime.lock_shared_admission_blocking()?;
            let mut state = manager.inner.state.lock().expect("database fence state");
            anyhow::ensure!(!state.quiescing, "database access is quiescing for backup");
            // A prior last-finisher may still be awaiting SQLx's asynchronous
            // return/ping barrier. A new generation joins that already-held
            // guard; the generation bump below prevents the older finalizer
            // from dropping it early.
            if state.shared.is_none() {
                state.shared = Some(manager.inner.runtime.lock_shared_quiescence_blocking()?);
            }
            state.active = state
                .active
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("database fence active count overflows"))?;
            state.generation = state
                .generation
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("database fence generation overflows"))?;
            let generation = state.generation;
            drop(state);
            crate::secure_fs::file_lock::unlock(&admission)?;
            manager.inner.runtime.revalidate()?;
            Ok::<_, anyhow::Error>(generation)
        })
        .await
        .map_err(|error| {
            anyhow::anyhow!(error).context("database fence admission task failed")
        })??;
        Ok(DatabaseFenceOperation {
            manager: self.clone(),
            generation,
            finished: false,
        })
    }

    pub fn mark_quiescing(&self) {
        self.inner
            .state
            .lock()
            .expect("database fence state")
            .quiescing = true;
    }

    pub fn clear_quiescing(&self) {
        self.inner
            .state
            .lock()
            .expect("database fence state")
            .quiescing = false;
    }

    pub async fn finish(
        &self,
        operation: &mut DatabaseFenceOperation,
        pool: &SqlitePool,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            Arc::ptr_eq(&self.inner, &operation.manager.inner) && !operation.finished,
            "database fence operation is foreign or already finished"
        );
        operation.finished = true;
        let final_generation = {
            let mut state = self.inner.state.lock().expect("database fence state");
            anyhow::ensure!(state.active > 0, "database fence active count underflows");
            state.active -= 1;
            (state.active == 0).then_some(state.generation)
        };
        let Some(final_generation) = final_generation else {
            return Ok(());
        };
        wait_for_pool_idle(pool).await?;
        let guard = {
            let mut state = self.inner.state.lock().expect("database fence state");
            if state.active == 0
                && state.generation == final_generation
                && operation.generation <= final_generation
            {
                state.shared.take()
            } else {
                None
            }
        };
        if let Some(guard) = guard {
            guard.revalidate()?;
            drop(guard);
            self.inner.runtime.revalidate()?;
        }
        Ok(())
    }

    pub fn runtime(&self) -> &RuntimeDatabaseFence {
        &self.inner.runtime
    }

    pub async fn close_pool_when_idle(&self, pool: &SqlitePool) -> anyhow::Result<()> {
        self.mark_quiescing();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30 * 60);
        loop {
            let active = self
                .inner
                .state
                .lock()
                .expect("database fence state")
                .active;
            if active == 0 {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "timed out draining process database operations during shutdown"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let guard = self.inner.runtime.acquire_one_off_shared().await?;
        anyhow::ensure!(
            self.inner
                .state
                .lock()
                .expect("database fence state")
                .active
                == 0,
            "database operation joined after shutdown quiescence"
        );
        pool.close().await;
        let process_guard = {
            let mut state = self.inner.state.lock().expect("database fence state");
            anyhow::ensure!(state.active == 0, "database operation joined during close");
            state.shared.take()
        };
        if let Some(process_guard) = process_guard {
            process_guard.revalidate()?;
            drop(process_guard);
        }
        guard.revalidate()?;
        drop(guard);
        self.inner.runtime.revalidate()?;
        Ok(())
    }
}

impl Drop for DatabaseFenceOperation {
    fn drop(&mut self) {
        // An un-finished token intentionally leaks its process-wide shared
        // guard/count. Early release could allow a snapshot while detached
        // SQLx rollback or return-to-pool work is still running. Process death
        // closes the kernel FD and is the only automatic recovery.
        if !self.finished {
            self.manager.mark_quiescing();
        }
    }
}

pub async fn wait_for_pool_idle(pool: &SqlitePool) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30 * 60);
    loop {
        let size = pool.size();
        let idle = u32::try_from(pool.num_idle())?;
        if idle == size {
            return Ok(());
        }
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for SQLite pool return/rollback/ping quiescence"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Test-only provisioning for directly constructed `ServerConfig` values.
/// Production configuration loading requires an absolute pre-existing fence
/// and never calls this mutation helper.
pub(crate) fn provision_test_runtime_fence(directory: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        std::fs::create_dir(directory)?;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        for name in [DB_ADMISSION_LOCK, DB_QUIESCENCE_LOCK] {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(RUNTIME_FENCE_FILE_MODE)
                .open(directory.join(name))?;
            file.set_permissions(std::fs::Permissions::from_mode(RUNTIME_FENCE_FILE_MODE))?;
            file.sync_all()?;
        }
        // Test fixtures remain owner-writable so tempfile can clean them and
        // multiple independently opened Database values can share the same
        // inode safely. Production `open` still accepts exactly 0500 only.
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        std::fs::File::open(directory)?.sync_all()?;
        Ok(())
    }
    #[cfg(not(unix))]
    anyhow::bail!("runtime database fence provisioning requires Unix metadata")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    fn fence(root: &tempfile::TempDir, name: &str) -> RuntimeDatabaseFence {
        let path = root.path().join(name);
        provision_test_runtime_fence(&path).unwrap();
        RuntimeDatabaseFence::open_test(&path).unwrap()
    }

    #[tokio::test]
    async fn exclusive_pair_is_association_safe_and_drains_shared_data_guard() {
        let root = tempfile::tempdir().unwrap();
        let left = fence(&root, "left-runtime-fence");
        let right = fence(&root, "right-runtime-fence");
        let shared = left.acquire_one_off_shared().await.unwrap();
        let admission = left.try_lock_exclusive_admission().unwrap().unwrap();
        assert!(left.try_lock_exclusive_quiescence().unwrap().is_none());
        drop(shared);
        let quiescence = left.try_lock_exclusive_quiescence().unwrap().unwrap();
        left.validate_exclusive_pair(&admission, &quiescence)
            .unwrap();

        let foreign_admission = right.try_lock_exclusive_admission().unwrap().unwrap();
        let foreign_quiescence = right.try_lock_exclusive_quiescence().unwrap().unwrap();
        assert!(
            left.validate_exclusive_pair(&foreign_admission, &foreign_quiescence)
                .is_err()
        );
    }

    #[tokio::test]
    async fn exclusive_admission_turnstile_rejects_new_shared_admission() {
        let root = tempfile::tempdir().unwrap();
        let fence = fence(&root, "runtime-fence");
        let admission = fence.try_lock_exclusive_admission().unwrap().unwrap();
        assert!(fence.try_acquire_one_off_shared().unwrap().is_none());
        admission.revalidate().unwrap();
        drop(admission);
        assert!(fence.try_acquire_one_off_shared().unwrap().is_some());
    }

    #[tokio::test]
    async fn new_generation_joins_guard_while_prior_finalizer_waits_for_pool_return() {
        let root = tempfile::tempdir().unwrap();
        let runtime = fence(&root, "runtime-fence");
        let manager = ProcessDatabaseFenceManager::new(runtime.clone());
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .min_connections(0)
            .max_lifetime(None)
            .idle_timeout(None)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let connection = pool.acquire().await.unwrap();
        let mut first = manager.begin().await.unwrap();
        let first_manager = manager.clone();
        let first_pool = pool.clone();
        let first_finish =
            tokio::spawn(async move { first_manager.finish(&mut first, &first_pool).await });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let first_is_at_pool_barrier = {
                let state = manager.inner.state.lock().unwrap();
                state.active == 0 && state.shared.is_some()
            };
            if first_is_at_pool_barrier {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "first finalizer did not reach its explicit pool-return barrier"
            );
            tokio::task::yield_now().await;
        }

        let mut second = manager.begin().await.unwrap();
        drop(connection);
        manager.finish(&mut second, &pool).await.unwrap();
        first_finish.await.unwrap().unwrap();
        assert!(runtime.try_lock_exclusive_quiescence().unwrap().is_some());
        pool.close().await;
    }

    #[tokio::test]
    async fn dropped_operation_never_releases_the_kernel_guard_early() {
        let root = tempfile::tempdir().unwrap();
        let runtime = fence(&root, "runtime-fence");
        let manager = ProcessDatabaseFenceManager::new(runtime.clone());
        let operation = manager.begin().await.unwrap();
        drop(operation);
        assert!(runtime.try_lock_exclusive_quiescence().unwrap().is_none());
        assert!(manager.begin().await.is_err());
    }

    #[tokio::test]
    async fn exclusive_turnstile_blocks_new_process_joins_while_old_generation_drains() {
        let root = tempfile::tempdir().unwrap();
        let runtime = fence(&root, "runtime-fence");
        let manager = ProcessDatabaseFenceManager::new(runtime.clone());
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .min_connections(0)
            .max_lifetime(None)
            .idle_timeout(None)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let mut old = manager.begin().await.unwrap();
        // `begin` returns only after the process has acquired its shared data
        // guard, providing an explicit acquisition barrier for this test.
        assert!(runtime.try_lock_exclusive_quiescence().unwrap().is_none());
        let admission = runtime.try_lock_exclusive_admission().unwrap().unwrap();
        let join_manager = manager.clone();
        let mut new_join = tokio::spawn(async move { join_manager.begin().await });
        assert!(
            tokio::time::timeout(Duration::from_millis(40), &mut new_join)
                .await
                .is_err(),
            "new operation crossed an exclusive admission turnstile"
        );
        manager.finish(&mut old, &pool).await.unwrap();
        {
            let state = manager.inner.state.lock().unwrap();
            assert_eq!(state.active, 0);
            assert!(state.shared.is_none());
        }
        let quiescence = runtime.try_lock_exclusive_quiescence().unwrap().unwrap();
        runtime
            .validate_exclusive_pair(&admission, &quiescence)
            .unwrap();
        drop(quiescence);
        drop(admission);
        let mut joined = new_join.await.unwrap().unwrap();
        manager.finish(&mut joined, &pool).await.unwrap();
        pool.close().await;
    }

    #[tokio::test]
    async fn exclusive_turnstile_drains_a_synchronized_continuous_reader() {
        let root = tempfile::tempdir().unwrap();
        let runtime = fence(&root, "runtime-fence");
        let first_shared_acquired = Arc::new(tokio::sync::Barrier::new(2));
        let release_first_shared = Arc::new(tokio::sync::Notify::new());
        let stop = Arc::new(AtomicBool::new(false));
        let completed_reads = Arc::new(AtomicU64::new(0));
        let reader_runtime = runtime.clone();
        let reader_barrier = Arc::clone(&first_shared_acquired);
        let reader_release = Arc::clone(&release_first_shared);
        let reader_stop = Arc::clone(&stop);
        let reader_count = Arc::clone(&completed_reads);
        let reader = tokio::spawn(async move {
            let mut first = true;
            while !reader_stop.load(Ordering::SeqCst) {
                let shared = reader_runtime.acquire_one_off_shared().await.unwrap();
                if first {
                    first = false;
                    // This explicit barrier proves that SH quiescence is held
                    // before the test asserts that EX quiescence is excluded.
                    reader_barrier.wait().await;
                    reader_release.notified().await;
                }
                shared.revalidate().unwrap();
                drop(shared);
                reader_count.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
            }
        });

        first_shared_acquired.wait().await;
        assert!(runtime.try_lock_exclusive_quiescence().unwrap().is_none());
        let admission = runtime.try_lock_exclusive_admission().unwrap().unwrap();
        release_first_shared.notify_one();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let quiescence = loop {
            if let Some(guard) = runtime.try_lock_exclusive_quiescence().unwrap() {
                break guard;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "continuous shared readers prevented quiescence after admission closed"
            );
            tokio::task::yield_now().await;
        };
        runtime
            .validate_exclusive_pair(&admission, &quiescence)
            .unwrap();
        assert_eq!(completed_reads.load(Ordering::SeqCst), 1);
        stop.store(true, Ordering::SeqCst);
        drop(quiescence);
        drop(admission);
        tokio::time::timeout(Duration::from_secs(2), reader)
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn test_fence_is_reopenable_and_one_drop_does_not_invalidate_another() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runtime-fence");
        provision_test_runtime_fence(&path).unwrap();
        let first = RuntimeDatabaseFence::open_test(&path).unwrap();
        let second = RuntimeDatabaseFence::open_test(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o700,
            "test fixture must remain owner-removable"
        );
        drop(first);
        second.revalidate().unwrap();
        drop(second);
        RuntimeDatabaseFence::open_test(&path)
            .unwrap()
            .revalidate()
            .unwrap();
    }

    #[tokio::test]
    async fn fence_rejects_special_mode_bits_hardlinks_and_leaf_replacement() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runtime-fence");
        provision_test_runtime_fence(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o1500)).unwrap();
        assert!(RuntimeDatabaseFence::open(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();

        let admission_path = path.join(DB_ADMISSION_LOCK);
        std::fs::set_permissions(&admission_path, std::fs::Permissions::from_mode(0o2400)).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500)).unwrap();
        assert!(RuntimeDatabaseFence::open(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&admission_path, std::fs::Permissions::from_mode(0o400)).unwrap();

        let hardlink = path.join("admission-hardlink");
        std::fs::hard_link(&admission_path, &hardlink).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500)).unwrap();
        assert!(RuntimeDatabaseFence::open(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_file(&hardlink).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500)).unwrap();

        let runtime = RuntimeDatabaseFence::open(&path).unwrap();
        let shared = runtime.acquire_one_off_shared().await.unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let old_quiescence = path.join("old-quiescence.lock");
        std::fs::rename(path.join(DB_QUIESCENCE_LOCK), &old_quiescence).unwrap();
        let replacement = std::fs::File::create(path.join(DB_QUIESCENCE_LOCK)).unwrap();
        replacement
            .set_permissions(std::fs::Permissions::from_mode(0o400))
            .unwrap();
        drop(replacement);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500)).unwrap();
        assert!(runtime.revalidate().is_err());
        assert!(shared.revalidate().is_err());
        drop(shared);
        drop(runtime);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_file(path.join(DB_QUIESCENCE_LOCK)).unwrap();
        std::fs::remove_file(old_quiescence).unwrap();
    }
}
