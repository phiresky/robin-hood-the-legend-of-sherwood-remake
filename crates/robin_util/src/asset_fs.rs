//! Ordered, read-only virtual filesystem for game assets.
//!
//! [`AssetVfs`] owns its mounts.  Fresh instances are completely isolated,
//! which makes converters and tests independent of the process-wide runtime
//! facade at the bottom of this module.  Mounts are searched in order and the
//! first file wins.

use std::collections::BTreeMap;
use std::ops::Deref;
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

/// Cheaply cloned immutable asset bytes.
///
/// Shipping missions keep one copy of each file in their mounted bundle. An
/// asset open clones only this `Arc`, rather than duplicating the complete
/// file into every `SbFile` cursor.
#[derive(Clone, Debug)]
pub struct AssetBytes(Arc<Vec<u8>>);

impl AssetBytes {
    pub fn into_vec(self) -> Vec<u8> {
        Arc::try_unwrap(self.0).unwrap_or_else(|shared| (*shared).clone())
    }
}

impl From<Vec<u8>> for AssetBytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self(Arc::new(bytes))
    }
}

impl From<&[u8]> for AssetBytes {
    fn from(bytes: &[u8]) -> Self {
        bytes.to_vec().into()
    }
}

impl AsRef<[u8]> for AssetBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl Deref for AssetBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl PartialEq<[u8]> for AssetBytes {
    fn eq(&self, other: &[u8]) -> bool {
        self.as_ref() == other
    }
}

impl<const N: usize> PartialEq<&[u8; N]> for AssetBytes {
    fn eq(&self, other: &&[u8; N]) -> bool {
        self.as_ref() == other.as_slice()
    }
}

/// Pre-bundled bytes keyed by a path relative to the game-data root.
pub type Bundle = BTreeMap<String, AssetBytes>;

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
    active_bundle: RwLock<Option<Arc<Bundle>>>,
    mounts: RwLock<Vec<Mount>>,
    preloaded: RwLock<Bundle>,
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

    /// Replace the one high-priority runtime bundle.
    ///
    /// Mission activation uses this slot so changing missions releases the
    /// previous VFS reference instead of permanently stacking one mount per
    /// visited mission. Static boot and loose-data mounts remain untouched.
    pub fn replace_active_bundle(&self, bundle: Arc<Bundle>) -> Result<(), AssetError> {
        validate_bundle(&bundle)?;
        *self
            .active_bundle
            .write()
            .expect("active asset bundle poisoned") = Some(bundle);
        Ok(())
    }

    /// Install or replace one loose asset supplied by the runtime host.
    ///
    /// Browser bootstrap uses this for the small core overlay whose files
    /// must remain visible while the replaceable mission bundle changes.
    /// Regular mounts deliberately retain priority over these loose files.
    pub fn install_preloaded_asset(
        &self,
        path: impl AsRef<Path>,
        bytes: Vec<u8>,
    ) -> Result<(), AssetError> {
        let normalized = normalize_virtual_path(path.as_ref())?;
        self.preloaded
            .write()
            .expect("preloaded asset bundle poisoned")
            .insert(bundle_key_from_normalized(&normalized), bytes.into());
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

    pub fn read_shared(&self, path: impl AsRef<Path>) -> Result<AssetBytes, AssetError> {
        let requested = path.as_ref();
        let relative = normalize_virtual_path(requested)?;
        let key = bundle_key_from_normalized(&relative);
        if let Some(bytes) = self
            .active_bundle
            .read()
            .expect("active asset bundle poisoned")
            .as_ref()
            .and_then(|bundle| bundle.get(&key))
        {
            return Ok(bytes.clone());
        }
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
                    let Some(resolved) = resolve_in_mount(root, &relative, requested)? else {
                        continue;
                    };
                    // Reading requires a file; a directory falls through to
                    // the next mount.
                    if !resolved.is_file() {
                        continue;
                    }
                    return std::fs::read(&resolved)
                        .map(AssetBytes::from)
                        .map_err(|source| AssetError::Io {
                            path: resolved,
                            source,
                        });
                }
            }
        }
        drop(mounts);
        if let Some(bytes) = self
            .preloaded
            .read()
            .expect("preloaded asset bundle poisoned")
            .get(&key)
        {
            return Ok(bytes.clone());
        }
        Err(AssetError::NotFound(requested.display().to_string()))
    }

    /// Read an owned buffer for compatibility with consumers that mutate or
    /// retain the bytes independently. Stream readers should prefer
    /// [`Self::read_shared`] to avoid copying memory-mounted assets.
    pub fn read(&self, path: impl AsRef<Path>) -> Result<Vec<u8>, AssetError> {
        self.read_shared(path).map(AssetBytes::into_vec)
    }

    /// Check for an asset while preserving invalid-path and I/O errors.
    pub fn try_exists(&self, path: impl AsRef<Path>) -> Result<bool, AssetError> {
        let requested = path.as_ref();
        let relative = normalize_virtual_path(requested)?;
        let key = bundle_key_from_normalized(&relative);
        if self
            .active_bundle
            .read()
            .expect("active asset bundle poisoned")
            .as_ref()
            .is_some_and(|bundle| bundle.contains_key(&key))
        {
            return Ok(true);
        }
        let mounts = self.mounts.read().expect("asset VFS mounts poisoned");
        for mount in mounts.iter() {
            match mount {
                Mount::Memory(bundle) if bundle.contains_key(&key) => return Ok(true),
                Mount::Memory(_) => {}
                #[cfg(not(target_arch = "wasm32"))]
                Mount::Directory(root) => {
                    // Existence deliberately accepts directories too (callers
                    // probe paths like `.`), unlike `read` / `resolve`.
                    if resolve_in_mount(root, &relative, requested)?.is_some() {
                        return Ok(true);
                    }
                }
            }
        }
        drop(mounts);
        if self
            .preloaded
            .read()
            .expect("preloaded asset bundle poisoned")
            .contains_key(&key)
        {
            return Ok(true);
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
            let Some(resolved) = resolve_in_mount(root, &relative, requested)? else {
                continue;
            };
            // Only files can be handed out as native paths; a directory
            // falls through to the next mount.
            if resolved.is_file() {
                return Ok(Some(resolved));
            }
        }
        Ok(None)
    }
}

