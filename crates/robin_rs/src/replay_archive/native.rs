//! Native mission recordings: a real directory under an exclusive lock file,
//! atomic manifest publication and fsynced append-only chunks.

use super::{MAX_BYTES, Manifest, assemble_replay, read_manifest};
use anyhow::{Context, Result, ensure};
use robin_engine::replay::ReplayData;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The exclusive `recording.lock` handle held for the archive's lifetime.
pub(super) type DirectoryLease = std::fs::File;

/// Never truncate an earlier mission, including --record overrides.
pub(super) fn create_directory(directory: &Path) -> Result<()> {
    if let Some(parent) = directory.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir(directory)
        .with_context(|| format!("create mission recording {}", directory.display()))?;
    Ok(())
}

/// Lock before reading so no other session can publish underneath this one.
pub(super) fn lease_and_read_manifest(directory: &Path) -> Result<(DirectoryLease, Manifest)> {
    let lock = lease_directory(directory)?;
    let manifest = read_manifest(directory)?;
    Ok((lock, manifest))
}

pub(super) fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    crate::save_file::atomic_write(path, bytes)
}

pub(super) fn sync_chunk(path: &Path) -> Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)?
        .sync_data()?;
    Ok(())
}

/// Freeze an earlier attempt's chronology at its immutable terminal chunk.
/// Later loads append new files and cannot change this attempt's replay.
pub(crate) fn load_through_chunk(path: &Path) -> Result<ReplayData> {
    let directory = path
        .parent()
        .context("replay chunk requires its mission directory")?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("replay chunk filename is not UTF-8")?;
    let mut manifest = read_manifest(directory)?;
    let index = manifest
        .chunks
        .iter()
        .position(|chunk| chunk.file == filename)
        .context("file is not a chunk in this mission recording")?;
    manifest.chunks.truncate(index + 1);
    let (_, data, _) = assemble_replay(directory, &manifest)?;
    Ok(data)
}

pub(super) fn canonical_directory(path: &Path) -> Result<PathBuf> {
    let path = path.canonicalize()?;
    ensure!(
        path.to_str().is_some(),
        "mission recording directory must be UTF-8"
    );
    Ok(path)
}

pub(super) fn lease_directory(directory: &Path) -> Result<DirectoryLease> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("recording.lock"))?;
    fs2::FileExt::try_lock_exclusive(&file)
        .context("mission recording is already open in another session")?;
    Ok(file)
}

pub(super) fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "mission recording exceeds {MAX_BYTES} bytes"
    );
    Ok(bytes)
}

pub(super) fn reserve_unreferenced_chunk(path: &Path) -> Result<()> {
    match create_chunk(path) {
        Ok(()) => return Ok(()),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists) => {}
        Err(error) => return Err(error),
    }
    // Only reserve_next_chunk calls this, under the archive's exclusive lease
    // and after proving the canonical name is absent from its manifest.
    ensure!(
        std::fs::symlink_metadata(path)?.file_type().is_file(),
        "unpublished replay path {} is not a regular file; preserve it and repair manually",
        path.display()
    );
    let parent = path.parent().context("replay chunk has no directory")?;
    let quarantine = tempfile::Builder::new()
        .prefix(".unpublished-replay-")
        .tempdir_in(parent)?;
    let retained = quarantine
        .path()
        .join(path.file_name().context("replay chunk has no filename")?);
    std::fs::rename(path, &retained)
        .with_context(|| format!("retain unpublished replay chunk {}", path.display()))?;
    // From here onward the directory contains user history, not disposable
    // scratch space. Preserve it even if durability or a later reserve fails.
    let quarantine = quarantine.keep();
    #[cfg(unix)]
    {
        std::fs::File::open(&quarantine)?.sync_all()?;
        std::fs::File::open(parent)?.sync_all()?;
    }
    tracing::warn!(path = %retained.display(), "retained unpublished replay chunk before retry");
    create_chunk(path).with_context(|| {
        format!(
            "reserve replay retry; previous bytes retained in {}",
            quarantine.display()
        )
    })
}

fn create_chunk(path: &Path) -> Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .sync_all()?;
    Ok(())
}

pub(super) fn open_chunk_writer(path: &Path) -> Result<Box<dyn Write + Send>> {
    Ok(Box::new(DurableChunk(
        std::fs::OpenOptions::new().append(true).open(path)?,
    )))
}

struct DurableChunk(std::fs::File);

impl Write for DurableChunk {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
