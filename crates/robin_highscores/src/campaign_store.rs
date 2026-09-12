//! Opaque, content-addressed campaign-state storage for verifier chaining.
//!
//! The service never decodes campaign bytes. It only enforces byte limits,
//! SHA-256 identity, private permissions, and non-symlink file access.

use bytes::Bytes;
use futures_util::{Stream, StreamExt as _};
use sha2::{Digest as _, Sha256};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _, AsyncWriteExt as _};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

#[derive(Debug, thiserror::Error)]
pub enum CampaignStoreError {
    #[error("campaign state is empty")]
    Empty,
    #[error("campaign state exceeds the {limit}-byte limit")]
    TooLarge { limit: u64 },
    #[error("campaign state length mismatch: signed {expected}, received {actual}")]
    LengthMismatch { expected: u64, actual: u64 },
    #[error("campaign state digest does not match its expected identity")]
    DigestMismatch,
    #[error("campaign state upload failed: {0}")]
    Upload(String),
    #[error("campaign state storage I/O: {0}")]
    Io(#[from] std::io::Error),
}

impl CampaignStoreError {
    /// Stable classification which never includes an operator filesystem
    /// location.
    pub const fn safe_log_code(&self) -> &'static str {
        match self {
            Self::Empty => "campaign_state_empty",
            Self::TooLarge { .. } => "campaign_state_too_large",
            Self::LengthMismatch { .. } => "campaign_state_length_mismatch",
            Self::DigestMismatch => "campaign_state_digest_mismatch",
            Self::Upload(_) => "campaign_state_upload",
            Self::Io(_) => "campaign_state_storage_io",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CampaignStore {
    root: PathBuf,
    root_dir: Arc<cap_std::fs::Dir>,
    max_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignInventoryEntry {
    pub sha256: [u8; 32],
    pub bytes: u64,
}

impl CampaignStore {
    pub async fn create(root: PathBuf, max_bytes: u64) -> Result<Self, CampaignStoreError> {
        match tokio::fs::symlink_metadata(&root).await {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(CampaignStoreError::Io(std::io::Error::other(
                    "campaign store root must be a real directory",
                )));
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let create_root = root.clone();
                crate::physical_work::spawn_blocking(move || std::fs::create_dir(create_root))
                    .await
                    .map_err(std::io::Error::other)??;
            }
            Err(error) => return Err(error.into()),
        }
        set_mode(&root, crate::secure_fs::SHARED_PRIVATE_DIRECTORY_MODE).await?;
        let pinned_path = root.clone();
        let root_dir = crate::physical_work::spawn_blocking(move || {
            crate::secure_fs::pin_private_root(&pinned_path)
        })
        .await
        .map_err(std::io::Error::other)??;
        Ok(Self {
            root,
            root_dir: Arc::new(root_dir),
            max_bytes,
        })
    }

    pub fn path_for_digest(&self, digest: &[u8; 32]) -> PathBuf {
        let hex = hex::encode(digest);
        self.root.join(&hex[..2]).join(format!("{hex}.campaign"))
    }

    pub(crate) fn storage_volume(
        &self,
    ) -> Result<crate::storage_admission::StorageVolume, std::io::Error> {
        crate::storage_admission::StorageVolume::from_pinned_dir("campaign", &self.root_dir)
    }

    pub async fn readiness_check(&self) -> Result<(), CampaignStoreError> {
        crate::secure_fs::probe_writable_root(Arc::clone(&self.root_dir)).await?;
        Ok(())
    }

    async fn ensure_shard(
        &self,
        digest: &[u8; 32],
    ) -> Result<Arc<cap_std::fs::Dir>, CampaignStoreError> {
        let shard = hex::encode(&digest[..1]);
        let root = Arc::clone(&self.root_dir);
        Ok(Arc::new(
            crate::physical_work::spawn_blocking(move || {
                crate::secure_fs::ensure_private_dir(&root, Path::new(&shard))
            })
            .await
            .map_err(std::io::Error::other)??,
        ))
    }

    fn object_name(digest: &[u8; 32]) -> String {
        format!("{}.campaign", hex::encode(digest))
    }

    pub async fn import_bytes(
        &self,
        digest: &[u8; 32],
        bytes: &[u8],
    ) -> Result<PathBuf, CampaignStoreError> {
        if bytes.is_empty() {
            return Err(CampaignStoreError::Empty);
        }
        if bytes.len() as u64 > self.max_bytes {
            return Err(CampaignStoreError::TooLarge {
                limit: self.max_bytes,
            });
        }
        if Sha256::digest(bytes).as_slice() != digest {
            return Err(CampaignStoreError::DigestMismatch);
        }
        let final_path = self.path_for_digest(digest);
        let shard = self.ensure_shard(digest).await?;
        let temporary = format!(".import-{}.tmp", uuid::Uuid::now_v7());
        let final_name = Self::object_name(digest);
        let create_shard = Arc::clone(&shard);
        let create_name = temporary.clone();
        let bytes = bytes.to_vec();
        // Keep all mutation in one tracked physical job. Tokio file writes can
        // themselves enqueue blocking work that outlives a dropped File waiter.
        crate::physical_work::spawn_blocking(move || {
            use std::io::Write as _;
            let mut file =
                crate::secure_fs::create_private_file(&create_shard, Path::new(&create_name))?;
            file.write_all(&bytes)?;
            #[cfg(unix)]
            file.set_permissions(std::fs::Permissions::from_mode(
                crate::secure_fs::SHARED_IMMUTABLE_FILE_MODE,
            ))?;
            file.sync_all()
        })
        .await
        .map_err(std::io::Error::other)??;
        let result = async {
            if !crate::secure_fs::link_immutable_object(
                Arc::clone(&shard),
                temporary.clone(),
                final_name.clone(),
            )
            .await?
            {
                drop(self.open_verified(digest).await?);
            }
            crate::secure_fs::remove_temporary_and_sync(Arc::clone(&shard), temporary.clone())
                .await?;
            Ok(final_path)
        }
        .await;
        if result.is_err() {
            let cleanup_shard = Arc::clone(&shard);
            let _ =
                crate::physical_work::spawn_blocking(move || cleanup_shard.remove_file(&temporary))
                    .await;
        }
        result
    }

    /// Stream an opaque campaign artifact directly into the private
    /// content-addressed store while enforcing its exact signed identity.
    /// The service deliberately does not decode campaign bytes.
    pub async fn store_stream<S, E>(
        &self,
        mut stream: S,
        expected_digest: [u8; 32],
        expected_bytes: u64,
    ) -> Result<CampaignInventoryEntry, CampaignStoreError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: std::fmt::Display,
    {
        if expected_bytes == 0 {
            return Err(CampaignStoreError::Empty);
        }
        if expected_bytes > self.max_bytes {
            return Err(CampaignStoreError::TooLarge {
                limit: self.max_bytes,
            });
        }
        let shard = self.ensure_shard(&expected_digest).await?;
        let temporary = format!(".import-{}.tmp", uuid::Uuid::now_v7());
        let final_name = Self::object_name(&expected_digest);
        let create_shard = Arc::clone(&shard);
        let create_name = temporary.clone();
        let file = crate::physical_work::spawn_blocking(move || {
            crate::secure_fs::create_private_file(&create_shard, Path::new(&create_name))
        })
        .await
        .map_err(std::io::Error::other)??;
        let mut file = tokio::fs::File::from_std(file);
        let result = async {
            let mut hasher = Sha256::new();
            let mut received = 0_u64;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|error| CampaignStoreError::Upload(error.to_string()))?;
                received = received.checked_add(chunk.len() as u64).ok_or({
                    CampaignStoreError::TooLarge {
                        limit: self.max_bytes,
                    }
                })?;
                if received > expected_bytes {
                    return Err(CampaignStoreError::LengthMismatch {
                        expected: expected_bytes,
                        actual: received,
                    });
                }
                hasher.update(&chunk);
                file.write_all(&chunk).await?;
            }
            if received != expected_bytes {
                return Err(CampaignStoreError::LengthMismatch {
                    expected: expected_bytes,
                    actual: received,
                });
            }
            let actual_digest: [u8; 32] = hasher.finalize().into();
            if actual_digest != expected_digest {
                return Err(CampaignStoreError::DigestMismatch);
            }
            #[cfg(unix)]
            file.set_permissions(std::fs::Permissions::from_mode(
                crate::secure_fs::SHARED_IMMUTABLE_FILE_MODE,
            ))
            .await?;
            file.sync_all().await?;
            drop(file);
            if !crate::secure_fs::link_immutable_object(
                Arc::clone(&shard),
                temporary.clone(),
                final_name.clone(),
            )
            .await?
            {
                let opened = self.open_verified(&expected_digest).await?;
                if opened.metadata().await?.len() != expected_bytes {
                    return Err(CampaignStoreError::Io(std::io::Error::other(
                        "content-addressed campaign length conflicts with signed artifact",
                    )));
                }
            }
            crate::secure_fs::remove_temporary_and_sync(Arc::clone(&shard), temporary.clone())
                .await?;
            Ok(CampaignInventoryEntry {
                sha256: expected_digest,
                bytes: expected_bytes,
            })
        }
        .await;
        if result.is_err() {
            let cleanup_shard = Arc::clone(&shard);
            let _ =
                crate::physical_work::spawn_blocking(move || cleanup_shard.remove_file(&temporary))
                    .await;
        }
        result
    }

