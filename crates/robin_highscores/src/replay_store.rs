//! Opaque, content-addressed quarantine storage for submitted replay bytes.
//!
//! The API process never decodes or canonicalizes this attacker-controlled
//! content. `open_verified` means only that the regular file's length and
//! SHA-256 match the signed artifact identity. Semantic and canonical replay
//! admission belongs exclusively to the systemd-contained verifier child.

use bytes::Bytes;
use futures_util::{Stream, StreamExt as _};
use sha2::{Digest as _, Sha256};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _, AsyncWriteExt as _};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("replay body is empty")]
    Empty,
    #[error("replay exceeds the {limit}-byte admission limit")]
    TooLarge { limit: u64 },
    #[error("replay length mismatch: signed {expected}, received {actual}")]
    LengthMismatch { expected: u64, actual: u64 },
    #[error("replay SHA-256 does not match the signed envelope")]
    DigestMismatch,
    #[error("replay upload failed: {0}")]
    Upload(String),
    #[error("replay storage I/O: {0}")]
    Io(#[from] std::io::Error),
}

impl StoreError {
    /// Stable classification which deliberately excludes upload text and
    /// filesystem paths from logs.
    pub const fn safe_log_code(&self) -> &'static str {
        match self {
            Self::Empty => "replay_empty",
            Self::TooLarge { .. } => "replay_too_large",
            Self::LengthMismatch { .. } => "replay_length_mismatch",
            Self::DigestMismatch => "replay_digest_mismatch",
            Self::Upload(_) => "replay_upload",
            Self::Io(_) => "replay_storage_io",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReplayStore {
    root: PathBuf,
    root_dir: Arc<cap_std::fs::Dir>,
    max_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredReplay {
    pub sha256: [u8; 32],
    pub bytes: u64,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayInventoryEntry {
    pub sha256: [u8; 32],
    pub bytes: u64,
    pub modified_at_unix_ms: u64,
}

impl ReplayStore {
    pub async fn create(root: PathBuf, max_bytes: u64) -> Result<Self, StoreError> {
        match tokio::fs::symlink_metadata(&root).await {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(StoreError::Io(std::io::Error::other(
                        "replay root must be a real directory, not a symlink",
                    )));
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let create_root = root.clone();
                crate::physical_work::spawn_blocking(move || std::fs::create_dir(create_root))
                    .await
                    .map_err(std::io::Error::other)??;
            }
            Err(error) => return Err(StoreError::Io(error)),
        }
        let metadata = tokio::fs::symlink_metadata(&root).await?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StoreError::Io(std::io::Error::other(
                "replay root must be a real directory, not a symlink",
            )));
        }
        set_private_directory_permissions(&root).await?;
        let pinned_path = root.clone();
        let root_dir = crate::physical_work::spawn_blocking(move || {
            crate::secure_fs::pin_private_root(&pinned_path)
        })
        .await
        .map_err(|error| std::io::Error::other(error))??;
        Ok(Self {
            root,
            root_dir: Arc::new(root_dir),
            max_bytes,
        })
    }

    pub fn path_for_digest(&self, digest: &[u8; 32]) -> PathBuf {
        let hex = hex::encode(digest);
        self.root
            .join(&hex[0..2])
            .join(&hex[2..4])
            .join(format!("{hex}.rhrec"))
    }

    pub(crate) fn storage_volume(
        &self,
    ) -> Result<crate::storage_admission::StorageVolume, std::io::Error> {
        crate::storage_admission::StorageVolume::from_pinned_dir("replay", &self.root_dir)
    }

    pub async fn readiness_check(&self) -> Result<(), StoreError> {
        crate::secure_fs::probe_writable_root(Arc::clone(&self.root_dir)).await?;
        Ok(())
    }

    async fn ensure_shard(&self, digest: &[u8; 32]) -> Result<Arc<cap_std::fs::Dir>, StoreError> {
        let encoded = hex::encode(digest);
        let first_name = encoded[..2].to_owned();
        let second_name = encoded[2..4].to_owned();
        let root = Arc::clone(&self.root_dir);
        Ok(Arc::new(
            crate::physical_work::spawn_blocking(move || {
                let first = crate::secure_fs::ensure_private_dir(&root, Path::new(&first_name))?;
                crate::secure_fs::ensure_private_dir(&first, Path::new(&second_name))
            })
            .await
            .map_err(|error| std::io::Error::other(error))??,
        ))
    }

    async fn open_shard(&self, digest: &[u8; 32]) -> Result<Arc<cap_std::fs::Dir>, StoreError> {
        let encoded = hex::encode(digest);
        let first_name = encoded[..2].to_owned();
        let second_name = encoded[2..4].to_owned();
        let root = Arc::clone(&self.root_dir);
        Ok(Arc::new(
            crate::physical_work::spawn_blocking(move || {
                let first = crate::secure_fs::open_private_dir(&root, Path::new(&first_name))?;
                crate::secure_fs::open_private_dir(&first, Path::new(&second_name))
            })
            .await
            .map_err(|error| std::io::Error::other(error))??,
        ))
    }

    fn object_name(digest: &[u8; 32]) -> String {
        format!("{}.rhrec", hex::encode(digest))
    }

    pub async fn store_stream<S, E>(
        &self,
        mut stream: S,
        expected_digest: [u8; 32],
        expected_bytes: u64,
    ) -> Result<StoredReplay, StoreError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: std::fmt::Display,
    {
        if expected_bytes == 0 {
            return Err(StoreError::Empty);
        }
        if expected_bytes > self.max_bytes {
            return Err(StoreError::TooLarge {
                limit: self.max_bytes,
            });
        }

        let final_path = self.path_for_digest(&expected_digest);
        let shard = self.ensure_shard(&expected_digest).await?;
        let temp_name = format!(".upload-{}.tmp", uuid::Uuid::now_v7());
        let final_name = Self::object_name(&expected_digest);
        let create_shard = Arc::clone(&shard);
        let create_name = temp_name.clone();
        let temp = crate::physical_work::spawn_blocking(move || {
            crate::secure_fs::create_private_file(&create_shard, Path::new(&create_name))
        })
        .await
        .map_err(|error| std::io::Error::other(error))??;
        let mut temp = tokio::fs::File::from_std(temp);
        let mut hasher = Sha256::new();
        let mut received = 0_u64;
        let result = async {
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|error| StoreError::Upload(error.to_string()))?;
                received =
                    received
                        .checked_add(chunk.len() as u64)
                        .ok_or(StoreError::TooLarge {
                            limit: self.max_bytes,
                        })?;
                if received > self.max_bytes || received > expected_bytes {
                    return Err(StoreError::TooLarge {
                        limit: self.max_bytes.min(expected_bytes),
                    });
                }
                hasher.update(&chunk);
                temp.write_all(&chunk).await?;
            }
            if received != expected_bytes {
                return Err(StoreError::LengthMismatch {
                    expected: expected_bytes,
                    actual: received,
                });
            }
            let actual_digest: [u8; 32] = hasher.finalize().into();
            if actual_digest != expected_digest {
                return Err(StoreError::DigestMismatch);
            }
            #[cfg(unix)]
            temp.set_permissions(std::fs::Permissions::from_mode(
                crate::secure_fs::SHARED_IMMUTABLE_FILE_MODE,
            ))
            .await?;
            temp.sync_all().await?;
            drop(temp);

            if !crate::secure_fs::link_immutable_object(
                Arc::clone(&shard),
                temp_name.clone(),
                final_name.clone(),
            )
            .await?
            {
                let opened = self.open_verified(&expected_digest, received).await?;
                if opened.metadata().await?.len() != received {
                    return Err(StoreError::Io(std::io::Error::other(
                        "content-addressed replay is not the expected regular file",
                    )));
                }
            }
            crate::secure_fs::remove_temporary_and_sync(Arc::clone(&shard), temp_name.clone())
                .await?;
            drop(self.open_verified(&expected_digest, received).await?);
            Ok(StoredReplay {
                sha256: actual_digest,
                bytes: received,
                path: final_path,
            })
        }
        .await;

