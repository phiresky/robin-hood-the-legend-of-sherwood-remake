//! Small, atomic, private native stores used by leaderboard preferences and
//! server-authored campaign-continuation receipts.
//!
//! These files are host bookkeeping, never simulation or replay state. Reads
//! and replacements reject symlink targets and are bounded before allocation.

#![cfg(not(target_arch = "wasm32"))]

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write as _};
use std::path::Path;

pub(crate) const MAX_PRIVATE_STORE_BYTES: u64 = 1024 * 1024;

pub(crate) fn read_private_utf8(path: &Path) -> io::Result<Option<String>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }

    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other(format!(
            "refusing non-regular leaderboard state file {}",
            path.display()
        )));
    }
    if metadata.len() > MAX_PRIVATE_STORE_BYTES {
        return Err(io::Error::other(format!(
            "leaderboard state file exceeds {MAX_PRIVATE_STORE_BYTES} bytes"
        )));
    }
    tighten_private_permissions(&file)?;

    let capacity = usize::try_from(metadata.len()).map_err(io::Error::other)?;
    let mut bytes = Vec::with_capacity(capacity);
    Read::by_ref(&mut file)
        .take(MAX_PRIVATE_STORE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PRIVATE_STORE_BYTES {
        return Err(io::Error::other(format!(
            "leaderboard state file exceeds {MAX_PRIVATE_STORE_BYTES} bytes"
        )));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(crate) fn replace_private(path: &Path, prefix: &str, encoded: &[u8]) -> io::Result<()> {
    if encoded.len() as u64 > MAX_PRIVATE_STORE_BYTES {
        return Err(io::Error::other(format!(
            "leaderboard state exceeds {MAX_PRIVATE_STORE_BYTES} bytes"
        )));
    }
    reject_unsafe_destination(path)?;

    let parent = path.parent().ok_or_else(|| {
        io::Error::other(format!(
            "leaderboard state path has no parent: {}",
            path.display()
        ))
    })?;
    std::fs::create_dir_all(parent)?;
    let parent_metadata = std::fs::symlink_metadata(parent)?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(io::Error::other(format!(
            "refusing unsafe leaderboard state directory {}",
            parent.display()
        )));
    }

    let mut temporary = tempfile::Builder::new()
        .prefix(prefix)
        .suffix(".json.tmp")
        .tempfile_in(parent)?;
    tighten_private_permissions(temporary.as_file())?;
    temporary.write_all(encoded)?;
    temporary.as_file().sync_all()?;

    // Re-check immediately before the atomic rename. This catches a target
    // replaced by a symlink while the temporary file was being written.
    reject_unsafe_destination(path)?;
    temporary.persist(path).map_err(|error| error.error)?;
    sync_parent_directory(parent)
}

fn reject_unsafe_destination(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(io::Error::other(format!(
                "refusing unsafe leaderboard state path {}",
                path.display()
            )))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn tighten_private_permissions(file: &File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    if file.metadata()?.permissions().mode() & 0o777 != 0o600 {
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn tighten_private_permissions(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> io::Result<()> {
    OpenOptions::new().read(true).open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_replacement_is_atomic_and_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("leaderboards.json");
        replace_private(&path, ".leaderboards-", br#"{"enabled":false}"#).unwrap();
        replace_private(&path, ".leaderboards-", br#"{"enabled":true}"#).unwrap();
        assert_eq!(
            read_private_utf8(&path).unwrap().as_deref(),
            Some(r#"{"enabled":true}"#)
        );
        assert_eq!(
            std::fs::read_dir(directory.path())
                .unwrap()
                .filter_map(Result::ok)
                .count(),
            1,
            "successful replacement must not strand temporary files"
        );
    }

    #[test]
    fn oversized_and_non_utf8_stores_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("leaderboards.json");
        assert!(
            replace_private(
                &path,
                ".leaderboards-",
                &vec![0; MAX_PRIVATE_STORE_BYTES as usize + 1]
            )
            .is_err()
        );
        std::fs::write(&path, [0xff]).unwrap();
        assert_eq!(
            read_private_utf8(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_targets_are_never_read_or_replaced() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("outside.json");
        std::fs::write(&outside, "private").unwrap();
        let path = directory.path().join("leaderboards.json");
        symlink(&outside, &path).unwrap();

        assert!(read_private_utf8(&path).is_err());
        assert!(replace_private(&path, ".leaderboards-", b"replacement").is_err());
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "private");
    }

    #[cfg(unix)]
    #[test]
    fn read_and_replacement_tighten_permissions() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("leaderboards.json");
        std::fs::write(&path, "private").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        read_private_utf8(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        replace_private(&path, ".leaderboards-", b"new").unwrap();
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