    pub async fn open_verified(
        &self,
        digest: &[u8; 32],
    ) -> Result<tokio::fs::File, CampaignStoreError> {
        let shard_name = hex::encode(&digest[..1]);
        let root = Arc::clone(&self.root_dir);
        let shard = Arc::new(
            crate::physical_work::spawn_blocking(move || {
                crate::secure_fs::open_private_dir(&root, Path::new(&shard_name))
            })
            .await
            .map_err(std::io::Error::other)??,
        );
        self.open_pinned_verified(shard, Self::object_name(digest), digest)
            .await
    }

    async fn open_pinned_verified(
        &self,
        directory: Arc<cap_std::fs::Dir>,
        name: String,
        digest: &[u8; 32],
    ) -> Result<tokio::fs::File, CampaignStoreError> {
        let opened_directory = Arc::clone(&directory);
        let opened_name = name.clone();
        let file = crate::physical_work::spawn_blocking(move || {
            crate::secure_fs::open_regular_file(&opened_directory, Path::new(&opened_name))
        })
        .await
        .map_err(std::io::Error::other)??;
        verify_open_campaign_file(file, digest, self.max_bytes, true).await
    }

    pub async fn open_operator_file(
        &self,
        path: &Path,
        digest: &[u8; 32],
    ) -> Result<tokio::fs::File, CampaignStoreError> {
        open_path_verified(path, digest, self.max_bytes, false).await
    }