        if result.is_err() {
            let cleanup_shard = Arc::clone(&shard);
            let _ =
                crate::physical_work::spawn_blocking(move || cleanup_shard.remove_file(&temp_name))
                    .await;
        }
        result
    }

    pub async fn open_verified(
        &self,
        digest: &[u8; 32],
        expected_bytes: u64,
    ) -> Result<tokio::fs::File, StoreError> {
        let shard = self.open_shard(digest).await?;
        self.open_pinned_verified(shard, Self::object_name(digest), digest, expected_bytes)
            .await
    }

    async fn open_pinned_verified(
        &self,
        directory: Arc<cap_std::fs::Dir>,
        name: String,
        digest: &[u8; 32],
        expected_bytes: u64,
    ) -> Result<tokio::fs::File, StoreError> {
        let opened_directory = Arc::clone(&directory);
        let opened_name = name.clone();
        let file = crate::physical_work::spawn_blocking(move || {
            crate::secure_fs::open_regular_file(&opened_directory, Path::new(&opened_name))
        })
        .await
        .map_err(|error| std::io::Error::other(error))??;
        let mut file = tokio::fs::File::from_std(file);
        let opened = file.metadata().await?;
        if !opened.is_file() || opened.len() != expected_bytes {
            return Err(StoreError::Io(std::io::Error::other(
                "opened replay metadata does not match the database",
            )));
        }
        #[cfg(unix)]
        if opened.permissions().mode() & 0o222 != 0 {
            return Err(StoreError::Io(std::io::Error::other(
                "stored replay must be read-only",
            )));
        }
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        let actual: [u8; 32] = hasher.finalize().into();
        if &actual != digest {
            return Err(StoreError::Io(std::io::Error::other(
                "stored replay digest does not match its content address",
            )));
        }
        file.seek(std::io::SeekFrom::Start(0)).await?;
        Ok(file)
    }

    /// Atomically move a claimed object to a token-specific quarantine name.
    /// A collector crash is resumed with the same token; no stale collector
    /// ever operates on the canonical path after it can be reused.
    pub async fn quarantine_for_purge(
        &self,
        digest: &[u8; 32],
        expected_bytes: u64,
        claim_token: &str,
    ) -> Result<PathBuf, StoreError> {
        if claim_token.is_empty()
            || claim_token.len() > 64
            || !claim_token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(StoreError::Io(std::io::Error::other(
                "invalid internal replay purge token",
            )));
        }
        let purge = {
            let root = Arc::clone(&self.root_dir);
            Arc::new(
                crate::physical_work::spawn_blocking(move || {
                    crate::secure_fs::ensure_private_dir(&root, Path::new(".purge"))
                })
                .await
                .map_err(|error| std::io::Error::other(error))??,
            )
        };
        let quarantine_name = format!("{}-{}.rhrec", claim_token, hex::encode(digest));
        match self
            .open_pinned_verified(
                Arc::clone(&purge),
                quarantine_name.clone(),
                digest,
                expected_bytes,
            )
            .await
        {
            Ok(file) => {
                drop(file);
                return Ok(self.root.join(".purge").join(quarantine_name));
            }
            Err(StoreError::Io(error)) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let shard = match self.open_shard(digest).await {
            Ok(shard) => shard,
            Err(StoreError::Io(error)) if error.kind() == ErrorKind::NotFound => {
                return Ok(self.root.join(".purge").join(quarantine_name));
            }
            Err(error) => return Err(error),
        };
        match self.open_verified(digest, expected_bytes).await {
            Ok(file) => drop(file),
            Err(StoreError::Io(error)) if error.kind() == ErrorKind::NotFound => {
                return Ok(self.root.join(".purge").join(quarantine_name));
            }
            Err(error) => return Err(error),
        }
        let source_name = Self::object_name(digest);
        let rename_shard = Arc::clone(&shard);
        let rename_purge = Arc::clone(&purge);
        let rename_destination = quarantine_name.clone();
        crate::physical_work::spawn_blocking(move || {
            rename_shard.rename(&source_name, &rename_purge, &rename_destination)?;
            crate::secure_fs::sync_private_dir(&rename_shard)?;
            crate::secure_fs::sync_private_dir(&rename_purge)
        })
        .await
        .map_err(|error| std::io::Error::other(error))??;
        Ok(self.root.join(".purge").join(quarantine_name))
    }

    pub async fn remove_quarantined(&self, path: &Path) -> Result<(), StoreError> {
        let quarantine_directory = self.root.join(".purge");
        if path.parent() != Some(quarantine_directory.as_path()) {
            return Err(StoreError::Io(std::io::Error::other(
                "purge path is outside the quarantine directory",
            )));
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| std::io::Error::other("replay purge name is not UTF-8"))?
            .to_owned();
        let root = Arc::clone(&self.root_dir);
        crate::physical_work::spawn_blocking(move || {
            let purge = match crate::secure_fs::open_private_dir(&root, Path::new(".purge")) {
                Ok(purge) => purge,
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error),
            };
            match crate::secure_fs::open_regular_file(&purge, Path::new(&name)) {
                Ok(file) => drop(file),
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error),
            }
            purge.remove_file(&name)?;
            crate::secure_fs::sync_private_dir(&purge)
        })
        .await
        .map_err(|error| std::io::Error::other(error))??;
        Ok(())
    }

    /// Enumerate canonical objects for startup/periodic DB reconciliation.
    /// Quarantine and temporary files are deliberately excluded.
    pub async fn inventory(
        &self,
        maximum_entries: usize,
    ) -> Result<Vec<ReplayInventoryEntry>, StoreError> {
        self.inventory_page(None, maximum_entries).await
    }

    /// Deterministic digest-ordered inventory page. The traversal itself may
    /// be unordered, but only the smallest bounded keys after `cursor` are
    /// retained, so repeated batches cannot starve later objects.
    pub async fn inventory_page(
        &self,
        cursor: Option<[u8; 32]>,
        maximum_entries: usize,
    ) -> Result<Vec<ReplayInventoryEntry>, StoreError> {
        if maximum_entries == 0 {
            return Ok(Vec::new());
        }
        let root = Arc::clone(&self.root_dir);
        let entries = crate::physical_work::spawn_blocking(move || {
            let mut entries = std::collections::BTreeMap::new();
            for first_entry in root.entries()? {
                let first_entry = first_entry?;
                let first_name = first_entry.file_name();
                let Some(first_name) = first_name.to_str() else {
                    continue;
                };
                if first_name.len() != 2
                    || !first_name
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    continue;
                }
                if !first_entry.file_type()?.is_dir() {
                    return Err(std::io::Error::other(
                        "replay inventory contains an unsafe first shard",
                    ));
                }
                let first = crate::secure_fs::open_private_dir(&root, Path::new(first_name))?;
                for second_entry in first.entries()? {
                    let second_entry = second_entry?;
                    let second_name = second_entry.file_name();
                    let Some(second_name) = second_name.to_str() else {
                        continue;
                    };
                    if second_name.len() != 2
                        || !second_name
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    {
                        continue;
                    }
                    if !second_entry.file_type()?.is_dir() {
                        return Err(std::io::Error::other(
                            "replay inventory contains an unsafe second shard",
                        ));
                    }
                    let second =
                        crate::secure_fs::open_private_dir(&first, Path::new(second_name))?;
                    for object_entry in second.entries()? {
                        let object_entry = object_entry?;
                        let name = object_entry.file_name();
                        let Some(name) = name.to_str() else {
                            continue;
                        };
                        let Some(hex_digest) = name.strip_suffix(".rhrec") else {
                            continue;
                        };
                        if hex_digest.len() != 64
                            || !hex_digest
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                            || &hex_digest[0..2] != first_name
                            || &hex_digest[2..4] != second_name
                        {
                            continue;
                        }
                        if !object_entry.file_type()?.is_file() {
                            return Err(std::io::Error::other(
                                "replay inventory contains an unsafe object",
                            ));
                        }
                        let metadata = object_entry.metadata()?;
                        let sha256: [u8; 32] = hex::decode(hex_digest)
                            .map_err(std::io::Error::other)?
                            .try_into()
                            .map_err(|_| std::io::Error::other("invalid inventory digest"))?;
                        if cursor.is_some_and(|cursor| sha256 <= cursor) {
                            continue;
                        }
                        let modified_at_unix_ms = metadata
                            .modified()?
                            .into_std()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_err(std::io::Error::other)?
                            .as_millis()
                            .try_into()
                            .map_err(|_| {
                                std::io::Error::other("replay modification time is too large")
                            })?;
                        entries.insert(
                            sha256,
                            ReplayInventoryEntry {
                                sha256,
                                bytes: metadata.len(),
                                modified_at_unix_ms,
                            },
                        );
                        if entries.len() > maximum_entries {
                            entries.pop_last();
                        }
                    }
                }
            }
            Ok::<_, std::io::Error>(entries.into_values().collect::<Vec<_>>())
        })
        .await
        .map_err(|error| std::io::Error::other(error))??;
        for entry in &entries {
            drop(self.open_verified(&entry.sha256, entry.bytes).await?);
        }
        Ok(entries)
    }
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

