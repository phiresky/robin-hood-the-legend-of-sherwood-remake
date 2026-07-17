//! Ordered, read-only virtual filesystem for game assets.
//!
//! [`AssetVfs`] owns its mounts.  Fresh instances are completely isolated,
//! which makes converters and tests independent of the process-wide runtime
//! facade at the bottom of this module.  Mounts are searched in order and the
//! first file wins.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("asset not found: {0}")]
    NotFound(String),
    #[error("asset path must be relative and contained by its mount: {0}")]
    InvalidPath(String),
    #[error("asset mount is not a directory: {0}")]
    MountNotDirectory(PathBuf),
    #[error("asset I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Pre-bundled bytes keyed by a path relative to the game-data root.
pub type Bundle = BTreeMap<String, Vec<u8>>;

#[derive(Debug)]
enum Mount {
    Memory(Arc<Bundle>),
    #[cfg(not(target_arch = "wasm32"))]
    Directory(PathBuf),
}

/// An instance-owned, ordered set of read-only asset mounts.
///
/// Paths passed to this type are virtual paths: absolute paths and `..`
/// components are rejected.  Native directory mounts are canonicalized when
/// installed, and a resolved file must remain below that root.  This also
/// prevents a symlink inside a mount from escaping it.
#[derive(Debug, Default)]
pub struct AssetVfs {
    mounts: RwLock<Vec<Mount>>,
}