    pub async fn inventory(&self) -> Result<Vec<CampaignInventoryEntry>, CampaignStoreError> {
        let root = Arc::clone(&self.root_dir);
        let mut entries = crate::physical_work::spawn_blocking(move || {
            let mut entries = Vec::new();
            for shard_entry in root.entries()? {
                let shard_entry = shard_entry?;
                let shard_name = shard_entry.file_name();
                let shard_name = shard_name
                    .to_str()
                    .ok_or_else(|| std::io::Error::other("campaign shard name is not UTF-8"))?;
                if shard_name == ".purge" {
                    continue;
                }
                if shard_name.len() != 2
                    || !shard_name
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    || !shard_entry.file_type()?.is_dir()
                {
                    return Err(std::io::Error::other(
                        "campaign store contains an invalid shard",
                    ));
                }
                let shard = crate::secure_fs::open_private_dir(&root, Path::new(shard_name))?;
                for object_entry in shard.entries()? {
                    let object_entry = object_entry?;
                    let name = object_entry.file_name();
                    let name = name.to_str().ok_or_else(|| {
                        std::io::Error::other("campaign object name is not UTF-8")
                    })?;
                    let file_type = object_entry.file_type()?;
                    let metadata = object_entry.metadata()?;
                    let Some(hex_digest) = name.strip_suffix(".campaign") else {
                        if name.starts_with(".import-") && name.ends_with(".tmp") {
                            if !file_type.is_file() {
                                return Err(std::io::Error::other(
                                    "campaign import temporary object is not a regular file",
                                ));
                            }
                            if SystemTime::now()
                                .duration_since(metadata.modified()?.into_std())
                                .is_ok_and(|age| age >= Duration::from_secs(24 * 60 * 60))
                            {
                                shard.remove_file(name)?;
                                crate::secure_fs::sync_private_dir(&shard)?;
                            }
                            continue;
                        }
                        return Err(std::io::Error::other(
                            "campaign store contains an unknown file",
                        ));
                    };
                    if !file_type.is_file()
                        || hex_digest.len() != 64
                        || !hex_digest
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                        || &hex_digest[..2] != shard_name
                    {
                        return Err(std::io::Error::other(
                            "campaign object has an invalid content address",
                        ));
                    }
                    let digest: [u8; 32] = hex::decode(hex_digest)
                        .map_err(std::io::Error::other)?
                        .try_into()
                        .map_err(|_| std::io::Error::other("invalid campaign digest"))?;
                    entries.push(CampaignInventoryEntry {
                        sha256: digest,
                        bytes: metadata.len(),
                    });
                }
            }
            Ok::<_, std::io::Error>(entries)
        })
        .await
        .map_err(std::io::Error::other)??;
        for entry in &entries {
            drop(self.open_verified(&entry.sha256).await?);
        }
        entries.sort_by_key(|entry| entry.sha256);
        Ok(entries)
    }

