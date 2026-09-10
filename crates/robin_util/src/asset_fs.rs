//! Ordered, read-only virtual filesystem for game assets.
//!
//! [`AssetVfs`] owns its mounts.  Fresh instances are completely isolated,
//! which makes converters and tests independent of the process-wide runtime
//! facade at the bottom of this module.  Mounts are searched in order and the
//! first file wins.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ops::Deref;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

/// A contained, case-folded logical name relative to the game's Data root.
/// Native host paths must continue to use the explicit filesystem adapters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AssetKey(String);

impl AssetKey {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, AssetError> {
        let path = normalize_virtual_path(path.as_ref())?;
        Ok(Self(bundle_key_from_normalized(&path)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for AssetKey {
    type Error = AssetError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<AssetKey> for String {
    fn from(value: AssetKey) -> Self {
        value.0
    }
}

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

/// Exhaustive read-only summary of every in-memory/directory authority an
/// [`AssetVfs`] can consult. The private projection exporter requires this to
/// be empty before it installs an authenticated shipping source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AssetVfsAuthoritySnapshot {
    pub has_locale_bundle: bool,
    pub overlay_bundle_count: usize,
    pub has_active_bundle: bool,
    pub mount_count: usize,
    pub preloaded_file_count: usize,
}

impl AssetVfsAuthoritySnapshot {
    pub const fn is_empty(self) -> bool {
        !self.has_locale_bundle
            && self.overlay_bundle_count == 0
            && !self.has_active_bundle
            && self.mount_count == 0
            && self.preloaded_file_count == 0
    }
}

#[derive(Debug, Clone)]
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
    /// A replaceable, highest-priority language overlay. Unlike ordinary
    /// mounts this slot is swapped atomically, so files missing from a newly
    /// selected pack can never leak through from a previously selected pack.
    selection: RwLock<AssetSelection>,
    /// The replaceable mission payload. Keeping this separate from the locale
    /// overlay lets either change without retaining assets from the previous
    /// mission or language.
    /// Engine-owned overlays searched before mission and shipping content.
    ///
    /// This is distinct from `mounts`: the core datadir must override a
    /// retail installation's incomplete font configuration, and it must keep
    /// that priority while `active_bundle` is replaced between missions.
    overlay_bundles: RwLock<Vec<Arc<Bundle>>>,
    mounts: RwLock<Vec<Mount>>,
    preloaded: RwLock<Bundle>,
}

/// One coherent publication of parsed identity and the corresponding raw
/// bundles. Retaining this snapshot pins the selected content generation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetSelection {
    pub generation: u64,
    pub mission_generation: u64,
    pub content_generation: u64,
    pub locale: Option<String>,
    pub mission: Option<String>,
    #[serde(skip)]
    pub locale_bundle: Option<Arc<Bundle>>,
    #[serde(skip)]
    pub active_bundle: Option<Arc<Bundle>>,
}

/// Provenance of the winning lookup layer, useful for diagnostics without
/// exposing or weakening the direct-host compatibility fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetSource {
    Locale,
    Overlay,
    Mission,
    MountedBundle,
    NativeFile(PathBuf),
    Preloaded,
}

#[derive(Debug)]
enum AssetLocation {
    Memory(AssetBytes, AssetSource),
    #[cfg(not(target_arch = "wasm32"))]
    Native(PathBuf),
}

impl AssetVfs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Capture independent lookup authority. Immutable byte bundles are shared;
    /// selection and mount locks are never shared with the source installation.
    /// Directory mounts pin their roots, not the contents of mutable disk files.
    pub fn snapshot(&self) -> Self {
        // Publishers hold one mount lock before selection. Use that same order,
        // and retain all mount locks until the selection has been captured.
        let overlays = self
            .overlay_bundles
            .read()
            .expect("asset overlays poisoned");
        let mounts = self.mounts.read().expect("asset mounts poisoned");
        let preloaded = self.preloaded.read().expect("preloaded assets poisoned");
        let selection = self.selection.read().expect("asset selection poisoned");
        Self {
            selection: RwLock::new(selection.clone()),
            overlay_bundles: RwLock::new(overlays.clone()),
            mounts: RwLock::new(mounts.clone()),
            preloaded: RwLock::new(preloaded.clone()),
        }
    }

    /// Notify parsed caches of a legacy filesystem authority change. Callers
    /// retain their own mount lock until this publication has completed.
    pub fn invalidate_content(&self, localized_only: bool) {
        let mut selection = self.selection.write().expect("asset selection poisoned");
        selection.generation = selection
            .generation
            .checked_add(1)
            .expect("asset generation exhausted");
        if !localized_only {
            selection.content_generation = selection.generation;
        }
    }