impl AssetVfs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an in-memory bundle as the lowest-priority mount.
    pub fn mount_bundle(&self, bundle: Arc<Bundle>) -> Result<(), AssetError> {
        validate_bundle(&bundle)?;
        self.mounts
            .write()
            .expect("asset VFS mounts poisoned")
            .push(Mount::Memory(bundle));
        Ok(())
    }

    /// Insert an in-memory bundle as the highest-priority mount.
    ///
    /// Runtime shipping data uses this so it retains the historical priority
    /// over host-preloaded and loose files regardless of bootstrap order.
    pub fn mount_bundle_first(&self, bundle: Arc<Bundle>) -> Result<(), AssetError> {
        validate_bundle(&bundle)?;
        self.mounts
            .write()
            .expect("asset VFS mounts poisoned")
            .insert(0, Mount::Memory(bundle));
        Ok(())
    }

    /// Append a native directory mount.
    ///
    /// Installation validates and canonicalizes the root immediately, so a
    /// missing/inaccessible root is reported at startup instead of turning
    /// every later open into a misleading file-not-found result.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn mount_directory(&self, root: impl AsRef<Path>) -> Result<(), AssetError> {
        let requested = root.as_ref();
        let root = std::fs::canonicalize(requested).map_err(|source| AssetError::Io {
            path: requested.to_path_buf(),
            source,
        })?;
        if !root.is_dir() {
            return Err(AssetError::MountNotDirectory(root));
        }
        self.mounts
            .write()
            .expect("asset VFS mounts poisoned")
            .push(Mount::Directory(root));
        Ok(())
    }

    pub fn read(&self, path: impl AsRef<Path>) -> Result<Vec<u8>, AssetError> {
        let requested = path.as_ref();
        let relative = normalize_virtual_path(requested)?;
        let key = bundle_key_from_normalized(&relative);
        let mounts = self.mounts.read().expect("asset VFS mounts poisoned");
        for mount in mounts.iter() {
            match mount {
                Mount::Memory(bundle) => {
                    if let Some(bytes) = bundle.get(&key) {
                        return Ok(bytes.clone());
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                Mount::Directory(root) => {
                    let candidate = root.join(&relative);
                    let resolved = match std::fs::canonicalize(&candidate) {
                        Ok(resolved) => resolved,
                        Err(source) if source.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(source) => {
                            return Err(AssetError::Io {
                                path: candidate,
                                source,
                            });
                        }
                    };
                    if !resolved.starts_with(root) {
                        return Err(AssetError::InvalidPath(requested.display().to_string()));
                    }
                    if !resolved.is_file() {
                        continue;
                    }
                    return std::fs::read(&resolved).map_err(|source| AssetError::Io {
                        path: resolved,
                        source,
                    });
                }
            }
        }
        Err(AssetError::NotFound(requested.display().to_string()))
    }

    /// Check for an asset while preserving invalid-path and I/O errors.
    pub fn try_exists(&self, path: impl AsRef<Path>) -> Result<bool, AssetError> {
        let requested = path.as_ref();
        let relative = normalize_virtual_path(requested)?;
        let key = bundle_key_from_normalized(&relative);
        let mounts = self.mounts.read().expect("asset VFS mounts poisoned");
        for mount in mounts.iter() {
            match mount {
                Mount::Memory(bundle) if bundle.contains_key(&key) => return Ok(true),
                Mount::Memory(_) => {}
                #[cfg(not(target_arch = "wasm32"))]
                Mount::Directory(root) => {
                    let candidate = root.join(&relative);
                    let resolved = match std::fs::canonicalize(&candidate) {
                        Ok(resolved) => resolved,
                        Err(source) if source.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(source) => {
                            return Err(AssetError::Io {
                                path: candidate,
                                source,
                            });
                        }
                    };
                    if !resolved.starts_with(root) {
                        return Err(AssetError::InvalidPath(requested.display().to_string()));
                    }
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Resolve a virtual asset to a contained native file path.
    ///
    /// Memory mounts cannot supply a host path and are skipped.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn resolve(&self, path: impl AsRef<Path>) -> Result<Option<PathBuf>, AssetError> {
        let requested = path.as_ref();
        let relative = normalize_virtual_path(requested)?;
        let mounts = self.mounts.read().expect("asset VFS mounts poisoned");
        for mount in mounts.iter() {
            let Mount::Directory(root) = mount else {
                continue;
            };
            let candidate = root.join(&relative);
            let resolved = match std::fs::canonicalize(&candidate) {
                Ok(resolved) => resolved,
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => continue,
                Err(source) => {
                    return Err(AssetError::Io {
                        path: candidate,
                        source,
                    });
                }
            };
            if !resolved.starts_with(root) {
                return Err(AssetError::InvalidPath(requested.display().to_string()));
            }
            if resolved.is_file() {
                return Ok(Some(resolved));
            }
        }
        Ok(None)
    }
}

fn validate_bundle(bundle: &Bundle) -> Result<(), AssetError> {
    for path in bundle.keys() {
        normalize_virtual_path(Path::new(path))?;
    }
    Ok(())
}

fn normalize_virtual_path(path: &Path) -> Result<PathBuf, AssetError> {
    let replaced = path.to_string_lossy().replace('\\', "/");
    let path = Path::new(&replaced);
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AssetError::InvalidPath(path.display().to_string()));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(AssetError::InvalidPath(path.display().to_string()));
    }
    Ok(normalized)
}

/// Normalize a path to the shipping bundle key scheme: forward slashes,
/// lowercase, no leading `./`, and no leading `Data/`.
pub fn bundle_key(path: &Path) -> String {
    // Keep this infallible compatibility helper for converter code. Actual VFS
    // opens validate containment with `normalize_virtual_path` before lookup.
    let mut key = path.to_string_lossy().replace('\\', "/");
    while let Some(rest) = key.strip_prefix("./") {
        key = rest.to_string();
    }
    strip_data_prefix(&key).to_ascii_lowercase()
}

fn bundle_key_from_normalized(path: &Path) -> String {
    strip_data_prefix(&path.to_string_lossy().replace('\\', "/")).to_ascii_lowercase()
}

fn strip_data_prefix(path: &str) -> &str {
    path.strip_prefix("Data/")
        .or_else(|| path.strip_prefix("data/"))
        .or_else(|| path.strip_prefix("DATA/"))
        .unwrap_or(path)
}

// ---------------------------------------------------------------------------
// Process runtime compatibility facade
// ---------------------------------------------------------------------------

static GLOBAL: OnceLock<Arc<AssetVfs>> = OnceLock::new();
static PRELOADED: OnceLock<RwLock<Bundle>> = OnceLock::new();

/// The runtime VFS used by legacy static call sites.
pub fn global() -> &'static Arc<AssetVfs> {
    GLOBAL.get_or_init(|| Arc::new(AssetVfs::new()))
}

/// Install the shipping bundle at highest priority.
pub fn install_bundle(bundle: Arc<Bundle>) -> Result<(), AssetError> {
    global().mount_bundle_first(bundle)
}

fn preloaded() -> &'static RwLock<Bundle> {
    PRELOADED.get_or_init(|| RwLock::new(Bundle::new()))
}

/// Install or replace one host-preloaded asset.
pub fn install_preloaded_asset<P: AsRef<Path>>(path: P, bytes: Vec<u8>) -> Result<(), AssetError> {
    let normalized = normalize_virtual_path(path.as_ref())?;
    preloaded()
        .write()
        .expect("preloaded asset bundle poisoned")
        .insert(bundle_key_from_normalized(&normalized), bytes);
    Ok(())
}

/// Read through the runtime mounts, then fall back to a direct host path on
/// native for call sites that have not yet been migrated to virtual paths.
pub fn read<P: AsRef<Path>>(path: P) -> Result<Vec<u8>, AssetError> {
    let path = path.as_ref();
    match global().read(path) {
        Ok(bytes) => return Ok(bytes),
        Err(AssetError::NotFound(_)) | Err(AssetError::InvalidPath(_)) => {}
        Err(error) => return Err(error),
    }
    if let Ok(normalized) = normalize_virtual_path(path)
        && let Some(bytes) = preloaded()
            .read()
            .expect("preloaded asset bundle poisoned")
            .get(&bundle_key_from_normalized(&normalized))
    {
        return Ok(bytes.clone());
    }
    imp::read(path)
}