    pub async fn quarantine_for_purge(
        &self,
        digest: &[u8; 32],
        claim_token: &str,
    ) -> Result<PathBuf, CampaignStoreError> {
        if claim_token.is_empty()
            || claim_token.len() > 64
            || !claim_token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(CampaignStoreError::Io(std::io::Error::other(
                "invalid campaign purge token",
            )));
        }
        let purge = {
            let root = Arc::clone(&self.root_dir);
            Arc::new(
                crate::physical_work::spawn_blocking(move || {
                    crate::secure_fs::ensure_private_dir(&root, Path::new(".purge"))
                })
                .await
                .map_err(std::io::Error::other)??,
            )
        };
        let destination_name = format!("{}-{}.campaign", claim_token, hex::encode(digest));
        match self
            .open_pinned_verified(Arc::clone(&purge), destination_name.clone(), digest)
            .await
        {
            Ok(file) => {
                drop(file);
                return Ok(self.root.join(".purge").join(destination_name));
            }
            Err(CampaignStoreError::Io(error)) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let shard = match self.ensure_shard(digest).await {
            Ok(shard) => shard,
            Err(CampaignStoreError::Io(error)) if error.kind() == ErrorKind::NotFound => {
                return Ok(self.root.join(".purge").join(destination_name));
            }
            Err(error) => return Err(error),
        };
        match self.open_verified(digest).await {
            Ok(file) => drop(file),
            Err(CampaignStoreError::Io(error)) if error.kind() == ErrorKind::NotFound => {
                return Ok(self.root.join(".purge").join(destination_name));
            }
            Err(error) => return Err(error),
        }
        let source_name = Self::object_name(digest);
        let rename_shard = Arc::clone(&shard);
        let rename_purge = Arc::clone(&purge);
        let rename_destination = destination_name.clone();
        crate::physical_work::spawn_blocking(move || {
            rename_shard.rename(&source_name, &rename_purge, &rename_destination)?;
            crate::secure_fs::sync_private_dir(&rename_shard)?;
            crate::secure_fs::sync_private_dir(&rename_purge)
        })
        .await
        .map_err(std::io::Error::other)??;
        Ok(self.root.join(".purge").join(destination_name))
    }

