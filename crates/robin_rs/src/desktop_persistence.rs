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
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn every_publication_stage_reports_visibility_and_allows_retry() {
        for stage in [
            PublicationStage::Prepare,
            PublicationStage::Write,
            PublicationStage::SyncFile,
            PublicationStage::Replace,
            PublicationStage::SyncDirectory,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("archive.json");
            write_json(&path, &"old").unwrap();
            let error = publish(
                &path,
                PublicationMode::Replace,
                |file| file.write_all(b"\"new\""),
                |current| {
                    if current == stage {
                        Err(io::Error::other("injected failure"))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            let outcome = error
                .get_ref()
                .unwrap()
                .downcast_ref::<PublicationFailure>()
                .unwrap();
            assert_eq!(outcome.stage, stage);
            let visible: String = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(visible, if outcome.published() { "new" } else { "old" });
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
            write_json(&path, &"new").unwrap();
            assert_eq!(
                serde_json::from_slice::<String>(&fs::read(path).unwrap()).unwrap(),
                "new"
            );
        }
    }

    #[test]
    fn create_new_publication_failures_preserve_visibility_and_competing_files() {
        for stage in [
            PublicationStage::Prepare,
            PublicationStage::Write,
            PublicationStage::SyncFile,
            PublicationStage::Replace,
            PublicationStage::SyncDirectory,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("replay.rhrec");
            let error = publish(
                &path,
                PublicationMode::CreateNew,
                |file| file.write_all(b"complete"),
                |current| {
                    if current == stage {
                        Err(io::Error::other("injected failure"))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            let published = error
                .get_ref()
                .unwrap()
                .downcast_ref::<PublicationFailure>()
                .unwrap()
                .published();
            assert_eq!(path.exists(), published);
            if published {
                assert_eq!(fs::read(&path).unwrap(), b"complete");
            } else {
                write_new_bytes(&path, b"retry").unwrap();
                assert_eq!(fs::read(&path).unwrap(), b"retry");
            }
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replay.rhrec");
        let error = publish(
            &path,
            PublicationMode::CreateNew,
            |file| file.write_all(b"ours"),
            |stage| {
                if stage == PublicationStage::Replace {
                    fs::write(&path, b"concurrent winner")?;
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&path).unwrap(), b"concurrent winner");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn partial_create_new_write_does_not_poison_the_final_filename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replay.rhrec");
        let error = publish(
            &path,
            PublicationMode::CreateNew,
            |file| {
                file.write_all(b"partial")?;
                Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "injected disk full",
                ))
            },
            |_| Ok(()),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::StorageFull);
        assert!(!path.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        write_new_bytes(&path, b"complete retry").unwrap();
        assert_eq!(fs::read(path).unwrap(), b"complete retry");
    }

    #[test]
    fn encoded_bytes_are_preserved_and_failed_replacement_cleans_staging() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("datadir.txt");
        write_bytes(&path, b"old\n").unwrap();
        write_bytes(&path, b"/game/data\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"/game/data\n");
        let blocked = dir.path().join("directory");
        fs::create_dir(&blocked).unwrap();
        let error = write_bytes(&blocked, b"must not replace directory").unwrap_err();
        let failure = error
            .get_ref()
            .unwrap()
            .downcast_ref::<PublicationFailure>()
            .unwrap();
        assert_eq!(failure.stage, PublicationStage::Replace);
        assert!(!failure.published());
        assert!(blocked.is_dir());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn partial_write_preserves_live_archive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive.json");
        write_json(&path, &"old").unwrap();
        let error = publish(
            &path,
            PublicationMode::Replace,
            |file| {
                file.write_all(b"{partial")?;
                Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "injected disk full",
                ))
            },
            |_| Ok(()),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::StorageFull);
        assert_eq!(fs::read(&path).unwrap(), b"\"old\"");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn serialization_failure_preserves_live_archive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive.json");
        write_json(&path, &"old").unwrap();
        // JSON cannot encode a compound map key. The map serializer has already
        // started the staged object when it discovers the unsupported key.
        let invalid = std::collections::BTreeMap::from([((1, 2), "value")]);
        let error = write_json(&path, &invalid).unwrap_err();
        assert!(
            !error
                .get_ref()
                .unwrap()
                .downcast_ref::<PublicationFailure>()
                .unwrap()
                .published()
        );
        assert_eq!(fs::read(&path).unwrap(), b"\"old\"");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn abandoned_staging_file_does_not_replace_live_archive_on_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive.json");
        write_json(&path, &"old").unwrap();
        fs::write(
            dir.path().join(".robin-user-store-staging-abandoned"),
            b"{partial",
        )
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<String>(&fs::read(&path).unwrap()).unwrap(),
            "old"
        );
        write_json(&path, &"new").unwrap();
        assert_eq!(
            serde_json::from_slice::<String>(&fs::read(&path).unwrap()).unwrap(),
            "new"
        );
    }
}
