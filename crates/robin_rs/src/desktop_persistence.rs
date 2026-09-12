//! Atomic publication for small native user archives, not server deployment data.
//!
//! An error before replacement leaves the previous archive untouched. An error
//! after replacement means the new archive is visible but durability is unknown.
//! In either case callers must retain their desired snapshot and may retry it;
//! this helper never marks application state clean or rolls back a published file.
//! Unix synchronizes the containing directory. Other native platforms provide
//! atomic replacement and file synchronization, without a directory durability
//! guarantee. This is not a multi-file transaction or a concurrent-writer lock.

use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublicationStage {
    Prepare,
    Write,
    SyncFile,
    Replace,
    SyncDirectory,
}

/// Available by downcasting the inner error of the returned `io::Error`.
#[derive(Debug, Serialize, Deserialize)]
pub struct PublicationFailure {
    pub stage: PublicationStage,
    pub detail: String,
}

impl PublicationFailure {
    pub fn published(&self) -> bool {
        self.stage == PublicationStage::SyncDirectory
    }
}

impl std::fmt::Display for PublicationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "archive {:?} failed (published={}): {}",
            self.stage,
            self.published(),
            self.detail
        )
    }
}

impl std::error::Error for PublicationFailure {}

fn failure(stage: PublicationStage, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        PublicationFailure {
            stage,
            detail: error.to_string(),
        },
    )
}

/// Atomically publish an already encoded archive using the same durability and
/// failure-stage contract as [`write_json`].
pub fn write_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    publish(
        path,
        PublicationMode::Replace,
        |file| file.write_all(bytes),
        |_| Ok(()),
    )
}

/// Publish a complete new archive without replacing any existing destination.
pub fn write_new_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    publish(
        path,
        PublicationMode::CreateNew,
        |file| file.write_all(bytes),
        |_| Ok(()),
    )
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum PublicationMode {
    Replace,
    CreateNew,
}

pub fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> io::Result<()> {
    publish(
        path,
        PublicationMode::Replace,
        |file| serde_json::to_writer_pretty(file, value).map_err(io::Error::other),
        |_| Ok(()),
    )
}

fn publish(
    path: &Path,
    mode: PublicationMode,
    write: impl FnOnce(&mut fs::File) -> io::Result<()>,
    mut before: impl FnMut(PublicationStage) -> io::Result<()>,
) -> io::Result<()> {
    use PublicationStage::*;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    before(Prepare).map_err(|error| failure(Prepare, error))?;
    fs::create_dir_all(parent).map_err(|error| failure(Prepare, error))?;
    // TODO: newly created ancestor directories are not individually fsynced;
    // full power-loss durability assumes the selected storage directory exists.
    let mut temporary = tempfile::Builder::new()
        .prefix(".robin-user-store-staging-")
        .tempfile_in(parent)
        .map_err(|error| failure(Prepare, error))?;
    before(Write)
        .and_then(|()| write(temporary.as_file_mut()))
        .map_err(|error| failure(Write, error))?;
    before(SyncFile)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| failure(SyncFile, error))?;
    before(Replace).map_err(|error| failure(Replace, error))?;
    let published = match mode {
        PublicationMode::Replace => temporary.persist(path),
        PublicationMode::CreateNew => temporary.persist_noclobber(path),
    };
    published.map_err(|error| failure(Replace, error.error))?;
    before(SyncDirectory).map_err(|error| failure(SyncDirectory, error))?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| failure(SyncDirectory, error))?;
    Ok(())
}

#[cfg(test)]
mod tests;
