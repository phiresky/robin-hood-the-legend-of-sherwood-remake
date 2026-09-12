//! Descriptor-backed adapter for a separately derived, pure topology.
//!
//! Inventory comparison and sealing operate only on the validated open files;
//! topology authoring never gets filesystem mutation authority.

use super::topology::ExpectedPublicationTopologyV3;
use super::{PublicationTreeInventoryV3, publication_tree_inventory_v3_from_fd};
use anyhow::{Context as _, Result, ensure};
use std::fs;
use std::path::Path;

impl ExpectedPublicationTopologyV3 {
    pub(super) fn register_inventory_file(
        &mut self,
        inventory: &PublicationTreeInventoryV3,
        path: String,
        executable: bool,
    ) -> Result<()> {
        let artifact = &inventory
            .files
            .iter()
            .find(|file| file.path == path)
            .with_context(|| format!("validated PublicationV3 omits expected file {path}"))?
            .artifact;
        self.register_file(path, artifact, executable)
    }

    pub(super) fn validate_inventory(&self, inventory: &PublicationTreeInventoryV3) -> Result<()> {
        // Descriptor inventory is sorted once at acquisition. Compare borrowed
        // entries directly; rebuilding maps cloned every digest/path and hid
        // duplicate entries. Length plus ordered equality rejects duplicates too.
        ensure!(
            inventory.files.len() == self.files.len()
                && inventory.files.iter().zip(&self.files).all(|(actual, (path, expected))|
                    actual.path == *path && actual.artifact == expected.artifact && actual.unix_mode == expected.unix_mode)
                && inventory.directories.len() == self.directories.len()
                && inventory.directories.iter().zip(&self.directories).all(|(actual, (path, mode))|
                    actual.path == *path && actual.unix_mode == *mode),
            "PublicationV3 file/directory topology differs from its independently derived typed closure"
        );
        Ok(())
    }

    pub(super) fn validate_inventory_content(
        &self,
        inventory: &PublicationTreeInventoryV3,
    ) -> Result<()> {
        ensure!(
            inventory.files.len() == self.files.len()
                && inventory
                    .files
                    .iter()
                    .zip(&self.files)
                    .all(|(actual, (path, expected))| actual.path == *path
                        && actual.artifact == expected.artifact)
                && inventory.directories.len() == self.directories.len()
                && inventory
                    .directories
                    .iter()
                    .zip(self.directories.keys())
                    .all(|(actual, path)| actual.path == *path),
            "PublicationV3 content topology differs from its independently derived typed closure"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub(super) fn seal_and_validate(&self, root_path: &Path, root: &fs::File) -> Result<()> {
        use rustix::fs::{Mode, fchmod};
        use std::os::fd::AsFd as _;

        let inventory = publication_tree_inventory_v3_from_fd(root_path, root)?;
        self.validate_inventory_content(&inventory)?;
        for file in &inventory.files {
            let expected = self
                .files
                .get(&file.path)
                .context("expected PublicationV3 file disappeared while sealing")?;
            fchmod(file.file.as_fd(), Mode::from_raw_mode(expected.unix_mode))?;
            file.file.sync_all()?;
        }
        let mut directories = inventory.directory_files.iter().collect::<Vec<_>>();
        directories
            .sort_by_key(|(path, _)| std::cmp::Reverse(Path::new(path).components().count()));
        for (path, directory) in directories {
            let mode = self
                .directories
                .get(path)
                .context("expected PublicationV3 directory disappeared while sealing")?;
            fchmod(directory.as_fd(), Mode::from_raw_mode(*mode))?;
            directory.sync_all()?;
        }
        let sealed = publication_tree_inventory_v3_from_fd(root_path, root)?;
        self.validate_inventory(&sealed)
    }
}