async fn set_private_directory_permissions(path: &Path) -> Result<(), StoreError> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        let fd = openat2(
            rustix::fs::CWD,
            path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(std::io::Error::from)?;
        let directory = std::fs::File::from(fd);
        if !directory.metadata()?.is_dir() {
            return Err(StoreError::Io(std::io::Error::other(
                "replay path is not a directory",
            )));
        }
        if directory.metadata()?.permissions().mode() & 0o7777
            != crate::secure_fs::SHARED_PRIVATE_DIRECTORY_MODE
        {
            directory.set_permissions(std::fs::Permissions::from_mode(
                crate::secure_fs::SHARED_PRIVATE_DIRECTORY_MODE,
            ))?;
        }
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    if std::fs::metadata(path)?.permissions().mode() & 0o7777
        != crate::secure_fs::SHARED_PRIVATE_DIRECTORY_MODE
    {
        std::fs::set_permissions(
            path,
            std::fs::Permissions::from_mode(crate::secure_fs::SHARED_PRIVATE_DIRECTORY_MODE),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    #[tokio::test]
    async fn stores_verified_content_by_digest_and_deduplicates() {
        let temp = tempfile::tempdir().unwrap();
        let store = ReplayStore::create(temp.path().join("replays"), 100)
            .await
            .unwrap();
        let bytes = Bytes::from_static(b"full replay");
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        for _ in 0..2 {
            let stored = store
                .store_stream(stream::iter([Ok::<_, &str>(bytes.clone())]), digest, 11)
                .await
                .unwrap();
            assert_eq!(stored.path, store.path_for_digest(&digest));
        }
        assert_eq!(
            tokio::fs::read(store.path_for_digest(&digest))
                .await
                .unwrap(),
            bytes
        );
        #[cfg(unix)]
        {
            assert_eq!(
                std::fs::metadata(temp.path().join("replays"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o7777,
                crate::secure_fs::SHARED_PRIVATE_DIRECTORY_MODE
            );
            assert_eq!(
                std::fs::metadata(store.path_for_digest(&digest))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                crate::secure_fs::SHARED_IMMUTABLE_FILE_MODE
            );
        }
    }

    #[tokio::test]
    async fn wrong_retry_body_preserves_existing_object_and_cleans_staging_file() {
        let temp = tempfile::tempdir().unwrap();
        let store = ReplayStore::create(temp.path().join("replays"), 100)
            .await
            .unwrap();
        let original = Bytes::from_static(b"exact replay");
        let digest: [u8; 32] = Sha256::digest(&original).into();
        store
            .store_stream(
                stream::iter([Ok::<_, &str>(original.clone())]),
                digest,
                original.len() as u64,
            )
            .await
            .unwrap();
        let wrong = Bytes::from(vec![b'x'; original.len()]);
        assert!(matches!(
            store
                .store_stream(
                    stream::iter([Ok::<_, &str>(wrong)]),
                    digest,
                    original.len() as u64,
                )
                .await,
            Err(StoreError::DigestMismatch)
        ));
        assert_eq!(
            tokio::fs::read(store.path_for_digest(&digest))
                .await
                .unwrap(),
            original
        );
        let mut shard = tokio::fs::read_dir(store.path_for_digest(&digest).parent().unwrap())
            .await
            .unwrap();
        while let Some(entry) = shard.next_entry().await.unwrap() {
            assert!(
                !entry.file_name().to_string_lossy().starts_with(".upload-"),
                "a failed exact retry left a replay staging file"
            );
        }
    }

    #[tokio::test]
    async fn mismatch_and_oversize_never_publish_a_file() {
        let temp = tempfile::tempdir().unwrap();
        let store = ReplayStore::create(temp.path().join("replays"), 4)
            .await
            .unwrap();
        let digest = [9; 32];
        let error = store
            .store_stream(
                stream::iter([Ok::<_, &str>(Bytes::from_static(b"hello"))]),
                digest,
                5,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::TooLarge { .. }));
        assert!(!store.path_for_digest(&digest).exists());
    }

    #[tokio::test]
    async fn same_length_corruption_is_rejected_on_open() {
        let temp = tempfile::tempdir().unwrap();
        let store = ReplayStore::create(temp.path().join("replays"), 100)
            .await
            .unwrap();
        let bytes = Bytes::from_static(b"abcd");
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        store
            .store_stream(stream::iter([Ok::<_, &str>(bytes)]), digest, 4)
            .await
            .unwrap();
        #[cfg(unix)]
        tokio::fs::set_permissions(
            store.path_for_digest(&digest),
            std::fs::Permissions::from_mode(0o600),
        )
        .await
        .unwrap();
        tokio::fs::write(store.path_for_digest(&digest), b"wxyz")
            .await
            .unwrap();
        assert!(store.open_verified(&digest, 4).await.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_object_is_never_opened() {
        let temp = tempfile::tempdir().unwrap();
        let store = ReplayStore::create(temp.path().join("replays"), 100)
            .await
            .unwrap();
        let bytes = b"abcd";
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        let path = store.path_for_digest(&digest);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        let target = temp.path().join("target");
        tokio::fs::write(&target, bytes).await.unwrap();
        std::os::unix::fs::symlink(target, &path).unwrap();
        assert!(store.open_verified(&digest, 4).await.is_err());
    }

    #[tokio::test]
    async fn inventory_pages_visit_every_digest_in_order() {
        let temp = tempfile::tempdir().unwrap();
        let store = ReplayStore::create(temp.path().join("replays"), 100)
            .await
            .unwrap();
        let mut expected = Vec::new();
        for byte in [9_u8, 2, 7, 1, 5] {
            let bytes = Bytes::from(vec![byte]);
            let digest: [u8; 32] = Sha256::digest(&bytes).into();
            store
                .store_stream(stream::iter([Ok::<_, &str>(bytes)]), digest, 1)
                .await
                .unwrap();
            expected.push(digest);
        }
        expected.sort_unstable();

        let mut actual = Vec::new();
        let mut cursor = None;
        loop {
            let page = store.inventory_page(cursor, 2).await.unwrap();
            if page.is_empty() {
                break;
            }
            cursor = page.last().map(|entry| entry.sha256);
            actual.extend(page.into_iter().map(|entry| entry.sha256));
        }
        assert_eq!(actual, expected);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn replay_root_rejects_a_symlinked_ancestor() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        tokio::fs::create_dir(&real).await.unwrap();
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(
            ReplayStore::create(link.join("replays"), 100)
                .await
                .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn pinned_replay_root_survives_ancestor_swap_without_touching_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let live_parent = temp.path().join("live");
        tokio::fs::create_dir(&live_parent).await.unwrap();
        let root = live_parent.join("replays");
        let store = ReplayStore::create(root.clone(), 100).await.unwrap();
        let original = Bytes::from_static(b"original replay");
        let original_digest: [u8; 32] = Sha256::digest(&original).into();
        store
            .store_stream(
                stream::iter([Ok::<_, &str>(original.clone())]),
                original_digest,
                original.len() as u64,
            )
            .await
            .unwrap();

        let displaced_parent = temp.path().join("displaced");
        tokio::fs::rename(&live_parent, &displaced_parent)
            .await
            .unwrap();
        tokio::fs::create_dir_all(&root).await.unwrap();
        let sentinel = root.join("outside-sentinel");
        tokio::fs::write(&sentinel, b"untouched").await.unwrap();
        let replacement_object = store.path_for_digest(&original_digest);
        tokio::fs::create_dir_all(replacement_object.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&replacement_object, vec![b'x'; original.len()])
            .await
            .unwrap();
        tokio::fs::set_permissions(
            &replacement_object,
            std::fs::Permissions::from_mode(crate::secure_fs::SHARED_IMMUTABLE_FILE_MODE),
        )
        .await
        .unwrap();

        let mut opened = store
            .open_verified(&original_digest, original.len() as u64)
            .await
            .unwrap();
        let mut opened_bytes = Vec::new();
        opened.read_to_end(&mut opened_bytes).await.unwrap();
        assert_eq!(opened_bytes, original);

        let second = Bytes::from_static(b"second replay");
        let second_digest: [u8; 32] = Sha256::digest(&second).into();
        store
            .store_stream(
                stream::iter([Ok::<_, &str>(second.clone())]),
                second_digest,
                second.len() as u64,
            )
            .await
            .unwrap();
        assert!(!store.path_for_digest(&second_digest).exists());
        let relative_second = store
            .path_for_digest(&second_digest)
            .strip_prefix(&root)
            .unwrap()
            .to_owned();
        assert!(
            displaced_parent
                .join("replays")
                .join(relative_second)
                .is_file()
        );

        let quarantine = store
            .quarantine_for_purge(&original_digest, original.len() as u64, "claim")
            .await
            .unwrap();
        store.remove_quarantined(&quarantine).await.unwrap();
        assert_eq!(tokio::fs::read(&sentinel).await.unwrap(), b"untouched");
        assert_eq!(
            tokio::fs::read(&replacement_object).await.unwrap(),
            vec![b'x'; original.len()]
        );
    }
}
