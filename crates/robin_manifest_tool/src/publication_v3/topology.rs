//! Pure expected publication topology. No filesystem descriptors are opened or
//! mutated here: callers provide artifact identities, and this module authors
//! the canonical lock/inventory independent of the tree being validated.

use super::{
    CloudflareMaterializedFileV1, CloudflareMaterializedTreeInventoryV1,
    PUBLICATION_LOCK_SCHEMA_VERSION, PublicationDirectoryV3, PublicationFileModeV3,
    PublicationLockV3, release_file_exposure,
};
use crate::{ReleaseFileV1, path_to_manifest};
use anyhow::{Context as _, Result, ensure};
use robin_run_protocol::{ArtifactRefV1, Digest32, Validate as _, canonical_json_bytes};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExpectedPublicationFileV3 {
    pub(super) artifact: ArtifactRefV1,
    pub(super) unix_mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExpectedPublicationTopologyV3 {
    pub(super) files: BTreeMap<String, ExpectedPublicationFileV3>,
    pub(super) directories: BTreeMap<String, u32>,
}

impl ExpectedPublicationTopologyV3 {
    pub(super) fn new() -> Self {
        Self::new_with_root_mode(0o700)
    }

    pub(super) fn new_with_root_mode(root_mode: u32) -> Self {
        Self {
            files: BTreeMap::new(),
            directories: BTreeMap::from([(".".to_owned(), root_mode)]),
        }
    }

    pub(super) fn register_directory(&mut self, path: &str) -> Result<()> {
        ensure!(
            path == "." || valid_publication_relative_path_v3(path),
            "invalid expected PublicationV3 directory {path}"
        );
        if path != "." {
            let mut parent = Path::new(path).parent().map(Path::to_path_buf);
            while let Some(directory_path) = parent {
                let directory = directory_path.as_path();
                if directory.as_os_str().is_empty() {
                    break;
                }
                let directory = path_to_manifest(directory)?;
                if let Some(mode) = self.directories.insert(directory.clone(), 0o555) {
                    ensure!(
                        mode == 0o555,
                        "conflicting expected PublicationV3 directory mode at {directory}"
                    )
                }
                parent = Path::new(&directory).parent().map(Path::to_path_buf);
            }
        }
        let mode = if path == "." {
            *self
                .directories
                .get(".")
                .context("expected PublicationV3 topology omits its root")?
        } else {
            0o555
        };
        if let Some(previous) = self.directories.insert(path.to_owned(), mode) {
            ensure!(
                previous == mode,
                "conflicting expected PublicationV3 directory mode at {path}"
            )
        }
        Ok(())
    }

    pub(super) fn register_file(
        &mut self,
        path: String,
        artifact: &ArtifactRefV1,
        executable: bool,
    ) -> Result<()> {
        ensure!(
            valid_publication_relative_path_v3(&path),
            "invalid expected PublicationV3 file {path}"
        );
        let parent = Path::new(&path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(path_to_manifest)
            .transpose()?
            .unwrap_or_else(|| ".".to_owned());
        self.register_directory(&parent)?;
        let mut artifact = artifact.clone();
        artifact.media_type = "application/octet-stream".into();
        ensure!(
            self.files
                .insert(
                    path.clone(),
                    ExpectedPublicationFileV3 {
                        artifact,
                        unix_mode: if executable { 0o555 } else { 0o444 },
                    },
                )
                .is_none(),
            "duplicate expected PublicationV3 file registration at {path}"
        );
        Ok(())
    }

    pub(super) fn register_canonical<T>(&mut self, path: String, document: &T) -> Result<()>
    where
        T: Serialize,
    {
        let bytes = canonical_json_bytes(document)?;
        self.register_file(
            path,
            &ArtifactRefV1 {
                sha256: Digest32::digest_bytes(&bytes),
                byte_length: u64::try_from(bytes.len())?,
                media_type: "application/json".into(),
            },
            false,
        )
    }

    pub(super) fn register_bytes(&mut self, path: String, bytes: &[u8]) -> Result<()> {
        self.register_file(
            path,
            &ArtifactRefV1 {
                sha256: Digest32::digest_bytes(bytes),
                byte_length: u64::try_from(bytes.len())?,
                media_type: "application/octet-stream".into(),
            },
            false,
        )
    }

    pub(super) fn materialized_inventory(&self) -> CloudflareMaterializedTreeInventoryV1 {
        CloudflareMaterializedTreeInventoryV1 {
            files: self
                .files
                .iter()
                .map(|(path, file)| CloudflareMaterializedFileV1 {
                    path: path.clone(),
                    artifact: file.artifact.clone(),
                    unix_mode: file.unix_mode,
                })
                .collect(),
            directories: self
                .directories
                .iter()
                .map(|(path, unix_mode)| PublicationDirectoryV3 {
                    path: path.clone(),
                    unix_mode: *unix_mode,
                })
                .collect(),
        }
    }

    pub(super) fn lock(&self, manifest_sha256: Digest32) -> Result<PublicationLockV3> {
        ensure!(
            !self.files.contains_key("publication-lock-v3.json")
                && !self.files.contains_key("publication-lock-v3.sha256"),
            "PublicationV3 lock files were registered before lock authoring"
        );
        let files = self
            .files
            .iter()
            .map(|(path, file)| ReleaseFileV1 {
                exposure: release_file_exposure(path),
                path: path.clone(),
                artifact: file.artifact.clone(),
            })
            .collect();
        let file_modes = self
            .files
            .iter()
            .map(|(path, file)| PublicationFileModeV3 {
                path: path.clone(),
                unix_mode: file.unix_mode,
            })
            .collect();
        let directories = self
            .directories
            .iter()
            .map(|(path, unix_mode)| PublicationDirectoryV3 {
                path: path.clone(),
                unix_mode: *unix_mode,
            })
            .collect();
        let lock = PublicationLockV3 {
            schema_version: PUBLICATION_LOCK_SCHEMA_VERSION,
            publication_manifest_sha256: manifest_sha256,
            files,
            directories,
            file_modes,
        };
        lock.validate()?;
        Ok(lock)
    }
}

pub(super) fn valid_publication_relative_path_v3(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.ends_with('/')
        && path.split('/').all(|component| {
            !component.is_empty() && !matches!(component, "." | "..") && !component.contains('\\')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_run_protocol::CanonicalDocument as _;

    #[test]
    fn canonical_lock_is_independent_of_registration_order() -> Result<()> {
        let mut first = ExpectedPublicationTopologyV3::new();
        first.register_bytes("backend/z.json".into(), b"z")?;
        first.register_bytes("cloudflare-public/a.json".into(), b"a")?;
        let mut second = ExpectedPublicationTopologyV3::new();
        second.register_bytes("cloudflare-public/a.json".into(), b"a")?;
        second.register_bytes("backend/z.json".into(), b"z")?;
        let manifest = Digest32::digest_bytes(b"manifest");
        let first_lock = first.lock(manifest)?;
        let second_lock = second.lock(manifest)?;
        assert_eq!(
            canonical_json_bytes(&first_lock)?,
            canonical_json_bytes(&second_lock)?
        );
        assert_eq!(
            first_lock.canonical_digest()?,
            second_lock.canonical_digest()?
        );
        assert_eq!(
            first_lock
                .directories
                .iter()
                .map(|entry| (entry.path.as_str(), entry.unix_mode))
                .collect::<Vec<_>>(),
            vec![
                (".", 0o700),
                ("backend", 0o555),
                ("cloudflare-public", 0o555)
            ]
        );
        assert!(
            first_lock
                .file_modes
                .iter()
                .all(|entry| entry.unix_mode == 0o444)
        );
        Ok(())
    }

    #[test]
    fn topology_normalizes_artifact_media_and_preserves_executable_modes() -> Result<()> {
        let mut expected = ExpectedPublicationTopologyV3::new();
        let artifact = ArtifactRefV1 {
            sha256: Digest32::digest_bytes(b"executable"),
            byte_length: 10,
            media_type: "application/x-executable".into(),
        };
        expected.register_file("private/verifier/bin/test".into(), &artifact, true)?;
        let inventory = expected.materialized_inventory();
        assert_eq!(inventory.files[0].artifact.sha256, artifact.sha256);
        assert_eq!(
            inventory.files[0].artifact.byte_length,
            artifact.byte_length
        );
        assert_eq!(
            inventory.files[0].artifact.media_type,
            "application/octet-stream"
        );
        assert_eq!(inventory.files[0].unix_mode, 0o555);
        Ok(())
    }

    #[test]
    fn topology_rejects_noncanonical_paths_and_lock_self_reference() -> Result<()> {
        for path in ["", "/absolute", "../escape", "a/../b", "a//b", "a/", "a\\b"] {
            let mut expected = ExpectedPublicationTopologyV3::new();
            assert!(
                expected.register_bytes(path.into(), b"data").is_err(),
                "{path}"
            );
        }
        let mut expected = ExpectedPublicationTopologyV3::new();
        expected.register_bytes("publication-lock-v3.json".into(), b"lock")?;
        assert!(expected.lock(Digest32::digest_bytes(b"manifest")).is_err());
        Ok(())
    }
}