    /// Snapshot every lookup layer without exposing mounted content.
    pub fn authority_snapshot(&self) -> AssetVfsAuthoritySnapshot {
        // Do not retain the selection lock while inspecting mount locks:
        // publishers hold their mount lock while advancing the generation.
        let selection = self.selection_snapshot();
        AssetVfsAuthoritySnapshot {
            has_locale_bundle: selection.locale_bundle.is_some(),
            overlay_bundle_count: self
                .overlay_bundles
                .read()
                .expect("asset VFS overlay bundles poisoned")
                .len(),
            has_active_bundle: selection.active_bundle.is_some(),
            mount_count: self.mounts.read().expect("asset VFS mounts poisoned").len(),
            preloaded_file_count: self
                .preloaded
                .read()
                .expect("preloaded asset bundle poisoned")
                .len(),
        }
    }

    /// Append an immutable engine overlay.
    ///
    /// Overlay bundles are searched in installation order, before the active
    /// mission, shipping boot data, loose directories, and host-preloaded
    /// compatibility assets. This mirrors `SbFile`'s native overlay order.
    pub fn mount_overlay_bundle(&self, bundle: Arc<Bundle>) -> Result<(), AssetError> {
        validate_bundle(&bundle)?;
        let mut mounted = self
            .overlay_bundles
            .write()
            .expect("asset VFS overlay bundles poisoned");
        mounted.push(bundle);
        self.invalidate_content(false);
        Ok(())
    }

    /// Append an in-memory bundle as the lowest-priority mount.
    pub fn mount_bundle(&self, bundle: Arc<Bundle>) -> Result<(), AssetError> {
        validate_bundle(&bundle)?;
        let mut mounted = self.mounts.write().expect("asset VFS mounts poisoned");
        mounted.push(Mount::Memory(bundle));
        self.invalidate_content(false);
        Ok(())
    }

    /// Insert an in-memory bundle as the highest-priority regular mount.
    ///
    /// Runtime shipping data uses this so it retains the historical priority
    /// over host-preloaded and loose files regardless of bootstrap order.
    pub fn mount_bundle_first(&self, bundle: Arc<Bundle>) -> Result<(), AssetError> {
        validate_bundle(&bundle)?;
        let mut mounted = self.mounts.write().expect("asset VFS mounts poisoned");
        mounted.insert(0, Mount::Memory(bundle));
        self.invalidate_content(false);
        Ok(())
    }

    pub fn selection_snapshot(&self) -> AssetSelection {
        self.selection
            .read()
            .expect("asset selection poisoned")
            .clone()
    }

    /// Observe the cache-invalidation generation without cloning selection
    /// metadata or bundle handles. This does not pin the selected resources;
    /// consumers needing a coherent resource view must use `selection_snapshot`.
    pub fn selection_generation(&self) -> u64 {
        self.selection
            .read()
            .expect("asset selection poisoned")
            .generation
    }

    pub fn set_locale_bundle(&self, bundle: Option<Arc<Bundle>>) -> Result<(), AssetError> {
        self.select_locale(None, bundle)
    }

    /// Validate first, then publish identity and bytes under one lock.
    pub fn select_locale(
        &self,
        locale: Option<String>,
        bundle: Option<Arc<Bundle>>,
    ) -> Result<(), AssetError> {
        if let Some(bundle) = &bundle {
            validate_bundle(bundle)?;
        }
        let mut selection = self.selection.write().expect("asset selection poisoned");
        selection.generation = selection
            .generation
            .checked_add(1)
            .expect("asset generation exhausted");
        selection.locale = locale;
        selection.locale_bundle = bundle;
        Ok(())
    }

    pub fn replace_active_bundle(&self, bundle: Arc<Bundle>) -> Result<(), AssetError> {
        self.select_mission(None, bundle)
    }

    pub fn select_mission(
        &self,
        mission: Option<String>,
        bundle: Arc<Bundle>,
    ) -> Result<(), AssetError> {
        validate_bundle(&bundle)?;
        let mut selection = self.selection.write().expect("asset selection poisoned");
        selection.generation = selection
            .generation
            .checked_add(1)
            .expect("asset generation exhausted");
        selection.mission = mission;
        selection.mission_generation = selection.generation;
        selection.active_bundle = Some(bundle);
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
        let mut mounted = self
            .preloaded
            .write()
            .expect("preloaded asset bundle poisoned");
        mounted.insert(bundle_key_from_normalized(&normalized), bytes.into());
        self.invalidate_content(false);
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
        let mut mounted = self.mounts.write().expect("asset VFS mounts poisoned");
        mounted.push(Mount::Directory(root));
        self.invalidate_content(false);
        Ok(())
    }