/// Resolve `relative` inside a canonicalized directory mount `root`.
///
/// Returns `Ok(None)` when the entry does not exist (caller moves on to the
/// next mount), the canonicalized path when it does, an I/O error for any
/// other filesystem failure, and `InvalidPath` when the resolved path (e.g.
/// via a symlink) escapes the mount.  Whether directories are acceptable is
/// each caller's decision.
#[cfg(not(target_arch = "wasm32"))]
fn resolve_in_mount(
    root: &Path,
    relative: &Path,
    requested: &Path,
) -> Result<Option<PathBuf>, AssetError> {
    let candidate = root.join(relative);
    let resolved = match std::fs::canonicalize(&candidate) {
        Ok(resolved) => resolved,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
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
    Ok(Some(resolved))
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

/// The runtime VFS used by legacy static call sites.
pub fn global() -> &'static Arc<AssetVfs> {
    GLOBAL.get_or_init(|| Arc::new(AssetVfs::new()))
}

/// Install the shipping bundle at highest priority.
pub fn install_bundle(bundle: Arc<Bundle>) -> Result<(), AssetError> {
    global().mount_bundle_first(bundle)
}

/// Install or replace one host-preloaded asset.
pub fn install_preloaded_asset<P: AsRef<Path>>(path: P, bytes: Vec<u8>) -> Result<(), AssetError> {
    global().install_preloaded_asset(path, bytes)
}

/// Read through the runtime mounts, then fall back to a direct host path on
/// native for call sites that have not yet been migrated to virtual paths.
pub fn read<P: AsRef<Path>>(path: P) -> Result<Vec<u8>, AssetError> {
    read_shared(path).map(AssetBytes::into_vec)
}

/// Read through the runtime mounts without copying memory-mounted bytes.
pub fn read_shared<P: AsRef<Path>>(path: P) -> Result<AssetBytes, AssetError> {
    let path = path.as_ref();
    match global().read_shared(path) {
        Ok(bytes) => return Ok(bytes),
        Err(AssetError::NotFound(_)) | Err(AssetError::InvalidPath(_)) => {}
        Err(error) => return Err(error),
    }
    imp::read(path).map(AssetBytes::from)
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
                .map(|(path, bytes)| ((*path).to_string(), AssetBytes::from(*bytes)))
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

        let shared_a = first.read_shared("shared.dat").unwrap();
        let shared_b = first.read_shared("shared.dat").unwrap();
        assert_eq!(shared_a.as_ptr(), shared_b.as_ptr());
    }

    #[test]
    fn replacing_active_bundle_releases_old_namespace_and_preserves_static_mounts() {
        let vfs = AssetVfs::new();
        vfs.mount_bundle(bundle(&[("boot.dat", b"boot"), ("shared.dat", b"boot")]))
            .unwrap();
        vfs.replace_active_bundle(bundle(&[("first.dat", b"first"), ("shared.dat", b"one")]))
            .unwrap();
        assert_eq!(vfs.read("first.dat").unwrap(), b"first");
        assert_eq!(vfs.read("shared.dat").unwrap(), b"one");

        vfs.replace_active_bundle(bundle(&[("second.dat", b"second")]))
            .unwrap();
        assert!(!vfs.try_exists("first.dat").unwrap());
        assert_eq!(vfs.read("second.dat").unwrap(), b"second");
        assert_eq!(vfs.read("boot.dat").unwrap(), b"boot");
        assert_eq!(vfs.read("shared.dat").unwrap(), b"boot");
    }

    #[test]
    fn preloaded_assets_are_instance_owned_replaceable_and_survive_mission_replacement() {
        let vfs = AssetVfs::new();
        vfs.install_preloaded_asset("Data/Interface/UI/panel.png", b"first".to_vec())
            .unwrap();
        assert_eq!(vfs.read("data/interface/ui/PANEL.PNG").unwrap(), b"first");

        vfs.install_preloaded_asset("Data/Interface/UI/panel.png", b"second".to_vec())
            .unwrap();
        vfs.replace_active_bundle(bundle(&[("mission.dat", b"mission")]))
            .unwrap();
        assert_eq!(vfs.read("Data/Interface/UI/panel.png").unwrap(), b"second");

        vfs.mount_bundle(bundle(&[("interface/ui/panel.png", b"mounted")]))
            .unwrap();
        assert_eq!(vfs.read("Data/Interface/UI/panel.png").unwrap(), b"mounted");
        assert!(
            !AssetVfs::new()
                .try_exists("Data/Interface/UI/panel.png")
                .unwrap()
        );
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