/// Compatibility boolean for legacy callers. New code should use
/// [`AssetVfs::try_exists`] so non-NotFound failures remain visible.
pub fn exists<P: AsRef<Path>>(path: P) -> bool {
    let path = path.as_ref();
    match global().try_exists(path) {
        Ok(true) => return true,
        Ok(false) | Err(AssetError::InvalidPath(_)) => {}
        Err(error) => {
            // TODO(asset-vfs): migrate remaining boolean callers to
            // AssetVfs::try_exists and propagate this error to their boundary.
            tracing_compat::warn_exists(path, &error);
            return false;
        }
    }
    if let Ok(normalized) = normalize_virtual_path(path)
        && preloaded()
            .read()
            .expect("preloaded asset bundle poisoned")
            .contains_key(&bundle_key_from_normalized(&normalized))
    {
        return true;
    }
    match imp::try_exists(path) {
        Ok(exists) => exists,
        Err(error) => {
            // TODO(asset-vfs): migrate remaining boolean callers to
            // AssetVfs::try_exists and propagate this error to their boundary.
            tracing_compat::warn_exists(path, &error);
            false
        }
    }
}

pub fn absolute<P: AsRef<Path>>(path: P) -> PathBuf {
    imp::absolute(path.as_ref())
}

// robin_util intentionally has no tracing dependency. Keep the compatibility
// warning explicit on stderr until all boolean exists callers are migrated.
mod tracing_compat {
    use super::*;

    pub fn warn_exists(path: &Path, error: &AssetError) {
        eprintln!(
            "asset existence check failed for {}: {error}",
            path.display()
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::*;

    pub fn read(path: &Path) -> Result<Vec<u8>, AssetError> {
        std::fs::read(path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                AssetError::NotFound(path.display().to_string())
            } else {
                AssetError::Io {
                    path: path.to_path_buf(),
                    source,
                }
            }
        })
    }

    pub fn absolute(path: &Path) -> PathBuf {
        path.to_path_buf()
    }

    pub fn try_exists(path: &Path) -> Result<bool, AssetError> {
        match std::fs::metadata(path) {
            Ok(_) => Ok(true),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(AssetError::Io {
                path: path.to_path_buf(),
                source,
            }),
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::*;

    pub fn read(path: &Path) -> Result<Vec<u8>, AssetError> {
        Err(AssetError::NotFound(path.display().to_string()))
    }

    pub fn absolute(path: &Path) -> PathBuf {
        path.to_path_buf()
    }

    pub fn try_exists(_path: &Path) -> Result<bool, AssetError> {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(entries: &[(&str, &[u8])]) -> Arc<Bundle> {
        Arc::new(
            entries
                .iter()
                .map(|(path, bytes)| ((*path).to_string(), bytes.to_vec()))
                .collect(),
        )
    }

    #[test]
    fn instances_are_isolated_and_mount_order_is_stable() {
        let first = AssetVfs::new();
        first
            .mount_bundle(bundle(&[("shared.dat", b"first")]))
            .unwrap();
        first
            .mount_bundle(bundle(&[("shared.dat", b"second"), ("only.dat", b"only")]))
            .unwrap();

        let isolated = AssetVfs::new();
        isolated
            .mount_bundle(bundle(&[("shared.dat", b"isolated")]))
            .unwrap();

        assert_eq!(first.read("shared.dat").unwrap(), b"first");
        assert_eq!(first.read("only.dat").unwrap(), b"only");
        assert_eq!(isolated.read("shared.dat").unwrap(), b"isolated");
        assert!(!isolated.try_exists("only.dat").unwrap());
    }

    #[test]
    fn rejects_parent_and_absolute_paths() {
        let vfs = AssetVfs::new();
        assert!(matches!(
            vfs.read("../secret.dat"),
            Err(AssetError::InvalidPath(_))
        ));
        assert!(matches!(
            vfs.read("/secret.dat"),
            Err(AssetError::InvalidPath(_))
        ));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn directory_mount_is_contained_and_install_failures_propagate() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("inside.dat"), b"inside").unwrap();
        let vfs = AssetVfs::new();
        vfs.mount_directory(root.path()).unwrap();
        assert_eq!(vfs.read("inside.dat").unwrap(), b"inside");

        let missing = root.path().join("missing");
        assert!(matches!(
            vfs.mount_directory(&missing),
            Err(AssetError::Io { path, .. }) if path == missing
        ));
    }

    #[cfg(all(not(target_arch = "wasm32"), unix))]
    #[test]
    fn directory_symlink_cannot_escape_mount() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.dat"), b"secret").unwrap();
        symlink(outside.path(), root.path().join("escape")).unwrap();

        let vfs = AssetVfs::new();
        vfs.mount_directory(root.path()).unwrap();
        assert!(matches!(
            vfs.read("escape/secret.dat"),
            Err(AssetError::InvalidPath(_))
        ));
    }
}