    pub fn read_shared(&self, path: impl AsRef<Path>) -> Result<AssetBytes, AssetError> {
        self.read_shared_with_source(path).map(|(bytes, _)| bytes)
    }

    pub fn read_shared_with_source(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<(AssetBytes, AssetSource), AssetError> {
        let requested = path.as_ref();
        match self.locate(requested, true)? {
            Some(AssetLocation::Memory(bytes, source)) => Ok((bytes, source)),
            #[cfg(not(target_arch = "wasm32"))]
            Some(AssetLocation::Native(path)) => std::fs::read(&path)
                .map(|bytes| {
                    (
                        AssetBytes::from(bytes),
                        AssetSource::NativeFile(path.clone()),
                    )
                })
                .map_err(|source| AssetError::Io { path, source }),
            None => Err(AssetError::NotFound(requested.display().to_string())),
        }
    }

    fn locate(
        &self,
        path: impl AsRef<Path>,
        // Memory mounts contain only files; only native mounts need this distinction.
        _require_file: bool,
    ) -> Result<Option<AssetLocation>, AssetError> {
        let requested = path.as_ref();
        let relative = normalize_virtual_path(requested)?;
        let key = bundle_key_from_normalized(&relative);
        let selection = self.selection_snapshot();
        if let Some(locale) = selection.locale_bundle.as_ref()
            && is_locale_overlay_key(&key)
        {
            if let Some(bytes) = locale.get(&key) {
                return Ok(Some(AssetLocation::Memory(
                    bytes.clone(),
                    AssetSource::Locale,
                )));
            }
            if is_required_locale_key(&key) {
                return Ok(None);
            }
        }
        let overlays = self
            .overlay_bundles
            .read()
            .expect("asset VFS overlay bundles poisoned");
        for bundle in overlays.iter() {
            if let Some(bytes) = bundle.get(&key) {
                return Ok(Some(AssetLocation::Memory(
                    bytes.clone(),
                    AssetSource::Overlay,
                )));
            }
        }
        drop(overlays);
        if let Some(bytes) = selection
            .active_bundle
            .as_ref()
            .and_then(|bundle| bundle.get(&key))
        {
            return Ok(Some(AssetLocation::Memory(
                bytes.clone(),
                AssetSource::Mission,
            )));
        }
        let mounts = self.mounts.read().expect("asset VFS mounts poisoned");
        for mount in mounts.iter() {
            match mount {
                Mount::Memory(bundle) => {
                    if let Some(bytes) = bundle.get(&key) {
                        return Ok(Some(AssetLocation::Memory(
                            bytes.clone(),
                            AssetSource::MountedBundle,
                        )));
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                Mount::Directory(root) => {
                    let Some(resolved) = resolve_in_mount(root, &relative, requested)? else {
                        continue;
                    };
                    // Reading requires a file; a directory falls through to
                    // the next mount.
                    if _require_file && !resolved.is_file() {
                        continue;
                    }
                    return Ok(Some(AssetLocation::Native(resolved)));
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
            return Ok(Some(AssetLocation::Memory(
                bytes.clone(),
                AssetSource::Preloaded,
            )));
        }
        Ok(None)
    }

    /// Read an owned buffer for compatibility with consumers that mutate or
    /// retain the bytes independently. Stream readers should prefer
    /// [`Self::read_shared`] to avoid copying memory-mounted assets.
    pub fn read(&self, path: impl AsRef<Path>) -> Result<Vec<u8>, AssetError> {
        self.read_shared(path).map(AssetBytes::into_vec)
    }

    /// Check for an asset while preserving invalid-path and I/O errors.
    pub fn try_exists(&self, path: impl AsRef<Path>) -> Result<bool, AssetError> {
        // Existence includes directories, while reads fall through to later
        // mounts when a layer contains a directory at the requested name.
        self.locate(path, false).map(|location| location.is_some())
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
    if path
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data/"))
    {
        &path[5..]
    } else {
        path
    }
}

// ---------------------------------------------------------------------------
// Process runtime compatibility facade
// ---------------------------------------------------------------------------

static GLOBAL: OnceLock<Arc<AssetVfs>> = OnceLock::new();

/// The runtime VFS used by legacy static call sites.
pub fn global() -> &'static Arc<AssetVfs> {
    GLOBAL.get_or_init(|| Arc::new(AssetVfs::new()))
}

/// Install the shipping bundle at the highest regular-mount priority.
pub fn install_bundle(bundle: Arc<Bundle>) -> Result<(), AssetError> {
    global().mount_bundle_first(bundle)
}

/// Replace the process runtime's current language overlay.
pub fn install_locale_bundle(bundle: Option<Arc<Bundle>>) -> Result<(), AssetError> {
    global().set_locale_bundle(bundle)
}

pub fn is_required_locale_key(key: &str) -> bool {
    key == "text" || key.starts_with("text/") || key.eq_ignore_ascii_case("interface/start.sxt")
}

pub fn is_optional_english_fallback_key(key: &str) -> bool {
    key == "sounds/exclamations"
        || key.starts_with("sounds/exclamations/")
        || key == "cinematics"
        || key.starts_with("cinematics/")
}

/// Locale classification consumes canonical keys; it does not grant access.
pub fn is_locale_overlay_key(key: &str) -> bool {
    key == "text"
        || key.starts_with("text/")
        || key == "interface"
        || key.starts_with("interface/")
        || is_optional_english_fallback_key(key)
}

/// Install an engine-owned overlay ahead of mission and shipping assets.
pub fn install_overlay_bundle(bundle: Arc<Bundle>) -> Result<(), AssetError> {
    global().mount_overlay_bundle(bundle)
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
    #[test]
    fn resource_snapshot_pins_selection_mounts_and_preloaded_authority() {
        let source = super::AssetVfs::new();
        source.mount_bundle(bundle(&[("boot", &[1])])).unwrap();
        source.install_preloaded_asset("staged", vec![2]).unwrap();
        source
            .select_mission(Some("first".into()), bundle(&[("mission", &[3])]))
            .unwrap();
        let captured = source.snapshot();
        source
            .mount_bundle_first(bundle(&[("boot", &[9])]))
            .unwrap();
        source.install_preloaded_asset("staged", vec![9]).unwrap();
        source
            .select_mission(Some("second".into()), bundle(&[("mission", &[9])]))
            .unwrap();
        assert_eq!(captured.read("boot").unwrap(), [1]);
        assert_eq!(captured.read("staged").unwrap(), [2]);
        assert_eq!(captured.read("mission").unwrap(), [3]);
        assert_eq!(
            captured.selection_snapshot().mission.as_deref(),
            Some("first")
        );
    }
    use super::*;

    #[test]
    fn logical_keys_validate_and_share_locale_policy() {
        for path in [
            "Data/Text/LEVEL.res",
            "./dAtA/text/level.res",
            "DATA\\Text\\Level.res",
        ] {
            let key = AssetKey::new(path).unwrap();
            assert_eq!(key.as_str(), "text/level.res");
            assert!(is_required_locale_key(key.as_str()));
            assert!(is_locale_overlay_key(key.as_str()));
            assert!(!is_optional_english_fallback_key(key.as_str()));
        }
        for path in ["../secret", "/absolute", "Data/../secret"] {
            assert!(AssetKey::new(path).is_err());
            assert!(
                serde_json::from_str::<AssetKey>(&serde_json::to_string(path).unwrap()).is_err()
            );
        }
        for (key, overlay, required, english) in [
            ("text", true, true, false),
            ("interface/start.sxt", true, true, false),
            ("interface/panel", true, false, false),
            ("cinematics/intro", true, false, true),
            ("sounds/exclamations/voice", true, false, true),
            ("configuration/profiles.cpf", false, false, false),
        ] {
            assert_eq!(is_locale_overlay_key(key), overlay);
            assert_eq!(is_required_locale_key(key), required);
            assert_eq!(is_optional_english_fallback_key(key), english);
        }
    }

    #[test]
    fn generation_observation_tracks_changes_without_updating_frozen_views() {
        let vfs = AssetVfs::new();
        assert_eq!(vfs.selection_generation(), 0);
        vfs.select_locale(Some("one".into()), Some(bundle(&[("text/value", b"one")])))
            .unwrap();
        let frozen = vfs.snapshot();
        let original = frozen.selection_generation();
        for localized_only in [false, true] {
            vfs.invalidate_content(localized_only);
            assert_eq!(
                vfs.selection_generation(),
                vfs.selection_snapshot().generation
            );
            assert!(vfs.selection_generation() > original);
            assert_eq!(frozen.selection_generation(), original);
        }
    }

    #[test]
    fn concurrent_selection_snapshots_pin_complete_generations() {
        let vfs = Arc::new(AssetVfs::new());
        vfs.select_locale(Some("one".into()), Some(bundle(&[("text/value", b"one")])))
            .unwrap();
        let initial = vfs.selection_snapshot();
        let writer = vfs.clone();
        let handle = std::thread::spawn(move || {
            for _ in 0..100 {
                for name in ["two", "one"] {
                    writer
                        .select_locale(
                            Some(name.into()),
                            Some(bundle(&[("text/value", name.as_bytes())])),
                        )
                        .unwrap();
                }
            }
        });
        for _ in 0..1000 {
            let selected = vfs.selection_snapshot();
            let name = selected.locale.as_ref().unwrap();
            assert_eq!(
                selected.locale_bundle.as_ref().unwrap()["text/value"].as_ref(),
                name.as_bytes()
            );
        }
        handle.join().unwrap();
        assert_eq!(initial.generation, 1);
        assert_eq!(
            initial.locale_bundle.as_ref().unwrap()["text/value"].as_ref(),
            b"one"
        );
        let current = vfs.selection_snapshot();
        assert!(
            vfs.select_locale(Some("bad".into()), Some(bundle(&[("../escape", b"bad")])))
                .is_err()
        );
        assert_eq!(vfs.selection_snapshot().generation, current.generation);
    }

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
        assert_eq!(
            vfs.read_shared_with_source("shared.dat").unwrap().1,
            AssetSource::MountedBundle
        );
        assert_eq!(
            vfs.read_shared_with_source("second.dat").unwrap().1,
            AssetSource::Mission
        );
    }

    #[test]
    fn locale_then_engine_overlay_precedence_survives_mission_replacement() {
        let vfs = AssetVfs::new();
        vfs.mount_bundle(bundle(&[("interface/shared.dat", b"shipping")]))
            .unwrap();
        vfs.replace_active_bundle(bundle(&[("interface/shared.dat", b"mission-one")]))
            .unwrap();
        vfs.mount_overlay_bundle(bundle(&[("interface/shared.dat", b"core")]))
            .unwrap();

        assert_eq!(vfs.read("interface/shared.dat").unwrap(), b"core");
        vfs.set_locale_bundle(Some(bundle(&[("interface/shared.dat", b"localized")])))
            .unwrap();
        assert_eq!(vfs.read("interface/shared.dat").unwrap(), b"localized");

        vfs.replace_active_bundle(bundle(&[("interface/shared.dat", b"mission-two")]))
            .unwrap();
        assert_eq!(vfs.read("interface/shared.dat").unwrap(), b"localized");
        vfs.set_locale_bundle(None).unwrap();
        assert_eq!(vfs.read("interface/shared.dat").unwrap(), b"core");
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
    fn locale_bundle_replacement_does_not_leak_the_previous_language() {
        let vfs = AssetVfs::new();
        vfs.mount_bundle(bundle(&[
            ("interface/shared.dat", b"base"),
            ("interface/old-only.dat", b"base-old"),
        ]))
        .unwrap();
        vfs.set_locale_bundle(Some(bundle(&[
            ("interface/shared.dat", b"german"),
            ("interface/old-only.dat", b"german-only"),
        ])))
        .unwrap();
        assert_eq!(
            vfs.read("Data/Interface/old-only.dat").unwrap(),
            b"german-only"
        );

        vfs.set_locale_bundle(Some(bundle(&[("interface/shared.dat", b"french")])))
            .unwrap();
        assert_eq!(vfs.read("Data/Interface/shared.dat").unwrap(), b"french");
        assert_eq!(
            vfs.read("Data/Interface/old-only.dat").unwrap(),
            b"base-old"
        );

        vfs.set_locale_bundle(None).unwrap();
        assert_eq!(vfs.read("Data/Interface/shared.dat").unwrap(), b"base");
    }

    #[test]
    fn required_locale_text_never_falls_through_to_base_mounts() {
        let vfs = AssetVfs::new();
        vfs.mount_bundle(bundle(&[("text/level.res", b"base")]))
            .unwrap();
        vfs.set_locale_bundle(Some(bundle(&[("text/other.res", b"selected")])))
            .unwrap();

        assert!(matches!(
            vfs.read("Data/Text/Level.res"),
            Err(AssetError::NotFound(_))
        ));
        assert!(!vfs.try_exists("Data/Text/Level.res").unwrap());
        assert_eq!(vfs.read("Data/Text/Other.res").unwrap(), b"selected");
    }

    #[test]
    fn locale_bundle_cannot_override_simulation_inputs() {
        let vfs = AssetVfs::new();
        vfs.mount_bundle(bundle(&[("configuration/profile.cpf", b"base")]))
            .unwrap();
        vfs.set_locale_bundle(Some(bundle(&[("configuration/profile.cpf", b"localized")])))
            .unwrap();

        assert_eq!(vfs.read("Data/Configuration/profile.cpf").unwrap(), b"base");
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
