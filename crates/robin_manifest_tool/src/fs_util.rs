//! Shared lexical and bounded-document policies. Descriptor-pinned inventory
//! walkers deliberately retain their stronger inode/mount/ownership contracts.
use anyhow::{Context as _, Result, ensure};
use serde::{Serialize, de::DeserializeOwned};
use std::fs;
use std::io::Read as _;
use std::path::Path;

pub(crate) fn validate_regular_file(path: &Path) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect required file {}", path.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "{} must be a regular non-symlink file",
        path.display()
    );
    Ok(metadata)
}

/// Check both admission and actual bytes; growth after metadata inspection must
/// never turn a bounded operator input into an unbounded allocation.
pub(crate) fn read_regular_file_bounded(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    let metadata = validate_regular_file(path)?;
    ensure!(
        metadata.len() <= maximum,
        "{} exceeds the {maximum} byte operator-document limit",
        path.display()
    );
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    let mut file = options.open(path)?;
    let opened = file.metadata()?;
    ensure!(
        opened.is_file() && opened.len() == metadata.len(),
        "{} changed before it was read",
        path.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        ensure!(
            opened.dev() == metadata.dev() && opened.ino() == metadata.ino(),
            "{} was replaced before it was read",
            path.display()
        );
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len())?);
    std::io::Read::by_ref(&mut file)
        .take(maximum.saturating_add(1))
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 == metadata.len() && bytes.len() as u64 <= maximum,
        "{} changed while it was read",
        path.display()
    );
    Ok(bytes)
}

/// Strict syntax/canonical presentation shared by typed and schema-specific
/// loaders. Semantic validation remains mandatory in each typed caller.
pub(crate) fn load_canonical_bytes<T: DeserializeOwned + Serialize>(
    path: &Path,
    maximum: u64,
) -> Result<(T, Vec<u8>)> {
    let bytes = read_regular_file_bounded(path, maximum)?;
    let document: T = crate::strict_json_from_slice(&bytes)
        .with_context(|| format!("parse canonical document {}", path.display()))?;
    ensure!(
        robin_run_protocol::canonical_json_bytes(&document)? == bytes,
        "document is not canonical JSON: {}",
        path.display()
    );
    Ok((document, bytes))
}

pub(crate) fn reject_placeholders(
    bytes: &[u8],
    label: &str,
    reject_zero_digest: bool,
) -> Result<()> {
    let lowercase = std::str::from_utf8(bytes)?.to_ascii_lowercase();
    ensure!(
        !lowercase.contains("placeholder")
            && !lowercase.contains("changeme")
            && !lowercase.contains("example.invalid")
            && (!reject_zero_digest || !lowercase.contains(&"0".repeat(64))),
        "{label} contains a zero/example/placeholder value"
    );
    Ok(())
}

pub(crate) fn valid_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\\')
        && value
            .split('/')
            .all(|part| !part.is_empty() && !matches!(part, "." | ".."))
}

/// Unpinned staging-tree inventory only. Activation/publication authority uses
/// its descriptor-retaining walker instead; callers must not substitute this
/// path-based helper at an acceptance or destructive-consumption boundary.
pub(crate) fn walk_regular_tree(
    root: &Path,
) -> Result<(
    Vec<(std::path::PathBuf, std::path::PathBuf)>,
    std::collections::BTreeSet<std::path::PathBuf>,
)> {
    let mut pending = vec![std::path::PathBuf::new()];
    let mut files = std::collections::BTreeMap::new();
    let mut directories = std::collections::BTreeSet::new();
    while let Some(relative_root) = pending.pop() {
        for entry in fs::read_dir(root.join(&relative_root))? {
            let entry = entry?;
            let relative = relative_root.join(entry.file_name());
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "tree contains forbidden symlink {}",
                path.display()
            );
            if metadata.is_dir() {
                ensure!(
                    directories.insert(relative.clone()),
                    "tree repeats directory {}",
                    relative.display()
                );
                pending.push(relative);
            } else {
                ensure!(
                    metadata.is_file(),
                    "tree contains non-regular entry {}",
                    path.display()
                );
                ensure!(
                    files.insert(relative, path).is_none(),
                    "tree repeats a file"
                );
            }
        }
    }
    Ok((files.into_iter().collect(), directories))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_documents_and_strict_duplicate_keys_fail_closed() -> Result<()> {
        let root = tempfile::tempdir()?;
        let file = root.path().join("authority.json");
        fs::write(&file, br#"{"x":1,"x":1}"#)?;
        let error = load_canonical_bytes::<serde_json::Value>(&file, 100).unwrap_err();
        assert!(
            error
                .downcast_ref::<robin_run_protocol::strict_json::StrictJsonError>()
                .is_some()
        );
        assert!(read_regular_file_bounded(&file, 2).is_err());
        fs::write(&file, b"")?;
        assert!(read_regular_file_bounded(&file, 0)?.is_empty());
        Ok(())
    }
    #[test]
    fn path_policy_is_lexical_not_host_normalized() {
        for invalid in ["", "/a", "a/", "a//b", "a/./b", "a/../b", "a\\b"] {
            assert!(!valid_relative_path(invalid), "{invalid}");
        }
        assert!(valid_relative_path("catalog/a.json"));
    }
}