    pub async fn remove_quarantined(&self, path: &Path) -> Result<(), CampaignStoreError> {
        if path.parent() != Some(self.root.join(".purge").as_path()) {
            return Err(CampaignStoreError::Io(std::io::Error::other(
                "campaign purge path escapes quarantine",
            )));
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| std::io::Error::other("campaign purge name is not UTF-8"))?
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
        .map_err(std::io::Error::other)??;
        Ok(())
    }
}

async fn open_path_verified(
    path: &Path,
    digest: &[u8; 32],
    max_bytes: u64,
    require_read_only: bool,
) -> Result<tokio::fs::File, CampaignStoreError> {
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let file = options.open(path).await?.into_std().await;
    verify_open_campaign_file(file, digest, max_bytes, require_read_only).await
}

async fn verify_open_campaign_file(
    file: std::fs::File,
    digest: &[u8; 32],
    max_bytes: u64,
    require_read_only: bool,
) -> Result<tokio::fs::File, CampaignStoreError> {
    let mut file = tokio::fs::File::from_std(file);
    let metadata = file.metadata().await?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > max_bytes {
        return Err(CampaignStoreError::Io(std::io::Error::other(
            "campaign state is non-regular, empty, or oversized",
        )));
    }
    #[cfg(unix)]
    if require_read_only && metadata.permissions().mode() & 0o222 != 0 {
        return Err(CampaignStoreError::Io(std::io::Error::other(
            "stored campaign state must be read-only",
        )));
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    if hasher.finalize().as_slice() != digest {
        return Err(CampaignStoreError::DigestMismatch);
    }
    file.seek(std::io::SeekFrom::Start(0)).await?;
    Ok(file)
}

#[cfg(target_os = "linux")]
async fn set_mode(path: &Path, mode: u32) -> Result<(), std::io::Error> {
    use rustix::fs::{Mode, OFlags};
    use std::os::unix::fs::PermissionsExt as _;
    let fd = crate::secure_fs::open_no_symlinks_at(
        rustix::fs::CWD,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        rustix::fs::ResolveFlags::empty(),
    )
    .map_err(std::io::Error::from)?;
    let directory = std::fs::File::from(fd);
    if !directory.metadata()?.is_dir() {
        return Err(std::io::Error::other(
            "campaign storage path is not a directory",
        ));
    }
    if directory.metadata()?.permissions().mode() & 0o7777 != mode {
        directory.set_permissions(std::fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "linux")))]
async fn set_mode(path: &Path, mode: u32) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt as _;
    if std::fs::metadata(path)?.permissions().mode() & 0o7777 != mode {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

#[cfg(not(unix))]
async fn set_mode(_path: &Path, _mode: u32) -> Result<(), std::io::Error> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn streamed_artifact_requires_exact_signed_length_and_digest() {
        let directory = tempfile::tempdir().unwrap();
        let store = CampaignStore::create(directory.path().join("campaigns"), 1024)
            .await
            .unwrap();
        let bytes = Bytes::from_static(b"private campaign bytes");
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        let entry = store
            .store_stream(
                futures_util::stream::iter([Ok::<_, &str>(bytes.clone())]),
                digest,
                bytes.len() as u64,
            )
            .await
            .unwrap();
        assert_eq!(entry.sha256, digest);
        assert_eq!(entry.bytes, bytes.len() as u64);
        drop(store.open_verified(&digest).await.unwrap());

        assert!(matches!(
            store
                .store_stream(
                    futures_util::stream::iter([Ok::<_, &str>(bytes.clone())]),
                    digest,
                    bytes.len() as u64 + 1,
                )
                .await,
            Err(CampaignStoreError::LengthMismatch { .. })
        ));
        assert!(matches!(
            store
                .store_stream(
                    futures_util::stream::iter([Ok::<_, &str>(bytes)]),
                    [9; 32],
                    entry.bytes,
                )
                .await,
            Err(CampaignStoreError::DigestMismatch)
        ));
        assert_eq!(
            tokio::fs::read(store.path_for_digest(&digest))
                .await
                .unwrap(),
            b"private campaign bytes"
        );
        #[cfg(unix)]
        {
            assert_eq!(
                std::fs::metadata(directory.path().join("campaigns"))
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
        let mut shard = tokio::fs::read_dir(store.path_for_digest(&digest).parent().unwrap())
            .await
            .unwrap();
        while let Some(entry) = shard.next_entry().await.unwrap() {
            assert!(
                !entry.file_name().to_string_lossy().starts_with(".import-"),
                "a failed exact retry left a campaign staging file"
            );
        }
    }

    #[tokio::test]
    async fn rejects_same_length_corruption_and_symlink() {
        let directory = tempfile::tempdir().unwrap();
        let store = CampaignStore::create(directory.path().join("campaigns"), 1024)
            .await
            .unwrap();
        let bytes = b"campaign";
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        let path = store.import_bytes(&digest, bytes).await.unwrap();
        #[cfg(unix)]
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .await
            .unwrap();
        tokio::fs::write(&path, b"campaiGn").await.unwrap();
        #[cfg(unix)]
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400))
            .await
            .unwrap();
        assert!(matches!(
            store.open_verified(&digest).await,
            Err(CampaignStoreError::DigestMismatch)
        ));
        assert!(store.import_bytes(&digest, bytes).await.is_err());
        let mut shard = tokio::fs::read_dir(path.parent().unwrap()).await.unwrap();
        while let Some(entry) = shard.next_entry().await.unwrap() {
            assert!(
                !entry.file_name().to_string_lossy().starts_with(".import-"),
                "a rejected duplicate import left a temporary campaign file"
            );
        }

        #[cfg(unix)]
        {
            tokio::fs::remove_file(&path).await.unwrap();
            std::os::unix::fs::symlink("/dev/null", &path).unwrap();
            assert!(store.open_verified(&digest).await.is_err());
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn campaign_root_rejects_a_symlinked_ancestor() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        tokio::fs::create_dir(&real).await.unwrap();
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(
            CampaignStore::create(link.join("campaigns"), 100)
                .await
                .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn pinned_campaign_root_survives_ancestor_swap_without_touching_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let live_parent = directory.path().join("live");
        tokio::fs::create_dir(&live_parent).await.unwrap();
        let root = live_parent.join("campaigns");
        let store = CampaignStore::create(root.clone(), 1024).await.unwrap();
        let original = b"original campaign";
        let original_digest: [u8; 32] = Sha256::digest(original).into();
        store
            .import_bytes(&original_digest, original)
            .await
            .unwrap();

        let displaced_parent = directory.path().join("displaced");
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

        let mut opened = store.open_verified(&original_digest).await.unwrap();
        let mut opened_bytes = Vec::new();
        opened.read_to_end(&mut opened_bytes).await.unwrap();
        assert_eq!(opened_bytes, original);

        let second = b"second campaign";
        let second_digest: [u8; 32] = Sha256::digest(second).into();
        store.import_bytes(&second_digest, second).await.unwrap();
        assert!(!store.path_for_digest(&second_digest).exists());
        let relative_second = store
            .path_for_digest(&second_digest)
            .strip_prefix(&root)
            .unwrap()
            .to_owned();
        assert!(
            displaced_parent
                .join("campaigns")
                .join(relative_second)
                .is_file()
        );

        let quarantine = store
            .quarantine_for_purge(&original_digest, "claim")
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
